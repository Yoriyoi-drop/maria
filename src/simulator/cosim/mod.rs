//! Co-simulation Bridge — VHDL/SystemVerilog inter-simulator communication.
//!
//! Provides a socket-based protocol for external simulators (GHDL, NVC, Questa, VCS)
//! to exchange signal values with Maria's simulation engine.
//!
//! # Protocol Overview
//!
//! The protocol uses a simple binary message format over TCP:
//! - Messages are length-prefixed (4-byte little-endian length header)
//! - Each message has a 1-byte type tag
//! - Payload follows the type tag
//!
//! # Message Types
//!
//! | Type | Code | Direction   | Payload                              |
//! |------|------|-------------|--------------------------------------|
//! | Connect  | 0x01 | Client→Server | signal_count, [signal_info]*        |
//! | ConnectAck | 0x02 | Server→Client | status (0=ok)                     |
//! | Sync     | 0x03 | Client→Server | time_step (u64)                    |
//! | SyncAck  | 0x04 | Server→Client | ready (u8)                         |
//! | SignalUpdate | 0x05 | Bidirectional | signal_id (u32), value_bytes     |
//! | Disconnect | 0x06 | Client→Server | (no payload)                     |
//! | Error    | 0x07 | Server→Client | error_message (UTF-8 string)        |
//!
//! # Signal Info format
//!
//! Each signal info entry:
//! - name_len (u32): length of signal name
//! - name (UTF-8): signal name string
//! - width (u32): signal width in bits
//! - direction (u8): 0=input, 1=output, 2=inout
//!
//! # Usage
//!
//! ```bash
//! cargo run -- test.sv --cosim-port 9876
//! ```
//!
//! Then connect from GHDL/VHDL side:
//! ```vhdl
//! -- In VHDL, use VHPIDIRECT or a C wrapper to connect to the socket
//! ```

mod protocol;
mod server;

pub use protocol::*;
pub use server::*;

use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// Shared state between the co-simulation server and the engine.
pub struct CosimState {
    /// Port the TCP server is listening on.
    pub port: u16,
    /// Whether the co-simulation bridge is active.
    pub active: bool,
    /// Current simulation time shared with external simulator.
    pub current_time: u64,
    /// Signal values to send to external simulator (output signals).
    pub outgoing_signals: Vec<(u32, Vec<u8>)>,
    /// Signal values received from external simulator (input signals).
    pub incoming_signals: Vec<(u32, Vec<u8>)>,
    /// Whether new data is available from external simulator.
    pub data_ready: bool,
}

impl CosimState {
    pub fn new(port: u16) -> Self {
        CosimState {
            port,
            active: false,
            current_time: 0,
            outgoing_signals: Vec::new(),
            incoming_signals: Vec::new(),
            data_ready: false,
        }
    }
}

/// Run the co-simulation server in a background thread.
/// Returns a shared state that the engine can poll for incoming signals.
pub fn start_cosim_server(port: u16, signal_count: usize) -> Option<Arc<Mutex<CosimState>>> {
    let state = Arc::new(Mutex::new(CosimState::new(port)));
    let state_clone = state.clone();

    std::thread::spawn(move || {
        let listener = match TcpListener::bind(format!("127.0.0.1:{}", port)) {
            Ok(l) => {
                if let Ok(mut s) = state_clone.lock() {
                    s.active = true;
                }
                eprintln!("Co-simulation server listening on port {}", port);
                l
            }
            Err(e) => {
                eprintln!("Co-simulation server failed to bind port {}: {}", port, e);
                return;
            }
        };

        // Accept one connection (external simulator)
        match listener.accept() {
            Ok((mut stream, addr)) => {
                eprintln!("Co-simulation connection from {}", addr);
                if let Err(e) = server::handle_cosim_connection(&mut stream, &state_clone, signal_count) {
                    eprintln!("Co-simulation error: {}", e);
                }
            }
            Err(e) => {
                eprintln!("Co-simulation accept failed: {}", e);
            }
        }

        if let Ok(mut s) = state_clone.lock() {
            s.active = false;
        }
        eprintln!("Co-simulation server stopped");
    });

    Some(state)
}
