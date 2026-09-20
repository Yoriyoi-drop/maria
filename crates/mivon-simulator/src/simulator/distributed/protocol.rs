//! Extended binary protocol for distributed simulation.
//!
//! # Message Types (extending cosim::protocol)
//!
//! | Type | Code | Direction   | Payload                              |
//! |------|------|-------------|--------------------------------------|
//! | PartitionAssign | 0x10 | Master→Slave | partition_data (JSON-like)       |
//! | PartitionAck    | 0x11 | Slave→Master | status (u8)                      |
//! | DeltaSync       | 0x12 | Master→Slave | delta_id (u64), time (u64)       |
//! | DeltaSyncAck    | 0x13 | Slave→Master | delta_id (u64), ready (u8)       |
//! | SignalExchange  | 0x14 | Bidirectional | [signal_id, value_bytes]*        |
//! | PartitionDone   | 0x15 | Slave→Master | time (u64)                       |
//! | Heartbeat       | 0x16 | Bidirectional | timestamp (u64)                  |

use std::io::{Read, Write};

/// Extended message types for distributed simulation.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistMessageType {
    PartitionAssign = 0x10,
    PartitionAck = 0x11,
    DeltaSync = 0x12,
    DeltaSyncAck = 0x13,
    SignalExchange = 0x14,
    PartitionDone = 0x15,
    Heartbeat = 0x16,
}

impl DistMessageType {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x10 => Some(DistMessageType::PartitionAssign),
            0x11 => Some(DistMessageType::PartitionAck),
            0x12 => Some(DistMessageType::DeltaSync),
            0x13 => Some(DistMessageType::DeltaSyncAck),
            0x14 => Some(DistMessageType::SignalExchange),
            0x15 => Some(DistMessageType::PartitionDone),
            0x16 => Some(DistMessageType::Heartbeat),
            _ => None,
        }
    }
}

/// Signal value exchange entry.
#[derive(Debug, Clone)]
pub struct SignalValue {
    pub signal_id: u32,
    pub value_bytes: Vec<u8>,
}

/// Partition assignment data sent from master to slave.
#[derive(Debug, Clone)]
pub struct PartitionAssignment {
    pub partition_id: u32,
    pub num_partitions: u32,
    pub num_signals: u32,
    pub num_processes: u32,
    /// Cross-partition signal mapping: (local_id, remote_id, width)
    pub cross_signals: Vec<(u32, u32, u32)>,
}

// ─── Write/Read helpers ───

/// Write a length-prefixed message.
pub fn write_message(stream: &mut impl Write, msg_type: u8, payload: &[u8]) -> std::io::Result<()> {
    let len = payload.len() as u32 + 1;
    let header = len.to_le_bytes();
    stream.write_all(&header)?;
    stream.write_all(&[msg_type])?;
    stream.write_all(payload)?;
    stream.flush()?;
    Ok(())
}

/// Read a length-prefixed message.
pub fn read_message(stream: &mut impl Read) -> std::io::Result<Option<(u8, Vec<u8>)>> {
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
    Ok(Some((buf[0], buf[1..].to_vec())))
}

// ─── Payload Encoders/Decoders ───

/// Encode PartitionAssign payload.
pub fn encode_partition_assign(assign: &PartitionAssignment) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&assign.partition_id.to_le_bytes());
    payload.extend_from_slice(&assign.num_partitions.to_le_bytes());
    payload.extend_from_slice(&assign.num_signals.to_le_bytes());
    payload.extend_from_slice(&assign.num_processes.to_le_bytes());
    let n = assign.cross_signals.len() as u32;
    payload.extend_from_slice(&n.to_le_bytes());
    for (local_id, remote_id, width) in &assign.cross_signals {
        payload.extend_from_slice(&local_id.to_le_bytes());
        payload.extend_from_slice(&remote_id.to_le_bytes());
        payload.extend_from_slice(&width.to_le_bytes());
    }
    payload
}

/// Decode PartitionAssign payload.
pub fn decode_partition_assign(payload: &[u8]) -> std::io::Result<PartitionAssignment> {
    if payload.len() < 16 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "truncated partition assign",
        ));
    }
    let mut offset = 0;
    let partition_id = u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap());
    offset += 4;
    let num_partitions = u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap());
    offset += 4;
    let num_signals = u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap());
    offset += 4;
    let num_processes = u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap());
    offset += 4;
    let n = u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap());
    offset += 4;
    let mut cross_signals = Vec::with_capacity(n as usize);
    for _ in 0..n {
        if offset + 12 > payload.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "truncated cross signals",
            ));
        }
        let local_id = u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap());
        offset += 4;
        let remote_id = u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap());
        offset += 4;
        let width = u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap());
        offset += 4;
        cross_signals.push((local_id, remote_id, width));
    }
    Ok(PartitionAssignment {
        partition_id,
        num_partitions,
        num_signals,
        num_processes,
        cross_signals,
    })
}

/// Encode DeltaSync payload.
pub fn encode_delta_sync(delta_id: u64, sim_time: u64) -> Vec<u8> {
    let mut payload = Vec::with_capacity(16);
    payload.extend_from_slice(&delta_id.to_le_bytes());
    payload.extend_from_slice(&sim_time.to_le_bytes());
    payload
}

