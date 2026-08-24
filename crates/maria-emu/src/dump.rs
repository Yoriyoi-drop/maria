//! Output text MHIR — `dump_mhir` (struktur lengkap) dan `dump_memory_map`
//! (region ber-alamat). Dipakai CLI `maria emu --dump-mhir/--dump-memory-map`.

use crate::mhir::types::{ClockEdgeKind, MemoryKind, MhirDesign, PortDir};

fn region_str(base: u64, size: u64) -> String {
    let end = base.saturating_add(size).saturating_sub(1);
    if size == 0 {
        format!("0x{:08x}", base)
    } else {
        format!("0x{:08x}-0x{:08x}", base, end)
    }
}

fn dir_str(d: PortDir) -> &'static str {
    match d {
        PortDir::Input => "in",
        PortDir::Output => "out",
        PortDir::Inout => "inout",
    }
}

/// Dump MHIR lengkap: clock, reset, register, memory, device per module.
pub fn dump_mhir(mhir: &MhirDesign) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "MHIR — Maria Hardware IR (top: {}, modules: {})\n",
        mhir.top.as_str(),
        mhir.modules.len()
    ));
    for m in &mhir.modules {
        out.push_str(&format!(
            "\n── Module: {} ({} signals) ──\n",
            m.name.as_str(),
            m.signal_count
        ));
        if !m.clocks.is_empty() {
            out.push_str("  Clock:\n");
            for c in &m.clocks {
                let edge = match c.edge {
                    ClockEdgeKind::PosEdge => "posedge",
                    ClockEdgeKind::NegEdge => "negedge",
                };
                out.push_str(&format!("    {:<20} {}\n", c.name.as_str(), edge));
            }
        }
        if !m.resets.is_empty() {
            out.push_str("  Reset:\n");
            for r in &m.resets {
                let pol = if r.polarity {
                    "active-high"
                } else {
                    "active-low"
                };
                let sync = if r.async_ { "async" } else { "sync" };
                out.push_str(&format!(
                    "    {:<20} {} ({})\n",
                    r.signal.as_str(),
                    pol,
                    sync
                ));
            }
        }
        if !m.registers.is_empty() {
            out.push_str("  Register:\n");
            for r in &m.registers {
                out.push_str(&format!(
                    "    {:<20} [{}]  clk={} reset={}  @{}\n",
                    r.name.as_str(),
                    r.width,
                    r.clock.map(|c| c.as_str()).unwrap_or("-"),
                    r.reset.map(|s| s.as_str()).unwrap_or("-"),
                    r.back.display()
                ));
            }
        }
        if !m.memories.is_empty() {
            out.push_str("  Memory:\n");
            for mem in &m.memories {
                let kind = match mem.kind {
                    MemoryKind::Ram => "ram",
                    MemoryKind::Rom => "rom",
                };
                out.push_str(&format!(
                    "    {:<20} [{}] x {} ({})  @{}\n",
                    mem.name.as_str(),
                    mem.elem_width,
                    mem.depth,
                    kind,
                    mem.back.display()
                ));
            }
        }
        if !m.devices.is_empty() {
            out.push_str("  Device:\n");
            for d in &m.devices {
                let mmio = d
                    .mmio
                    .map(|r| region_str(r.base, r.size))
                    .unwrap_or_else(|| "-".to_string());
                let irq = d
                    .irq
                    .map(|i| i.to_string())
                    .unwrap_or_else(|| "-".to_string());
                let ports: Vec<String> = d
                    .ports
                    .iter()
                    .map(|p| format!("{} {}[{}]", p.name.as_str(), dir_str(p.direction), p.width))
                    .collect();
                let port_str = if ports.is_empty() {
                    String::new()
                } else {
                    format!("  ({})", ports.join(", "))
                };
                out.push_str(&format!(
                    "    {:<20} {:<6} ({})  mmio={} irq={}  @{}{}\n",
                    d.name.as_str(),
                    d.kind.label(),
                    d.module.as_str(),
                    mmio,
                    irq,
                    d.back.display(),
                    port_str
                ));
            }
        }
        if m.clocks.is_empty()
            && m.resets.is_empty()
            && m.registers.is_empty()
            && m.memories.is_empty()
            && m.devices.is_empty()
        {
            out.push_str("    (tidak ada clock/reset/register/memory/device terdeteksi)\n");
        }
    }
    out
}

