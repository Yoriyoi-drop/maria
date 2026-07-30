//! TCP server handler for co-simulation connections.
//!
//! Runs in a background thread. Accepts one external simulator connection
//! and exchanges signal values synchronously.

use super::protocol::*;
use super::CosimState;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

/// Handle a single co-simulation connection.
///
/// # Protocol Flow
///
/// 1. Client sends Connect (with signal list)
/// 2. Server sends ConnectAck
/// 3. Loop:
///    a. Server sends SignalUpdate for output signals
///    b. Server sends Sync (time step)
///    c. Client sends SyncAck
///    d. Client sends SignalUpdate for input signals
/// 4. Client sends Disconnect
pub fn handle_cosim_connection(
    stream: &mut TcpStream,
    state: &Arc<Mutex<CosimState>>,
    _signal_count: usize,
) -> std::io::Result<()> {
    // 1. Read Connect message
    let (msg_type, payload) = match read_message(stream)? {
        Some(m) => m,
        None => return Ok(()),
    };
    if msg_type != CosimMessageType::Connect {
        write_message(stream, CosimMessageType::Error, &encode_error("expected Connect"))?;
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "expected Connect"));
    }

    // Set read timeout to avoid blocking forever if external simulator stalls
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));

    let signals = match decode_connect(&payload) {
        Ok(s) => s,
        Err(e) => {
            write_message(stream, CosimMessageType::Error, &encode_error(&format!("invalid connect: {}", e)))?;
            return Err(e);
        }
    };

    if signals.is_empty() {
        write_message(stream, CosimMessageType::Error, &encode_error("no signals in connect"))?;
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "no signals"));
    }

    eprintln!("Co-simulation connected with {} signals", signals.len());

    // 2. Send ConnectAck
    write_message(stream, CosimMessageType::ConnectAck, &encode_connect_ack(0))?;

    // 3. Co-simulation loop
    loop {
        // a. Get output signals from engine and send them
        let out_signals = {
            if let Ok(s) = state.lock() {
                s.outgoing_signals.clone()
            } else {
                Vec::new()
            }
        };

        for (sig_id, val_bytes) in &out_signals {
            let update_payload = encode_signal_update(*sig_id, val_bytes);
            write_message(stream, CosimMessageType::SignalUpdate, &update_payload)?;
        }

        // b. Send Sync (next time step)
        let current_time = {
            if let Ok(s) = state.lock() {
                s.current_time
            } else {
                break;
            }
        };
        let sync_payload = encode_sync(current_time);
        write_message(stream, CosimMessageType::Sync, &sync_payload)?;

        // c. Read SyncAck
        let (ack_type, _ack_payload) = match read_message(stream)? {
            Some(m) => m,
            None => {
                eprintln!("Co-simulation client disconnected");
                break;
            }
        };
        match ack_type {
            CosimMessageType::SyncAck => {
                // Continue
            }
            CosimMessageType::Disconnect => {
                eprintln!("Co-simulation client sent disconnect");
                break;
            }
            CosimMessageType::Error => {
                eprintln!("Co-simulation client reported error");
                break;
            }
            _ => {
                // Unexpected message
                break;
            }
        }

        // d. Read SignalUpdate messages from client
        let mut incoming = Vec::new();
        loop {
            match read_message(stream)? {
                Some((CosimMessageType::SignalUpdate, update_payload)) => {
                    if let Ok((id, val)) = decode_signal_update(&update_payload) {
                        incoming.push((id, val));
                    }
                }
                Some((CosimMessageType::Sync, _)) => {
                    // Back-to-back Sync: client is done sending updates
                    // Put the Sync back... actually we need to handle this differently
                    break;
                }
                Some((CosimMessageType::Disconnect, _)) => {
                    if let Ok(mut s) = state.lock() {
                        s.incoming_signals = incoming;
                        s.data_ready = true;
                    }
                    eprintln!("Co-simulation client disconnected");
                    return Ok(());
                }
                None => {
                    return Ok(());
                }
                _ => break,
            }
        }

        // Store incoming signals
        if let Ok(mut s) = state.lock() {
            s.incoming_signals = incoming;
            s.data_ready = true;
        }
    }

    Ok(())
}
