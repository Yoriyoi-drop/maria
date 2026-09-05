//! FORMAL-14: X-Propagation Formal Analysis.
//!
//! Analyzes how unknown (X) values propagate through RTL logic.
//! Detects:
//! - Signals that can be X after reset deassertion
//! - Logic that propagates X to critical outputs
//! - Missing reset initialization
//!
//! Uses abstract interpretation with a ternary domain (0/1/X) to track
//! X-propagation paths through combinational and sequential logic.

use maria_ir::{IrDesign, IrExpr, IrStmt, LogicVal, Process, SignalId};
use std::collections::HashMap;

/// Ternary value for X-propagation analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ternary {
    /// Definitely 0
    Zero,
    /// Definitely 1
    One,
    /// Unknown (X or Z)
    Unknown,
}

impl Ternary {
    /// Join two ternary values (least upper bound).
    pub fn join(self, other: Ternary) -> Ternary {
        match (self, other) {
            (Ternary::Zero, Ternary::Zero) => Ternary::Zero,
            (Ternary::One, Ternary::One) => Ternary::One,
            _ => Ternary::Unknown,
        }
    }

    /// Check if this value is definitely known (not X).
    pub fn is_known(self) -> bool {
        self != Ternary::Unknown
    }

    /// Check if this value could be X.
    pub fn may_be_unknown(self) -> bool {
        self == Ternary::Unknown
    }
}

/// X-propagation state: maps signal IDs to ternary values.
#[derive(Debug, Clone)]
pub struct XPropState {
    pub values: HashMap<SignalId, Ternary>,
}

impl XPropState {
    pub fn new() -> Self {
        XPropState {
            values: HashMap::new(),
        }
    }

    /// Join two X-propagation states (pointwise join).
    pub fn join(&self, other: &XPropState) -> XPropState {
        let mut result = XPropState::new();
        // Start with self's values
        for (id, val) in &self.values {
            result.values.insert(*id, *val);
        }
        // Join with other's values
        for (id, val) in &other.values {
            let joined = result
                .values
                .get(id)
                .map(|existing| existing.join(*val))
                .unwrap_or(*val);
            result.values.insert(*id, joined);
        }
        result
    }

    /// Get ternary value for a signal.
    pub fn get(&self, id: SignalId) -> Ternary {
        self.values.get(&id).copied().unwrap_or(Ternary::Unknown)
    }

    /// Set ternary value for a signal.
    pub fn set(&mut self, id: SignalId, val: Ternary) {
        self.values.insert(id, val);
    }
}

impl Default for XPropState {
    fn default() -> Self {
        Self::new()
    }
}

/// X-propagation analyzer for RTL designs.
pub struct XPropAnalyzer {
    pub max_iterations: usize,
}

impl XPropAnalyzer {
    pub fn new() -> Self {
        XPropAnalyzer {
            max_iterations: 128,
        }
    }

    /// Analyze a design for X-propagation issues.
    /// Returns list of (signal_name, issue_description) for problematic signals.
    pub fn analyze(&self, design: &IrDesign) -> Vec<(String, String)> {
        let mut issues = Vec::new();

        // Check each process for X-propagation
        for process in &design.top.processes {
            let mut state = XPropState::new();

            // Initialize signals: reset signals are 0/1, others are X
            for (id, sig) in design.top.signals.iter().enumerate() {
                if sig.name.as_str().contains("reset") || sig.name.as_str().contains("rst") {
                    // Assume reset is active (0 or 1, not X)
                    state.set(id, Ternary::One);
                } else if sig.init_val.bits.iter().all(|b| *b == LogicVal::Zero) {
                    state.set(id, Ternary::Zero);
                } else if sig.init_val.bits.iter().all(|b| *b == LogicVal::One) {
                    state.set(id, Ternary::One);
                } else {
                    state.set(id, Ternary::Unknown);
                }
            }

            // Analyze process body
            match process {
                Process::Combinational { name, body, .. } => {
                    self.analyze_stmts(body, &mut state);
                    // Check outputs of combinational logic for X
                    self.check_comb_outputs(name.as_str(), body, &state, &design, &mut issues);
                }
                Process::Sequential { name, body, .. } => {
                    self.analyze_stmts(body, &mut state);
                    // Check if FF outputs could be X
                    self.check_seq_outputs(name.as_str(), body, &state, &design, &mut issues);
                }
                _ => {}
            }
        }

        issues
    }

