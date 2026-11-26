//! Roland RQ1/DT1 SysEx framing for the AE-30/AE-20 (model ID `00 00 00 7D`).
//!
//! ```text
//! RQ1: F0 41 dev 00 00 00 7D 11  a a a a  s s s s  sum F7
//! DT1: F0 41 dev 00 00 00 7D 12  a a a a  data...  sum F7
//! ```

use std::fmt;

pub const ROLAND: u8 = 0x41;
pub const MODEL_ID: [u8; 4] = [0x00, 0x00, 0x00, 0x7D];
pub const RQ1: u8 = 0x11;
pub const DT1: u8 = 0x12;
/// Factory default device ID.
pub const DEFAULT_DEVICE: u8 = 0x10;
/// Most data bytes Roland allows in one DT1 packet.
pub const MAX_DT1_DATA: usize = 256;

/// Bytes before the address: F0, 41, dev, model ID (4), command.
const HEADER_LEN: usize = 8;

/// A parameter address. Stored as a plain number; on the wire it is four
/// 7-bit bytes, so `00 00 7F` + 1 = `00 01 00`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Addr(pub u32);

impl Addr {
    pub const fn from_bytes(b: [u8; 4]) -> Self {
        Addr(
            ((b[0] as u32 & 0x7F) << 21)
                | ((b[1] as u32 & 0x7F) << 14)
                | ((b[2] as u32 & 0x7F) << 7)
                | (b[3] as u32 & 0x7F),
        )
    }

    pub const fn to_bytes(self) -> [u8; 4] {
        to_7bit(self.0)
    }

    /// `self` + `offset`, where `offset` is written the way Roland's map
    /// writes it (e.g. `00 30 00` for Scene Part 1).
    pub const fn offset(self, offset: Addr) -> Addr {
        Addr(self.0 + offset.0)
    }

    /// Distance from `base` to `self`, if `self` is not before `base`.
    pub fn since(self, base: Addr) -> Option<u32> {
        self.0.checked_sub(base.0)
    }
}

impl fmt::Debug for Addr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let [a, b, c, d] = self.to_bytes();
        write!(f, "{a:02X} {b:02X} {c:02X} {d:02X}")
    }
}

impl fmt::Display for Addr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// Shorthand for an address written as four map bytes.
pub const fn addr(a: u8, b: u8, c: u8, d: u8) -> Addr {
    Addr::from_bytes([a, b, c, d])
}

/// A number as four 7-bit bytes (used for RQ1 sizes and addresses).
pub const fn to_7bit(n: u32) -> [u8; 4] {
    [
        ((n >> 21) & 0x7F) as u8,
        ((n >> 14) & 0x7F) as u8,
        ((n >> 7) & 0x7F) as u8,
        (n & 0x7F) as u8,
    ]
}

/// Roland checksum over the address and data (or size) bytes.
pub fn checksum(bytes: &[u8]) -> u8 {
    let sum: u32 = bytes.iter().map(|&b| b as u32).sum();
    ((128 - (sum % 128)) & 0x7F) as u8
}

fn frame(device: u8, command: u8, body: &[u8]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(HEADER_LEN + body.len() + 2);
    msg.extend_from_slice(&[0xF0, ROLAND, device]);
    msg.extend_from_slice(&MODEL_ID);
    msg.push(command);
    msg.extend_from_slice(body);
    msg.push(checksum(body));
    msg.push(0xF7);
    msg
}

/// Request `size` bytes starting at `addr`.
pub fn rq1(device: u8, addr: Addr, size: u32) -> Vec<u8> {
    let mut body = addr.to_bytes().to_vec();
    body.extend_from_slice(&to_7bit(size));
    frame(device, RQ1, &body)
}

/// Write `data` starting at `addr`. Callers split data longer than
/// [`MAX_DT1_DATA`] themselves (see [`dt1_packets`]).
pub fn dt1(device: u8, addr: Addr, data: &[u8]) -> Vec<u8> {
    let mut body = addr.to_bytes().to_vec();
    body.extend_from_slice(data);
    frame(device, DT1, &body)
}

