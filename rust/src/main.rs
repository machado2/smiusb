mod frame;
mod jpeg;
mod protocol;
mod screencast;
mod usb;

use frame::{Frame, FrameId, FrameLayout, FramePipeline, SubmitOutcome};
use jpeg::Compressor;
use std::collections::BTreeMap;
use std::env;
use std::ffi::c_int;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use usb::{DeviceInfo, DeviceKey, UsbContext};

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

unsafe extern "C" {
    fn signal(signal_number: c_int, handler: extern "C" fn(c_int)) -> usize;
}

extern "C" fn request_stop(_signal_number: c_int) {
    STOP_REQUESTED.store(true, Ordering::Relaxed);
}

fn install_signal_handlers() {
    const SIGINT: c_int = 2;
    const SIGTERM: c_int = 15;
    // SAFETY: the handler only performs a lock-free atomic store, which is
    // async-signal-safe, and has the required C calling convention.
    unsafe {
        signal(SIGINT, request_stop);
        signal(SIGTERM, request_stop);
    }
}

#[derive(Debug, Eq, PartialEq)]
enum Command {
    Observe {
        duration: Option<Duration>,
    },
    DecodeHex(Vec<u8>),
    ProtocolInfo,
    Pattern {
        options: PatternOptions,
        encode: bool,
    },
    ScreenCastVersion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PatternOptions {
    width: u32,
    height: u32,
    frames: u64,
    queue_capacity: usize,
    quality: u8,
}

const DEFAULT_PATTERN_FRAMES: u64 = 6;
const DEFAULT_QUEUE_CAPACITY: usize = 2;
const DEFAULT_JPEG_QUALITY: u8 = 80;
const MAX_PATTERN_FRAMES: u64 = 1_000;
const MAX_QUEUE_CAPACITY: usize = 256;
const MAX_PATTERN_WORK_BYTES: usize = 512 * 1024 * 1024;

fn parse_hex(input: &str) -> Result<Vec<u8>, String> {
    let mut nibbles = Vec::new();
    for character in input.chars() {
        if character.is_ascii_whitespace() || character == ':' {
            continue;
        }
        if !character.is_ascii_hexdigit() {
            return Err(format!("invalid hex digit: {character:?}"));
        }
        nibbles.push(character.to_digit(16).expect("validated ASCII hex digit") as u8);
    }
    if !nibbles.len().is_multiple_of(2) {
        return Err("hex input must contain an even number of digits".to_owned());
    }

    Ok(nibbles
        .chunks_exact(2)
        .map(|pair| (pair[0] << 4) | pair[1])
        .collect())
}

fn parse_args(arguments: &[String]) -> Result<Command, String> {
    if arguments
        .first()
        .is_some_and(|mode| mode == "--pattern" || mode == "--encode-plan")
    {
        let encode = arguments[0] == "--encode-plan";
        return parse_pattern_args(&arguments[1..])
            .map(|options| Command::Pattern { options, encode });
    }

    match arguments {
        [mode] if mode == "--observe" => Ok(Command::Observe { duration: None }),
        [mode, duration_flag, seconds] if mode == "--observe" && duration_flag == "--duration" => {
            let seconds = seconds
                .parse::<u64>()
                .map_err(|_| "duration must be a positive integer".to_owned())?;
            if seconds == 0 {
                return Err("duration must be a positive integer".to_owned());
            }
            Ok(Command::Observe {
                duration: Some(Duration::from_secs(seconds)),
            })
        }
        [mode, input] if mode == "--decode-hex" => Ok(Command::DecodeHex(parse_hex(input)?)),
        [mode] if mode == "--protocol-info" => Ok(Command::ProtocolInfo),
        [mode] if mode == "--screen-cast-version" => Ok(Command::ScreenCastVersion),
        _ => Err(
            "usage: smiusbd-rs --observe [--duration SECONDS]\n       smiusbd-rs --decode-hex HEX\n       smiusbd-rs --protocol-info\n       smiusbd-rs --pattern WIDTHxHEIGHT [--frames COUNT] [--queue CAPACITY]\n       smiusbd-rs --encode-plan WIDTHxHEIGHT [--frames COUNT] [--queue CAPACITY] [--quality 1..100]\n       smiusbd-rs --screen-cast-version"
                .to_owned(),
        ),
    }
}

fn parse_pattern_args(arguments: &[String]) -> Result<PatternOptions, String> {
    let (dimensions, options) = arguments
        .split_first()
        .ok_or_else(|| "--pattern requires WIDTHxHEIGHT".to_owned())?;
    let separator = dimensions
        .find(['x', 'X'])
        .ok_or_else(|| "pattern dimensions must use WIDTHxHEIGHT".to_owned())?;
    let (width, height_with_separator) = dimensions.split_at(separator);
    let height = &height_with_separator[1..];
    if width.is_empty() || height.is_empty() || height.contains(['x', 'X']) {
        return Err("pattern dimensions must use WIDTHxHEIGHT".to_owned());
    }
    let width = width
        .parse::<u32>()
        .map_err(|_| "pattern width must be a positive integer".to_owned())?;
    let height = height
        .parse::<u32>()
        .map_err(|_| "pattern height must be a positive integer".to_owned())?;

    let mut frames = DEFAULT_PATTERN_FRAMES;
    let mut queue_capacity = DEFAULT_QUEUE_CAPACITY;
    let mut quality = DEFAULT_JPEG_QUALITY;
    let mut saw_frames = false;
    let mut saw_queue = false;
    let mut saw_quality = false;
    let mut options = options.iter();
    while let Some(option) = options.next() {
        let value = options
            .next()
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option.as_str() {
            "--frames" if !saw_frames => {
                frames = value
                    .parse::<u64>()
                    .map_err(|_| "frame count must be a positive integer".to_owned())?;
                saw_frames = true;
            }
            "--queue" if !saw_queue => {
                queue_capacity = value
                    .parse::<usize>()
                    .map_err(|_| "queue capacity must be a positive integer".to_owned())?;
                saw_queue = true;
            }
            "--quality" if !saw_quality => {
                quality = value
                    .parse::<u8>()
                    .map_err(|_| "JPEG quality must be an integer from 1 to 100".to_owned())?;
                saw_quality = true;
            }
            "--frames" | "--queue" | "--quality" => {
                return Err(format!("duplicate option: {option}"));
            }
            _ => return Err(format!("unknown pattern option: {option}")),
        }
    }
    if frames == 0 || frames > MAX_PATTERN_FRAMES {
        return Err(format!(
            "frame count must be between 1 and {MAX_PATTERN_FRAMES}"
        ));
    }
    if queue_capacity == 0 || queue_capacity > MAX_QUEUE_CAPACITY {
        return Err(format!(
            "queue capacity must be between 1 and {MAX_QUEUE_CAPACITY}"
        ));
    }
    if !(1..=100).contains(&quality) {
        return Err("JPEG quality must be between 1 and 100".to_owned());
    }

    Ok(PatternOptions {
        width,
        height,
        frames,
        queue_capacity,
        quality,
    })
}

fn observe(duration: Option<Duration>) -> Result<(), String> {
    let context = UsbContext::new()?;
    install_signal_handlers();
    let deadline = duration
        .map(|value| {
            Instant::now()
                .checked_add(value)
                .ok_or_else(|| "duration is too large for this platform".to_owned())
        })
        .transpose()?;
    let mut previous = BTreeMap::<DeviceKey, DeviceInfo>::new();
    let mut generation = 0_u64;

    while !STOP_REQUESTED.load(Ordering::Relaxed)
        && deadline.is_none_or(|value| Instant::now() < value)
    {
        let current = context.target_devices()?;
        for (key, info) in &current {
            if !previous.contains_key(key) {
                generation += 1;
                eprintln!(
                    "smiusbd-rs: generation={generation} arrived bus={} address={} port={} usb={} device={} speed={:?} (passive; no open, claim, or transfer)",
                    key.bus,
                    key.address,
                    info.port,
                    usb::format_bcd(info.usb_version),
                    usb::format_bcd(info.device_version),
                    info.speed
                );
            }
        }
        for key in previous.keys() {
            if !current.contains_key(key) {
                eprintln!(
                    "smiusbd-rs: departed bus={} address={} (passive observer)",
                    key.bus, key.address
                );
            }
        }
        previous = current;
        thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}

fn decode(packet: &[u8]) -> Result<(), String> {
    let header = protocol::parse_header(packet).map_err(|error| error.to_string())?;
    println!(
        "bytes={} word12={:#010x} word16={:#010x} byte20={:#04x}",
        packet.len(),
        header.word12,
        header.word16,
        header.byte20
    );
    if let Ok(request) = protocol::parse_command_request(packet) {
        println!(
            "request class={:#x} opcode={:#04x} length={}",
            protocol::COMMAND_CLASS,
            request.opcode,
            request.length
        );
        return Ok(());
    }
    let modes = protocol::parse_observed_modes(packet).map_err(|error| error.to_string())?;
    for (index, mode) in modes.iter().enumerate() {
        println!(
            "mode[{index}]={}x{}@{} {}bpp",
            mode.width, mode.height, mode.refresh_hz, mode.bits_per_pixel
        );
    }
    Ok(())
}

fn print_protocol_info() {
    println!(
        "display-interface={} alt={} interrupt-in={:#04x} interrupt-out={:#04x} bulk-out={:#04x}",
        protocol::DISPLAY_INTERFACE,
        protocol::DISPLAY_ALT_SETTING,
        protocol::INTERRUPT_IN_ENDPOINT,
        protocol::INTERRUPT_OUT_ENDPOINT,
        protocol::BULK_OUT_ENDPOINT
    );
    for (name, packet) in [
        (
            "capabilities",
            protocol::build_capabilities_request().to_vec(),
        ),
        (
            "bulk-heartbeat",
            protocol::build_bulk_heartbeat_request(0)
                .expect("single-digit client tag fits")
                .to_vec(),
        ),
    ] {
        let hex: String = packet.iter().map(|byte| format!("{byte:02x}")).collect();
        println!("{name}={hex}");
    }
    println!("transmission=disabled");
}

fn simulate_pattern(options: PatternOptions, encode: bool) -> Result<(), String> {
    let layout = FrameLayout::new(options.width, options.height)
        .map_err(|error| format!("invalid pattern layout: {error}"))?;
    let total_work = layout
        .byte_len()
        .checked_mul(options.frames as usize)
        .ok_or_else(|| "pattern workload size overflow".to_owned())?;
    if total_work > MAX_PATTERN_WORK_BYTES {
        return Err(format!(
            "pattern would generate {total_work} bytes; reduce dimensions or frame count below the {} byte safety limit",
            MAX_PATTERN_WORK_BYTES
        ));
    }
    let resident_frames = options.queue_capacity.min(options.frames as usize);
    let resident_bytes = layout
        .byte_len()
        .checked_mul(resident_frames)
        .ok_or_else(|| "pattern queue size overflow".to_owned())?;
    if resident_bytes > MAX_PATTERN_WORK_BYTES {
        return Err(format!(
            "pattern queue could retain {resident_bytes} bytes; reduce dimensions or queue capacity"
        ));
    }

    let generation = 1_u64;
    let pipeline =
        FramePipeline::new(options.queue_capacity, 0).map_err(|error| error.to_string())?;
    pipeline
        .begin_generation(generation)
        .map_err(|error| error.to_string())?;
    println!(
        "pipeline format=BGRx width={} height={} stride={} frame-bytes={} queue-capacity={} generation={generation}",
        layout.width(),
        layout.height(),
        layout.stride(),
        layout.byte_len(),
        options.queue_capacity
    );
    println!(
        "encode-plan=JPEG-4:2:2-after-BGRx quality={} execute={} transport=disabled",
        options.quality, encode
    );

    for sequence in 0..options.frames {
        let frame = Frame::test_pattern(
            FrameId {
                generation,
                sequence,
            },
            layout,
        )
        .map_err(|error| format!("cannot generate test pattern: {error}"))?;
        let checksum = frame.checksum();
        match pipeline.submit(frame) {
            SubmitOutcome::Queued => {
                println!("capture id={generation}:{sequence} checksum={checksum:016x} queued")
            }
            SubmitOutcome::DroppedOldest(dropped) => println!(
                "capture id={generation}:{sequence} checksum={checksum:016x} queued drop-old={dropped}"
            ),
            unexpected => {
                return Err(format!(
                    "offline pipeline rejected generated frame: {unexpected:?}"
                ));
            }
        }
    }

    let mut compressor = encode
        .then(Compressor::new)
        .transpose()
        .map_err(|error| format!("cannot initialize offline JPEG encoder: {error}"))?;
    while let Some(frame) = pipeline.try_take() {
        if let Some(compressor) = &mut compressor {
            let jpeg = compressor
                .compress_bgrx(&frame, options.quality)
                .map_err(|error| format!("cannot encode frame {}: {error}", frame.id()))?;
            pipeline
                .with_current_generation(jpeg.id(), || ())
                .map_err(|error| format!("dropping encoded frame before sink: {error}"))?;
            println!(
                "encoded id={} raw-checksum={:016x} jpeg-bytes={} jpeg-fnv1a64={:016x} jpeg={}x{} quality={} subsampling=4:2:2",
                jpeg.id(),
                frame.checksum(),
                jpeg.bytes().len(),
                jpeg.checksum(),
                jpeg.width(),
                jpeg.height(),
                jpeg.quality()
            );
        } else {
            println!(
                "encoder-input id={} checksum={:016x} bytes={}",
                frame.id(),
                frame.checksum(),
                frame.layout().byte_len()
            );
        }
    }
    let snapshot = pipeline.snapshot();
    println!(
        "summary generation={} capacity={} accepted={} dequeued={} dropped-capacity={} queued={} usb-transmission=disabled",
        snapshot.generation,
        snapshot.capacity,
        snapshot.stats.accepted,
        snapshot.stats.dequeued,
        snapshot.stats.dropped_capacity,
        snapshot.queued
    );
    Ok(())
}

fn print_screen_cast_version() -> Result<(), String> {
    let info = screencast::discover()?;
    println!("screen-cast-version={}", info.version);
    println!(
        "required-minimum={} compatible={}",
        screencast::REQUIRED_API_VERSION,
        info.compatible
    );
    Ok(())
}

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    let result = match parse_args(&arguments) {
        Ok(Command::Observe { duration }) => observe(duration),
        Ok(Command::DecodeHex(packet)) => decode(&packet),
        Ok(Command::ProtocolInfo) => {
            print_protocol_info();
            Ok(())
        }
        Ok(Command::Pattern { options, encode }) => simulate_pattern(options, encode),
        Ok(Command::ScreenCastVersion) => print_screen_cast_version(),
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("smiusbd-rs: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_observe_duration() {
        assert_eq!(
            parse_args(&["--observe".into(), "--duration".into(), "5".into()]).unwrap(),
            Command::Observe {
                duration: Some(Duration::from_secs(5))
            }
        );
    }

    #[test]
    fn hex_parser_accepts_capture_friendly_separators() {
        assert_eq!(parse_hex("73:6d 69\n66").unwrap(), b"smif");
    }

    #[test]
    fn rejects_zero_duration_and_odd_hex() {
        assert!(parse_args(&["--observe".into(), "--duration".into(), "0".into()]).is_err());
        assert!(parse_hex("abc").is_err());
    }

    #[test]
    fn hex_parser_rejects_non_ascii_without_panicking() {
        assert!(parse_hex("aéx").is_err());
    }

    #[test]
    fn parses_pattern_options_and_alias() {
        let expected_options = PatternOptions {
            width: 1600,
            height: 900,
            frames: 3,
            queue_capacity: 1,
            quality: 91,
        };
        assert_eq!(
            parse_args(&[
                "--pattern".into(),
                "1600x900".into(),
                "--queue".into(),
                "1".into(),
                "--frames".into(),
                "3".into(),
                "--quality".into(),
                "91".into(),
            ])
            .unwrap(),
            Command::Pattern {
                options: expected_options,
                encode: false,
            }
        );
        assert_eq!(
            parse_args(&[
                "--encode-plan".into(),
                "1600X900".into(),
                "--frames".into(),
                "3".into(),
                "--queue".into(),
                "1".into(),
                "--quality".into(),
                "91".into(),
            ])
            .unwrap(),
            Command::Pattern {
                options: expected_options,
                encode: true,
            }
        );
    }

    #[test]
    fn rejects_unsafe_pattern_options() {
        assert!(parse_args(&["--pattern".into(), "0x900".into()]).is_ok());
        assert!(
            simulate_pattern(
                PatternOptions {
                    width: 0,
                    height: 900,
                    frames: 1,
                    queue_capacity: 1,
                    quality: 80,
                },
                false
            )
            .is_err()
        );
        assert!(
            parse_args(&[
                "--pattern".into(),
                "800x600".into(),
                "--frames".into(),
                "0".into(),
            ])
            .is_err()
        );
        assert!(
            parse_args(&[
                "--pattern".into(),
                "800x600".into(),
                "--queue".into(),
                "999".into(),
            ])
            .is_err()
        );
        assert!(
            parse_args(&[
                "--encode-plan".into(),
                "800x600".into(),
                "--quality".into(),
                "0".into(),
            ])
            .is_err()
        );
    }
}
