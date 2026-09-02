use crate::frame::{Frame, FrameId, FrameLayout};
use std::ffi::{CStr, c_char, c_int, c_uchar, c_ulong, c_void};
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

const TJPF_BGRX: c_int = 3;
const TJSAMP_422: c_int = 1;
const TJFLAG_NOREALLOC: c_int = 1024;
const MAX_JPEG_BUFFER_BYTES: usize = 512 * 1024 * 1024;

#[link(name = "turbojpeg")]
unsafe extern "C" {
    fn tjInitCompress() -> *mut c_void;
    fn tjCompress2(
        handle: *mut c_void,
        source: *const c_uchar,
        width: c_int,
        pitch: c_int,
        height: c_int,
        pixel_format: c_int,
        jpeg_buffer: *mut *mut c_uchar,
        jpeg_size: *mut c_ulong,
        jpeg_subsampling: c_int,
        jpeg_quality: c_int,
        flags: c_int,
    ) -> c_int;
    fn tjBufSize(width: c_int, height: c_int, jpeg_subsampling: c_int) -> c_ulong;
    fn tjDestroy(handle: *mut c_void) -> c_int;
    fn tjGetErrorStr2(handle: *mut c_void) -> *mut c_char;
}

pub struct Compressor {
    raw: NonNull<c_void>,
    scratch: Vec<u8>,
    // A TurboJPEG handle is owned and used by exactly one encoder worker.
    // Rc is only a zero-sized marker that intentionally makes this !Send/!Sync.
    _thread_confined: PhantomData<Rc<()>>,
}

impl Compressor {
    pub fn new() -> Result<Self, String> {
        // SAFETY: this has no preconditions and returns either an owned handle
        // or NULL. The owned handle is destroyed exactly once by Drop.
        let raw = unsafe { tjInitCompress() };
        let raw = NonNull::new(raw).ok_or_else(|| error_string(None))?;
        Ok(Self {
            raw,
            scratch: Vec::new(),
            _thread_confined: PhantomData,
        })
    }

