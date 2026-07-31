//! Distributed Simulation Master Node.
//!
//! Master node bertanggung jawab untuk:
//! 1. Partisi desain via DesignPartitioner
//! 2. Setup TCP server dan terima koneksi dari slave nodes
//! 3. Kirim partition assignment ke setiap slave
//! 4. Koordinasi delta cycle: kirim Sync, terima hasil dari semua slave
//! 5. Exchange cross-partition signal values antar slave
//! 6. Collect final results

use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::ir::IrDesign;
use crate::simulator::distributed::partitioner::{DesignPartitioner, Partition};
use crate::simulator::distributed::protocol::*;

/// Status of a connected slave node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlaveStatus {
    Connecting,
    Ready,
    Simulating,
    Done,
    Error,
}

/// Information about a connected slave node.
#[derive(Debug)]
pub struct SlaveInfo {
    pub partition_id: usize,
    pub stream: Arc<Mutex<TcpStream>>,
    pub status: SlaveStatus,
    pub last_heartbeat: u64,
    pub num_deltas: u64,
}

/// Configuration for the distributed master node.
#[derive(Debug, Clone)]
pub struct MasterConfig {
    /// Port to listen on for slave connections.
    pub port: u16,
    /// Number of partitions (= number of expected slaves).
    pub num_partitions: usize,
    /// Connection timeout in seconds.
    pub timeout_secs: u64,
    /// Heartbeat interval in seconds.
    pub heartbeat_interval_secs: u64,
    /// Whether to print verbose log messages.
    pub verbose: bool,
}

impl Default for MasterConfig {
    fn default() -> Self {
        MasterConfig {
            port: 9876,
            num_partitions: 1,
            timeout_secs: 300,
            heartbeat_interval_secs: 5,
            verbose: true,
        }
    }
}

/// Distributed simulation master node.
pub struct DistributedMaster {
    config: MasterConfig,
    slaves: Vec<SlaveInfo>,
    partition_result: Option<Partition>,
}

impl DistributedMaster {
    pub fn new(config: MasterConfig) -> Self {
        DistributedMaster {
            config,
            slaves: Vec::new(),
            partition_result: None,
        }
    }