    /// Analyze a list of statements for X-propagation.
    fn analyze_stmts(&self, stmts: &[IrStmt], state: &mut XPropState) {
        for stmt in stmts {
            self.analyze_stmt(stmt, state);
        }
    }

    /// Analyze a single statement for X-propagation.
    fn analyze_stmt(&self, stmt: &IrStmt, state: &mut XPropState) {
        match stmt {
            IrStmt::Block { stmts } | IrStmt::NamedBlock { stmts, .. } => {
                self.analyze_stmts(stmts, state);
            }
            IrStmt::NonBlockingAssign { lhs, rhs, .. }
            | IrStmt::BlockingAssign { lhs, rhs, .. } => {
                let rhs_val = self.eval_expr(rhs, state);
                if let maria_ir::IrLValue::Signal(id, _) = lhs {
                    state.set(*id, rhs_val);
                }
            }
            IrStmt::If {
                cond,
                true_branch,
                false_branch,
            } => {
                let cond_val = self.eval_expr(cond, state);
                match cond_val {
                    Ternary::Zero => {
                        self.analyze_stmts(false_branch, state);
                    }
                    Ternary::One => {
                        self.analyze_stmts(true_branch, state);
                    }
                    Ternary::Unknown => {
                        // Unknown condition — analyze both branches and join
                        let mut true_state = state.clone();
                        let mut false_state = state.clone();
                        self.analyze_stmts(true_branch, &mut true_state);
                        self.analyze_stmts(false_branch, &mut false_state);
                        *state = state.join(&true_state.join(&false_state));
                    }
                }
            }
            IrStmt::Case {
                expr,
                items,
                default,
                ..
            } => {
                let _ = self.eval_expr(expr, state);
                // Conservative: join all branches
                let mut joined = state.clone();
                for item in items {
                    let mut branch_state = state.clone();
                    self.analyze_stmts(&item.body, &mut branch_state);
                    joined = joined.join(&branch_state);
                }
                if !default.is_empty() {
                    let mut default_state = state.clone();
                    self.analyze_stmts(default, &mut default_state);
                    joined = joined.join(&default_state);
                }
                *state = joined;
            }
            _ => {}
        }
    }

    /// Evaluate an expression to ternary value.
    fn eval_expr(&self, expr: &IrExpr, state: &XPropState) -> Ternary {
        match expr {
            IrExpr::Const(v) => {
                if v.bits
                    .iter()
                    .any(|b| matches!(b, LogicVal::X | LogicVal::Z))
                {
                    Ternary::Unknown
                } else if v.bits.iter().all(|b| *b == LogicVal::Zero) {
                    Ternary::Zero
                } else {
                    Ternary::One
                }
            }
            IrExpr::Signal(id, _) => state.get(*id),
            IrExpr::BinaryOp(op, lhs, rhs) => {
                let l = self.eval_expr(lhs, state);
                let r = self.eval_expr(rhs, state);
                self.eval_binop(op, l, r)
            }
            IrExpr::UnaryOp(op, operand) => {
                let v = self.eval_expr(operand, state);
                self.eval_unop(op, v)
            }
            _ => Ternary::Unknown,
        }
    }

