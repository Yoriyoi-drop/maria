//! MHIR — Maria Hardware IR (EMULATOR.md §4).
//!
//! Representasi hardware yang sudah diekstraksi dari `IrDesign`, tetap
//! menunjuk balik ke source RTL (back-pointer) agar debugger lintas-lapisan
//! (OS → bus → RTL → signal → baris source) bisa dibangun di atasnya.

pub mod backptr;
pub mod extract;
pub mod types;

pub use extract::{apply_address_map, extract};
pub use types::{
    AddressRegion, BackPointer, ClockDesc, ClockEdgeKind, DeviceKind, MhirDesign, MhirDevice,
    MhirMemory, MhirModule, MhirRegister, PortDesc, PortDir, ResetDesc,
};