    pub fn compress_bgrx(&mut self, frame: &Frame, quality: u8) -> Result<EncodedFrame, String> {
        if !(1..=100).contains(&quality) {
            return Err(format!(
                "JPEG quality must be between 1 and 100, got {quality}"
            ));
        }

        frame
            .validate()
            .map_err(|error| format!("refusing invalid frame at JPEG boundary: {error}"))?;
        let layout = frame.layout();
        let canonical = FrameLayout::new(layout.width(), layout.height())
            .map_err(|error| format!("refusing invalid JPEG layout: {error}"))?;
        if canonical != layout
            || canonical.stride() != layout.stride()
            || canonical.byte_len() != frame.pixels().len()
        {
            return Err("refusing inconsistent BGRx layout at JPEG boundary".to_owned());
        }
        let width = c_int::try_from(layout.width())
            .map_err(|_| "frame width does not fit TurboJPEG ABI".to_owned())?;
        let height = c_int::try_from(layout.height())
            .map_err(|_| "frame height does not fit TurboJPEG ABI".to_owned())?;
        let pitch = c_int::try_from(layout.stride())
            .map_err(|_| "frame stride does not fit TurboJPEG ABI".to_owned())?;

        // SAFETY: dimensions are positive c_int values and the subsampling
        // value is the ABI constant verified against turbojpeg.h.
        let maximum = unsafe { tjBufSize(width, height, TJSAMP_422) };
        if maximum == c_ulong::MAX {
            return Err(format!(
                "TurboJPEG rejected output dimensions: {}",
                error_string(None)
            ));
        }
        let maximum = usize::try_from(maximum)
            .map_err(|_| "TurboJPEG buffer size does not fit this platform".to_owned())?;
        if maximum == 0 || maximum > MAX_JPEG_BUFFER_BYTES {
            return Err(format!(
                "TurboJPEG requested a {maximum} byte destination; safety limit is {MAX_JPEG_BUFFER_BYTES}"
            ));
        }

        // TJFLAG_NOREALLOC explicitly permits caller-managed memory. Keeping
        // the destination in a Vec avoids crossing allocators and gives normal
        // Rust RAII even on every error path.
        if self.scratch.capacity() < maximum {
            let additional = maximum - self.scratch.len();
            self.scratch
                .try_reserve_exact(additional)
                .map_err(|error| format!("cannot allocate bounded JPEG buffer: {error}"))?;
        }
        self.scratch.resize(maximum, 0);
        let original_pointer = self.scratch.as_mut_ptr();
        let mut jpeg_pointer = original_pointer;
        let mut jpeg_size = c_ulong::try_from(maximum)
            .map_err(|_| "JPEG capacity does not fit TurboJPEG ABI".to_owned())?;

        // SAFETY: the immutable source covers exactly layout.byte_len bytes;
        // width/pitch/height describe that buffer. `self.scratch` is a writable
        // `maximum`-byte destination and NOREALLOC keeps its pointer owned by
        // Rust. `&mut self` excludes simultaneous use of this handle.
        let result = unsafe {
            tjCompress2(
                self.raw.as_ptr(),
                frame.pixels().as_ptr(),
                width,
                pitch,
                height,
                TJPF_BGRX,
                &mut jpeg_pointer,
                &mut jpeg_size,
                TJSAMP_422,
                c_int::from(quality),
                TJFLAG_NOREALLOC,
            )
        };
        if result != 0 {
            return Err(format!(
                "TurboJPEG compression failed: {}",
                error_string(Some(self.raw))
            ));
        }
        if jpeg_pointer != original_pointer {
            return Err("TurboJPEG changed a TJFLAG_NOREALLOC buffer pointer".to_owned());
        }
        let jpeg_size = usize::try_from(jpeg_size)
            .map_err(|_| "encoded JPEG size does not fit this platform".to_owned())?;
        if jpeg_size > maximum {
            return Err(format!(
                "TurboJPEG reported {jpeg_size} bytes for a {maximum} byte destination"
            ));
        }
        let encoded = &self.scratch[..jpeg_size];
        if encoded.len() < 4
            || encoded[..2] != [0xff, 0xd8]
            || encoded[encoded.len() - 2..] != [0xff, 0xd9]
        {
            return Err("TurboJPEG returned a malformed JPEG boundary".to_owned());
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(jpeg_size)
            .map_err(|error| format!("cannot allocate compact JPEG result: {error}"))?;
        bytes.extend_from_slice(encoded);

        Ok(EncodedFrame {
            id: frame.id(),
            bytes,
            width: layout.width(),
            height: layout.height(),
            quality,
        })
    }
}

impl Drop for Compressor {
    fn drop(&mut self) {
        // SAFETY: raw is the unique handle returned by tjInitCompress and Drop
        // runs exactly once. Destructors cannot report a library error.
        unsafe {
            tjDestroy(self.raw.as_ptr());
        }
    }
}

pub struct EncodedFrame {
    id: FrameId,
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    quality: u8,
}

impl EncodedFrame {
    pub fn id(&self) -> FrameId {
        self.id
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn quality(&self) -> u8 {
        self.quality
    }

    pub fn checksum(&self) -> u64 {
        fnv1a64(&self.bytes)
    }
}

pub fn fnv1a64(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    bytes.iter().fold(FNV_OFFSET, |checksum, byte| {
        (checksum ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

fn error_string(handle: Option<NonNull<c_void>>) -> String {
    // SAFETY: TurboJPEG returns a library-owned NUL-terminated string. It is
    // copied immediately because later library calls may replace its content.
    let pointer = unsafe { tjGetErrorStr2(handle.map_or(std::ptr::null_mut(), NonNull::as_ptr)) };
    if pointer.is_null() {
        "unknown TurboJPEG error".to_owned()
    } else {
        // SAFETY: the pointer contract above guarantees a C string.
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{FrameId, FrameLayout, FramePipeline, GenerationMismatch, SubmitOutcome};

    const TJPF_BGRA: c_int = 8;

    #[link(name = "turbojpeg")]
    unsafe extern "C" {
        fn tjInitDecompress() -> *mut c_void;
        fn tjDecompressHeader3(
            handle: *mut c_void,
            jpeg_buffer: *const c_uchar,
            jpeg_size: c_ulong,
            width: *mut c_int,
            height: *mut c_int,
            jpeg_subsampling: *mut c_int,
            jpeg_color_space: *mut c_int,
        ) -> c_int;
        fn tjDecompress2(
            handle: *mut c_void,
            jpeg_buffer: *const c_uchar,
            jpeg_size: c_ulong,
            destination: *mut c_uchar,
            width: c_int,
            pitch: c_int,
            height: c_int,
            pixel_format: c_int,
            flags: c_int,
        ) -> c_int;
    }

    struct Decompressor(NonNull<c_void>);

    impl Decompressor {
        fn new() -> Self {
            // SAFETY: no preconditions; the result is checked and owned.
            let raw = unsafe { tjInitDecompress() };
            Self(NonNull::new(raw).expect("test decompressor must initialize"))
        }

        fn decode_bgra(&mut self, jpeg: &EncodedFrame) -> (u32, u32, c_int, Vec<u8>) {
            let jpeg_size = c_ulong::try_from(jpeg.bytes().len()).unwrap();
            let mut width = 0;
            let mut height = 0;
            let mut subsampling = -1;
            let mut color_space = -1;
            // SAFETY: all output pointers are valid and the JPEG slice remains
            // alive for the call.
            let header_result = unsafe {
                tjDecompressHeader3(
                    self.0.as_ptr(),
                    jpeg.bytes().as_ptr(),
                    jpeg_size,
                    &mut width,
                    &mut height,
                    &mut subsampling,
                    &mut color_space,
                )
            };
            assert_eq!(header_result, 0, "{}", error_string(Some(self.0)));
            let layout = FrameLayout::new(width as u32, height as u32).unwrap();
            let mut pixels = vec![0_u8; layout.byte_len()];
            // SAFETY: dimensions came from the validated JPEG header and the
            // destination is exactly pitch * height bytes.
            let decode_result = unsafe {
                tjDecompress2(
                    self.0.as_ptr(),
                    jpeg.bytes().as_ptr(),
                    jpeg_size,
                    pixels.as_mut_ptr(),
                    width,
                    c_int::try_from(layout.stride()).unwrap(),
                    height,
                    TJPF_BGRA,
                    0,
                )
            };
            assert_eq!(decode_result, 0, "{}", error_string(Some(self.0)));
            (width as u32, height as u32, subsampling, pixels)
        }
    }

    impl Drop for Decompressor {
        fn drop(&mut self) {
            // SAFETY: this is the unique handle returned by tjInitDecompress.
            unsafe {
                tjDestroy(self.0.as_ptr());
            }
        }
    }

    #[test]
    fn compression_is_deterministic_and_roundtrips_as_422() {
        let layout = FrameLayout::new(96, 64).unwrap();
        let frame = Frame::test_pattern(
            FrameId {
                generation: 2,
                sequence: 7,
            },
            layout,
        )
        .unwrap();
        let mut compressor = Compressor::new().unwrap();
        let first = compressor.compress_bgrx(&frame, 90).unwrap();
        let scratch_pointer = compressor.scratch.as_ptr();
        let scratch_capacity = compressor.scratch.capacity();
        let second = compressor.compress_bgrx(&frame, 90).unwrap();
        assert_eq!(compressor.scratch.as_ptr(), scratch_pointer);
        assert_eq!(compressor.scratch.capacity(), scratch_capacity);
        assert!(first.bytes.capacity() < scratch_capacity);
        assert_eq!(first.bytes(), second.bytes());
        assert_eq!(first.checksum(), second.checksum());
        assert_eq!(first.id(), frame.id());
        assert_eq!(
            (first.width(), first.height(), first.quality()),
            (96, 64, 90)
        );

        let mut decompressor = Decompressor::new();
        let (width, height, subsampling, decoded) = decompressor.decode_bgra(&first);
        assert_eq!((width, height, subsampling), (96, 64, TJSAMP_422));
        assert!(decoded.chunks_exact(4).all(|pixel| pixel[3] == 255));

        let total_error: u64 = frame
            .pixels()
            .chunks_exact(4)
            .zip(decoded.chunks_exact(4))
            .map(|(source, decoded)| {
                (0..3)
                    .map(|channel| u64::from(source[channel].abs_diff(decoded[channel])))
                    .sum::<u64>()
            })
            .sum();
        let channel_count = u64::from(layout.width()) * u64::from(layout.height()) * 3;
        assert!(total_error / channel_count < 35);
    }

    #[test]
    fn rejects_invalid_quality_before_ffi() {
        let frame = Frame::test_pattern(
            FrameId {
                generation: 1,
                sequence: 0,
            },
            FrameLayout::new(8, 8).unwrap(),
        )
        .unwrap();
        let mut compressor = Compressor::new().unwrap();
        assert!(compressor.compress_bgrx(&frame, 0).is_err());
        assert!(compressor.compress_bgrx(&frame, 101).is_err());
    }

    #[test]
    fn encoded_frame_from_old_generation_is_rejected_before_sink() {
        let pipeline = FramePipeline::new(1, 4).unwrap();
        let frame = Frame::test_pattern(
            FrameId {
                generation: 4,
                sequence: 12,
            },
            FrameLayout::new(64, 48).unwrap(),
        )
        .unwrap();
        assert_eq!(pipeline.submit(frame), SubmitOutcome::Queued);
        let in_flight = pipeline.try_take().unwrap();
        let mut compressor = Compressor::new().unwrap();
        let encoded = compressor.compress_bgrx(&in_flight, 80).unwrap();
        assert_eq!(encoded.id(), in_flight.id());

        pipeline.begin_generation(5).unwrap();
        let mut reached_sink = false;
        assert_eq!(
            pipeline.with_current_generation(encoded.id(), || reached_sink = true),
            Err(GenerationMismatch {
                active: 5,
                received: 4
            })
        );
        assert!(!reached_sink);
    }

    #[test]
    fn checksum_matches_known_fnv1a_vectors() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
