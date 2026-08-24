//! Analisis graph netlist (SYNTHESIS.md §13 — netlist optimization/level).
//!
//! Netlist harus DAG (acyclic, 1 driver / N loads). Modul ini menyediakan:
//! - `verify_dag()` — deteksi cycle + double-driver (kualitas netlist).
//! - `combinational_levels()` — level logika tiap sel (dari input port / FF-Q).
//! - `stats()` — ringkasan (jumlah sel/FF/net, fanout maks, level maks).

use crate::cell::CellId;
use crate::net::Netlist;

/// Hasil verifikasi struktur netlist.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DagCheck {
    pub ok: bool,
    pub cycles: Vec<String>,
    pub double_drivers: Vec<String>,
    pub floating: Vec<String>,
}

/// Statistik netlist (untuk report).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NetlistStats {
    pub cell_count: usize,
    pub comb_cells: usize,
    pub ff_cells: usize,
    pub net_count: usize,
    pub max_fanout: usize,
    pub max_level: usize,
}

/// Verifikasi netlist: DAG + satu driver per net + tidak ada net mengambang.
pub fn verify_dag(nl: &Netlist) -> DagCheck {
    let mut check = DagCheck {
        ok: true,
        ..Default::default()
    };
    let n = nl.cells.len();

    // 1. Double driver: net internal yang di-drive dua sel / sel+konstanta.
    let mut seen: Vec<Option<CellId>> = vec![None; nl.nets.len()];
    for (cid, c) in nl.cells.iter().enumerate() {
        for pin in &c.outputs {
            match seen[pin.net] {
                Some(prev) if prev != cid => {
                    check.ok = false;
                    check.double_drivers.push(format!(
                        "{} (sel {} & {})",
                        nl.nets[pin.net].name.as_str(),
                        prev,
                        cid
                    ));
                }
                _ => seen[pin.net] = Some(cid),
            }
        }
    }

    // 2. Net internal tanpa driver & tanpa konstanta (mengambang).
    for (id, net) in nl.nets.iter().enumerate() {
        let is_port = nl.ports.iter().any(|p| p.name == net.name);
        if net.driver.is_none()
            && net.const_value.is_none()
            && !is_port
            && !net.is_clock
            && !net.is_reset
        {
            check.ok = false;
            check
                .floating
                .push(format!("{} (net {})", net.name.as_str(), id));
        }
    }

    // 3. Cycle detection: DFS dari setiap sel (hanya jalur kombinasional).
    //    Level = 1 + max(level driver). Iterasi sampai stabil (DAG → selesai).
    let mut level = vec![0usize; n];
    for _ in 0..=n {
        let mut changed = false;
        for (cid, c) in nl.cells.iter().enumerate() {
            if c.kind.is_sequential() {
                continue; // FF memutus jalur kombinasional
            }
            // Level = 1 (sel itu sendiri) + level driver komb terbesar.
            // Input port/konstanta (tanpa driver sel) → kontribusi 0.
            let mut l = 1usize;
            for pin in &c.inputs {
                if let Some(d) = &nl.nets[pin.net].driver {
                    if !nl.cells[d.cell].kind.is_sequential() {
                        l = l.max(level[d.cell] + 1);
                    }
                }
            }
            if l != level[cid] {
                level[cid] = l;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // Ada cycle bila level masih berubah setelah n iterasi (tidak tercapai di
    // sini karena loop di atas berhenti saat stabil — cycle = tak pernah
    // stabil). Deteksi eksplisit: iterasi n+1 kali dan cek perubahan terakhir.
    for (cid, c) in nl.cells.iter().enumerate() {
        if c.kind.is_sequential() {
            continue;
        }
        let mut l = 1usize;
        for pin in &c.inputs {
            if let Some(d) = &nl.nets[pin.net].driver {
                if !nl.cells[d.cell].kind.is_sequential() {
                    l = l.max(level[d.cell] + 1);
                }
            }
        }
        if l != level[cid] {
            check.ok = false;
            check
                .cycles
                .push(format!("{} (sel {})", c.name.as_str(), cid));
        }
    }

    check
}

/// Level kombinasional maksimum tiap sel (0 = level FF/input).
pub fn combinational_levels(nl: &Netlist) -> Vec<usize> {
    let n = nl.cells.len();
    let mut level = vec![0usize; n];
    for _ in 0..=n {
        let mut changed = false;
        for (cid, c) in nl.cells.iter().enumerate() {
            if c.kind.is_sequential() {
                continue;
            }
            let mut l = 1usize;
            for pin in &c.inputs {
                if let Some(d) = &nl.nets[pin.net].driver {
                    if !nl.cells[d.cell].kind.is_sequential() {
                        l = l.max(level[d.cell] + 1);
                    }
                }
            }
            if l != level[cid] {
                level[cid] = l;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    level
}

fn stats_impl(nl: &Netlist, max_level: usize) -> NetlistStats {
    let mut s = NetlistStats {
        cell_count: nl.cells.len(),
        comb_cells: 0,
        ff_cells: nl.ff_count(),
        net_count: nl.nets.len(),
        max_fanout: 0,
        max_level,
    };
    for c in &nl.cells {
        if !c.kind.is_sequential() {
            s.comb_cells += 1;
        }
    }
    for net in &nl.nets {
        s.max_fanout = s.max_fanout.max(net.loads.len());
    }
    s
}

/// Statistik ringkas (dipakai emit summary).
pub fn stats(nl: &Netlist) -> NetlistStats {
    let levels = combinational_levels(nl);
    let max_level = levels.iter().copied().max().unwrap_or(0);
    stats_impl(nl, max_level)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::{CellInstance, CellKind, PinConn};
    use maria_core::intern::Symbol;

    fn two_gates() -> Netlist {
        // y = ~(a & b): AND lalu NOT — DAG 2 level.
        let mut nl = Netlist::new(Symbol::intern("top"));
        let a = nl.add_net(Symbol::intern("a"), 1);
        let b = nl.add_net(Symbol::intern("b"), 1);
        let t = nl.add_net(Symbol::intern("t"), 1);
        let y = nl.add_net(Symbol::intern("y"), 1);
        let mut and = CellInstance::new(Symbol::intern("u0"), CellKind::And, 1);
        and.inputs = vec![
            PinConn {
                net: a,
                pin: "a".into(),
                bit: None,
            },
            PinConn {
                net: b,
                pin: "b".into(),
                bit: None,
            },
        ];
        and.outputs = vec![PinConn {
            net: t,
            pin: "y".into(),
            bit: None,
        }];
        let mut not = CellInstance::new(Symbol::intern("u1"), CellKind::Not, 1);
        not.inputs = vec![PinConn {
            net: t,
            pin: "a".into(),
            bit: None,
        }];
        not.outputs = vec![PinConn {
            net: y,
            pin: "y".into(),
            bit: None,
        }];
        nl.add_cell(and);
        nl.add_cell(not);
        nl.add_port(Symbol::intern("a"), crate::net::PortDir::Input, 1);
        nl.add_port(Symbol::intern("b"), crate::net::PortDir::Input, 1);
        nl.add_port(Symbol::intern("y"), crate::net::PortDir::Output, 1);
        nl
    }

    #[test]
    fn dag_levels_and_stats() {
        let nl = two_gates();
        let check = verify_dag(&nl);
        assert!(check.ok, "{:?}", check);
        let levels = combinational_levels(&nl);
        assert_eq!(levels, vec![1, 2], "AND level 1, NOT level 2");
        let s = stats(&nl);
        assert_eq!(s.cell_count, 2);
        assert_eq!(s.comb_cells, 2);
        assert_eq!(s.ff_cells, 0);
        assert_eq!(s.max_level, 2);
        assert_eq!(s.max_fanout, 1);
    }

    #[test]
    fn dag_detects_floating_net() {
        let mut nl = two_gates();
        // Tambah net tanpa driver.
        nl.add_net(Symbol::intern("ghost"), 1);
        let check = verify_dag(&nl);
        assert!(!check.ok);
        assert!(!check.floating.is_empty());
    }
}
