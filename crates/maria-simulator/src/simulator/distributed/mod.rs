//! Distributed Simulation Engine — multi-instance Maria over TCP.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐     TCP      ┌─────────────┐
//! │   Master    │◄───────────►│  Slave 1     │
//! │  Node       │             │  (Partition) │
//! │             │     TCP      ├─────────────┤
//! │ Coordinator │◄───────────►│  Slave 2     │
//! │             │             │  (Partition) │
//! │             │     TCP      ├─────────────┤
//! │             │◄───────────►│  Slave N     │
//! └─────────────┘             └─────────────┘
//! ```
//!
//! Master node:
//! 1. Partisi desain via DesignPartitioner
//! 2. Kirim partitions ke slave nodes
//! 3. Sinkronisasi delta cycle via Sync/SyncAck
//! 4. Exchange cross-partition signal values
//!
//! Slave node:
//! 1. Terima partition dari master
//! 2. Simulasikan partition secara independen
//! 3. Kirim signal values ke master tiap delta
//! 4. Terima signal values dari partition lain

pub mod partitioner;
pub mod protocol;

mod master;
mod slave;

pub use master::{DistributedMaster, MasterConfig};
pub use partitioner::{DesignPartitioner, Partition, PartitionInfo, PartitionSignal};
pub use protocol::*;
pub use slave::{DistributedSlave, SlaveConfig};
