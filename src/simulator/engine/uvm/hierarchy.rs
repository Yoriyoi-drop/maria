use super::super::SimulationEngine;
use crate::error::SimError;
use crate::ir::*;
use crate::Symbol;

impl SimulationEngine {
    pub(crate) fn find_phase_class_name(&self) -> Option<String> {
        let phase_methods = ["build_phase", "connect_phase", "run_phase"];
        let mut best: Option<(String, usize)> = None;
        for (name, cls) in &self.design.classes {
            if !self.is_uvm_test_hierarchy(name.as_str()) {
                continue;
            }
            let count = phase_methods
                .iter()
                .filter(|pm| cls.methods.iter().any(|m| &m.name == *pm))
                .count();
            if count > 0 && best.as_ref().is_none_or(|b| count > b.1) {
                best = Some((name.to_string(), count));
            }
        }
        // fallback: any class with phase methods
        if best.is_none() {
            for (name, cls) in &self.design.classes {
                let count = phase_methods
                    .iter()
                    .filter(|pm| cls.methods.iter().any(|m| &m.name == *pm))
                    .count();
                if count > 0 && best.as_ref().is_none_or(|b| count > b.1) {
                    best = Some((name.to_string(), count));
                }
            }
        }
        best.map(|(name, _)| name)
    }

    pub(crate) fn is_uvm_test_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_test" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn execute_phases(&mut self) -> Result<(), SimError> {
        let class_name = match self.find_phase_class_name() {
            Some(c) => c,
            None => return Ok(()),
        };
        if std::env::var("DBG_UVM").is_ok() {
            eprintln!("[DBG-UVM] phase class = {}", class_name);
        }
        // Create root test object once, shared across all phases
        let obj_id = self.state.alloc_object(Symbol::intern(class_name.as_str()));
        self.root_test_obj_id = Some(obj_id);

        // build_phase: root then children
        if self
            .find_method_in_hierarchy(&class_name, "build_phase")
            .is_ok()
        {
            self.current_this = Some(obj_id);
            if std::env::var("DBG_UVM").is_ok() {
                eprintln!("[DBG-UVM] calling build_phase");
            }
            self.execute_method(obj_id, "build_phase", &[])?;
            self.current_this = None;
            if std::env::var("DBG_UVM").is_ok() {
                eprintln!("[DBG-UVM] build_phase done");
            }
            self.call_phase_on_children(obj_id, "build_phase")?;
        }
        // connect_phase: root then children
        if self
            .find_method_in_hierarchy(&class_name, "connect_phase")
            .is_ok()
        {
            self.current_this = Some(obj_id);
            if std::env::var("DBG_UVM").is_ok() {
                eprintln!("[DBG-UVM] calling connect_phase");
            }
            self.execute_method(obj_id, "connect_phase", &[])?;
            self.current_this = None;
            if std::env::var("DBG_UVM").is_ok() {
                eprintln!("[DBG-UVM] connect_phase done");
            }
            self.call_phase_on_children(obj_id, "connect_phase")?;
        }
        // run_phase: call root's run_phase (blocking since delays in methods are no-ops)
        if self
            .find_method_in_hierarchy(&class_name, "run_phase")
            .is_ok()
        {
            self.current_this = Some(obj_id);
            if std::env::var("DBG_UVM").is_ok() {
                eprintln!("[DBG-UVM] calling run_phase");
            }
            self.execute_method(obj_id, "run_phase", &[])?;
            self.current_this = None;
            if std::env::var("DBG_UVM").is_ok() {
                eprintln!("[DBG-UVM] run_phase done");
            }
        }
        Ok(())
    }

    pub(crate) fn call_phase_on_children(&mut self, obj_id: ObjId, phase: &str) -> Result<(), SimError> {
        if let Some(cdata) = self.uvm_component_data.get(&obj_id) {
            let children = cdata.children.clone();
            for child_id in children {
                if let Some(obj) = self.state.get_object(child_id) {
                    let child_class = &obj.class_name;
                    if self.find_method_in_hierarchy(child_class.as_str(), phase).is_ok() {
                        self.current_this = Some(child_id);
                        self.execute_method(child_id, phase, &[])?;
                        self.current_this = None;
                    }
                }
                self.call_phase_on_children(child_id, phase)?;
            }
        }
        Ok(())
    }

    pub(crate) fn is_uvm_object_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_object" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_component_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_component" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_report_object_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_report_object" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_sequence_item_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_sequence_item" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_sequence_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_sequence" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_sequencer_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_sequencer" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_monitor_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_monitor" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_analysis_port_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_analysis_port" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_analysis_imp_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_analysis_imp" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_callback_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_callback" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_callbacks_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_callbacks" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_driver_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_driver" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_reg_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_reg" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_reg_field_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_reg_field" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_reg_block_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_reg_block" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_reg_map_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_reg_map" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

}
