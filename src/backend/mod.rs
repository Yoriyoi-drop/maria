//! Backend — simulator, waveform, coverage, debugger.
//! Sedang dalam migrasi dari src/simulator/ dan src/waveform/ ke struktur baru.

pub mod simulator {
    //! Re-export existing simulator
    pub use crate::simulator::*;
}

pub mod waveform {
    //! Re-export existing waveform
    pub use crate::waveform::*;
}
