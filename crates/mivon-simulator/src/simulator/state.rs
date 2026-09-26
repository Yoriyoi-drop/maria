use mivon_ir::{IrDesign, IrModule, LogicVec, ObjId, ObjectData, SignalId};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

/// Per-signal delay information from SDF annotation.
#[derive(Debug, Clone)]
pub struct SignalDelay {
    pub rise: u64, // rise delay in ps
    pub fall: u64, // fall delay in ps
}

pub struct SimulationState {
    pub signals: Vec<LogicVec>,
    pub next_signals: Vec<LogicVec>,
    pub changed: Vec<bool>,
    /// Sinyal besar (width ≥ `LogicVec::LAZY_ZERO_THRESHOLD`) yang SUDAH
    /// pernah ditulis nyata (bukan default X). Sinyal besar yang belum
    /// ter-tulis di-snapshot LAZY (tanpa materialisasi) selama evaluasi —
    /// array memory flat (cache 65536x32x512 = 1.07e9 entry) dibaca tiap
    /// delta tapi TIDAK perlu di-clone penuh → puncak RSS turun drastis.
    pub large_dirty: Vec<bool>,
    /// LANG-08: net alias redirect — member SignalId → canonical SignalId.
    /// `alias a = b;` → net_aliases {b: a} (canonical = id terkecil); read/
    /// write member di-direct ke canonical sehingga semua anggota satu
    /// jaringan (short). Identity default (id → id).
    pub alias_redirect: Vec<SignalId>,
    pub time: u64,
    pub objects: Vec<ObjectData>,
    next_obj_id: ObjId,
    /// Fallback LogicVec returned for out-of-bounds read requests.
    /// Prevents panic; instead returns a zero-width X value.
    dummy_signal: LogicVec,
    /// Format untuk `%t` (dari `$timeformat`).
    pub timeformat: crate::simulator::types::TimeFormat,
}

/// Nilai state awal sebuah sinyal dari `init_val` (SignalInfo). Untuk array
/// memory flat raksasa (cache 65536x32x512 = 1.07e9 entry) `init_val.clone()`
/// = materialisasi penuh (1-3 GB/OOM mesin kecil). Default-nya X (memori tak
/// terinisialisasi) → pakai lazy-zero X tanpa copy: driver realistis selalu
/// menulis SEBELUM membaca slot memory; pada akses nyata halaman ter-write
/// kala itu juga. Initializer non-X (jarang utk array besar) tetap di-clone.
fn state_init_signal(init_val: &mivon_core::LogicVec) -> mivon_core::LogicVec {
    if init_val.width >= mivon_core::LogicVec::LAZY_ZERO_THRESHOLD
        && init_val
            .bits
            .iter()
            .take(8)
            .all(|b| *b == mivon_core::LogicVal::X)
    {
        return mivon_core::LogicVec::new(init_val.width);
    }
    init_val.clone()
}

impl SimulationState {
    pub fn new(design: &IrDesign) -> Self {
        let mut signals = Vec::new();
        let mut next_signals = Vec::new();

        for sig in &design.top.signals {
            signals.push(state_init_signal(&sig.init_val));
            next_signals.push(state_init_signal(&sig.init_val));
        }

        let changed = vec![true; signals.len()];
        let large_dirty = vec![false; signals.len()];
        // LANG-08: net alias redirect — canonical utk tiap member (identity
        // default); member yang di-alias di-direct ke canonical.
        let mut alias_redirect: Vec<SignalId> = (0..signals.len()).collect();
        for (member, canonical) in &design.net_aliases {
            if *member < signals.len() && *canonical < signals.len() {
                alias_redirect[*member] = *canonical;
            }
        }

        // Index 0 is reserved for null handle
        let objects = vec![ObjectData {
            class_name: mivon_core::intern::Symbol::EMPTY,
            fields: HashMap::new(),
        }];

        // Seed basis unit %t dari timescale desain (mis. 1ps → -12).
        // Tanpa ini, %t mengasumsikan basis 1ns untuk desain 1ps/1us.
        let mut timeformat = crate::simulator::types::TimeFormat::default();
        if let Some((ref unit, _)) = design.timescale {
            if let Some(exp) = crate::simulator::types::TimeFormat::unit_exponent(unit) {
                timeformat.base_units = exp;
            }
        }

        SimulationState {
            signals,
            next_signals,
            changed,
            large_dirty,
            alias_redirect,
            time: 0,
            objects,
            next_obj_id: 1,
            dummy_signal: LogicVec::new(1),
            timeformat,
        }
    }

