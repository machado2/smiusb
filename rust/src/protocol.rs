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

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated slice"),
    )
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

pub fn parse_command_request(packet: &[u8]) -> Result<CommandRequest, ParseError> {
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

    #[test]
    fn heartbeat_contains_client_tag_and_zeroes_remaining_tail() {
        let packet = build_bulk_heartbeat_request(10_346).unwrap();
        assert_eq!(parse_command_request(&packet).unwrap().opcode, 0xec);
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
        assert_eq!(parse_command_request(&packet).unwrap().opcode, 0x65);
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
            parse_command_request(&packet),
            Err(ParseError::InvalidRequestLength { .. })
        ));
    }
}
