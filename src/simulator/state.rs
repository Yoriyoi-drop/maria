use crate::ir::{LogicVec, SignalId, IrModule, IrDesign, ObjectData, ObjId};
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
        let objects = vec![ObjectData { class_name: String::new(), fields: HashMap::new() }];

        SimulationState { signals, next_signals, changed, time: 0, objects, next_obj_id: 1 }
    }

    pub fn alloc_object(&mut self, class_name: &str) -> ObjId {
        let id = self.next_obj_id;
        self.next_obj_id += 1;
        self.objects.push(ObjectData {
            class_name: class_name.to_string(),
            fields: HashMap::new(),
        });
        id
    }

    pub fn reset_objects(&mut self) {
        self.next_obj_id = 1;
        self.objects.clear();
        // Index 0 is reserved for null
        self.objects.push(ObjectData {
            class_name: String::new(),
            fields: HashMap::new(),
        });
    }

    pub fn get_object(&self, id: ObjId) -> Option<&ObjectData> {
        self.objects.get(id)
    }

    pub fn get_object_mut(&mut self, id: ObjId) -> Option<&mut ObjectData> {
        self.objects.get_mut(id)
    }

    pub fn read_signal(&self, id: SignalId) -> &LogicVec {
        if self.changed[id] {
            &self.next_signals[id]
        } else {
            &self.signals[id]
        }
    }

    pub fn write_signal(&mut self, id: SignalId, val: LogicVec) {
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
        module.signals.get(id)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| format!("sig_{}", id))
    }

    pub fn dump_all_signals(&self, module: &IrModule) {
        println!("--- Time {} ---", self.time);
        for sig in &module.signals {
            let val = self.read_signal(self.find_signal_id(&sig.name, module).unwrap_or(0));
            println!("  {} = {} ({}b)", sig.name, val, sig.width);
        }
    }

    fn find_signal_id(&self, name: &str, module: &IrModule) -> Option<SignalId> {
        module.signals.iter().position(|s| s.name == name)
    }
}