    pub fn alloc_object(&mut self, class_name: mivon_core::intern::Symbol) -> ObjId {
        let id = self.next_obj_id;
        self.next_obj_id += 1;
        self.objects.push(ObjectData {
            class_name,
            fields: HashMap::new(),
        });
        id
    }

    pub fn reset_objects(&mut self) {
        self.next_obj_id = 1;
        self.objects.clear();
        // Index 0 is reserved for null
        self.objects.push(ObjectData {
            class_name: mivon_core::intern::Symbol::EMPTY,
            fields: HashMap::new(),
        });
    }

    pub fn get_object(&self, id: ObjId) -> Option<&ObjectData> {
        if id > 0 && !self.check_obj_bounds(id) {
            return None;
        }
        self.objects.get(id)
    }

    pub fn get_object_mut(&mut self, id: ObjId) -> Option<&mut ObjectData> {
        if id > 0 && !self.check_obj_bounds(id) {
            return None;
        }
        self.objects.get_mut(id)
    }

    /// Returns false if id is out of bounds, emitting at most one warning per process lifetime.
    #[inline(always)]
    fn check_signal_bounds(&self, id: SignalId) -> bool {
        if id < self.signals.len() {
            return true;
        }
        static WARNED: AtomicBool = AtomicBool::new(false);
        if !WARNED.swap(true, Ordering::Relaxed) {
            eprintln!(
                "[WARN] (internal) SimulationState: signal id {} out of bounds (signals.len={}, next_signals.len={}, changed.len={}) — bukan dari source HDL, tidak ada lokasi source",
                id,
                self.signals.len(),
                self.next_signals.len(),
                self.changed.len()
            );
        }
        false
    }

    /// Returns false if id is out of bounds, emitting at most one warning per process lifetime.
    #[inline(always)]
    fn check_obj_bounds(&self, id: ObjId) -> bool {
        if id < self.objects.len() {
            return true;
        }
        static WARNED: AtomicBool = AtomicBool::new(false);
        if !WARNED.swap(true, Ordering::Relaxed) {
            eprintln!(
                "[WARN] (internal) SimulationState: object id {} out of bounds (objects.len={}) — bukan dari source HDL, tidak ada lokasi source",
                id,
                self.objects.len()
            );
        }
        false
    }

    pub fn read_signal(&self, id: SignalId) -> &LogicVec {
        if !self.check_signal_bounds(id) {
            return &self.dummy_signal;
        }
        // LANG-08: net alias — baca member → baca canonical (nilai sama).
        let id = self.alias_redirect.get(id).copied().unwrap_or(id);
        if self.changed[id] {
            &self.next_signals[id]
        } else {
            &self.signals[id]
        }
    }

    pub fn write_signal(&mut self, id: SignalId, val: LogicVec) {
        if !self.check_signal_bounds(id) {
            return; // silently drop
        }
        // LANG-08: net alias — tulis ke canonical (semua anggota satu jaringan).
        let id = self.alias_redirect.get(id).copied().unwrap_or(id);
        // Sinyal besar yang menerima tulis nyata → tandai dirty (snapshot
        // berikutnya harus clone kenyataan, bukan lazy X).
        if self.large_dirty.len() > id
            && self.signals[id].width >= mivon_core::LogicVec::LAZY_ZERO_THRESHOLD
        {
            self.large_dirty[id] = true;
        }
        // Compare against pending (next_signals) if already changed this delta,
        // otherwise compare against committed (signals)
        if self.changed[id] {
            if self.next_signals[id] != val {
                self.next_signals[id] = val;
            }
        } else if self.signals[id] != val {
            self.next_signals[id] = val;
            self.changed[id] = true;
        }
    }