/// Dump memory map: region ber-alamat (sorted by base) + region unassigned.
pub fn dump_memory_map(mhir: &MhirDesign) -> String {
    let mut out = String::new();
    out.push_str(&format!("Memory map — top: {}\n", mhir.top.as_str()));
    if mhir.address_map.is_empty() {
        out.push_str("  (kosong — assign alamat via --addr NAME=BASE:SIZE atau [[devices]] di config .meu)\n");
    } else {
        for (name, r) in &mhir.address_map {
            out.push_str(&format!(
                "  0x{:08x}-0x{:08x}  {:<20}\n",
                r.base,
                r.base + r.size - 1,
                name.as_str()
            ));
        }
    }

    // Region terdeteksi tapi belum ber-alamat.
    let mut unassigned: Vec<String> = Vec::new();
    for m in &mhir.modules {
        for mem in &m.memories {
            if !mhir
                .address_map
                .iter()
                .any(|(n, _)| n.as_str() == mem.name.as_str())
            {
                unassigned.push(format!(
                    "{} [{}] x {} ({})",
                    mem.name.as_str(),
                    mem.elem_width,
                    mem.depth,
                    match mem.kind {
                        MemoryKind::Ram => "ram",
                        MemoryKind::Rom => "rom",
                    }
                ));
            }
        }
        for d in &m.devices {
            if d.mmio.is_none() {
                unassigned.push(format!("{} ({})", d.name.as_str(), d.module.as_str()));
            }
        }
    }
    if !unassigned.is_empty() {
        unassigned.sort();
        unassigned.dedup();
        out.push_str("\nBelum ber-alamat:\n");
        for u in &unassigned {
            out.push_str(&format!("  {}\n", u));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mhir::types::*;
    use maria_core::intern::Symbol;

    fn sample_design() -> MhirDesign {
        let mut top = MhirModule {
            name: Symbol::intern("soc"),
            signal_count: 12,
            ..Default::default()
        };
        top.clocks.push(ClockDesc {
            name: Symbol::intern("clk"),
            signal_id: 0,
            edge: ClockEdgeKind::PosEdge,
            hier: None,
        });
        top.resets.push(ResetDesc {
            signal: Symbol::intern("rst_n"),
            polarity: false,
            async_: true,
        });
        top.registers.push(MhirRegister {
            name: Symbol::intern("pc"),
            width: 32,
            clock: Some(Symbol::intern("clk")),
            reset: Some(Symbol::intern("rst_n")),
            back: BackPointer::known(Some("cpu_core.sv".into()), 12, 3),
        });
        top.memories.push(MhirMemory {
            name: Symbol::intern("mem"),
            elem_width: 32,
            depth: 1024,
            dims: vec![1024],
            kind: MemoryKind::Ram,
            back: BackPointer::known(None, 40, 2),
        });
        top.devices.push(MhirDevice {
            name: Symbol::intern("u_uart"),
            module: Symbol::intern("uart"),
            kind: DeviceKind::Uart,
            ports: vec![
                PortDesc {
                    name: Symbol::intern("clk"),
                    direction: PortDir::Input,
                    width: 1,
                },
                PortDesc {
                    name: Symbol::intern("tx"),
                    direction: PortDir::Output,
                    width: 1,
                },
            ],
            mmio: Some(AddressRegion {
                base: 0x1000_0000,
                size: 0x1000,
            }),
            irq: Some(5),
            back: BackPointer::known(None, 55, 2),
        });
        MhirDesign {
            top: Symbol::intern("soc"),
            modules: vec![top],
            address_map: vec![(
                Symbol::intern("u_uart"),
                AddressRegion {
                    base: 0x1000_0000,
                    size: 0x1000,
                },
            )],
            source_file: None,
        }
    }

    #[test]
    fn test_dump_mhir_contains_sections() {
        let d = sample_design();
        let out = dump_mhir(&d);
        assert!(out.contains("MHIR — Maria Hardware IR"));
        assert!(out.contains("Module: soc"));
        assert!(out.contains("clk") && out.contains("posedge"));
        assert!(out.contains("rst_n") && out.contains("active-low") && out.contains("async"));
        assert!(out.contains("pc") && out.contains("[32]"));
        assert!(out.contains("mem") && out.contains("[32] x 1024") && out.contains("ram"));
        assert!(out.contains("u_uart") && out.contains("uart"));
        assert!(out.contains("0x10000000-0x10000fff"));
        assert!(out.contains("irq=5"));
        assert!(
            out.contains("cpu_core.sv:12:3"),
            "back-pointer file:line:col"
        );
    }

    #[test]
    fn test_dump_memory_map_sorted_and_unassigned() {
        let mut d = sample_design();
        // Tambah device tanpa alamat → masuk daftar "Belum ber-alamat".
        d.modules[0].devices.push(MhirDevice {
            name: Symbol::intern("u_timer"),
            module: Symbol::intern("timer"),
            kind: DeviceKind::Timer,
            ports: vec![],
            mmio: None,
            irq: None,
            back: BackPointer::default(),
        });
        let out = dump_memory_map(&d);
        assert!(out.contains("0x10000000-0x10000fff") && out.contains("u_uart"));
        assert!(out.contains("Belum ber-alamat"));
        assert!(out.contains("u_timer"));
        assert!(out.contains("mem"));
    }

    #[test]
    fn test_dump_memory_map_empty_hint() {
        let d = MhirDesign::default();
        let out = dump_memory_map(&d);
        assert!(out.contains("kosong"));
        assert!(out.contains("--addr"));
    }

    #[test]
    fn test_region_str() {
        assert_eq!(region_str(0x1000_0000, 0x1000), "0x10000000-0x10000fff");
        assert_eq!(region_str(0x8000_0000, 0), "0x80000000");
    }
}
