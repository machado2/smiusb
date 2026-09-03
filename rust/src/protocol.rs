use std::fmt;

pub const VENDOR_ID: u16 = 0x090c;
pub const SM768_PRODUCT_ID: u16 = 0x0768;
pub const WIRE_MAGIC: &[u8; 12] = b"smifalconsta";
pub const HEADER_SIZE: usize = 48;
pub const HEARTBEAT_SIZE: usize = 44;
pub const COMMAND_CLASS: u32 = 6;
pub const CAPABILITIES_OPCODE: u8 = 0x65;
pub const BULK_HEARTBEAT_OPCODE: u8 = 0xec;
pub const DISPLAY_INTERFACE: u8 = 4;
pub const DISPLAY_ALT_SETTING: u8 = 1;
pub const INTERRUPT_IN_ENDPOINT: u8 = 0x81;
pub const INTERRUPT_OUT_ENDPOINT: u8 = 0x01;
pub const BULK_OUT_ENDPOINT: u8 = 0x02;
pub const HEARTBEAT_TAG_OFFSET: usize = 35;
pub const HEARTBEAT_TAG_CAPACITY: usize = HEARTBEAT_SIZE - HEARTBEAT_TAG_OFFSET - 1;
pub const FRAME_SIGNATURE: u32 = 0xa000_0002;
pub const FRAME_TYPE: u8 = 0x04;
pub const FRAME_SUBTYPE: u8 = 0x01;
pub const JPEG_DECODER_METADATA_SIZE: usize = 0x5000;
pub const HIGH_SPEED_MAX_PACKET_SIZE: usize = 512;
pub const SUPER_SPEED_MAX_PACKET_SIZE: usize = 1024;
pub const FRAME_ACK_SIZE: usize = 1024;
pub const FRAME_ACK_SIGNATURE: u32 = 0x0100_01e9;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WireHeader {
    /// This is a byte count in observed requests, but not in every response.
    pub word12: u32,
    /// Request class 6 occupies this word. Responses use a different layout.
    pub word16: u32,
    pub byte20: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandRequest {
    pub length: usize,
    pub opcode: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
    pub bits_per_pixel: u32,
}

/// A validated SM768 frame envelope. All three byte ranges borrow the input
/// packet: decoding a capture never copies or allocates frame data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameEnvelope<'packet> {
    pub sequence: u8,
    /// Observed as zero, but deliberately left uninterpreted until a counter
    /// wrap establishes whether it extends `sequence`.
    pub unclassified_byte23: u8,
    pub jpeg: &'packet [u8],
    pub decoder_metadata: &'packet [u8],
    pub padding: &'packet [u8],
}

