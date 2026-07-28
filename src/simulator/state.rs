use crate::ir::{IrDesign, IrModule, LogicVec, ObjId, ObjectData, SignalId};
use std::collections::HashMap;

pub struct SimulationState {
    pub signals: Vec<LogicVec>,
    pub next_signals: Vec<LogicVec>,
    pub changed: Vec<bool>,
    pub time: u64,
    pub objects: Vec<ObjectData>,
    next_obj_id: ObjId,
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
        if id > 0 {
            self.check_obj_bounds(id);
        }
        self.objects.get(id)
    }

    pub fn get_object_mut(&mut self, id: ObjId) -> Option<&mut ObjectData> {
        if id > 0 {
            self.check_obj_bounds(id);
        }
        self.objects.get_mut(id)
    }

    #[inline(always)]
    fn check_signal_bounds(&self, id: SignalId) {
        if id >= self.signals.len() {
            panic!(
                "SimulationState::signal access: signal id {} out of bounds (signals.len={}, next_signals.len={}, changed.len={})",
                id,
                self.signals.len(),
                self.next_signals.len(),
                self.changed.len()
            );
        }
    }

    #[inline(always)]
    fn check_obj_bounds(&self, id: ObjId) {
        if id >= self.objects.len() {
            panic!(
                "SimulationState::object access: object id {} out of bounds (objects.len={})",
                id,
                self.objects.len()
            );
        }
    }

    pub fn read_signal(&self, id: SignalId) -> &LogicVec {
        self.check_signal_bounds(id);
        if self.changed[id] {
            &self.next_signals[id]
        } else {
            &self.signals[id]
        }
    }

    pub fn write_signal(&mut self, id: SignalId, val: LogicVec) {
        self.check_signal_bounds(id);
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