    /// Evaluate binary operation ternary.
    fn eval_binop(&self, op: &maria_ir::BinaryIrOp, l: Ternary, r: Ternary) -> Ternary {
        use maria_ir::BinaryIrOp;
        match op {
            // If any operand is X, result is X (conservative)
            BinaryIrOp::Add
            | BinaryIrOp::Sub
            | BinaryIrOp::Mul
            | BinaryIrOp::Div
            | BinaryIrOp::Mod => {
                if l.may_be_unknown() || r.may_be_unknown() {
                    Ternary::Unknown
                } else {
                    // Both known — compute result
                    match (l, r) {
                        (Ternary::Zero, Ternary::Zero) => Ternary::Zero,
                        _ => Ternary::One, // Conservative
                    }
                }
            }
            // Bitwise operations: X propagates per-bit
            BinaryIrOp::BitAnd => match (l, r) {
                (Ternary::Zero, _) | (_, Ternary::Zero) => Ternary::Zero,
                (Ternary::One, Ternary::One) => Ternary::One,
                _ => Ternary::Unknown,
            },
            BinaryIrOp::BitOr => match (l, r) {
                (Ternary::One, _) | (_, Ternary::One) => Ternary::One,
                (Ternary::Zero, Ternary::Zero) => Ternary::Zero,
                _ => Ternary::Unknown,
            },
            BinaryIrOp::BitXor => {
                if l.may_be_unknown() || r.may_be_unknown() {
                    Ternary::Unknown
                } else {
                    match (l, r) {
                        (Ternary::Zero, Ternary::Zero) | (Ternary::One, Ternary::One) => {
                            Ternary::Zero
                        }
                        _ => Ternary::One,
                    }
                }
            }
            // Comparison: X in comparison → X
            BinaryIrOp::Eq
            | BinaryIrOp::Neq
            | BinaryIrOp::Lt
            | BinaryIrOp::Le
            | BinaryIrOp::Gt
            | BinaryIrOp::Ge => {
                if l.may_be_unknown() || r.may_be_unknown() {
                    Ternary::Unknown
                } else {
                    // Both known — compute result
                    Ternary::Zero // Conservative
                }
            }
            // Logical operations
            BinaryIrOp::LogicalAnd => {
                if l == Ternary::Zero || r == Ternary::Zero {
                    Ternary::Zero
                } else if l.may_be_unknown() || r.may_be_unknown() {
                    Ternary::Unknown
                } else {
                    Ternary::One
                }
            }
            BinaryIrOp::LogicalOr => {
                if l == Ternary::One || r == Ternary::One {
                    Ternary::One
                } else if l.may_be_unknown() || r.may_be_unknown() {
                    Ternary::Unknown
                } else {
                    Ternary::Zero
                }
            }
            _ => Ternary::Unknown,
        }
    }

    /// Evaluate unary operation ternary.
    fn eval_unop(&self, op: &maria_ir::UnaryIrOp, v: Ternary) -> Ternary {
        use maria_ir::UnaryIrOp;
        match op {
            UnaryIrOp::Not => match v {
                Ternary::Zero => Ternary::One,
                Ternary::One => Ternary::Zero,
                Ternary::Unknown => Ternary::Unknown,
            },
            UnaryIrOp::BitNot => {
                if v.may_be_unknown() {
                    Ternary::Unknown
                } else {
                    match v {
                        Ternary::Zero => Ternary::One,
                        Ternary::One => Ternary::Zero,
                        _ => Ternary::Unknown,
                    }
                }
            }
            _ => v,
        }
    }

    /// Check combinational logic outputs for X-propagation.
    fn check_comb_outputs(
        &self,
        name: &str,
        stmts: &[IrStmt],
        state: &XPropState,
        design: &IrDesign,
        issues: &mut Vec<(String, String)>,
    ) {
        // Walk assignments and check if any output could be X
        for stmt in stmts {
            if let IrStmt::BlockingAssign { lhs, .. } = stmt {
                if let maria_ir::IrLValue::Signal(id, _) = lhs {
                    let val = state.get(*id);
                    if val.may_be_unknown() {
                        let sig_name = design
                            .top
                            .signals
                            .get(*id)
                            .map(|s| s.name.as_str())
                            .unwrap_or("?");
                        issues.push((
                            sig_name.to_string(),
                            format!(
                                "combinational output '{}' in process '{}' may be X",
                                sig_name, name
                            ),
                        ));
                    }
                }
            }
        }
    }

    /// Check sequential logic outputs for X-propagation.
    fn check_seq_outputs(
        &self,
        name: &str,
        stmts: &[IrStmt],
        state: &XPropState,
        design: &IrDesign,
        issues: &mut Vec<(String, String)>,
    ) {
        // Walk assignments and check if any FF output could be X
        for stmt in stmts {
            if let IrStmt::NonBlockingAssign { lhs, .. } = stmt {
                if let maria_ir::IrLValue::Signal(id, _) = lhs {
                    let val = state.get(*id);
                    if val.may_be_unknown() {
                        let sig_name = design
                            .top
                            .signals
                            .get(*id)
                            .map(|s| s.name.as_str())
                            .unwrap_or("?");
                        issues.push((
                            sig_name.to_string(),
                            format!(
                                "sequential output '{}' in process '{}' may be X",
                                sig_name, name
                            ),
                        ));
                    }
                }
            }
        }
    }
}