/// The fields with established meaning in an observed interrupt-IN frame ACK.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameAck {
    pub sequence: u8,
    /// Observed as zero; retained verbatim instead of being declared reserved.
    pub unclassified_bytes21_23: [u8; 3],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameAckParseError {
    InvalidPacketLength { actual: usize, expected: usize },
    BadMagic,
    InvalidDeclaredLength(u32),
    InvalidSignature(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameParseError {
    InvalidMaxPacketSize(usize),
    TooShort {
        actual: usize,
        minimum: usize,
    },
    BadMagic,
    DeclaredLengthDoesNotFit(u32),
    DeclaredLengthMismatch {
        declared: usize,
        actual: usize,
    },
    InvalidSignature(u32),
    InvalidFrameType {
        byte20: u8,
        byte21: u8,
    },
    MissingJpegStart,
    ExpectedJpegMarker {
        offset: usize,
        byte: u8,
    },
    TruncatedJpegMarker {
        offset: usize,
    },
    TruncatedJpegSegmentLength {
        offset: usize,
        marker: u8,
    },
    InvalidJpegSegmentLength {
        offset: usize,
        marker: u8,
        length: usize,
    },
    TruncatedJpegSegment {
        offset: usize,
        marker: u8,
        length: usize,
        available: usize,
    },
    InvalidJpegScanHeader {
        offset: usize,
        length: usize,
        components: usize,
    },
    UnexpectedJpegMarker {
        offset: usize,
        marker: u8,
    },
    JpegEndBeforeScan {
        offset: usize,
    },
    MissingJpegEnd,
    LengthOverflow,
    TruncatedDecoderMetadata {
        actual: usize,
        expected: usize,
    },
    PaddingTooLong {
        actual: usize,
        maximum: usize,
    },
    NonZeroPadding {
        offset: usize,
        value: u8,
    },
    UnalignedPayload {
        actual: usize,
        expected: usize,
        max_packet_size: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseError {
    TooShort { actual: usize, minimum: usize },
    BadMagic,
    InvalidRequestLength { declared: usize, actual: usize },
    InvalidRequestClass(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildError {
    ClientTagTooLong { digits: usize, maximum: usize },
}

impl fmt::Display for BuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClientTagTooLong { digits, maximum } => write!(
                formatter,
                "heartbeat client tag has {digits} digits; maximum is {maximum}"
            ),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { actual, minimum } => {
                write!(
                    formatter,
                    "packet is {actual} bytes; need at least {minimum}"
                )
            }
            Self::BadMagic => write!(formatter, "packet does not start with SMIUSB magic"),
            Self::InvalidRequestLength { declared, actual } => {
                write!(
                    formatter,
                    "request declares {declared} bytes but has {actual}"
                )
            }
            Self::InvalidRequestClass(class) => {
                write!(formatter, "unsupported request class {class:#x}")
            }
        }
    }
}

impl fmt::Display for FrameParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMaxPacketSize(actual) => write!(
                formatter,
                "unsupported bulk max-packet size {actual}; expected 512 or 1024"
            ),
            Self::TooShort { actual, minimum } => {
                write!(
                    formatter,
                    "frame is {actual} bytes; need at least {minimum}"
                )
            }
            Self::BadMagic => write!(formatter, "frame does not start with SMIUSB magic"),
            Self::DeclaredLengthDoesNotFit(declared) => write!(
                formatter,
                "declared frame length {declared} does not fit this platform"
            ),
            Self::DeclaredLengthMismatch { declared, actual } => write!(
                formatter,
                "frame declares {declared} bytes but capture has {actual}"
            ),
            Self::InvalidSignature(actual) => write!(
                formatter,
                "invalid SM768 frame signature {actual:#010x}; expected {FRAME_SIGNATURE:#010x}"
            ),
            Self::InvalidFrameType { byte20, byte21 } => write!(
                formatter,
                "invalid SM768 frame type {byte20:#04x}/{byte21:#04x}; expected {FRAME_TYPE:#04x}/{FRAME_SUBTYPE:#04x}"
            ),
            Self::MissingJpegStart => {
                write!(formatter, "frame payload does not start with JPEG SOI")
            }
            Self::ExpectedJpegMarker { offset, byte } => write!(
                formatter,
                "expected a JPEG marker at payload offset {offset}, found {byte:#04x}"
            ),
            Self::TruncatedJpegMarker { offset } => {
                write!(
                    formatter,
                    "truncated JPEG marker at payload offset {offset}"
                )
            }
            Self::TruncatedJpegSegmentLength { offset, marker } => write!(
                formatter,
                "truncated JPEG {marker:#04x} segment length at payload offset {offset}"
            ),
            Self::InvalidJpegSegmentLength {
                offset,
                marker,
                length,
            } => write!(
                formatter,
                "invalid JPEG {marker:#04x} segment length {length} at payload offset {offset}"
            ),
            Self::TruncatedJpegSegment {
                offset,
                marker,
                length,
                available,
            } => write!(
                formatter,
                "JPEG {marker:#04x} segment at payload offset {offset} declares {length} bytes but only {available} remain"
            ),
            Self::InvalidJpegScanHeader {
                offset,
                length,
                components,
            } => write!(
                formatter,
                "invalid JPEG SOS header at payload offset {offset}: length={length}, components={components}"
            ),
            Self::UnexpectedJpegMarker { offset, marker } => write!(
                formatter,
                "unexpected JPEG marker {marker:#04x} at payload offset {offset}"
            ),
            Self::JpegEndBeforeScan { offset } => write!(
                formatter,
                "JPEG EOI at payload offset {offset} appears before any scan"
            ),
            Self::MissingJpegEnd => write!(formatter, "JPEG payload has no structural EOI marker"),
            Self::LengthOverflow => {
                write!(formatter, "frame layout length overflows this platform")
            }
            Self::TruncatedDecoderMetadata { actual, expected } => write!(
                formatter,
                "decoder metadata has {actual} bytes; expected exactly {expected}"
            ),
            Self::PaddingTooLong { actual, maximum } => write!(
                formatter,
                "frame has {actual} padding bytes; maximum is {maximum}"
            ),
            Self::NonZeroPadding { offset, value } => write!(
                formatter,
                "frame padding byte {offset} is {value:#04x}; expected zero"
            ),
            Self::UnalignedPayload {
                actual,
                expected,
                max_packet_size,
            } => write!(
                formatter,
                "post-header payload is {actual} bytes; expected {expected} for {max_packet_size}-byte packet alignment"
            ),
        }
    }
}

impl fmt::Display for FrameAckParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPacketLength { actual, expected } => write!(
                formatter,
                "frame ACK packet is {actual} bytes; expected exactly {expected}"
            ),
            Self::BadMagic => write!(formatter, "frame ACK does not start with SMIUSB magic"),
            Self::InvalidDeclaredLength(actual) => write!(
                formatter,
                "frame ACK declares {actual} bytes; expected {FRAME_ACK_SIZE}"
            ),
            Self::InvalidSignature(actual) => write!(
                formatter,
                "invalid frame ACK signature {actual:#010x}; expected {FRAME_ACK_SIGNATURE:#010x}"
            ),
        }
    }
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated slice"),
    )
}

fn read_u16_be(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("validated slice"),
    )
}

