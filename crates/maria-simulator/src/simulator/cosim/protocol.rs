//! Binary protocol definitions for co-simulation bridge.
//!
//! Messages are length-prefixed binary blobs exchanged over TCP.
//! Length prefix is 4-byte little-endian u32 (excluding the header itself).

use std::io::{Read, Write};

/// Message type tags.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CosimMessageType {
    Connect = 0x01,
    ConnectAck = 0x02,
    Sync = 0x03,
    SyncAck = 0x04,
    SignalUpdate = 0x05,
    Disconnect = 0x06,
    Error = 0x07,
}

impl CosimMessageType {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x01 => Some(CosimMessageType::Connect),
            0x02 => Some(CosimMessageType::ConnectAck),
            0x03 => Some(CosimMessageType::Sync),
            0x04 => Some(CosimMessageType::SyncAck),
            0x05 => Some(CosimMessageType::SignalUpdate),
            0x06 => Some(CosimMessageType::Disconnect),
            0x07 => Some(CosimMessageType::Error),
            _ => None,
        }
    }
}

/// Signal direction for co-simulation signal mapping.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CosimDirection {
    Input = 0,
    Output = 1,
    Inout = 2,
}

impl CosimDirection {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0 => Some(CosimDirection::Input),
            1 => Some(CosimDirection::Output),
            2 => Some(CosimDirection::Inout),
            _ => None,
        }
    }
}

/// Information about a single signal in the co-simulation mapping.
#[derive(Debug, Clone)]
pub struct CosimSignalInfo {
    pub name: String,
    pub width: u32,
    pub direction: CosimDirection,
}

/// Write a length-prefixed message to a stream.
pub fn write_message(
    stream: &mut impl Write,
    msg_type: CosimMessageType,
    payload: &[u8],
) -> std::io::Result<()> {
    let len = payload.len() as u32 + 1; // +1 for type byte
    let header = len.to_le_bytes();
    stream.write_all(&header)?;
    stream.write_all(&[msg_type as u8])?;
    stream.write_all(payload)?;
    stream.flush()?;
    Ok(())
}

/// Read a length-prefixed message from a stream.
/// Returns (message_type, payload) or None on connection close.
pub fn read_message(
    stream: &mut impl Read,
) -> std::io::Result<Option<(CosimMessageType, Vec<u8>)>> {
    let mut len_buf = [0u8; 4];
    match stream.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let total_len = u32::from_le_bytes(len_buf) as usize;
    if total_len < 1 {
        return Ok(None);
    }
    let mut buf = vec![0u8; total_len];
    stream.read_exact(&mut buf)?;

    let msg_type = match CosimMessageType::from_byte(buf[0]) {
        Some(t) => t,
        None => return Ok(None),
    };
    let payload = buf[1..].to_vec();
    Ok(Some((msg_type, payload)))
}

// ─── Connect/ConnectAck Payload ───

/// Encode a Connect message payload from a list of signal infos.
pub fn encode_connect(signals: &[CosimSignalInfo]) -> Vec<u8> {
    let mut payload = Vec::new();
    let count = signals.len() as u32;
    payload.extend_from_slice(&count.to_le_bytes());
    for sig in signals {
        let name_bytes = sig.name.as_bytes();
        let name_len = name_bytes.len() as u32;
        payload.extend_from_slice(&name_len.to_le_bytes());
        payload.extend_from_slice(name_bytes);
        payload.extend_from_slice(&sig.width.to_le_bytes());
        payload.push(sig.direction as u8);
    }
    payload
}

/// Decode a Connect message payload into a list of signal infos.
pub fn decode_connect(payload: &[u8]) -> std::io::Result<Vec<CosimSignalInfo>> {
    if payload.len() < 4 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "truncated connect payload",
        ));
    }
    let count = u32::from_le_bytes(payload[0..4].try_into().unwrap()) as usize;
    let mut signals = Vec::with_capacity(count);
    let mut offset = 4usize;
    for _ in 0..count {
        if offset + 4 > payload.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "truncated signal name length",
            ));
        }
        let name_len = u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        if offset + name_len > payload.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "truncated signal name",
            ));
        }
        let name = String::from_utf8_lossy(&payload[offset..offset + name_len]).to_string();
        offset += name_len;
        if offset + 4 > payload.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "truncated signal width",
            ));
        }
        let width = u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap());
        offset += 4;
        if offset >= payload.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "truncated signal direction",
            ));
        }
        let direction = CosimDirection::from_byte(payload[offset]).unwrap_or(CosimDirection::Input);
        offset += 1;
        signals.push(CosimSignalInfo {
            name,
            width,
            direction,
        });
    }
    Ok(signals)
}

