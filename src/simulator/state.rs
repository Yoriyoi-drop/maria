use crate::ir::{IrDesign, IrModule, LogicVec, ObjId, ObjectData, SignalId};
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
    pub time: u64,
    pub objects: Vec<ObjectData>,
    next_obj_id: ObjId,
    /// Fallback LogicVec returned for out-of-bounds read requests.
    /// Prevents panic; instead returns a zero-width X value.
    dummy_signal: LogicVec,
}

impl SimulationState {
    pub fn new(design: &IrDesign) -> Self {
        let mut signals = Vec::new();
        let mut next_signals = Vec::new();

        for sig in &design.top.signals {
            signals.push(sig.init_val.clone());
            next_signals.push(sig.init_val.clone());
        }

        let changed = vec![true; signals.len()];

        // Index 0 is reserved for null handle
        let objects = vec![ObjectData {
            class_name: crate::intern::Symbol::EMPTY,
            fields: HashMap::new(),
        }];

        SimulationState {
            signals,
            next_signals,
            changed,
            time: 0,
            objects,
            next_obj_id: 1,
            dummy_signal: LogicVec::new(1),
        }
    }

    pub fn alloc_object(&mut self, class_name: crate::intern::Symbol) -> ObjId {
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
            class_name: crate::intern::Symbol::EMPTY,
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
                "[WARN] SimulationState: signal id {} out of bounds (signals.len={}, next_signals.len={}, changed.len={})",
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
                "[WARN] SimulationState: object id {} out of bounds (objects.len={})",
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

    pub fn commit_changes(&mut self) -> Vec<(SignalId, LogicVec, LogicVec)> {
        let mut changed = Vec::new();
        for i in 0..self.signals.len() {
            if self.changed[i] {
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