fn parse_jpeg_end(jpeg_and_tail: &[u8]) -> Result<usize, FrameParseError> {
    if !jpeg_and_tail.starts_with(&[0xff, 0xd8]) {
        return Err(FrameParseError::MissingJpegStart);
    }

    let mut cursor = 2_usize;
    let mut in_scan = false;
    let mut saw_scan = false;
    loop {
        let (marker_offset, marker) = if in_scan {
            loop {
                let Some(relative) = jpeg_and_tail[cursor..]
                    .iter()
                    .position(|byte| *byte == 0xff)
                else {
                    return Err(FrameParseError::MissingJpegEnd);
                };
                cursor = cursor
                    .checked_add(relative)
                    .ok_or(FrameParseError::LengthOverflow)?;
                let marker_offset = cursor;
                cursor = cursor
                    .checked_add(1)
                    .ok_or(FrameParseError::LengthOverflow)?;
                while jpeg_and_tail.get(cursor) == Some(&0xff) {
                    cursor = cursor
                        .checked_add(1)
                        .ok_or(FrameParseError::LengthOverflow)?;
                }
                let marker =
                    *jpeg_and_tail
                        .get(cursor)
                        .ok_or(FrameParseError::TruncatedJpegMarker {
                            offset: marker_offset,
                        })?;
                cursor = cursor
                    .checked_add(1)
                    .ok_or(FrameParseError::LengthOverflow)?;

                match marker {
                    0x00 | 0xd0..=0xd7 => continue,
                    _ => {
                        in_scan = false;
                        break (marker_offset, marker);
                    }
                }
            }
        } else {
            let marker_offset = cursor;
            let prefix = *jpeg_and_tail.get(cursor).ok_or(if saw_scan {
                FrameParseError::MissingJpegEnd
            } else {
                FrameParseError::TruncatedJpegMarker {
                    offset: marker_offset,
                }
            })?;
            if prefix != 0xff {
                return Err(FrameParseError::ExpectedJpegMarker {
                    offset: marker_offset,
                    byte: prefix,
                });
            }
            cursor = cursor
                .checked_add(1)
                .ok_or(FrameParseError::LengthOverflow)?;
            while jpeg_and_tail.get(cursor) == Some(&0xff) {
                cursor = cursor
                    .checked_add(1)
                    .ok_or(FrameParseError::LengthOverflow)?;
            }
            let marker =
                *jpeg_and_tail
                    .get(cursor)
                    .ok_or(FrameParseError::TruncatedJpegMarker {
                        offset: marker_offset,
                    })?;
            cursor = cursor
                .checked_add(1)
                .ok_or(FrameParseError::LengthOverflow)?;
            (marker_offset, marker)
        };

        match marker {
            0xd9 if saw_scan => return Ok(cursor),
            0xd9 => {
                return Err(FrameParseError::JpegEndBeforeScan {
                    offset: marker_offset,
                });
            }
            // SOI may only be the first marker. Stuffed data and restart/TEM
            // markers are only meaningful inside entropy-coded scan data.
            0x00 | 0x01 | 0xd0..=0xd8 => {
                return Err(FrameParseError::UnexpectedJpegMarker {
                    offset: marker_offset,
                    marker,
                });
            }
            _ => {}
        }

        let available = jpeg_and_tail.len().saturating_sub(cursor);
        if available < 2 {
            return Err(FrameParseError::TruncatedJpegSegmentLength {
                offset: marker_offset,
                marker,
            });
        }
        let segment_length = usize::from(read_u16_be(jpeg_and_tail, cursor));
        if segment_length < 2 {
            return Err(FrameParseError::InvalidJpegSegmentLength {
                offset: marker_offset,
                marker,
                length: segment_length,
            });
        }
        let segment_end = cursor
            .checked_add(segment_length)
            .ok_or(FrameParseError::LengthOverflow)?;
        if segment_end > jpeg_and_tail.len() {
            return Err(FrameParseError::TruncatedJpegSegment {
                offset: marker_offset,
                marker,
                length: segment_length,
                available,
            });
        }

        if marker == 0xda {
            let components = if segment_length >= 3 {
                usize::from(jpeg_and_tail[cursor + 2])
            } else {
                0
            };
            let expected_length = components
                .checked_mul(2)
                .and_then(|value| value.checked_add(6))
                .ok_or(FrameParseError::LengthOverflow)?;
            if components == 0 || segment_length != expected_length {
                return Err(FrameParseError::InvalidJpegScanHeader {
                    offset: marker_offset,
                    length: segment_length,
                    components,
                });
            }
            saw_scan = true;
            in_scan = true;
        }
        cursor = segment_end;
    }
}

fn align_payload_length(
    content_length: usize,
    max_packet_size: usize,
) -> Result<usize, FrameParseError> {
    let remainder = content_length % max_packet_size;
    if remainder == 0 {
        Ok(content_length)
    } else {
        content_length
            .checked_add(max_packet_size - remainder)
            .ok_or(FrameParseError::LengthOverflow)
    }
}