/// Decode DeltaSync payload.
pub fn decode_delta_sync(payload: &[u8]) -> std::io::Result<(u64, u64)> {
    if payload.len() < 16 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "truncated delta sync",
        ));
    }
    let delta_id = u64::from_le_bytes(payload[0..8].try_into().unwrap());
    let sim_time = u64::from_le_bytes(payload[8..16].try_into().unwrap());
    Ok((delta_id, sim_time))
}

/// Encode SignalExchange payload from a list of signal values.
pub fn encode_signal_exchange(values: &[SignalValue]) -> Vec<u8> {
    let mut payload = Vec::new();
    let n = values.len() as u32;
    payload.extend_from_slice(&n.to_le_bytes());
    for sv in values {
        payload.extend_from_slice(&sv.signal_id.to_le_bytes());
        let len = sv.value_bytes.len() as u32;
        payload.extend_from_slice(&len.to_le_bytes());
        payload.extend_from_slice(&sv.value_bytes);
    }
    payload
}

/// Decode SignalExchange payload.
pub fn decode_signal_exchange(payload: &[u8]) -> std::io::Result<Vec<SignalValue>> {
    if payload.len() < 4 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "truncated signal exchange",
        ));
    }
    let n = u32::from_le_bytes(payload[0..4].try_into().unwrap()) as usize;
    let mut values = Vec::with_capacity(n);
    let mut offset = 4;
    for _ in 0..n {
        if offset + 8 > payload.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "truncated signal entry",
            ));
        }
        let signal_id = u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap());
        offset += 4;
        let len = u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        if offset + len > payload.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "truncated signal value",
            ));
        }
        let value_bytes = payload[offset..offset + len].to_vec();
        offset += len;
        values.push(SignalValue {
            signal_id,
            value_bytes,
        });
    }
    Ok(values)
}

/// Encode a Heartbeat payload.
pub fn encode_heartbeat(timestamp: u64) -> Vec<u8> {
    timestamp.to_le_bytes().to_vec()
}

/// Decode a Heartbeat payload.
pub fn decode_heartbeat(payload: &[u8]) -> std::io::Result<u64> {
    if payload.len() < 8 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "truncated heartbeat",
        ));
    }
    Ok(u64::from_le_bytes(payload[0..8].try_into().unwrap()))
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_roundtrip() {
        let mut buf = Vec::new();
        let payload = vec![0xAB, 0xCD, 0xEF];
        write_message(&mut buf, 0x10, &payload).unwrap();
        let (msg_type, decoded) = read_message(&mut buf.as_slice()).unwrap().unwrap();
        assert_eq!(msg_type, 0x10);
        assert_eq!(decoded, payload);
    }

    #[test]
    fn test_partition_assign_roundtrip() {
        let assign = PartitionAssignment {
            partition_id: 1,
            num_partitions: 4,
            num_signals: 100,
            num_processes: 50,
            cross_signals: vec![(0, 10, 8), (1, 11, 1), (2, 12, 32)],
        };
        let payload = encode_partition_assign(&assign);
        let decoded = decode_partition_assign(&payload).unwrap();
        assert_eq!(decoded.partition_id, 1);
        assert_eq!(decoded.num_partitions, 4);
        assert_eq!(decoded.num_signals, 100);
        assert_eq!(decoded.num_processes, 50);
        assert_eq!(decoded.cross_signals.len(), 3);
        assert_eq!(decoded.cross_signals[0], (0, 10, 8));
    }

    #[test]
    fn test_delta_sync_roundtrip() {
        let payload = encode_delta_sync(42, 1000);
        let (delta_id, sim_time) = decode_delta_sync(&payload).unwrap();
        assert_eq!(delta_id, 42);
        assert_eq!(sim_time, 1000);
    }

    #[test]
    fn test_signal_exchange_roundtrip() {
        let values = vec![
            SignalValue {
                signal_id: 0,
                value_bytes: vec![0x00, 0x01],
            },
            SignalValue {
                signal_id: 1,
                value_bytes: vec![0xFF],
            },
            SignalValue {
                signal_id: 5,
                value_bytes: vec![0xAB, 0xCD, 0xEF, 0x01],
            },
        ];
        let payload = encode_signal_exchange(&values);
        let decoded = decode_signal_exchange(&payload).unwrap();
        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[0].signal_id, 0);
        assert_eq!(decoded[0].value_bytes, vec![0x00, 0x01]);
        assert_eq!(decoded[1].signal_id, 1);
        assert_eq!(decoded[2].signal_id, 5);
    }

    #[test]
    fn test_heartbeat_roundtrip() {
        let payload = encode_heartbeat(1234567890);
        let ts = decode_heartbeat(&payload).unwrap();
        assert_eq!(ts, 1234567890);
    }

    #[test]
    fn test_truncated_payloads() {
        assert!(decode_partition_assign(&[0; 4]).is_err());
        assert!(decode_delta_sync(&[0; 4]).is_err());
        assert!(decode_signal_exchange(&[0; 2]).is_err());
        assert!(decode_heartbeat(&[0; 4]).is_err());
    }

    #[test]
    fn test_empty_signal_exchange() {
        let payload = encode_signal_exchange(&[]);
        let decoded = decode_signal_exchange(&payload).unwrap();
        assert!(decoded.is_empty());
    }
}
