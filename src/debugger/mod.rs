use crate::ir::*;
use crate::simulator::*;

/// Debugger wrapper untuk SimulationEngine.
/// Menyediakan API high-level untuk debug, step, breakpoint, dll.
pub struct Debugger {
    pub engine: SimulationEngine,
}

impl Debugger {
    pub fn new(engine: SimulationEngine) -> Self {
        Debugger { engine }
    }

    pub fn run(&mut self) -> Result<(), String> {
        self.engine.paused = false;
        self.engine.step_mode = StepMode::Running;
        self.engine.run()
    }

    pub fn step_cycle(&mut self) -> Result<(), String> {
        self.engine.paused = false;
        self.engine.step_mode = StepMode::StepCycle;
        self.engine.run()
    }

    pub fn continue_run(&mut self) -> Result<(), String> {
        self.engine.paused = false;
        self.engine.step_mode = StepMode::Running;
        self.engine.run()
    }

    pub fn pause(&mut self) {
        self.engine.paused = true;
    }

    pub fn reset(&mut self) {
        let design = self.engine.design.clone();
        let max_time = self.engine.max_time;
        let debug_mode = self.engine.debug_mode.clone();
        let breakpoints = self.engine.breakpoints.clone();
        let watchpoints = self.engine.watchpoints.clone();
        let snapshot_interval = self.engine.snapshot_interval;

        self.engine = SimulationEngine::new(design, max_time);
        self.engine.debug_mode = debug_mode;
        self.engine.breakpoints = breakpoints;
        self.engine.watchpoints = watchpoints;
        self.engine.snapshot_interval = snapshot_interval;
        self.engine.paused = false;
        self.engine.step_mode = StepMode::Running;
    }

    pub fn add_breakpoint(&mut self, bp: Breakpoint) {
        self.engine.breakpoints.push(bp);
    }

    pub fn clear_breakpoints(&mut self) {
        self.engine.breakpoints.clear();
    }

    pub fn add_watchpoint(&mut self, wp: Watchpoint) {
        self.engine.watchpoints.push(wp);
    }

    pub fn clear_watchpoints(&mut self) {
        self.engine.watchpoints.clear();
    }

    pub fn find_signal_id(&self, name: &str) -> Option<SignalId> {
        self.engine.design.top.signals.iter()
            .position(|s| s.name == name)
            .or_else(|| {
                self.engine.design.hier_signal_map.get(name).copied()
            })
    }

    pub fn read_signal_by_name(&self, name: &str) -> Option<LogicVec> {
        let id = self.find_signal_id(name)?;
        Some(self.engine.state.read_signal(id).clone())
    }

    pub fn print_signal(&self, name: &str) -> String {
        let id = match self.find_signal_id(name) {
            Some(id) => id,
            None => return format!("signal '{}' not found", name),
        };
        let val = self.engine.state.read_signal(id);
        let sig = &self.engine.design.top.signals[id];
        format!("{} = {} ({}b)", sig.name, val, sig.width)
    }

    pub fn print_signals_filtered(&self, filter: &str) -> String {
        let mut out = String::new();
        let filt_lower = filter.to_lowercase();
        for sig in &self.engine.design.top.signals {
            if sig.name.to_lowercase().contains(&filt_lower) {
                let id = self.find_signal_id(&sig.name).unwrap_or(0);
                let val = self.engine.state.read_signal(id);
                out.push_str(&format!("  {} = {} ({}b)\n", sig.name, val, sig.width));
            }
        }
        if out.is_empty() {
            out.push_str(&format!("no signals matching '{}'\n", filter));
        }
        out
    }