/// Parses one already captured interrupt-IN frame ACK without performing I/O.
/// Only capture-confirmed header fields are interpreted; bytes 24 onward remain
/// opaque until their meaning is established.
pub fn parse_frame_ack(packet: &[u8]) -> Result<FrameAck, FrameAckParseError> {
    if packet.len() != FRAME_ACK_SIZE {
        return Err(FrameAckParseError::InvalidPacketLength {
            actual: packet.len(),
            expected: FRAME_ACK_SIZE,
        });
    }
    if &packet[..WIRE_MAGIC.len()] != WIRE_MAGIC {
        return Err(FrameAckParseError::BadMagic);
    }

    let declared_length = read_u32_le(packet, 12);
    if declared_length != FRAME_ACK_SIZE as u32 {
        return Err(FrameAckParseError::InvalidDeclaredLength(declared_length));
    }

    let signature = read_u32_le(packet, 16);
    if signature != FRAME_ACK_SIGNATURE {
        return Err(FrameAckParseError::InvalidSignature(signature));
    }

    Ok(FrameAck {
        sequence: packet[20],
        unclassified_bytes21_23: packet[21..24]
            .try_into()
            .expect("validated ACK header slice"),
    })
}

/// Parses an already captured frame packet. This is deliberately an offline
/// function: it accepts only borrowed bytes and has no access to the USB layer.
/// The endpoint descriptor's max-packet size must be supplied explicitly.
pub fn parse_frame_envelope(
    packet: &[u8],
    max_packet_size: usize,
) -> Result<FrameEnvelope<'_>, FrameParseError> {
    if !matches!(
        max_packet_size,
        HIGH_SPEED_MAX_PACKET_SIZE | SUPER_SPEED_MAX_PACKET_SIZE
    ) {
        return Err(FrameParseError::InvalidMaxPacketSize(max_packet_size));
    }
    if packet.len() < HEADER_SIZE {
        return Err(FrameParseError::TooShort {
            actual: packet.len(),
            minimum: HEADER_SIZE,
        });
    }
    if &packet[..WIRE_MAGIC.len()] != WIRE_MAGIC {
        return Err(FrameParseError::BadMagic);
    }

    let declared_wire = read_u32_le(packet, 12);
    let declared = usize::try_from(declared_wire)
        .map_err(|_| FrameParseError::DeclaredLengthDoesNotFit(declared_wire))?;
    if declared != packet.len() {
        return Err(FrameParseError::DeclaredLengthMismatch {
            declared,
            actual: packet.len(),
        });
    }
    let signature = read_u32_le(packet, 16);
    if signature != FRAME_SIGNATURE {
        return Err(FrameParseError::InvalidSignature(signature));
    }
    if packet[20] != FRAME_TYPE || packet[21] != FRAME_SUBTYPE {
        return Err(FrameParseError::InvalidFrameType {
            byte20: packet[20],
            byte21: packet[21],
        });
    }
    // Captures have byte 22 incrementing through 0x89 while byte 23 remains
    // zero. Keep the proven u8 field and unclassified byte separate until a
    // wrap demonstrates whether the latter extends the sequence number.
    let sequence = packet[22];
    let unclassified_byte23 = packet[23];
    let jpeg_length = parse_jpeg_end(&packet[HEADER_SIZE..])?;
    let jpeg_end = HEADER_SIZE
        .checked_add(jpeg_length)
        .ok_or(FrameParseError::LengthOverflow)?;
    let decoder_end = jpeg_end
        .checked_add(JPEG_DECODER_METADATA_SIZE)
        .ok_or(FrameParseError::LengthOverflow)?;
    if decoder_end > packet.len() {
        return Err(FrameParseError::TruncatedDecoderMetadata {
            actual: packet.len().saturating_sub(jpeg_end),
            expected: JPEG_DECODER_METADATA_SIZE,
        });
    }

    let padding = &packet[decoder_end..];
    if padding.len() >= max_packet_size {
        return Err(FrameParseError::PaddingTooLong {
            actual: padding.len(),
            maximum: max_packet_size - 1,
        });
    }
    if let Some((offset, value)) = padding
        .iter()
        .copied()
        .enumerate()
        .find(|(_, byte)| *byte != 0)
    {
        return Err(FrameParseError::NonZeroPadding { offset, value });
    }

    let content_length = jpeg_length
        .checked_add(JPEG_DECODER_METADATA_SIZE)
        .ok_or(FrameParseError::LengthOverflow)?;
    let expected_payload = align_payload_length(content_length, max_packet_size)?;
    let actual_payload = packet
        .len()
        .checked_sub(HEADER_SIZE)
        .ok_or(FrameParseError::LengthOverflow)?;
    if actual_payload != expected_payload {
        return Err(FrameParseError::UnalignedPayload {
            actual: actual_payload,
            expected: expected_payload,
            max_packet_size,
        });
    }

    Ok(FrameEnvelope {
        sequence,
        unclassified_byte23,
        jpeg: &packet[HEADER_SIZE..jpeg_end],
        decoder_metadata: &packet[jpeg_end..decoder_end],
        padding,
    })
}