    /// Run the distributed simulation.
    ///
    /// 1. Partition the design
    /// 2. Accept connections from slaves (one per partition)
    /// 3. Send partition assignments
    /// 4. Coordinate delta cycles
    /// 5. Collect results
    pub fn run(&mut self, design: &IrDesign, max_time: u64) -> Result<(), String> {
        if self.config.verbose {
            eprintln!("[Master] Partitioning design into {} partitions...", self.config.num_partitions);
        }

        // Step 1: Partition the design
        let partition = DesignPartitioner::partition(design, self.config.num_partitions);
        self.partition_result = Some(partition.clone());

        if partition.num_partitions <= 1 {
            if self.config.verbose {
                eprintln!("[Master] Only 1 partition — running locally");
            }
            return Ok(()); // Single partition: run locally
        }

        // Step 2: Setup TCP server
        let listener = TcpListener::bind(format!("0.0.0.0:{}", self.config.port))
            .map_err(|e| format!("[Master] Cannot bind port {}: {}", self.config.port, e))?;

        listener.set_nonblocking(true)
            .map_err(|e| format!("[Master] Cannot set nonblocking: {}", e))?;

        if self.config.verbose {
            eprintln!("[Master] Listening on port {} for {} slaves...",
                self.config.port, partition.num_partitions);
        }

        // Step 3: Accept connections from all slaves
        let start_time = std::time::Instant::now();
        let timeout_duration = Duration::from_secs(self.config.timeout_secs);

        while self.slaves.len() < partition.num_partitions {
            if start_time.elapsed() > timeout_duration {
                return Err(format!("[Master] Timeout waiting for {} slaves (got {})",
                    partition.num_partitions, self.slaves.len()));
            }

            match listener.accept() {
                Ok((stream, addr)) => {
                    let slave_id = self.slaves.len();
                    if self.config.verbose {
                        eprintln!("[Master] Slave {} connected from {}", slave_id, addr);
                    }

                    let mut info = SlaveInfo {
                        partition_id: slave_id,
                        stream: Arc::new(Mutex::new(stream)),
                        status: SlaveStatus::Connecting,
                        last_heartbeat: 0,
                        num_deltas: 0,
                    };

                    // Send partition assignment
                    let assign = PartitionAssignment {
                        partition_id: slave_id as u32,
                        num_partitions: partition.num_partitions as u32,
                        num_signals: design.top.signals.len() as u32,
                        num_processes: 0,
                        cross_signals: partition.partitions.get(slave_id)
                            .map(|p| p.cross_signals.iter().map(|cs| {
                                (cs.signal_id as u32, cs.signal_id as u32, cs.width as u32)
                            }).collect())
                            .unwrap_or_default(),
                    };

                    let payload = encode_partition_assign(&assign);
                    {
                        let mut s = info.stream.lock().map_err(|e| format!("lock: {}", e))?;
                        write_message(&mut *s, 0x10, &payload)
                            .map_err(|e| format!("[Master] Send partition assign failed: {}", e))?;

                        // Read PartitionAck
                        let (msg_type, _) = read_message(&mut *s)
                            .map_err(|e| format!("[Master] Read ack failed: {}", e))?
                            .ok_or_else(|| "[Master] Slave disconnected during handshake".to_string())?;

                        if msg_type == 0x11 {
                            info.status = SlaveStatus::Ready;
                        } else {
                            return Err(format!("[Master] Unexpected msg type {:#x} from slave", msg_type));
                        }
                    }

                    self.slaves.push(info);
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    return Err(format!("[Master] Accept failed: {}", e));
                }
            }
        }

        if self.config.verbose {
            eprintln!("[Master] All {} slaves connected and ready.", self.slaves.len());
        }

        // Step 4: Coordinate simulation delta cycles
        let mut delta_id = 0u64;
        let mut current_time = 0u64;

        // Simple delta loop: for each time step, sync all slaves
        // (Heartbeat/fault tolerance is handled via faulty_slaves detection below)
        let mut faulty_slaves: Vec<usize> = Vec::new();
        while current_time < max_time {
            // Check for faulty slaves
            if !faulty_slaves.is_empty() {
                eprintln!("[Master] {} slave(s) faulty — continuing with remaining {} slaves",
                    faulty_slaves.len(), self.slaves.len() - faulty_slaves.len());
                if self.slaves.len() <= faulty_slaves.len() {
                    return Err("[Master] All slaves faulty — aborting".to_string());
                }
            }

            // Send DeltaSync to all non-faulty slaves
            for (idx, slave) in self.slaves.iter().enumerate() {
                if faulty_slaves.contains(&idx) {
                    continue;
                }
                let payload = encode_delta_sync(delta_id, current_time);
                match slave.stream.lock() {
                    Ok(mut s) => {
                        if write_message(&mut *s, 0x12, &payload).is_err() {
                            eprintln!("[Master] Slave {} disconnected — marking faulty", idx);
                            faulty_slaves.push(idx);
                        }
                    }
                    Err(e) => {
                        eprintln!("[Master] Slave {} lock error: {} — marking faulty", idx, e);
                        faulty_slaves.push(idx);
                    }
                }
            }

            // Collect SignalExchange from each non-faulty slave
            let mut all_exchange_data: Vec<Vec<SignalValue>> = Vec::new();
            for (idx, slave) in self.slaves.iter().enumerate() {
                if faulty_slaves.contains(&idx) {
                    continue;
                }
                match slave.stream.lock() {
                    Ok(mut s) => {
                        match read_message(&mut *s) {
                            Ok(Some((msg_type, payload))) => {
                                match msg_type {
                                    0x14 => {
                                        match decode_signal_exchange(&payload) {
                                            Ok(values) => all_exchange_data.push(values),
                                            Err(e) => eprintln!("[Master] Slave {} decode error: {}", idx, e),
                                        }
                                    }
                                    0x15 => {
                                        if self.config.verbose {
                                            eprintln!("[Master] Slave {} finished simulation", idx);
                                        }
                                    }
                                    0x16 => {
                                        // Heartbeat response — update timestamp
                                        if let Ok(ts) = decode_heartbeat(&payload) {
                                            if self.config.verbose && delta_id.is_multiple_of(1000) {
                                                eprintln!("[Master] Heartbeat from slave {} at ts={}", idx, ts);
                                            }
                                        }
                                    }
                                    _ => {
                                        eprintln!("[Master] Unexpected msg {:#x} from slave {}", msg_type, idx);
                                    }
                                }
                            }
                            Ok(None) => {
                                // EOF — slave disconnected
                                if !faulty_slaves.contains(&idx) {
                                    eprintln!("[Master] Slave {} disconnected (EOF) — marking faulty", idx);
                                    faulty_slaves.push(idx);
                                }
                            }
                            Err(e) => {
                                eprintln!("[Master] Slave {} read error: {} — marking faulty", idx, e);
                                if !faulty_slaves.contains(&idx) {
                                    faulty_slaves.push(idx);
                                }
                            }
                        }
                    }
                    Err(_) => {
                        if !faulty_slaves.contains(&idx) {
                            faulty_slaves.push(idx);
                        }
                    }
                }
            }

            // Broadcast cross-partition signals to all relevant slaves
            for (slave_idx, values) in all_exchange_data.iter().enumerate() {
                if faulty_slaves.contains(&slave_idx) {
                    continue;
                }
                for sv in values {
                    for (other_idx, other) in self.slaves.iter().enumerate() {
                        if other_idx == slave_idx || faulty_slaves.contains(&other_idx) {
                            continue;
                        }
                        if let Ok(mut s) = other.stream.lock() {
                            let exchange_payload = encode_signal_exchange(&[SignalValue {
                                signal_id: sv.signal_id,
                                value_bytes: sv.value_bytes.clone(),
                            }]);
                            let _ = write_message(&mut *s, 0x14, &exchange_payload);
                        }
                    }
                }
            }

            delta_id += 1;
            current_time += 1;

            if self.config.verbose && delta_id.is_multiple_of(1000) {
                eprintln!("[Master] Delta {} completed (time={}, slaves={})",
                    delta_id, current_time, self.slaves.len() - faulty_slaves.len());
            }
        }

        // All slaves completed or disconnected — done

        // Step 5: Send PartitionDone to all slaves
        for slave in &self.slaves {
            let payload = encode_delta_sync(delta_id, current_time); // reuse payload format
            let mut s = slave.stream.lock().map_err(|e| format!("lock: {}", e))?;
            write_message(&mut *s, 0x15, &payload)
                .map_err(|e| format!("[Master] Done notify failed: {}", e))?;
        }

        if self.config.verbose {
            eprintln!("[Master] Distributed simulation complete: {} deltas, time={}", delta_id, current_time);
        }

        Ok(())
    }

    /// Get the partition information after run.
    pub fn partition(&self) -> Option<&Partition> {
        self.partition_result.as_ref()
    }
}