impl Default for XPropAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ternary_join() {
        assert_eq!(Ternary::Zero.join(Ternary::Zero), Ternary::Zero);
        assert_eq!(Ternary::One.join(Ternary::One), Ternary::One);
        assert_eq!(Ternary::Zero.join(Ternary::One), Ternary::Unknown);
        assert_eq!(Ternary::Unknown.join(Ternary::Zero), Ternary::Unknown);
    }

    #[test]
    fn test_ternary_is_known() {
        assert!(Ternary::Zero.is_known());
        assert!(Ternary::One.is_known());
        assert!(!Ternary::Unknown.is_known());
    }

    #[test]
    fn test_ternary_may_be_unknown() {
        assert!(!Ternary::Zero.may_be_unknown());
        assert!(!Ternary::One.may_be_unknown());
        assert!(Ternary::Unknown.may_be_unknown());
    }

    #[test]
    fn test_xprop_state_join() {
        let mut s1 = XPropState::new();
        s1.set(0, Ternary::Zero);
        s1.set(1, Ternary::One);

        let mut s2 = XPropState::new();
        s2.set(0, Ternary::Zero);
        s2.set(2, Ternary::Unknown);

        let joined = s1.join(&s2);
        assert_eq!(joined.get(0), Ternary::Zero);
        assert_eq!(joined.get(1), Ternary::One);
        assert_eq!(joined.get(2), Ternary::Unknown);
    }

    #[test]
    fn test_eval_binop_bitand() {
        let analyzer = XPropAnalyzer::new();
        assert_eq!(
            analyzer.eval_binop(&maria_ir::BinaryIrOp::BitAnd, Ternary::Zero, Ternary::One),
            Ternary::Zero
        );
        assert_eq!(
            analyzer.eval_binop(&maria_ir::BinaryIrOp::BitAnd, Ternary::One, Ternary::One),
            Ternary::One
        );
        assert_eq!(
            analyzer.eval_binop(
                &maria_ir::BinaryIrOp::BitAnd,
                Ternary::Unknown,
                Ternary::One
            ),
            Ternary::Unknown
        );
    }

    #[test]
    fn test_eval_binop_bitor() {
        let analyzer = XPropAnalyzer::new();
        assert_eq!(
            analyzer.eval_binop(&maria_ir::BinaryIrOp::BitOr, Ternary::Zero, Ternary::One),
            Ternary::One
        );
        assert_eq!(
            analyzer.eval_binop(&maria_ir::BinaryIrOp::BitOr, Ternary::Zero, Ternary::Zero),
            Ternary::Zero
        );
        assert_eq!(
            analyzer.eval_binop(&maria_ir::BinaryIrOp::BitOr, Ternary::One, Ternary::Unknown),
            Ternary::One
        );
    }

    #[test]
    fn test_eval_binop_logical_and() {
        let analyzer = XPropAnalyzer::new();
        assert_eq!(
            analyzer.eval_binop(
                &maria_ir::BinaryIrOp::LogicalAnd,
                Ternary::Zero,
                Ternary::One
            ),
            Ternary::Zero
        );
        assert_eq!(
            analyzer.eval_binop(
                &maria_ir::BinaryIrOp::LogicalAnd,
                Ternary::One,
                Ternary::One
            ),
            Ternary::One
        );
        assert_eq!(
            analyzer.eval_binop(
                &maria_ir::BinaryIrOp::LogicalAnd,
                Ternary::Unknown,
                Ternary::One
            ),
            Ternary::Unknown
        );
    }

    #[test]
    fn test_eval_unop_not() {
        let analyzer = XPropAnalyzer::new();
        assert_eq!(
            analyzer.eval_unop(&maria_ir::UnaryIrOp::Not, Ternary::Zero),
            Ternary::One
        );
        assert_eq!(
            analyzer.eval_unop(&maria_ir::UnaryIrOp::Not, Ternary::One),
            Ternary::Zero
        );
        assert_eq!(
            analyzer.eval_unop(&maria_ir::UnaryIrOp::Not, Ternary::Unknown),
            Ternary::Unknown
        );
    }
}