pub fn parse_header(packet: &[u8]) -> Result<WireHeader, ParseError> {
    if packet.len() < 21 {
        return Err(ParseError::TooShort {
            actual: packet.len(),
            minimum: 21,
        });
    }
    if &packet[..WIRE_MAGIC.len()] != WIRE_MAGIC {
        return Err(ParseError::BadMagic);
    }

    Ok(WireHeader {
        word12: read_u32_le(packet, 12),
        word16: read_u32_le(packet, 16),
        byte20: packet[20],
    })
}

/// Validates the canonical request shape emitted by this implementation.
/// Captures show that other vendor opcodes give words 12 and 16 opcode-specific
/// meanings, so this must not be used as a universal packet classifier.
pub fn parse_canonical_command_request(packet: &[u8]) -> Result<CommandRequest, ParseError> {
    let header = parse_header(packet)?;
    let declared = header.word12 as usize;
    if declared != packet.len() {
        return Err(ParseError::InvalidRequestLength {
            declared,
            actual: packet.len(),
        });
    }
    if header.word16 != COMMAND_CLASS {
        return Err(ParseError::InvalidRequestClass(header.word16));
    }

    Ok(CommandRequest {
        length: declared,
        opcode: header.byte20,
    })
}

fn build_zeroed_request<const N: usize>(opcode: u8) -> [u8; N] {
    let mut packet = [0_u8; N];
    packet[..WIRE_MAGIC.len()].copy_from_slice(WIRE_MAGIC);
    packet[12..16].copy_from_slice(&(N as u32).to_le_bytes());
    packet[16..20].copy_from_slice(&COMMAND_CLASS.to_le_bytes());
    packet[20] = opcode;
    packet
}

/// Builds the deterministic form of the observed request. A passive uprobe
/// identified an ASCII decimal worker TID/client tag starting at byte 35. The
/// vendor's final tail bytes appeared uninitialized; this implementation keeps
/// the tag's NUL terminator and every remaining byte zero. This module never
/// transmits the packet.
pub fn build_bulk_heartbeat_request(client_tag: u32) -> Result<[u8; HEARTBEAT_SIZE], BuildError> {
    let text = client_tag.to_string();
    if text.len() > HEARTBEAT_TAG_CAPACITY {
        return Err(BuildError::ClientTagTooLong {
            digits: text.len(),
            maximum: HEARTBEAT_TAG_CAPACITY,
        });
    }
    let mut packet = build_zeroed_request(BULK_HEARTBEAT_OPCODE);
    let end = HEARTBEAT_TAG_OFFSET + text.len();
    packet[HEARTBEAT_TAG_OFFSET..end].copy_from_slice(text.as_bytes());
    Ok(packet)
}

/// Builds the initial request shape inferred from the vendor daemon. Sending
/// it remains disabled until a complete attach trace is available.
pub fn build_capabilities_request() -> [u8; HEADER_SIZE] {
    build_zeroed_request(CAPABILITIES_OPCODE)
}