/// Split a long write into DT1 packets of at most `chunk` data bytes each.
pub fn dt1_packets(device: u8, addr: Addr, data: &[u8], chunk: usize) -> Vec<Vec<u8>> {
    let chunk = chunk.clamp(1, MAX_DT1_DATA);
    data.chunks(chunk)
        .enumerate()
        .map(|(i, part)| dt1(device, Addr(addr.0 + (i * chunk) as u32), part))
        .collect()
}

/// A DT1 message received from the unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataSet {
    pub device: u8,
    pub addr: Addr,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    NotSysEx,
    NotAerophone,
    NotDt1,
    TooShort,
    BadChecksum { expected: u8, got: u8 },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ParseError::NotSysEx => write!(f, "not a SysEx message"),
            ParseError::NotAerophone => write!(f, "not a Roland Aerophone message"),
            ParseError::NotDt1 => write!(f, "not a DT1 (data set) message"),
            ParseError::TooShort => write!(f, "message too short"),
            ParseError::BadChecksum { expected, got } => {
                write!(f, "bad checksum: expected {expected:02X}, got {got:02X}")
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// Parse a DT1 from the unit.
pub fn parse_dt1(msg: &[u8]) -> Result<DataSet, ParseError> {
    if msg.first() != Some(&0xF0) || msg.last() != Some(&0xF7) {
        return Err(ParseError::NotSysEx);
    }
    if msg.len() < HEADER_LEN + 4 + 2 {
        return Err(ParseError::TooShort);
    }
    if msg[1] != ROLAND || msg[3..7] != MODEL_ID {
        return Err(ParseError::NotAerophone);
    }
    if msg[7] != DT1 {
        return Err(ParseError::NotDt1);
    }
    let body = &msg[HEADER_LEN..msg.len() - 2];
    let got = msg[msg.len() - 2];
    let expected = checksum(body);
    if got != expected {
        return Err(ParseError::BadChecksum { expected, got });
    }
    Ok(DataSet {
        device: msg[2],
        addr: Addr::from_bytes([body[0], body[1], body[2], body[3]]),
        data: body[4..].to_vec(),
    })
}

/// Universal Identity Request, sent to all devices.
pub const IDENTITY_REQUEST: [u8; 6] = [0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    Ae30,
    Ae20,
    /// An Aerophone family reply with a family number we don't know.
    Other(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub device: u8,
    pub model: Model,
    /// The four software-revision bytes, as sent.
    pub revision: [u8; 4],
}

/// Parse an Identity Reply: `F0 7E dev 06 02 41 7D 03 nn 00 r r r r F7`.
/// Returns `None` for anything that isn't an AE-30/AE-20 reply.
pub fn parse_identity(msg: &[u8]) -> Option<Identity> {
    match msg {
        [0xF0, 0x7E, dev, 0x06, 0x02, ROLAND, 0x7D, 0x03, nn, 0x00, r0, r1, r2, r3, 0xF7] => {
            Some(Identity {
                device: *dev,
                model: match nn {
                    0x00 => Model::Ae30,
                    0x01 => Model::Ae20,
                    n => Model::Other(*n),
                },
                revision: [*r0, *r1, *r2, *r3],
            })
        }
        _ => None,
    }
}

/// Read a value stored one nibble per byte, most significant first
/// (marked `#` in Roland's map).
pub fn unpack_nibbles(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0, |acc, &b| (acc << 4) | (b as u32 & 0x0F))
}

/// Store `value` as `count` nibbles, most significant first.
pub fn pack_nibbles(value: u32, count: usize) -> Vec<u8> {
    (0..count)
        .rev()
        .map(|i| ((value >> (4 * i)) & 0x0F) as u8)
        .collect()
}

/// Bytes as space-separated hex, for logs.
pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}
