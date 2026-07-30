//! UPF (Unified Power Format, IEEE 1801) power simulation support.
//!
//! Parses UPF power intent commands and provides power-aware simulation:
//! - Power domain tracking (ON/OFF states)
//! - Supply net definitions
//! - Isolation cells (signal clamping between OFF→ON domains)
//! - Power switches (controllable power gating)
//! - Supply sets (grouped supply nets)
//! - Enhanced power transitions (corruption, retention semantics)
//! - X-propagation for signals in OFF domains
//!
//! # Supported UPF Commands (Phase 1 MVP + Phase 6)
//!
//! - `create_power_domain <name>` — define a power domain
//! - `create_supply_net <name>` — define a supply net
//! - `create_supply_set <name>` — define a supply set (Phase 6)
//! - `set_domain_supply_net <domain> -primary_power_net <net> -primary_ground_net <net>`
//! - `add_power_state <name> -state {state_name}` — define power states
//! - `set_isolation <domain> -isolation_power_net <net> -isolation_ground_net <net> -clamp_value <val>` (Phase 6)
//! - `create_power_switch <name> -domain <domain> -output_supply_net <net> -input_supply_net <net> -on_state {ctrl_expr}` (Phase 6)
//!
//! # Example UPF input
//!
//! ```text
//! create_power_domain PD_TOP
//! create_supply_net VDD -domain PD_TOP
//! create_supply_net VSS -domain PD_TOP
//! set_domain_supply_net PD_TOP -primary_power_net VDD -primary_ground_net VSS
//! add_power_state PD_TOP -state {ON} -logic_expr {VDD == 1 && VSS == 0}
//! add_power_state PD_TOP -state {OFF} -logic_expr {VDD == 0}
//! set_isolation PD_TOP -clamp_value 0
//! create_power_switch SW_CORE -domain PD_TOP -output_supply_net VDD_SW -input_supply_net VDD -on_state {ctrl == 1}
//! ```

use std::collections::{HashMap, HashSet};
use std::fs;
use crate::ir::*;
use crate::simulator::value::*;

// ─── Data Structures ───

/// A power domain in the design.
#[derive(Debug, Clone, PartialEq)]
pub struct PowerDomain {
    pub name: String,
    /// Elements (hierarchical paths or signal names) in this domain
    pub elements: Vec<String>,
    /// Primary power supply net name
    pub primary_power_net: Option<String>,
    /// Primary ground supply net name
    pub primary_ground_net: Option<String>,
    /// Power states for this domain
    pub power_states: Vec<PowerState>,
    /// Current active power state
    pub current_state: String,
}

/// A power state definition.
#[derive(Debug, Clone, PartialEq)]
pub struct PowerState {
    pub name: String,
    /// Logic expression for this state (simplified: stored as string, evaluated at runtime)
    pub logic_expr: Option<String>,
}

/// A supply net.
#[derive(Debug, Clone, PartialEq)]
pub struct SupplyNet {
    pub name: String,
    pub domain: Option<String>,
}

/// A supply set (grouped power/ground nets).
#[derive(Debug, Clone, PartialEq)]
pub struct SupplySet {
    pub name: String,
    pub power_net: Option<String>,
    pub ground_net: Option<String>,
    pub domain: Option<String>,
}

/// An isolation cell (clamps signals crossing from OFF→ON domain).
#[derive(Debug, Clone, PartialEq)]
pub struct IsolationCell {
    pub domain: String,
    pub clamp_value: LogicVal,
    pub isolation_power_net: Option<String>,
    pub isolation_ground_net: Option<String>,
    pub enable_signal: Option<String>,
}

/// A power switch (controllable power gate).
#[derive(Debug, Clone, PartialEq)]
pub struct PowerSwitch {
    pub name: String,
    pub domain: String,
    pub output_supply_net: String,
    pub input_supply_net: String,
    /// Control expression that enables the switch (simplified: signal name that must be 1)
    pub on_expression: String,
    /// Current state: ON or OFF
    pub is_on: bool,
}

/// Power intent database for the design.
#[derive(Debug, Clone)]
pub struct PowerIntent {
    pub domains: HashMap<String, PowerDomain>,
    pub supply_nets: HashMap<String, SupplyNet>,
    pub supply_sets: HashMap<String, SupplySet>,
    pub isolation_cells: Vec<IsolationCell>,
    pub power_switches: Vec<PowerSwitch>,
    /// Mapping from signal name to power domain
    pub signal_domain_map: HashMap<String, String>,
    /// Supply net values (set externally via `set_supply_net_value`)
    pub supply_values: HashMap<String, bool>,
    /// Whether power-aware simulation is enabled
    pub enabled: bool,
}