    pub fn print_all_signals(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("--- Cycle {} ---\n", self.engine.state.time));
        for sig in &self.engine.design.top.signals {
            let id = self.find_signal_id(&sig.name).unwrap_or(0);
            let val = self.engine.state.read_signal(id);
            out.push_str(&format!("  {} = {} ({}b)\n", sig.name, val, sig.width));
        }
        out
    }

    pub fn print_state_summary(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("Cycle: {}\n", self.engine.state.time));
        out.push('\n');

        let mut pc_val = String::new();
        let mut reg_lines = Vec::new();
        let mut other_lines = Vec::new();

        for (i, sig) in self.engine.design.top.signals.iter().enumerate() {
            let val = self.engine.state.read_signal(i);
            let s = format!("{} = {}", sig.name, val);
            let lower = sig.name.to_lowercase();
            if lower == "pc" || sig.name == "PC" {
                pc_val = format!("PC = {}", val);
            } else if lower.contains("reg") || lower.contains("rf") || lower.starts_with('r') && sig.name.len() <= 3 {
                reg_lines.push(s);
            } else {
                other_lines.push(s);
            }
        }

        if !pc_val.is_empty() {
            out.push_str(&format!("{}\n\n", pc_val));
        }
        if !reg_lines.is_empty() {
            for l in &reg_lines {
                out.push_str(&format!("{}\n", l));
            }
            out.push('\n');
        }
        for l in &other_lines {
            out.push_str(&format!("{}\n", l));
        }
        out
    }

    pub fn timeline(&self, name: &str, max_entries: usize) -> String {
        let mut out = String::new();
        let history = match self.engine.signal_history.get(name) {
            Some(h) => h,
            None => return format!("no timeline data for '{}'\n", name),
        };
        let w = if history.is_empty() { 8 } else {
            let mut max_len = 4usize;
            for (_, v) in history.iter() {
                let s = format!("{}", v);
                if s.len() > max_len { max_len = s.len(); }
            }
            max_len
        };
        out.push_str(&format!("{:<8} {:>width$}\n", "Cycle", name, width = w));
        out.push_str(&"-".repeat(8 + w + 2));
        out.push('\n');
        let start = if history.len() > max_entries { history.len() - max_entries } else { 0 };
        for (cycle, val) in history.iter().skip(start) {
            out.push_str(&format!("{:<8} {:>width$}\n", cycle, format!("{}", val), width = w));
        }
        out
    }

    pub fn hierarchy_tree(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("{}\n", self.engine.design.top.name));
        for (i, inst) in self.engine.design.top.sub_instances.iter().enumerate() {
            let branch = if i == self.engine.design.top.sub_instances.len() - 1 { "└── " } else { "├── " };
            out.push_str(&format!("{}{}\n", branch, inst.instance_name));
        }
        out
    }

    pub fn memory_inspect(&self, addr: u64, len: usize) -> String {
        let mut out = String::new();
        for offset in 0..len {
            let a = addr + offset as u64;
            let mut found = false;
            for sig in &self.engine.design.top.signals {
                if sig.array_depth > 0 || sig.width > 8 {
                    let id = self.find_signal_id(&sig.name).unwrap_or(0);
                    let val = self.engine.state.read_signal(id);
                    let byte_count = (sig.width + 7) / 8;
                    if a >= offset as u64 && a < offset as u64 + byte_count as u64 {
                        let byte_offset = (a - offset as u64) as usize;
                        if byte_offset < byte_count {
                            let mut b = 0u8;
                            for bi in 0..8 {
                                let idx = byte_offset * 8 + bi;
                                if idx < val.bits.len() && val.bits[idx] == LogicVal::One {
                                    b |= 1 << bi;
                                }
                            }
                            out.push_str(&format!("0x{:04X} : {:02X}\n", a, b));
                            found = true;
                        }
                    }
                }
                if found { break; }
            }
            if !found {
                out.push_str(&format!("0x{:04X} : --\n", a));
            }
        }
        out
    }

    pub fn reverse_step(&mut self) -> Result<(), String> {
        if let Some(snap) = self.engine.snapshots.pop() {
            self.engine.state.signals = snap.signals;
            self.engine.state.next_signals = snap.next_signals;
            self.engine.state.changed = snap.changed;
            self.engine.state.time = snap.time;
            self.engine.paused = true;
            self.engine.step_mode = StepMode::Paused;
            Ok(())
        } else {
            Err("no snapshot available for reverse step".to_string())
        }
    }

    pub fn reverse_continue(&mut self, target_time: u64) -> Result<(), String> {
        while let Some(snap) = self.engine.snapshots.last() {
            if snap.time <= target_time {
                let snap = self.engine.snapshots.pop().unwrap();
                self.engine.state.signals = snap.signals;
                self.engine.state.next_signals = snap.next_signals;
                self.engine.state.changed = snap.changed;
                self.engine.state.time = snap.time;
                self.engine.paused = true;
                self.engine.step_mode = StepMode::Paused;
                return Ok(());
            }
            self.engine.snapshots.pop();
        }
        Err(format!("no snapshot at or before time {}", target_time))
    }

    pub fn get_module_names(&self) -> Vec<String> {
        let mut names = vec![self.engine.design.top.name.clone()];
        for inst in &self.engine.design.top.sub_instances {
            names.push(inst.instance_name.clone());
        }
        names
    }

    pub fn print_breakpoints(&self) -> String {
        if self.engine.breakpoints.is_empty() {
            return "no breakpoints set\n".to_string();
        }
        let mut out = String::new();
        for (i, bp) in self.engine.breakpoints.iter().enumerate() {
            out.push_str(&format!("  [{}] {}\n", i, bp));
        }
        out
    }

    pub fn print_watchpoints(&self) -> String {
        if self.engine.watchpoints.is_empty() {
            return "no watchpoints set\n".to_string();
        }
        let mut out = String::new();
        for (i, wp) in self.engine.watchpoints.iter().enumerate() {
            out.push_str(&format!("  [{}] {}\n", i, wp));
        }
        out
    }

    pub fn print_event_log(&self) -> String {
        if self.engine.event_log.is_empty() {
            return "no debug events\n".to_string();
        }
        let mut out = String::new();
        for ev in &self.engine.event_log {
            out.push_str(&format!("[cycle {}] {}\n", ev.time, ev.message));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile_str;

    fn make_debugger(source: &str) -> Debugger {
        let design = compile_str(source).unwrap();
        let engine = SimulationEngine::new(design, 100);
        Debugger { engine }
    }

    #[test]
    fn test_debug_mode_normal() {
        let mut dbg = make_debugger("module top; initial #1 $finish; endmodule");
        dbg.engine.debug_mode = DebugMode::Normal;
        let r = dbg.run();
        assert!(r.is_ok());
    }

    #[test]
    fn test_debug_mode_debug() {
        let mut dbg = make_debugger("module top; initial #1 $finish; endmodule");
        dbg.engine.debug_mode = DebugMode::Debug;
        let r = dbg.run();
        assert!(r.is_ok());
    }

    #[test]
    fn test_debug_step_cycle() {
        let mut dbg = make_debugger(r#"
module top;
    reg [7:0] cnt;
    initial begin
        cnt = 0;
        #1;
        cnt = 1;
        #1 $finish;
    end
endmodule"#);
        dbg.engine.debug_mode = DebugMode::Debug;
        let r = dbg.step_cycle();
        assert!(r.is_ok(), "step cycle failed: {:?}", r);
        assert!(dbg.engine.paused, "engine should be paused after step");
        let state = dbg.print_state_summary();
        assert!(!state.is_empty());
    }

    #[test]
    fn test_breakpoint_cycle() {
        let mut dbg = make_debugger("module top; initial #2 $finish; endmodule");
        dbg.engine.debug_mode = DebugMode::Debug;
        dbg.add_breakpoint(Breakpoint::Cycle(1));
        let r = dbg.run();
        assert!(r.is_ok());
        assert!(dbg.engine.paused);
        assert!(!dbg.engine.event_log.is_empty());
    }

    #[test]
    fn test_breakpoint_signal_eq() {
        let mut dbg = make_debugger(r#"
module top;
    reg [7:0] x;
    initial begin
        x = 42;
        #1 $finish;
    end
endmodule"#);
        dbg.engine.debug_mode = DebugMode::Debug;
        dbg.add_breakpoint(Breakpoint::SignalEq("x".to_string(), LogicVec::from_u64(42, 8)));
        let r = dbg.run();
        assert!(r.is_ok(), "break signal eq failed: {:?}", r);
    }

    #[test]
    fn test_breakpoint_signal_change() {
        let mut dbg = make_debugger(r#"
module top;
    reg [7:0] x;
    initial begin
        x = 10;
        #1;
        x = 20;
        #1 $finish;
    end
endmodule"#);
        dbg.engine.debug_mode = DebugMode::Debug;
        dbg.add_breakpoint(Breakpoint::SignalChange("x".to_string()));
        let r = dbg.run();
        assert!(r.is_ok(), "break signal change failed: {:?}", r);
    }

    #[test]
    fn test_print_all_signals() {
        let dbg = make_debugger(r#"
module top;
    reg [7:0] a;
    reg [3:0] b;
    initial begin
        a = 10;
        b = 5;
        #1 $finish;
    end
endmodule"#);
        let out = dbg.print_all_signals();
        assert!(out.contains("a"));
        assert!(out.contains("b"));
    }

    #[test]
    fn test_print_signal_by_name() {
        let dbg = make_debugger(r#"
module top;
    reg [7:0] alu_result;
    initial #1 $finish;
endmodule"#);
        let out = dbg.print_signal("alu_result");
        assert!(out.contains("alu_result"));
    }

    #[test]
    fn test_print_signal_not_found() {
        let dbg = make_debugger("module top; initial #1 $finish; endmodule");
        let out = dbg.print_signal("nonexistent");
        assert!(out.contains("not found"));
    }

    #[test]
    fn test_hierarchy_tree() {
        let dbg = make_debugger(r#"
module top;
    wire a;
    sub u_sub(.a(a));
endmodule
module sub(input a);
    initial #1 $finish;
endmodule"#);
        let tree = dbg.hierarchy_tree();
        assert!(tree.contains("top"), "tree should contain 'top': {}", tree);
    }

    #[test]
    fn test_timeline() {
        let mut dbg = make_debugger(r#"
module top;
    reg [7:0] x;
    initial begin
        x = 1;
        #1;
        x = 2;
        #1;
        x = 3;
        #1 $finish;
    end
endmodule"#);
        dbg.engine.debug_mode = DebugMode::Debug;
        dbg.run().ok();
        let tl = dbg.timeline("x", 10);
        assert!(tl.contains("x"));
    }

    #[test]
    fn test_watchpoint() {
        let mut dbg = make_debugger(r#"
module top;
    reg [7:0] x;
    initial begin
        x = 0;
        #1;
        x = 5;
        #1 $finish;
    end
endmodule"#);
        dbg.engine.debug_mode = DebugMode::Debug;
        dbg.add_watchpoint(Watchpoint::Signal("x".to_string()));
        let r = dbg.run();
        assert!(r.is_ok(), "watchpoint run failed: {:?}", r);
    }

    #[test]
    fn test_reset() {
        let mut dbg = make_debugger("module top; initial #1 $finish; endmodule");
        dbg.engine.debug_mode = DebugMode::Debug;
        dbg.step_cycle().ok();
        assert!(dbg.engine.paused);
        dbg.reset();
        assert!(!dbg.engine.paused);
        assert_eq!(dbg.engine.state.time, 0);
    }

    #[test]
    fn test_deep_debug_snapshot() {
        let mut dbg = make_debugger("module top; initial #5 $finish; endmodule");
        dbg.engine.debug_mode = DebugMode::DeepDebug;
        dbg.engine.snapshot_interval = 2;
        dbg.run().ok();
        assert!(dbg.engine.snapshots.len() > 0);
    }

    #[test]
    fn test_reverse_step() {
        let mut dbg = make_debugger("module top; initial #10 $finish; endmodule");
        dbg.engine.debug_mode = DebugMode::DeepDebug;
        dbg.engine.snapshot_interval = 1;
        dbg.run().ok();
        if dbg.engine.snapshots.len() > 0 {
            let r = dbg.reverse_step();
            assert!(r.is_ok(), "reverse step failed: {:?}", r);
        }
    }

    #[test]
    fn test_event_log() {
        let mut dbg = make_debugger("module top; initial #1 $finish; endmodule");
        dbg.engine.debug_mode = DebugMode::Debug;
        dbg.add_breakpoint(Breakpoint::Cycle(0));
        dbg.run().ok();
        let log = dbg.print_event_log();
        assert!(log.contains("cycle"));
    }

    #[test]
    fn test_breakpoint_module() {
        let mut dbg = make_debugger(r#"
module top;
    sub u_sub();
endmodule
module sub;
    initial #1 $finish;
endmodule"#);
        dbg.engine.debug_mode = DebugMode::Debug;
        dbg.add_breakpoint(Breakpoint::Module("sub".to_string()));
        let r = dbg.run();
        assert!(r.is_ok(), "module breakpoint failed: {:?}", r);
    }

    #[test]
    fn test_memory_inspect() {
        let dbg = make_debugger(r#"
module top;
    reg [31:0] mem [0:7];
    initial begin
        mem[0] = 32'hDEADBEEF;
        #1 $finish;
    end
endmodule"#);
        let mem = dbg.memory_inspect(0, 8);
        assert!(!mem.is_empty());
    }

    #[test]
    fn test_get_module_names() {
        let dbg = make_debugger(r#"
module top;
    sub u_sub();
endmodule
module sub;
    initial #1 $finish;
endmodule"#);
        let names = dbg.get_module_names();
        assert!(names.contains(&"top".to_string()));
    }

    #[test]
    fn test_print_signals_filtered() {
        let dbg = make_debugger(r#"
module top;
    reg [7:0] pc;
    reg [31:0] alu_result;
    reg mem_addr;
    initial #1 $finish;
endmodule"#);
        let out = dbg.print_signals_filtered("alu");
        assert!(out.contains("alu_result"));
        let empty = dbg.print_signals_filtered("zzzzz");
        assert!(empty.contains("no signals matching"));
    }

    #[test]
    fn test_continue_after_step() {
        let mut dbg = make_debugger("module top; initial #3 $finish; endmodule");
        dbg.engine.debug_mode = DebugMode::Debug;
        dbg.step_cycle().ok();
        assert!(dbg.engine.paused);
        let r = dbg.continue_run();
        assert!(r.is_ok(), "continue after step failed: {:?}", r);
    }
}