/// Encode a ConnectAck message payload.
pub fn encode_connect_ack(status: u8) -> Vec<u8> {
    vec![status]
}

/// Encode a Sync message payload.
pub fn encode_sync(time_step: u64) -> Vec<u8> {
    time_step.to_le_bytes().to_vec()
}

/// Decode a Sync message payload.
pub fn decode_sync(payload: &[u8]) -> std::io::Result<u64> {
    if payload.len() < 8 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "truncated sync payload",
        ));
    }
    Ok(u64::from_le_bytes(payload[0..8].try_into().unwrap()))
}

/// Encode a SyncAck message payload.
pub fn encode_sync_ack(ready: u8) -> Vec<u8> {
    vec![ready]
}

/// Encode a SignalUpdate message payload.
pub fn encode_signal_update(signal_id: u32, value_bytes: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(4 + value_bytes.len());
    payload.extend_from_slice(&signal_id.to_le_bytes());
    payload.extend_from_slice(value_bytes);
    payload
}

/// Decode a SignalUpdate message payload.
pub fn decode_signal_update(payload: &[u8]) -> std::io::Result<(u32, Vec<u8>)> {
    if payload.len() < 4 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "truncated signal update payload",
        ));
    }
    let signal_id = u32::from_le_bytes(payload[0..4].try_into().unwrap());
    let value_bytes = payload[4..].to_vec();
    Ok((signal_id, value_bytes))
}

/// Encode an Error message payload.
pub fn encode_error(message: &str) -> Vec<u8> {
    message.as_bytes().to_vec()
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_roundtrip() {
        let mut buf = Vec::new();
        let payload = vec![1, 2, 3, 4];
        write_message(&mut buf, CosimMessageType::Connect, &payload).unwrap();
        let (msg_type, decoded) = read_message(&mut buf.as_slice()).unwrap().unwrap();
        assert_eq!(msg_type, CosimMessageType::Connect);
        assert_eq!(decoded, payload);
    }

    #[test]
    fn test_empty_payload() {
        let mut buf = Vec::new();
        write_message(&mut buf, CosimMessageType::Disconnect, &[]).unwrap();
        let (msg_type, decoded) = read_message(&mut buf.as_slice()).unwrap().unwrap();
        assert_eq!(msg_type, CosimMessageType::Disconnect);
        assert!(decoded.is_empty());
    }

    #[test]
    fn test_connect_encode_decode() {
        let signals = vec![
            CosimSignalInfo {
                name: "clk".to_string(),
                width: 1,
                direction: CosimDirection::Input,
            },
            CosimSignalInfo {
                name: "data_out".to_string(),
                width: 32,
                direction: CosimDirection::Output,
            },
        ];
        let payload = encode_connect(&signals);
        let decoded = decode_connect(&payload).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].name, "clk");
        assert_eq!(decoded[0].width, 1);
        assert_eq!(decoded[0].direction, CosimDirection::Input);
        assert_eq!(decoded[1].name, "data_out");
        assert_eq!(decoded[1].width, 32);
        assert_eq!(decoded[1].direction, CosimDirection::Output);
    }

    #[test]
    fn test_sync_encode_decode() {
        let payload = encode_sync(42);
        let time = decode_sync(&payload).unwrap();
        assert_eq!(time, 42);
    }

    #[test]
    fn test_signal_update_encode_decode() {
        let payload = encode_signal_update(5, &[0xAB, 0xCD]);
        let (id, bytes) = decode_signal_update(&payload).unwrap();
        assert_eq!(id, 5);
        assert_eq!(bytes, vec![0xAB, 0xCD]);
    }

    #[test]
    fn test_message_type_from_byte() {
        assert_eq!(
            CosimMessageType::from_byte(0x01),
            Some(CosimMessageType::Connect)
        );
        assert_eq!(
            CosimMessageType::from_byte(0x07),
            Some(CosimMessageType::Error)
        );
        assert_eq!(CosimMessageType::from_byte(0xFF), None);
    }

    #[test]
    fn test_direction_from_byte() {
        assert_eq!(CosimDirection::from_byte(0), Some(CosimDirection::Input));
        assert_eq!(CosimDirection::from_byte(2), Some(CosimDirection::Inout));
        assert_eq!(CosimDirection::from_byte(99), None);
    }

    #[test]
    fn test_truncated_payload_handling() {
        // Empty payload for decode_connect
        assert!(decode_connect(&[]).is_err());
        // Truncated sync
        assert!(decode_sync(&[1, 2, 3]).is_err());
        // Truncated signal update
        assert!(decode_signal_update(&[1, 2, 3]).is_err());
    }
}
