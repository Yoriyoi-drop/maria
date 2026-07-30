//! Distributed Simulation Slave Node.
//!
//! Slave node bertanggung jawab untuk:
//! 1. Connect ke master node via TCP
//! 2. Terima partition assignment
//! 3. Simulasikan partition secara independen (full run)
//! 4. Kirim signal values ke master setelah sim selesai
//! 5. Terima signal values dari partition lain

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

use crate::ir::IrDesign;
use crate::simulator::distributed::protocol::*;
use crate::simulator::SimulationEngine;

/// Configuration for a distributed slave node.
#[derive(Debug, Clone)]
pub struct SlaveConfig {
    /// Master hostname or IP.
    pub master_host: String,
    /// Master port.
    pub master_port: u16,
    /// Maximum simulation time.
    pub max_time: u64,
    /// Whether to print verbose log messages.
    pub verbose: bool,
}

impl Default for SlaveConfig {
    fn default() -> Self {
        SlaveConfig {
            master_host: "127.0.0.1".to_string(),
            master_port: 9876,
            max_time: 1000,
            verbose: true,
        }
    }
}

/// Distributed simulation slave node.
pub struct DistributedSlave {
    config: SlaveConfig,
    stream: Option<Arc<Mutex<TcpStream>>>,
    partition_id: u32,
}

impl DistributedSlave {
    pub fn new(config: SlaveConfig) -> Self {
        DistributedSlave {
            config,
            stream: None,
            partition_id: 0,
        }
    }

    /// Run the slave node: connect to master, receive partition, simulate fully.
    pub fn run(&mut self, design: &IrDesign) -> Result<(), String> {
        if self.config.verbose {
            eprintln!("[Slave] Connecting to master at {}:{}...",
                self.config.master_host, self.config.master_port);
        }

        // Step 1: Connect to master
        let stream = TcpStream::connect(format!("{}:{}",
            self.config.master_host, self.config.master_port))
            .map_err(|e| format!("[Slave] Cannot connect to master: {}", e))?;

        let stream = Arc::new(Mutex::new(stream));
        self.stream = Some(stream.clone());

        if self.config.verbose {
            eprintln!("[Slave] Connected to master");
        }

        // Step 2: Receive partition assignment
        let assignment = {
            let mut s = stream.lock().map_err(|e| format!("lock: {}", e))?;
            let (msg_type, payload) = read_message(&mut *s)
                .map_err(|e| format!("[Slave] Read partition assign failed: {}", e))?
                .ok_or_else(|| "[Slave] Master disconnected during handshake".to_string())?;

            if msg_type != 0x10 {
                return Err(format!("[Slave] Expected PartitionAssign (0x10), got {:#x}", msg_type));
            }

            let assign = decode_partition_assign(&payload)
                .map_err(|e| format!("[Slave] Decode partition assign: {}", e))?;

            // Send PartitionAck
            write_message(&mut *s, 0x11, &[0])
                .map_err(|e| format!("[Slave] Send ack failed: {}", e))?;

            if self.config.verbose {
                eprintln!("[Slave] Assigned partition {} of {} ({} signals, {} cross signals)",
                    assign.partition_id, assign.num_partitions,
                    assign.num_signals, assign.cross_signals.len());
            }

            assign
        };

        self.partition_id = assignment.partition_id;

        // Step 3: Extract sub-design for this partition
        // Phase 9 enhancement: only simulate the partition's portion of the design
        let partition = crate::simulator::distributed::partitioner::DesignPartitioner::partition(
            design, assignment.num_partitions as usize
        );
        let sub_design = if assignment.num_partitions > 1 {
            crate::simulator::distributed::partitioner::DesignPartitioner::extract_partition_design(
                design, assignment.partition_id as usize, &partition
            )
        } else {
            design.clone()
        };

        if self.config.verbose {
            eprintln!("[Slave] Running partition {} simulation (sub_design: {} signals, {} processes, max_time={})...",
                self.partition_id, sub_design.top.signals.len(), sub_design.top.processes.len(),
                self.config.max_time);
        }

        // Step 5: Run simulation on sub-design (heartbeats handled synchronously in main loop)
        let mut engine = SimulationEngine::new(sub_design, self.config.max_time);
        engine.run().map_err(|e| format!("[Slave] Simulation error: {}", e))?;

        if self.config.verbose {
            eprintln!("[Slave] Simulation complete. Sending results...");
        }

        // Step 6: Send final signal values to master (PartitionDone)
        let payload = encode_delta_sync(0, engine.state.time);
        {
            let mut s = stream.lock().map_err(|e| format!("lock: {}", e))?;
            write_message(&mut *s, 0x15, &payload)
                .map_err(|e| format!("[Slave] Send partition done failed: {}", e))?;
        }

        if self.config.verbose {
            eprintln!("[Slave] Done. Partition {} simulated {} time units ({} signals).",
                self.partition_id, engine.state.time, engine.design.top.signals.len());
        }

        Ok(())
    }
}