impl PowerIntent {
    pub fn new() -> Self {
        PowerIntent {
            domains: HashMap::new(),
            supply_nets: HashMap::new(),
            supply_sets: HashMap::new(),
            isolation_cells: Vec::new(),
            power_switches: Vec::new(),
            signal_domain_map: HashMap::new(),
            supply_values: HashMap::new(),
            enabled: false,
        }
    }

    /// UPF is enabled if there are any domains, isolation cells, or power switches
    fn update_enabled(&mut self) {
        self.enabled = !self.domains.is_empty()
            || !self.isolation_cells.is_empty()
            || !self.power_switches.is_empty();
    }

    /// Parse UPF from a file path.
    pub fn parse_file(path: &str) -> Result<Self, String> {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("cannot read UPF file '{}': {}", path, e))?;
        Self::parse(&content)
    }

    /// Parse UPF commands from a string.
    pub fn parse(content: &str) -> Result<Self, String> {
        let mut upf = PowerIntent::new();
        let lines = tokenize_upf(content);

        for line in &lines {
            upf.execute_command(line)?;
        }

        upf.enabled = !upf.domains.is_empty();
        Ok(upf)
    }

    /// Execute a single UPF command.
    fn execute_command(&mut self, tokens: &[String]) -> Result<(), String> {
        if tokens.is_empty() {
            return Ok(());
        }

        let cmd = tokens[0].to_lowercase();
        let result = match cmd.as_str() {
            "create_power_domain" => self.exec_create_power_domain(tokens),
            "create_supply_net" => self.exec_create_supply_net(tokens),
            "set_domain_supply_net" => self.exec_set_domain_supply_net(tokens),
            "add_power_state" => self.exec_add_power_state(tokens),
            "set_supply_net_value" => self.exec_set_supply_net_value(tokens),
            "create_supply_set" => self.exec_create_supply_set(tokens),
            "set_isolation" => self.exec_set_isolation(tokens),
            "create_power_switch" => self.exec_create_power_switch(tokens),
            _ => {
                // Unknown command — skip (UPF has many commands, we only support a subset)
                Ok(())
            }
        };
        self.update_enabled();
        result
    }

    /// `create_power_domain <name> [-elements {list}]`
    fn exec_create_power_domain(&mut self, tokens: &[String]) -> Result<(), String> {
        if tokens.len() < 2 {
            return Err("create_power_domain requires a name".to_string());
        }
        let name = tokens[1].clone();

        let mut elements = Vec::new();
        let mut i = 2;
        while i < tokens.len() {
            if tokens[i].starts_with('-') {
                let flag = tokens[i].to_lowercase();
                i += 1;
                if flag == "-elements" || flag == "-element" {
                    // Read element list (may be in {braces} or space-separated)
                    while i < tokens.len() && !tokens[i].starts_with('-') {
                        let raw = tokens[i].clone();
                        if raw != "{" && raw != "}" {
                            // Strip leading { and trailing } if present
                            let cleaned = raw.trim_start_matches('{').trim_end_matches('}').trim().to_string();
                            if !cleaned.is_empty() {
                                // Split space-separated names within a single brace token
                                for part in cleaned.split_whitespace() {
                                    if !part.is_empty() {
                                        elements.push(part.to_string());
                                    }
                                }
                            }
                        }
                        i += 1;
                    }
                } else {
                    // Skip unknown flag value
                    if i < tokens.len() && !tokens[i].starts_with('-') {
                        i += 1;
                    }
                }
            } else {
                i += 1;
            }
        }

        let domain = PowerDomain {
            name: name.clone(),
            elements,
            primary_power_net: None,
            primary_ground_net: None,
            power_states: vec![
                PowerState {
                    name: "ON".to_string(),
                    logic_expr: None,
                },
            ],
            current_state: "ON".to_string(),
        };
        self.domains.insert(name, domain);
        Ok(())
    }

    /// `create_supply_net <name> [-domain <domain>]`
    fn exec_create_supply_net(&mut self, tokens: &[String]) -> Result<(), String> {
        if tokens.len() < 2 {
            return Err("create_supply_net requires a name".to_string());
        }
        let name = tokens[1].clone();
        let mut domain = None;
        let mut i = 2;
        while i < tokens.len() {
            if tokens[i].starts_with('-') {
                let flag = tokens[i].to_lowercase();
                i += 1;
                if flag == "-domain" && i < tokens.len() {
                    domain = Some(tokens[i].clone());
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
        self.supply_nets.insert(name.clone(), SupplyNet {
            name,
            domain,
        });
        Ok(())
    }

    /// `set_domain_supply_net <domain> -primary_power_net <net> -primary_ground_net <net>`
    fn exec_set_domain_supply_net(&mut self, tokens: &[String]) -> Result<(), String> {
        if tokens.len() < 2 {
            return Err("set_domain_supply_net requires a domain name".to_string());
        }
        let domain_name = tokens[1].clone();
        let domain = self.domains.get_mut(&domain_name)
            .ok_or_else(|| format!("power domain '{}' not found", domain_name))?;

        let mut i = 2;
        while i < tokens.len() {
            if tokens[i].starts_with('-') {
                let flag = tokens[i].to_lowercase();
                i += 1;
                match flag.as_str() {
                    "-primary_power_net" | "-power_net" => {
                        if i < tokens.len() {
                            domain.primary_power_net = Some(tokens[i].clone());
                            i += 1;
                        }
                    }
                    "-primary_ground_net" | "-ground_net" => {
                        if i < tokens.len() {
                            domain.primary_ground_net = Some(tokens[i].clone());
                            i += 1;
                        }
                    }
                    _ => {
                        if i < tokens.len() && !tokens[i].starts_with('-') {
                            i += 1;
                        }
                    }
                }
            } else {
                i += 1;
            }
        }
        Ok(())
    }

    /// `add_power_state <domain> -state {name} [-logic_expr {expr}]`
    fn exec_add_power_state(&mut self, tokens: &[String]) -> Result<(), String> {
        if tokens.len() < 2 {
            return Err("add_power_state requires a domain name".to_string());
        }
        let domain_name = tokens[1].clone();
        let domain = self.domains.get_mut(&domain_name)
            .ok_or_else(|| format!("power domain '{}' not found", domain_name))?;

        let mut state_name = String::new();
        let mut logic_expr = None;
        let mut i = 2;
        while i < tokens.len() {
            if tokens[i].starts_with('-') {
                let flag = tokens[i].to_lowercase();
                i += 1;
                match flag.as_str() {
                    "-state" => {
                        if i < tokens.len() {
                            state_name = tokens[i].clone();
                            // Strip { } if present
                            if state_name.starts_with('{') && state_name.ends_with('}') {
                                state_name = state_name[1..state_name.len()-1].to_string();
                            }
                            i += 1;
                        }
                    }
                    "-logic_expr" => {
                        if i < tokens.len() {
                            let mut expr = tokens[i].clone();
                            if expr.starts_with('{') && expr.ends_with('}') {
                                expr = expr[1..expr.len()-1].to_string();
                            }
                            logic_expr = Some(expr);
                            i += 1;
                        }
                    }
                    _ => {
                        if i < tokens.len() && !tokens[i].starts_with('-') {
                            i += 1;
                        }
                    }
                }
            } else {
                i += 1;
            }
        }

        if !state_name.is_empty() {
            domain.power_states.push(PowerState {
                name: state_name,
                logic_expr,
            });
        }
        Ok(())
    }

    /// `set_supply_net_value <name> <value>` — non-standard, for testability.
    fn exec_set_supply_net_value(&mut self, tokens: &[String]) -> Result<(), String> {
        if tokens.len() < 3 {
            return Err("set_supply_net_value requires name and value".to_string());
        }
        let name = tokens[1].clone();
        let val_str = tokens[2].to_lowercase();
        let val = val_str == "1" || val_str == "true" || val_str == "on";
        self.supply_values.insert(name, val);
        Ok(())
    }

    /// `create_supply_set <name> [-power_net <net>] [-ground_net <net>] [-domain <domain>]`
    fn exec_create_supply_set(&mut self, tokens: &[String]) -> Result<(), String> {
        if tokens.len() < 2 {
            return Err("create_supply_set requires a name".to_string());
        }
        let name = tokens[1].clone();
        let mut power_net = None;
        let mut ground_net = None;
        let mut domain = None;
        let mut i = 2;
        while i < tokens.len() {
            if tokens[i].starts_with('-') {
                let flag = tokens[i].to_lowercase();
                i += 1;
                match flag.as_str() {
                    "-power_net" | "-power" => {
                        if i < tokens.len() { power_net = Some(tokens[i].clone()); i += 1; }
                    }
                    "-ground_net" | "-ground" => {
                        if i < tokens.len() { ground_net = Some(tokens[i].clone()); i += 1; }
                    }
                    "-domain" => {
                        if i < tokens.len() { domain = Some(tokens[i].clone()); i += 1; }
                    }
                    _ => { if i < tokens.len() && !tokens[i].starts_with('-') { i += 1; } }
                }
            } else {
                i += 1;
            }
        }
        self.supply_sets.insert(name.clone(), SupplySet {
            name,
            power_net,
            ground_net,
            domain,
        });
        Ok(())
    }

    /// `set_isolation <domain> [-clamp_value <0|1|X|Z>] [-isolation_power_net <net>] [-isolation_ground_net <net>] [-enable <signal>]`
    fn exec_set_isolation(&mut self, tokens: &[String]) -> Result<(), String> {
        if tokens.len() < 2 {
            return Err("set_isolation requires a domain name".to_string());
        }
        let domain = tokens[1].clone();
        let mut clamp_value = LogicVal::X;
        let mut isolation_power_net = None;
        let mut isolation_ground_net = None;
        let mut enable_signal = None;
        let mut i = 2;
        while i < tokens.len() {
            if tokens[i].starts_with('-') {
                let flag = tokens[i].to_lowercase();
                i += 1;
                match flag.as_str() {
                    "-clamp_value" => {
                        if i < tokens.len() {
                            clamp_value = match tokens[i].to_lowercase().as_str() {
                                "0" | "zero" => LogicVal::Zero,
                                "1" | "one" => LogicVal::One,
                                "z" | "highz" => LogicVal::Z,
                                _ => LogicVal::X,
                            };
                            i += 1;
                        }
                    }
                    "-isolation_power_net" | "-power_net" => {
                        if i < tokens.len() { isolation_power_net = Some(tokens[i].clone()); i += 1; }
                    }
                    "-isolation_ground_net" | "-ground_net" => {
                        if i < tokens.len() { isolation_ground_net = Some(tokens[i].clone()); i += 1; }
                    }
                    "-enable" => {
                        if i < tokens.len() { enable_signal = Some(tokens[i].clone()); i += 1; }
                    }
                    _ => { if i < tokens.len() && !tokens[i].starts_with('-') { i += 1; } }
                }
            } else {
                i += 1;
            }
        }
        self.isolation_cells.push(IsolationCell {
            domain,
            clamp_value,
            isolation_power_net,
            isolation_ground_net,
            enable_signal,
        });
        Ok(())
    }

    /// `create_power_switch <name> -domain <domain> -output_supply_net <net> -input_supply_net <net> -on_state {ctrl_expr}`
    fn exec_create_power_switch(&mut self, tokens: &[String]) -> Result<(), String> {
        if tokens.len() < 2 {
            return Err("create_power_switch requires a name".to_string());
        }
        let name = tokens[1].clone();
        let mut domain = String::new();
        let mut output_supply_net = String::new();
        let mut input_supply_net = String::new();
        let mut on_expression = String::new();
        let mut i = 2;
        while i < tokens.len() {
            if tokens[i].starts_with('-') {
                let flag = tokens[i].to_lowercase();
                i += 1;
                match flag.as_str() {
                    "-domain" => { if i < tokens.len() { domain = tokens[i].clone(); i += 1; } }
                    "-output_supply_net" => { if i < tokens.len() { output_supply_net = tokens[i].clone(); i += 1; } }
                    "-input_supply_net" => { if i < tokens.len() { input_supply_net = tokens[i].clone(); i += 1; } }
                    "-on_state" => {
                        if i < tokens.len() {
                            let mut expr = tokens[i].clone();
                            if expr.starts_with('{') && expr.ends_with('}') {
                                expr = expr[1..expr.len()-1].to_string();
                            }
                            on_expression = expr;
                            i += 1;
                        }
                    }
                    _ => { if i < tokens.len() && !tokens[i].starts_with('-') { i += 1; } }
                }
            } else {
                i += 1;
            }
        }
        self.power_switches.push(PowerSwitch {
            name,
            domain,
            output_supply_net,
            input_supply_net,
            on_expression,
            is_on: false,
        });
        Ok(())
    }

    /// Evaluate power states for all domains based on current supply net values.
    pub fn evaluate_power_states(&mut self) {
        // ── Step 1: Evaluate power switches (propagate supply through switches) ──
        for sw in &self.power_switches {
            // Simplify: check if control expression is a signal name that equals 1
            let control_expr = sw.on_expression.trim();
            let ctrl_on = if let Some(eq_pos) = control_expr.find("==") {
                let name = control_expr[..eq_pos].trim();
                let expected = control_expr[eq_pos+2..].trim() == "1";
                self.supply_values.get(name).copied().unwrap_or(false) == expected
            } else {
                // Just a signal name: true if it's 1
                self.supply_values.get(control_expr).copied().unwrap_or(false)
            };

            // Propagate input supply to output supply when switch is ON
            if ctrl_on {
                let input_val = self.supply_values.get(&sw.input_supply_net).copied().unwrap_or(false);
                self.supply_values.insert(sw.output_supply_net.clone(), input_val);
            } else {
                // Switch OFF: output supply is 0
                self.supply_values.insert(sw.output_supply_net.clone(), false);
            }
        }

        // ── Step 2: Evaluate domain power states ──
        let domain_names: Vec<String> = self.domains.keys().cloned().collect();
        for dname in &domain_names {
            let (mut is_on, pwr_net) = {
                let domain = match self.domains.get(dname) {
                    Some(d) => d,
                    None => continue,
                };
                let mut is_on = true;
                let pwr_net = domain.primary_power_net.clone();
                if let Some(ref pwr) = pwr_net {
                    // If primary power net is set and NOT found in supply_values, default OFF
                    // (no supply connected = power down)
                    let val = self.supply_values.get(pwr).copied().unwrap_or(false);
                    if !val {
                        is_on = false;
                    }
                }
                (is_on, pwr_net)
            };

            // Check all power states for logic expressions that evaluate to true
            let mut new_state = "ON".to_string();
            if let Some(domain) = self.domains.get(dname) {
                for state in &domain.power_states {
                    if let Some(ref expr) = state.logic_expr {
                        if self.evaluate_simple_expr(expr) {
                            new_state = state.name.clone();
                            break;
                        }
                    }
                }
            }

            // Default: if primary power net is 0, domain is OFF
            if !is_on && new_state == "ON" {
                new_state = "OFF".to_string();
            }

            if let Some(domain) = self.domains.get_mut(dname) {
                domain.current_state = new_state;
            }
        }
    }

    /// Get the isolation clamp value for a signal crossing from OFF to ON domain.
    /// Returns Some(LogicVal) if the signal should be clamped, None if no isolation applies.
    pub fn get_isolation_clamp(&self, signal_name: &str) -> Option<LogicVal> {
        if !self.enabled || self.isolation_cells.is_empty() {
            return None;
        }
        // Check if the signal's source domain is OFF
        let src_domain = self.find_domain_for_signal(signal_name);
        let src_off = match src_domain {
            Some(ref d) => self.domains.get(d)
                .map(|dom| dom.current_state != "ON" 
                    && (dom.current_state == "OFF" || dom.current_state.to_uppercase().starts_with("OFF")))
                .unwrap_or(false),
            None => false,
        };
        if !src_off {
            return None;
        }

        // Find matching isolation cell for this domain
        for cell in &self.isolation_cells {
            // If isolation is enabled and has an enable signal, check it
            if let Some(ref en_sig) = cell.enable_signal {
                let en_val = self.supply_values.get(en_sig).copied().unwrap_or(false);
                if !en_val {
                    continue; // Isolation disabled
                }
            }
            // Check if signal domains match
            if let Some(ref d) = src_domain {
                if cell.domain == *d || cell.domain == "*" {
                    return Some(cell.clamp_value);
                }
            }
        }
        None
    }

    /// Evaluate a simple UPF logic expression like `VDD == 1 && VSS == 0`.
    /// Supports: ==, !=, &&, ||, supply net name comparisons.
    fn evaluate_simple_expr(&self, expr: &str) -> bool {
        // Split on && (AND)
        let and_terms: Vec<&str> = expr.split("&&").map(|s| s.trim()).collect();
        if and_terms.is_empty() {
            return true;
        }
        for term in &and_terms {
            let or_terms: Vec<&str> = term.split("||").map(|s| s.trim()).collect();
            let mut term_ok = false;
            for or_term in &or_terms {
                // Try to parse: `name == val` or `name != val`
                if let Some(eq_pos) = or_term.find("==") {
                    let name = or_term[..eq_pos].trim();
                    let val_str = or_term[eq_pos+2..].trim();
                    let expected = val_str == "1" || val_str == "true" || val_str == "ON"
                        || val_str == "on" || val_str == "TRUE";
                    let actual = self.supply_values.get(name).copied().unwrap_or(false);
                    if actual == expected {
                        term_ok = true;
                        break;
                    }
                } else if let Some(ne_pos) = or_term.find("!=") {
                    let name = or_term[..ne_pos].trim();
                    let val_str = or_term[ne_pos+2..].trim();
                    let expected = val_str == "1" || val_str == "true" || val_str == "ON"
                        || val_str == "on" || val_str == "TRUE";
                    let actual = self.supply_values.get(name).copied().unwrap_or(false);
                    if actual != expected {
                        term_ok = true;
                        break;
                    }
                } else {
                    // Just check if the name evaluates to true (single net reference)
                    let name = or_term.trim();
                    if self.supply_values.get(name).copied().unwrap_or(false) {
                        term_ok = true;
                        break;
                    }
                }
            }
            if !term_ok {
                return false;
            }
        }
        true
    }

    /// Check if a signal with the given name is in a powered-off domain.
    /// Returns true if the signal should be X (domain is OFF).
    pub fn is_signal_powered_off(&self, signal_name: &str) -> bool {
        if !self.enabled {
            return false;
        }
        // Find domain containing this signal
        let domain_name = self.find_domain_for_signal(signal_name);
        if let Some(dname) = domain_name {
            if let Some(domain) = self.domains.get(&dname) {
                return domain.current_state == "OFF" 
                    || domain.current_state == "OFF_N" 
                    || domain.current_state == "OFF_NRET"
                    || domain.current_state == "RETENTION"
                    || domain.current_state.to_uppercase().starts_with("OFF");
            }
        }
        false
    }

    /// Find which domain a signal belongs to (by checking domain elements).
    fn find_domain_for_signal(&self, signal_name: &str) -> Option<String> {
        // First check cached mapping
        if let Some(d) = self.signal_domain_map.get(signal_name) {
            return Some(d.clone());
        }

        // Search domains
        for (dname, domain) in &self.domains {
            for elem in &domain.elements {
                // Elements can be hierarchy prefixes or exact signal names
                if signal_name.starts_with(elem) || signal_name == elem {
                    // Cache it
                    // (can't write to cache here due to borrow, but caller can)
                    return Some(dname.clone());
                }
            }
        }
        None
    }

    /// Build signal-to-domain mapping from design signals.
    pub fn build_signal_mapping(&mut self, signals: &[SignalInfo]) {
        self.signal_domain_map.clear();
        for (sid, sig) in signals.iter().enumerate() {
            let sig_name = sig.name.as_str();
            for (dname, domain) in &self.domains {
                for elem in &domain.elements {
                    if sig_name.starts_with(elem) || sig_name == elem {
                        self.signal_domain_map.insert(sig_name.to_string(), dname.clone());
                        break;
                    }
                }
            }
        }
    }
}

impl Default for PowerIntent {
    fn default() -> Self {
        Self::new()
    }
}

// ─── UPF Tokenizer ───

/// Tokenize a UPF command line into tokens.
/// UPF is Tcl-based: tokens are separated by whitespace,
/// but {braces} groups are treated as single tokens.
fn tokenize_upf(content: &str) -> Vec<Vec<String>> {
    let mut lines = Vec::new();
    let mut current_line = Vec::new();
    let mut chars = content.chars().peekable();
    let mut token = String::new();
    let mut in_brace: i32 = 0;
    let mut in_comment = false;

    while let Some(c) = chars.next() {
        if in_comment {
            if c == '\n' {
                in_comment = false;
            }
            continue;
        }

        match c {
            '#' => {
                if !token.is_empty() {
                    current_line.push(token.clone());
                    token.clear();
                }
                in_comment = true;
            }
            '\n' | ';' => {
                if !token.is_empty() {
                    current_line.push(token.clone());
                    token.clear();
                }
                if !current_line.is_empty() {
                    lines.push(current_line.clone());
                    current_line.clear();
                }
            }
            '{' => {
                token.push(c);
                in_brace += 1;
            }
            '}' => {
                token.push(c);
                in_brace = in_brace.saturating_sub(1);
            }
            ' ' | '\t' => {
                if in_brace > 0 {
                    token.push(c);
                } else if !token.is_empty() {
                    current_line.push(token.clone());
                    token.clear();
                }
            }
            _ => {
                token.push(c);
            }
        }
    }

    // Flush remaining
    if !token.is_empty() {
        current_line.push(token);
    }
    if !current_line.is_empty() {
        lines.push(current_line);
    }

    lines
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upf_create_power_domain() {
        let upf = PowerIntent::parse("create_power_domain PD_TOP").unwrap();
        assert!(upf.domains.contains_key("PD_TOP"));
        let domain = upf.domains.get("PD_TOP").unwrap();
        assert_eq!(domain.name, "PD_TOP");
        assert_eq!(domain.current_state, "ON");
        assert_eq!(domain.elements.len(), 0);
    }

    #[test]
    fn test_upf_create_supply_net() {
        let src = r#"
create_power_domain PD_TOP
create_supply_net VDD -domain PD_TOP
create_supply_net VSS -domain PD_TOP
"#;
        let upf = PowerIntent::parse(src).unwrap();
        assert!(upf.supply_nets.contains_key("VDD"));
        assert!(upf.supply_nets.contains_key("VSS"));
        assert_eq!(upf.supply_nets.get("VDD").unwrap().domain, Some("PD_TOP".to_string()));
    }

    #[test]
    fn test_upf_set_domain_supply_net() {
        let src = r#"
create_power_domain PD_TOP
create_supply_net VDD
create_supply_net VSS
set_domain_supply_net PD_TOP -primary_power_net VDD -primary_ground_net VSS
"#;
        let upf = PowerIntent::parse(src).unwrap();
        let domain = upf.domains.get("PD_TOP").unwrap();
        assert_eq!(domain.primary_power_net, Some("VDD".to_string()));
        assert_eq!(domain.primary_ground_net, Some("VSS".to_string()));
    }

    #[test]
    fn test_upf_add_power_state() {
        let src = r#"
create_power_domain PD_TOP
create_supply_net VDD
create_supply_net VSS
set_domain_supply_net PD_TOP -primary_power_net VDD -primary_ground_net VSS
add_power_state PD_TOP -state ON -logic_expr {VDD == 1 && VSS == 0}
add_power_state PD_TOP -state OFF -logic_expr {VDD == 0}
"#;
        let upf = PowerIntent::parse(src).unwrap();
        let domain = upf.domains.get("PD_TOP").unwrap();
        assert_eq!(domain.power_states.len(), 3); // 1 default (ON) + 2 added
    }

    #[test]
    fn test_upf_power_state_evaluation() {
        let src = r#"
create_power_domain PD_TOP
create_supply_net VDD
set_domain_supply_net PD_TOP -primary_power_net VDD
add_power_state PD_TOP -state ON -logic_expr {VDD == 1}
add_power_state PD_TOP -state OFF -logic_expr {VDD == 0}
"#;
        let mut upf = PowerIntent::parse(src).unwrap();

        // Initially VDD is not set → domain should be on default
        upf.evaluate_power_states();
        // VDD defaults to false, so domain should be OFF
        assert_eq!(upf.domains.get("PD_TOP").unwrap().current_state, "OFF");

        // Set VDD = 1 → domain should be ON
        upf.supply_values.insert("VDD".to_string(), true);
        upf.evaluate_power_states();
        assert_eq!(upf.domains.get("PD_TOP").unwrap().current_state, "ON");

        // Set VDD = 0 → domain should be OFF
        upf.supply_values.insert("VDD".to_string(), false);
        upf.evaluate_power_states();
        assert_eq!(upf.domains.get("PD_TOP").unwrap().current_state, "OFF");
    }

    #[test]
    fn test_upf_complex_expr_evaluation() {
        let src = r#"
create_power_domain PD_TOP
create_supply_net VDD
create_supply_net VSS
set_domain_supply_net PD_TOP -primary_power_net VDD -primary_ground_net VSS
add_power_state PD_TOP -state ON -logic_expr {VDD == 1 && VSS == 0}
add_power_state PD_TOP -state OFF -logic_expr {VDD == 0}
"#;
        let mut upf = PowerIntent::parse(src).unwrap();

        // ON condition: VDD=1 && VSS=0
        upf.supply_values.insert("VDD".to_string(), true);
        upf.supply_values.insert("VSS".to_string(), false);
        upf.evaluate_power_states();
        assert_eq!(upf.domains.get("PD_TOP").unwrap().current_state, "ON");

        // VDD=1 && VSS=1 → neither ON nor OFF → default
        upf.supply_values.insert("VDD".to_string(), true);
        upf.supply_values.insert("VSS".to_string(), true);
        upf.evaluate_power_states();
        // VDD=1 so primary power check says ON, but the AND expression doesn't match
        // Primary power check: VDD=1 → is_on=true → stays ON
        assert_eq!(upf.domains.get("PD_TOP").unwrap().current_state, "ON");
    }

    #[test]
    fn test_upf_signal_powered_off() {
        let src = r#"
create_power_domain PD_CORE -elements {u_core}
create_supply_net VDD_CORE -domain PD_CORE
set_domain_supply_net PD_CORE -primary_power_net VDD_CORE
add_power_state PD_CORE -state ON -logic_expr {VDD_CORE == 1}
add_power_state PD_CORE -state OFF -logic_expr {VDD_CORE == 0}
"#;
        let mut upf = PowerIntent::parse(src).unwrap();
        upf.build_signal_mapping(&[]);

        // Domain is ON by default → signal should not be powered off
        assert!(!upf.is_signal_powered_off("u_core.some_signal"));

        // Set VDD_CORE = 0 → domain OFF
        upf.supply_values.insert("VDD_CORE".to_string(), false);
        upf.evaluate_power_states();
        assert!(upf.is_signal_powered_off("u_core.some_signal"));
    }

    #[test]
    fn test_upf_parse_file() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("maria_upf_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let upf_path = dir.join("test.upf");
        let upf_content = r#"
create_power_domain PD_TOP
create_supply_net VDD
create_supply_net VSS
set_domain_supply_net PD_TOP -primary_power_net VDD -primary_ground_net VSS
"#;
        {
            let mut f = std::fs::File::create(&upf_path).unwrap();
            f.write_all(upf_content.as_bytes()).unwrap();
        }

        let upf = PowerIntent::parse_file(upf_path.to_str().unwrap()).unwrap();
        assert!(upf.domains.contains_key("PD_TOP"));
        assert!(upf.enabled);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_upf_tokenizer_braces() {
        let tokens = tokenize_upf(r#"add_power_state PD_TOP -state ON -logic_expr {VDD == 1 && VSS == 0}"#);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0][0], "add_power_state");
        assert_eq!(tokens[0][1], "PD_TOP");
        assert_eq!(tokens[0][2], "-state");
        assert_eq!(tokens[0][4], "-logic_expr");
        // The brace group should be a single token
        assert!(tokens[0][5].contains("VDD == 1 && VSS == 0") || tokens[0][5].contains('{'));
    }

    #[test]
    fn test_upf_set_supply_net_value() {
        let src = r#"
create_supply_net VDD
set_supply_net_value VDD 1
"#;
        let upf = PowerIntent::parse(src).unwrap();
        assert!(upf.supply_values.get("VDD").copied().unwrap_or(false));
    }

    #[test]
    fn test_upf_domain_with_elements() {
        let src = r#"
create_power_domain PD_CORE -elements {u_core_top u_core_mem}
"#;
        let upf = PowerIntent::parse(src).unwrap();
        let domain = upf.domains.get("PD_CORE").unwrap();
        assert_eq!(domain.elements.len(), 2);
        assert!(domain.elements.contains(&"u_core_top".to_string()));
        assert!(domain.elements.contains(&"u_core_mem".to_string()));
    }

    #[test]
    fn test_upf_signal_mapping() {
        let src = r#"
create_power_domain PD_CORE -elements {u_core}
create_power_domain PD_IO -elements {u_io}
create_supply_net VDD_CORE -domain PD_CORE
create_supply_net VDD_IO -domain PD_IO
set_domain_supply_net PD_CORE -primary_power_net VDD_CORE
set_domain_supply_net PD_IO -primary_power_net VDD_IO
"#;
        let mut upf = PowerIntent::parse(src).unwrap();

        // Build signal-to-domain mapping from mock signals
        let sigs = vec![
            make_signal_info("u_core.clk"),
            make_signal_info("u_core.data"),
            make_signal_info("u_io.pad"),
            make_signal_info("u_io.oe"),
            make_signal_info("u_other.signal"), // Not in any domain
        ];
        upf.build_signal_mapping(&sigs);

        // No supply values set → evaluate_power_states sets domains OFF (primary power net VDD_CORE defaults to false)
        upf.evaluate_power_states();
        assert!(upf.domains.get("PD_CORE").unwrap().current_state == "OFF", "PD_CORE should be OFF");

        // Core signals should be in PD_CORE
        assert!(upf.is_signal_powered_off("u_core.clk"), "u_core.clk should be powered off");
        assert!(upf.is_signal_powered_off("u_core.data"));

        // IO signals should be in PD_IO
        assert!(upf.is_signal_powered_off("u_io.pad"));

        // u_other.signal is not in any domain → not powered off
        assert!(!upf.is_signal_powered_off("u_other.signal"));
    }

    fn make_signal_info(name: &str) -> SignalInfo {
        SignalInfo {
            name: crate::Symbol::intern(name),
            width: 1,
            kind: crate::ir::SignalKind::Wire,
            net_type: crate::ir::NetType::Wire,
            multi_driver: false,
            init_val: LogicVec::new(1),
            array_depth: 0,
            elem_width: 1,
            array_dims: Vec::new(),
            class_name: None,
            is_string: false,
            is_real: false,
            is_mailbox: false,
            is_semaphore: false,
            is_2state: false,
            is_dynamic: false,
            is_queue: false,
            is_associative: false,
            is_signed: false,
            is_const: false,
            msb: 0,
            lsb: 0,
            struct_fields: Vec::new(),
            packed_dims: Vec::new(),
            delay_rise: None,
            delay_fall: None,
            iface_type: None,
            iface_modport: None,
        }
    }
}