/// Decodes the 16-byte mode rows observed at offset 48 in SM768 responses.
/// Parsing stops at a zero row or at the first implausible row, so an opaque
/// response trailer cannot be mistaken for attacker-controlled mode data.
pub fn parse_observed_modes(packet: &[u8]) -> Result<Vec<Mode>, ParseError> {
    parse_header(packet)?;
    if packet.len() < HEADER_SIZE {
        return Err(ParseError::TooShort {
            actual: packet.len(),
            minimum: HEADER_SIZE,
        });
    }

    let mut modes = Vec::new();
    for row in packet[HEADER_SIZE..].chunks_exact(16).take(256) {
        let mode = Mode {
            width: read_u32_le(row, 0),
            height: read_u32_le(row, 4),
            refresh_hz: read_u32_le(row, 8),
            bits_per_pixel: read_u32_le(row, 12),
        };
        if mode
            == (Mode {
                width: 0,
                height: 0,
                refresh_hz: 0,
                bits_per_pixel: 0,
            })
        {
            break;
        }

        let plausible = (320..=16_384).contains(&mode.width)
            && (200..=16_384).contains(&mode.height)
            && (1..=1_000).contains(&mode.refresh_hz)
            && matches!(mode.bits_per_pixel, 8 | 16 | 24 | 30 | 32);
        if !plausible {
            break;
        }
        modes.push(mode);
    }
    Ok(modes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_frame_ack(sequence: u8) -> [u8; FRAME_ACK_SIZE] {
        // Keep the unclassified tail nonzero so the fixture only relies on
        // fields whose meanings have actually been observed.
        let mut packet = [0xa5_u8; FRAME_ACK_SIZE];
        packet[..WIRE_MAGIC.len()].copy_from_slice(WIRE_MAGIC);
        packet[12..16].copy_from_slice(&(FRAME_ACK_SIZE as u32).to_le_bytes());
        packet[16..20].copy_from_slice(&FRAME_ACK_SIGNATURE.to_le_bytes());
        packet[20] = sequence;
        packet[21..24].fill(0);
        packet
    }

    fn synthetic_jpeg() -> Vec<u8> {
        vec![
            0xff, 0xd8, // SOI
            0xff, 0xe1, 0x00, 0x08, 0x11, 0xff, 0xd9, 0x22, 0x33,
            0x44, // APP1 containing a false EOI byte pair
            0xff, 0xda, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3f, 0x00, // one-component SOS
            0x10, 0xff, 0x00, 0xd9, 0x20, // byte-stuffed 0xff followed by data 0xd9
            0xff, 0xd0, 0x30, // restart marker inside entropy data
            0xff, 0xd9, // structural EOI
        ]
    }

    fn frame_with_parts(jpeg: &[u8], decoder_length: usize, padding_length: usize) -> Vec<u8> {
        let total = HEADER_SIZE + jpeg.len() + decoder_length + padding_length;
        let mut packet = vec![0_u8; total];
        packet[..WIRE_MAGIC.len()].copy_from_slice(WIRE_MAGIC);
        packet[12..16].copy_from_slice(
            &u32::try_from(total)
                .expect("synthetic packet fits wire length")
                .to_le_bytes(),
        );
        packet[16..20].copy_from_slice(&FRAME_SIGNATURE.to_le_bytes());
        packet[20] = FRAME_TYPE;
        packet[21] = FRAME_SUBTYPE;
        packet[22] = 0x89;
        packet[HEADER_SIZE..HEADER_SIZE + jpeg.len()].copy_from_slice(jpeg);
        packet[HEADER_SIZE + jpeg.len()..HEADER_SIZE + jpeg.len() + decoder_length].fill(0xa5);
        packet
    }

    fn synthetic_frame(max_packet_size: usize) -> Vec<u8> {
        let jpeg = synthetic_jpeg();
        let content_length = jpeg.len() + JPEG_DECODER_METADATA_SIZE;
        let padded_length = align_payload_length(content_length, max_packet_size).unwrap();
        frame_with_parts(
            &jpeg,
            JPEG_DECODER_METADATA_SIZE,
            padded_length - content_length,
        )
    }

    #[test]
    fn parses_observed_frame_ack_fixture() {
        let packet = synthetic_frame_ack(0x89);

        assert_eq!(
            parse_frame_ack(&packet),
            Ok(FrameAck {
                sequence: 0x89,
                unclassified_bytes21_23: [0; 3],
            })
        );
    }

    #[test]
    fn frame_ack_rejects_every_truncation_and_an_oversized_packet() {
        let packet = synthetic_frame_ack(0x37);
        for actual in 0..FRAME_ACK_SIZE {
            assert_eq!(
                parse_frame_ack(&packet[..actual]),
                Err(FrameAckParseError::InvalidPacketLength {
                    actual,
                    expected: FRAME_ACK_SIZE,
                }),
                "accepted truncated ACK of {actual} bytes"
            );
        }

        let mut oversized = packet.to_vec();
        oversized.push(0);
        assert_eq!(
            parse_frame_ack(&oversized),
            Err(FrameAckParseError::InvalidPacketLength {
                actual: FRAME_ACK_SIZE + 1,
                expected: FRAME_ACK_SIZE,
            })
        );
    }

    #[test]
    fn frame_ack_rejects_bad_magic_length_and_signature() {
        let mut bad_magic = synthetic_frame_ack(1);
        bad_magic[0] ^= 0xff;
        assert_eq!(
            parse_frame_ack(&bad_magic),
            Err(FrameAckParseError::BadMagic)
        );

        let mut bad_length = synthetic_frame_ack(2);
        bad_length[12..16].copy_from_slice(&512_u32.to_le_bytes());
        assert_eq!(
            parse_frame_ack(&bad_length),
            Err(FrameAckParseError::InvalidDeclaredLength(512))
        );

        let mut bad_signature = synthetic_frame_ack(3);
        bad_signature[16..20].copy_from_slice(&0xdead_beef_u32.to_le_bytes());
        assert_eq!(
            parse_frame_ack(&bad_signature),
            Err(FrameAckParseError::InvalidSignature(0xdead_beef))
        );
    }

    #[test]
    fn frame_ack_preserves_unclassified_bytes_without_rejecting_them() {
        for offset in 21..=23 {
            let mut packet = synthetic_frame_ack(0xfe);
            packet[offset] = 0x80 | offset as u8;

            let ack = parse_frame_ack(&packet).unwrap();
            assert_eq!(ack.sequence, 0xfe);
            assert_eq!(
                ack.unclassified_bytes21_23[offset - 21],
                0x80 | offset as u8
            );
        }
    }

    #[test]
    fn frame_ack_errors_have_specific_display_messages() {
        assert_eq!(
            FrameAckParseError::InvalidPacketLength {
                actual: 23,
                expected: FRAME_ACK_SIZE,
            }
            .to_string(),
            "frame ACK packet is 23 bytes; expected exactly 1024"
        );
        assert_eq!(
            FrameAckParseError::InvalidSignature(0xdead_beef).to_string(),
            "invalid frame ACK signature 0xdeadbeef; expected 0x010001e9"
        );
    }

    #[test]
    fn heartbeat_contains_client_tag_and_zeroes_remaining_tail() {
        let packet = build_bulk_heartbeat_request(10_346).unwrap();
        assert_eq!(
            parse_canonical_command_request(&packet).unwrap().opcode,
            0xec
        );
        assert!(
            packet[21..HEARTBEAT_TAG_OFFSET]
                .iter()
                .all(|byte| *byte == 0)
        );
        assert_eq!(&packet[HEARTBEAT_TAG_OFFSET..40], b"10346");
        assert!(packet[40..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn heartbeat_rejects_a_tag_that_cannot_be_nul_terminated() {
        assert!(build_bulk_heartbeat_request(99_999_999).is_ok());
        assert_eq!(
            build_bulk_heartbeat_request(100_000_000),
            Err(BuildError::ClientTagTooLong {
                digits: 9,
                maximum: 8
            })
        );
    }

    #[test]
    fn capabilities_request_has_observed_shape() {
        let packet = build_capabilities_request();
        assert_eq!(packet.len(), 48);
        assert_eq!(
            parse_canonical_command_request(&packet).unwrap().opcode,
            0x65
        );
    }

    #[test]
    fn parses_observed_mode_table_and_stops_at_zero_row() {
        let mut packet = vec![0_u8; 96];
        packet[..12].copy_from_slice(WIRE_MAGIC);
        packet[12..16].copy_from_slice(&32_u32.to_le_bytes());
        packet[48..52].copy_from_slice(&1600_u32.to_le_bytes());
        packet[52..56].copy_from_slice(&900_u32.to_le_bytes());
        packet[56..60].copy_from_slice(&75_u32.to_le_bytes());
        packet[60..64].copy_from_slice(&32_u32.to_le_bytes());
        packet[64..68].copy_from_slice(&1280_u32.to_le_bytes());
        packet[68..72].copy_from_slice(&720_u32.to_le_bytes());
        packet[72..76].copy_from_slice(&60_u32.to_le_bytes());
        packet[76..80].copy_from_slice(&16_u32.to_le_bytes());

        assert_eq!(
            parse_observed_modes(&packet).unwrap(),
            vec![
                Mode {
                    width: 1600,
                    height: 900,
                    refresh_hz: 75,
                    bits_per_pixel: 32,
                },
                Mode {
                    width: 1280,
                    height: 720,
                    refresh_hz: 60,
                    bits_per_pixel: 16,
                },
            ]
        );
    }

    #[test]
    fn rejects_bad_magic_and_inconsistent_request_length() {
        assert_eq!(parse_header(&[0_u8; 21]), Err(ParseError::BadMagic));

        let mut packet = build_bulk_heartbeat_request(1).unwrap();
        packet[12] = 48;
        assert!(matches!(
            parse_canonical_command_request(&packet),
            Err(ParseError::InvalidRequestLength { .. })
        ));
    }

    #[test]
    fn parses_borrowed_superspeed_frame_fixture() {
        let packet = synthetic_frame(SUPER_SPEED_MAX_PACKET_SIZE);
        let frame = parse_frame_envelope(&packet, SUPER_SPEED_MAX_PACKET_SIZE).unwrap();

        assert_eq!(frame.sequence, 0x89);
        assert_eq!(frame.unclassified_byte23, 0);
        assert_eq!(frame.jpeg, synthetic_jpeg());
        assert_eq!(frame.jpeg.as_ptr(), packet[HEADER_SIZE..].as_ptr());
        assert_eq!(frame.decoder_metadata.len(), JPEG_DECODER_METADATA_SIZE);
        assert!(frame.decoder_metadata.iter().all(|byte| *byte == 0xa5));
        assert_eq!(frame.padding.len(), 992);
        assert!(frame.padding.iter().all(|byte| *byte == 0));
        assert_eq!(
            (packet.len() - HEADER_SIZE) % SUPER_SPEED_MAX_PACKET_SIZE,
            0
        );
    }

    #[test]
    fn accepts_high_speed_alignment_explicitly() {
        let packet = synthetic_frame(HIGH_SPEED_MAX_PACKET_SIZE);
        let frame = parse_frame_envelope(&packet, HIGH_SPEED_MAX_PACKET_SIZE).unwrap();

        assert_eq!(frame.jpeg.len(), 32);
        assert_eq!(frame.padding.len(), 480);
        assert_eq!((packet.len() - HEADER_SIZE) % HIGH_SPEED_MAX_PACKET_SIZE, 0);
    }

    #[test]
    fn jpeg_parser_skips_segment_eoi_and_entropy_byte_stuffing() {
        let jpeg = synthetic_jpeg();
        assert_eq!(parse_jpeg_end(&jpeg), Ok(jpeg.len()));
        assert_eq!(&jpeg[7..9], &[0xff, 0xd9]);
        assert_eq!(&jpeg[23..26], &[0xff, 0x00, 0xd9]);

        assert_eq!(
            parse_jpeg_end(&[0xff, 0xd8, 0xff, 0xda, 0x00, 0x02]),
            Err(FrameParseError::InvalidJpegScanHeader {
                offset: 2,
                length: 2,
                components: 0,
            })
        );
    }

    #[test]
    fn reports_header_jpeg_and_decoder_truncation_separately() {
        assert_eq!(
            parse_frame_envelope(&[0_u8; HEADER_SIZE - 1], HIGH_SPEED_MAX_PACKET_SIZE),
            Err(FrameParseError::TooShort {
                actual: HEADER_SIZE - 1,
                minimum: HEADER_SIZE,
            })
        );

        let truncated_marker = frame_with_parts(&[0xff, 0xd8, 0xff], 0, 0);
        assert_eq!(
            parse_frame_envelope(&truncated_marker, HIGH_SPEED_MAX_PACKET_SIZE),
            Err(FrameParseError::TruncatedJpegMarker { offset: 2 })
        );

        let jpeg = synthetic_jpeg();
        let truncated_metadata = frame_with_parts(&jpeg, JPEG_DECODER_METADATA_SIZE - 1, 0);
        assert_eq!(
            parse_frame_envelope(&truncated_metadata, HIGH_SPEED_MAX_PACKET_SIZE),
            Err(FrameParseError::TruncatedDecoderMetadata {
                actual: JPEG_DECODER_METADATA_SIZE - 1,
                expected: JPEG_DECODER_METADATA_SIZE,
            })
        );
    }

    #[test]
    fn rejects_declared_length_mismatch_and_checked_length_overflow() {
        let mut packet = synthetic_frame(HIGH_SPEED_MAX_PACKET_SIZE);
        let declared = packet.len() - 1;
        packet[12..16].copy_from_slice(&(declared as u32).to_le_bytes());
        assert_eq!(
            parse_frame_envelope(&packet, HIGH_SPEED_MAX_PACKET_SIZE),
            Err(FrameParseError::DeclaredLengthMismatch {
                declared,
                actual: packet.len(),
            })
        );
        assert_eq!(
            align_payload_length(usize::MAX, HIGH_SPEED_MAX_PACKET_SIZE),
            Err(FrameParseError::LengthOverflow)
        );
    }

    #[test]
    fn rejects_nonzero_and_excess_padding() {
        let mut nonzero = synthetic_frame(HIGH_SPEED_MAX_PACKET_SIZE);
        let last = nonzero.len() - 1;
        nonzero[last] = 0x7f;
        assert_eq!(
            parse_frame_envelope(&nonzero, HIGH_SPEED_MAX_PACKET_SIZE),
            Err(FrameParseError::NonZeroPadding {
                offset: 479,
                value: 0x7f,
            })
        );

        let jpeg = synthetic_jpeg();
        let excessive = frame_with_parts(
            &jpeg,
            JPEG_DECODER_METADATA_SIZE,
            HIGH_SPEED_MAX_PACKET_SIZE,
        );
        assert_eq!(
            parse_frame_envelope(&excessive, HIGH_SPEED_MAX_PACKET_SIZE),
            Err(FrameParseError::PaddingTooLong {
                actual: HIGH_SPEED_MAX_PACKET_SIZE,
                maximum: HIGH_SPEED_MAX_PACKET_SIZE - 1,
            })
        );
    }

    #[test]
    fn rejects_unaligned_payload_and_unknown_packet_size() {
        let jpeg = synthetic_jpeg();
        let unaligned = frame_with_parts(&jpeg, JPEG_DECODER_METADATA_SIZE, 0);
        assert_eq!(
            parse_frame_envelope(&unaligned, HIGH_SPEED_MAX_PACKET_SIZE),
            Err(FrameParseError::UnalignedPayload {
                actual: jpeg.len() + JPEG_DECODER_METADATA_SIZE,
                expected: HIGH_SPEED_MAX_PACKET_SIZE * 41,
                max_packet_size: HIGH_SPEED_MAX_PACKET_SIZE,
            })
        );
        assert_eq!(
            parse_frame_envelope(&unaligned, 64),
            Err(FrameParseError::InvalidMaxPacketSize(64))
        );
    }

    #[test]
    fn keeps_observed_sequence_byte_separate_from_unclassified_byte_23() {
        let mut packet = synthetic_frame(HIGH_SPEED_MAX_PACKET_SIZE);
        packet[23] = 1;
        let frame = parse_frame_envelope(&packet, HIGH_SPEED_MAX_PACKET_SIZE).unwrap();
        assert_eq!(frame.sequence, 0x89);
        assert_eq!(frame.unclassified_byte23, 1);
    }
}