    /// Snapshot nilai sinyal utk evaluasi/preponed/history. Sinyal ultra-lebar
    /// (array memori flat) SELALU di-snapshot LAZY (tanpa materialisasi):
    /// - par-eval utk array raksasa SUDAH di-disable (`has_wide_signals` →
    ///   sequential) sehingga snapshot tidak pernah dipakai utk nilai evaluasi;
    /// - evaluasi sequential membaca state langsung;
    /// - preponed/signal_history hanya butuh skalar/edge (array tak relevan).
    ///
    /// Klon penuh 1e9 bit per delta = OOM mesin kecil.
    #[inline]
    pub fn snapshot_signal(&self, id: SignalId) -> LogicVec {
        let lv = if self.changed[id] {
            &self.next_signals[id]
        } else {
            &self.signals[id]
        };
        if lv.width >= mivon_core::LogicVec::LAZY_ZERO_THRESHOLD {
            return mivon_core::LogicVec::new(lv.width);
        }
        lv.clone()
    }

    /// Tulis segmen sinyal langsung ke pending (`next`) tanpa clone penuh —
    /// utk array memori flat ultra-lebar (RMW penuh = clone 1e9 bit/OOM).
    /// `start..end` (end exclusive) diisi dari `val.bits` (sebanyak len);
    /// out-of-bounds mengikuti LRM (tulis dibatasi panjang, OOB diabaikan).
    #[inline]
    pub fn write_signal_slice(&mut self, id: SignalId, start: usize, end: usize, val: &LogicVec) {
        let id = self.alias_redirect.get(id).copied().unwrap_or(id);
        if id >= self.next_signals.len() {
            return;
        }
        if self.large_dirty.len() > id
            && self.next_signals[id].width >= mivon_core::LogicVec::LAZY_ZERO_THRESHOLD
        {
            self.large_dirty[id] = true;
        }
        let n = val.bits.len();
        let mut end = end.min(start.saturating_add(n));
        if end < start {
            return;
        }
        let target = &mut self.next_signals[id];
        if end > target.bits.len() {
            end = target.bits.len();
        }
        if start < end {
            target.bits[start..end].copy_from_slice(&val.bits[..end - start]);
        }
        self.changed[id] = true;
    }

    pub fn commit_changes(&mut self) -> Vec<(SignalId, LogicVec, LogicVec)> {
        let mut changed = Vec::new();
        for i in 0..self.signals.len() {
            if self.changed[i] {
                if self.signals[i].width >= mivon_core::LogicVec::LAZY_ZERO_THRESHOLD {
                    // Array memory flat besar: pindahkan OWNERSHIP (swap, O(1))
                    // tanpa clone 1e9 bit (spike 3 GB/OOM saat miss tulis).
                    // next_signals di-reset lazy-X untuk delta berikutnya.
                    let w = self.next_signals[i].width;
                    let old = mivon_core::LogicVec::new(w);
                    std::mem::swap(&mut self.signals[i], &mut self.next_signals[i]);
                    self.next_signals[i] = mivon_core::LogicVec::new(w);
                    self.changed[i] = false;
                    changed.push((i, old, mivon_core::LogicVec::new(w)));
                    self.large_dirty[i] = true;
                    continue;
                }
                let old = self.signals[i].clone();
                let new = self.next_signals[i].clone();
                self.signals[i] = new.clone();
                self.next_signals[i] = new.clone();
                self.changed[i] = false;
                if self.signals[i] != old {
                    changed.push((i, old, self.signals[i].clone()));
                }
            }
        }
        changed
    }

    pub fn advance_time(&mut self) {
        self.time += 1;
    }

    pub fn signal_name(&self, id: SignalId, module: &IrModule) -> String {
        module
            .signals
            .get(id)
            .map(|s| s.name.to_string())
            .unwrap_or_else(|| format!("sig_{}", id))
    }

    pub fn dump_all_signals(&self, module: &IrModule) {
        println!("--- Time {} ---", self.time);
        for sig in &module.signals {
            let val = self.read_signal(self.find_signal_id(sig.name.as_str(), module).unwrap_or(0));
            println!("  {} = {} ({}b)", sig.name, val, sig.width);
        }
    }

    fn find_signal_id(&self, name: &str, module: &IrModule) -> Option<SignalId> {
        module.signals.iter().position(|s| s.name.as_str() == name)
    }
}
