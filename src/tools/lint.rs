//! `mlint` — Static RTL Linter.
//!
//! Check: unused signal, width mismatch, latch detection, combinational loop,
//! FSM state register.

use std::collections::{HashMap, HashSet};

use crate::ast::expr::Expr;
use crate::ast::stmt::Stmt;
use crate::ast::types::ModuleItem;
use crate::error::SimError;
use crate::intern::Symbol;
use crate::tools::{open_project, section};

/// Opsi mlint.
pub struct LintArgs<'a> {
    pub targets: &'a [String],
    pub incdirs: &'a [String],
    pub defines: &'a [String],
    pub all: bool,
    pub unused: bool,
    pub width: bool,
    pub latch: bool,
    pub loop_check: bool,
    pub fsm: bool,
    pub quiet: bool,
}

/// Satu temuan lint.
struct Finding {
    module: String,
    check: &'static str,
    severity: &'static str, // "W" warning, "E" error
    message: String,
}

/// Jalankan mlint.
pub fn run(args: &LintArgs) -> Result<(), SimError> {
    let all = args.all;
    let chk = |flag: bool| flag || all;

    let do_unused = chk(args.unused);
    let do_width = chk(args.width);
    let do_latch = chk(args.latch);
    let do_loop = chk(args.loop_check);
    let do_fsm = chk(args.fsm);

    let (design, _session) = open_project(args.targets, args.incdirs, args.defines, None)?;

    let mut findings: Vec<Finding> = Vec::new();
    for module in &design.modules {
        lint_module(module, do_unused, do_width, do_latch, do_loop, do_fsm, &mut findings);
    }
    // Interface juga di-lint (items = ModuleItem)
    for iface in &design.interfaces {
        lint_items(
            iface.name.as_str(),
            &iface.decls,
            &iface.ports,
            &iface.items,
            do_unused,
            do_width,
            do_latch,
            do_loop,
            do_fsm,
            &mut findings,
        );
    }

    findings.sort_by(|a, b| a.module.cmp(&b.module).then(a.check.cmp(b.check)));

    let n_warn = findings.iter().filter(|f| f.severity == "W").count();
    let n_err = findings.iter().filter(|f| f.severity == "E").count();

    section("mlint Report");
    for f in &findings {
        if args.quiet {
            continue;
        }
        println!(
            "  [{}] {:<10} {:<12} {}",
            f.severity,
            f.check,
            f.module,
            f.message
        );
    }
    println!("\n  {} warning, {} error", n_warn, n_err);

    if n_err > 0 {
        return Err(SimError::with_diag(
            crate::diagnostics::DiagCode::InvalidSyntax,
            format!("mlint menemukan {} error", n_err),
        ));
    }
    Ok(())
}

/// ── Analisis per module ──

fn lint_module(
    module: &crate::ast::types::Module,
    do_unused: bool,
    do_width: bool,
    do_latch: bool,
    do_loop: bool,
    do_fsm: bool,
    out: &mut Vec<Finding>,
) {
    lint_items(
        module.name.as_str(),
        &module.decls,
        &module.ports,
        &module.items,
        do_unused,
        do_width,
        do_latch,
        do_loop,
        do_fsm,
        out,
    );
}

fn lint_items(
    scope: &str,
    decls: &[crate::ast::types::Decl],
    ports: &[crate::ast::types::Port],
    items: &[ModuleItem],
    do_unused: bool,
    do_width: bool,
    do_latch: bool,
    do_loop: bool,
    do_fsm: bool,
    out: &mut Vec<Finding>,
) {
    // ── Kumpulkan deklarasi sinyal (decls + decl items + ports) ──
    let mut declared: HashMap<Symbol, usize> = HashMap::new(); // name → width
    let mut declared_raw: Vec<Symbol> = Vec::new();
    let mut collect_decls = |d: &crate::ast::types::Decl| {
        for v in &d.names {
            let w = decl_width(d, v);
            declared.insert(v.name, w);
            declared_raw.push(v.name);
        }
    };
    for d in decls {
        collect_decls(d);
    }
    for item in items {
        if let ModuleItem::Decl(d) = item {
            collect_decls(d);
        }
    }
    let port_set: HashSet<Symbol> = ports.iter().map(|p| p.name).collect();
    for p in ports {
        let w = p
            .range
            .as_ref()
            .map(|r| r.width())
            .unwrap_or(1);
        declared.insert(p.name, w);
    }

    // ── Walk semua always/initial/assign ──
    let mut reads: HashSet<Symbol> = HashSet::new();
    let mut writes: HashSet<Symbol> = HashSet::new();
    let mut always_blocks: Vec<&crate::ast::stmt::AlwaysBlock> = Vec::new();

    for item in items {
        match item {
            ModuleItem::Always(block) => {
                always_blocks.push(block);
                scan_stmt_reads(&block.stmts, &mut reads, &mut writes);
            }
            ModuleItem::Initial(block) => {
                scan_stmt_reads(&block.stmts, &mut reads, &mut writes);
            }
            ModuleItem::Final(block) => {
                scan_stmt_reads(&block.stmts, &mut reads, &mut writes);
            }
            ModuleItem::Assign(ca) => {
                scan_expr_reads(&ca.lhs, &mut reads, &mut writes);
                scan_expr_reads(&ca.rhs, &mut reads, &mut writes);
                if let Some(root) = lvalue_root(&ca.lhs) {
                    writes.insert(root);
                }
                if do_width {
                    check_width(scope, &ca.lhs, &ca.rhs, &declared, out);
                }
            }
            ModuleItem::Generate(g) => {
                for gi in &g.items {
                    walk_generate(gi, &mut reads, &mut writes, &mut always_blocks, do_width, &declared, out);
                }
            }
            _ => {}
        }
    }

    // Signal yang juga dipakai di generate item declarations? Abaikan untuk now.

    if do_unused {
        for name in &declared_raw {
            if port_set.contains(name) {
                continue;
            }
            let r = reads.contains(name);
            let w = writes.contains(name);
            if !r && !w {
                out.push(Finding {
                    module: scope.to_string(),
                    check: "unused",
                    severity: "W",
                    message: format!("signal '{}' tidak pernah dipakai", name.as_str()),
                });
            } else if w && !r {
                out.push(Finding {
                    module: scope.to_string(),
                    check: "unused",
                    severity: "W",
                    message: format!("signal '{}' hanya ditulis, tidak pernah dibaca", name.as_str()),
                });
            }
        }
    }

    if do_latch {
        for block in &always_blocks {
            if matches!(block.kind, crate::ast::stmt::AlwaysKind::AlwaysLatch) {
                out.push(Finding {
                    module: scope.to_string(),
                    check: "latch",
                    severity: "W",
                    message: "block memakai always_latch — pastikan intentional".to_string(),
                });
            }
            // always_comb dengan if tanpa else → potensi latch
            if matches!(block.kind, crate::ast::stmt::AlwaysKind::AlwaysComb | crate::ast::stmt::AlwaysKind::AlwaysLatch) {
                find_incomplete_if(scope, &block.stmts, out);
            }
        }
    }

    if do_loop {
        for block in &always_blocks {
            if !matches!(block.kind, crate::ast::stmt::AlwaysKind::AlwaysComb) {
                continue;
            }
            let mut r = HashSet::new();
            let mut w = HashSet::new();
            scan_stmt_reads(&block.stmts, &mut r, &mut w);
            let overlap: Vec<Symbol> = w.intersection(&r).copied().collect();
            for s in overlap {
                if port_set.contains(&s) {
                    continue;
                }
                out.push(Finding {
                    module: scope.to_string(),
                    check: "comb_loop",
                    severity: "W",
                    message: format!("signal '{}' dibaca & ditulis di block yang sama → potensi combinational loop", s.as_str()),
                });
            }
        }
    }

    if do_fsm {
        for block in &always_blocks {
            if !matches!(block.kind, crate::ast::stmt::AlwaysKind::AlwaysFF) {
                continue;
            }
            find_fsm(scope, &block.stmts, out);
        }
    }
}

fn walk_generate<'a>(
    gi: &'a crate::ast::types::GenerateItem,
    reads: &mut HashSet<Symbol>,
    writes: &mut HashSet<Symbol>,
    always_blocks: &mut Vec<&'a crate::ast::stmt::AlwaysBlock>,
    do_width: bool,
    declared: &HashMap<Symbol, usize>,
    out: &mut Vec<Finding>,
) {
    use crate::ast::types::GenerateItem;
    match gi {
        GenerateItem::Items(items) => walk_items(items, reads, writes, always_blocks, do_width, declared, out),
        GenerateItem::If { true_items, false_items, .. } => {
            walk_items(true_items, reads, writes, always_blocks, do_width, declared, out);
            walk_items(false_items, reads, writes, always_blocks, do_width, declared, out);
        }
        GenerateItem::For { body_items, .. } => {
            walk_items(body_items, reads, writes, always_blocks, do_width, declared, out);
        }
        GenerateItem::Case { items, default, .. } => {
            for ci in items {
                walk_items(&ci.body, reads, writes, always_blocks, do_width, declared, out);
            }
            if let Some(d) = default {
                walk_items(d, reads, writes, always_blocks, do_width, declared, out);
            }
        }
    }
}

fn walk_items<'a>(
    items: &'a [ModuleItem],
    reads: &mut HashSet<Symbol>,
    writes: &mut HashSet<Symbol>,
    always_blocks: &mut Vec<&'a crate::ast::stmt::AlwaysBlock>,
    do_width: bool,
    declared: &HashMap<Symbol, usize>,
    out: &mut Vec<Finding>,
) {
    for item in items {
        match item {
            ModuleItem::Always(b) => always_blocks.push(b),
            ModuleItem::Assign(ca) => {
                scan_expr_reads(&ca.lhs, reads, writes);
                scan_expr_reads(&ca.rhs, reads, writes);
                if let Some(root) = lvalue_root(&ca.lhs) {
                    writes.insert(root);
                }
                if do_width {
                    check_width("generate", &ca.lhs, &ca.rhs, declared, out);
                }
            }
            ModuleItem::Generate(g) => {
                for gi in &g.items {
                    walk_generate(gi, reads, writes, always_blocks, do_width, declared, out);
                }
            }
            _ => {}
        }
    }
}

/// ── Width ──

fn decl_width(d: &crate::ast::types::Decl, v: &crate::ast::types::DeclVar) -> usize {
    if let Some(r) = &v.range {
        return r.width();
    }
    match &d.dtype {
        crate::ast::types::DataType::Byte => 8,
        crate::ast::types::DataType::Shortint => 16,
        crate::ast::types::DataType::Int | crate::ast::types::DataType::Integer => 32,
        crate::ast::types::DataType::Longint => 64,
        crate::ast::types::DataType::Bit | crate::ast::types::DataType::Logic => 1,
        _ => 1,
    }
}

fn expr_width(e: &Expr, declared: &HashMap<Symbol, usize>) -> Option<usize> {
    match e {
        Expr::Ident { name, .. } => declared.get(name).copied(),
        Expr::Value(v) => Some(match v {
            crate::ast::expr::Value::Binary { width, .. }
            | crate::ast::expr::Value::Hex { width, .. }
            | crate::ast::expr::Value::Octal { width, .. } => width.unwrap_or(1),
            crate::ast::expr::Value::Decimal(_) => 32,
            crate::ast::expr::Value::Real(_) => 64,
        }),
        Expr::Concat(parts) => {
            let mut total = 0usize;
            for p in parts {
                total += expr_width(p, declared)?;
            }
            Some(total)
        }
        Expr::BitSelect { expr, .. } => Some(1),
        Expr::RangeSelect { expr, msb, lsb, .. } => {
            // width = |msb - lsb| + 1 jika keduanya konstanta
            if let (Expr::Value(crate::ast::expr::Value::Decimal(m)), Expr::Value(crate::ast::expr::Value::Decimal(l))) =
                (&**msb, &**lsb)
            {
                Some(m.abs_diff(*l) as usize + 1)
            } else {
                expr_width(expr, declared)
            }
        }
        Expr::BinaryOp { lhs, rhs, op } => {
            let lw = expr_width(lhs, declared)?;
            let rw = expr_width(rhs, declared)?;
            match op {
                crate::ast::expr::BinaryOp::Eq
                | crate::ast::expr::BinaryOp::Neq
                | crate::ast::expr::BinaryOp::CaseEq
                | crate::ast::expr::BinaryOp::CaseNeq
                | crate::ast::expr::BinaryOp::Lt
                | crate::ast::expr::BinaryOp::Le
                | crate::ast::expr::BinaryOp::Gt
                | crate::ast::expr::BinaryOp::Ge
                | crate::ast::expr::BinaryOp::LogicalAnd
                | crate::ast::expr::BinaryOp::LogicalOr => Some(1),
                _ => Some(lw.max(rw)),
            }
        }
        Expr::UnaryOp { expr, .. } => expr_width(expr, declared),
        Expr::TernaryOp { true_expr, false_expr, .. } => {
            let t = expr_width(true_expr, declared)?;
            let f = expr_width(false_expr, declared)?;
            Some(t.max(f))
        }
        Expr::Paren(inner) => expr_width(inner, declared),
        Expr::FillLit(_) => Some(1),
        Expr::Cast { expr, .. } => expr_width(expr, declared),
        Expr::CastWidth { width, .. } => expr_width(width, declared),
        Expr::MemberAccess { .. } => None,
        _ => None,
    }
}

fn check_width(
    scope: &str,
    lhs: &Expr,
    rhs: &Expr,
    declared: &HashMap<Symbol, usize>,
    out: &mut Vec<Finding>,
) {
    let Some(lw) = expr_width(lhs, declared) else { return };
    let Some(rw) = expr_width(rhs, declared) else { return };
    if lw != rw {
        out.push(Finding {
            module: scope.to_string(),
            check: "width",
            severity: "W",
            message: format!(
                "width mismatch: lhs {} bit vs rhs {} bit",
                lw, rw
            ),
        });
    }
}

/// ── Read/write collection ──

fn lvalue_root(e: &Expr) -> Option<Symbol> {
    match e {
        Expr::Ident { name, .. } => Some(*name),
        Expr::BitSelect { expr, .. } | Expr::RangeSelect { expr, .. } | Expr::PartSelect { expr, .. } => {
            lvalue_root(expr)
        }
        Expr::MemberAccess { obj, .. } => lvalue_root(obj),
        Expr::Concat(parts) => parts.iter().find_map(lvalue_root),
        _ => None,
    }
}

fn scan_expr_reads(e: &Expr, reads: &mut HashSet<Symbol>, writes: &mut HashSet<Symbol>) {
    match e {
        Expr::Ident { name, .. } => {
            reads.insert(*name);
        }
        Expr::Value(_) | Expr::Null | Expr::String(_) | Expr::FillLit(_) => {}
        Expr::FuncCall { name, args } => {
            reads.insert(*name);
            for a in args {
                scan_expr_reads(a, reads, writes);
            }
        }
        Expr::ScopedIdent { package, item, .. } => {
            reads.insert(*package);
            reads.insert(*item);
        }
        Expr::RangeSelect { expr, msb, lsb } => {
            scan_expr_reads(expr, reads, writes);
            scan_expr_reads(msb, reads, writes);
            scan_expr_reads(lsb, reads, writes);
        }
        Expr::BitSelect { expr, index } => {
            scan_expr_reads(expr, reads, writes);
            scan_expr_reads(index, reads, writes);
        }
        Expr::PartSelect { expr, base, width } => {
            scan_expr_reads(expr, reads, writes);
            scan_expr_reads(base, reads, writes);
            scan_expr_reads(width, reads, writes);
        }
        Expr::Concat(parts) => {
            for p in parts {
                scan_expr_reads(p, reads, writes);
            }
        }
        Expr::Replicate { count, expr } => {
            scan_expr_reads(count, reads, writes);
            scan_expr_reads(expr, reads, writes);
        }
        Expr::UnaryOp { expr, .. } => scan_expr_reads(expr, reads, writes),
        Expr::BinaryOp { lhs, rhs, .. } => {
            scan_expr_reads(lhs, reads, writes);
            scan_expr_reads(rhs, reads, writes);
        }
        Expr::TernaryOp { cond, true_expr, false_expr } => {
            scan_expr_reads(cond, reads, writes);
            scan_expr_reads(true_expr, reads, writes);
            scan_expr_reads(false_expr, reads, writes);
        }
        Expr::Paren(inner) => scan_expr_reads(inner, reads, writes),
        Expr::MethodCall { obj, method, args, with_clause } => {
            reads.insert(*method);
            scan_expr_reads(obj, reads, writes);
            for a in args {
                scan_expr_reads(a, reads, writes);
            }
            if let Some(wc) = with_clause {
                scan_expr_reads(wc, reads, writes);
            }
        }
        Expr::MemberAccess { obj, field } => {
            reads.insert(*field);
            scan_expr_reads(obj, reads, writes);
        }
        Expr::Inside { expr, range_list } => {
            scan_expr_reads(expr, reads, writes);
            for r in range_list {
                scan_expr_reads(r, reads, writes);
            }
        }
        Expr::StreamingConcat { slice_size, slices, .. } => {
            if let Some(s) = slice_size {
                scan_expr_reads(s, reads, writes);
            }
            for s in slices {
                scan_expr_reads(s, reads, writes);
            }
        }
        Expr::Cast { dtype, expr } => {
            reads.insert(*dtype);
            scan_expr_reads(expr, reads, writes);
        }
        Expr::CastWidth { width, expr } => {
            scan_expr_reads(width, reads, writes);
            scan_expr_reads(expr, reads, writes);
        }
        Expr::Dist { expr, items } => {
            scan_expr_reads(expr, reads, writes);
            for it in items {
                match it {
                    crate::ast::expr::DistItem::Value(v, _) => scan_expr_reads(v, reads, writes),
                    crate::ast::expr::DistItem::Range(a, b, _) => {
                        scan_expr_reads(a, reads, writes);
                        scan_expr_reads(b, reads, writes);
                    }
                }
            }
        }
    }
}

fn scan_stmt_reads(stmts: &[Stmt], reads: &mut HashSet<Symbol>, writes: &mut HashSet<Symbol>) {
    for stmt in stmts {
        match stmt {
            Stmt::Block { stmts } | Stmt::LoopForever { stmts } | Stmt::NamedBlock { stmts, .. } => {
                scan_stmt_reads(stmts, reads, writes);
            }
            Stmt::IfElse { cond, true_branch, false_branch } => {
                scan_expr_reads(cond, reads, writes);
                scan_stmt_reads(std::slice::from_ref(true_branch), reads, writes);
                if let Some(fb) = false_branch {
                    scan_stmt_reads(std::slice::from_ref(fb), reads, writes);
                }
            }
            Stmt::Case { expr, items, default }
            | Stmt::CaseX { expr, items, default }
            | Stmt::CaseZ { expr, items, default }
            | Stmt::StmtCase { expr, items, default }
            | Stmt::UniqueCase { expr, items, default }
            | Stmt::PriorityCase { expr, items, default }
            | Stmt::CaseInside { expr, items, default } => {
                scan_expr_reads(expr, reads, writes);
                for ci in items {
                    for l in &ci.labels {
                        scan_expr_reads(l, reads, writes);
                    }
                    scan_stmt_reads(std::slice::from_ref(&ci.stmt), reads, writes);
                }
                if let Some(d) = default {
                    scan_stmt_reads(std::slice::from_ref(d), reads, writes);
                }
            }
            Stmt::LoopWhile { cond, stmts } | Stmt::Repeat { count: cond, stmts } | Stmt::DoWhile { cond, stmts } => {
                scan_expr_reads(cond, reads, writes);
                scan_stmt_reads(stmts, reads, writes);
            }
            Stmt::LoopFor { init, cond, step, stmts } => {
                if let Some(i) = init {
                    scan_stmt_reads(std::slice::from_ref(i), reads, writes);
                }
                if let Some(c) = cond {
                    scan_expr_reads(c, reads, writes);
                }
                if let Some(s) = step {
                    scan_stmt_reads(std::slice::from_ref(s), reads, writes);
                }
                scan_stmt_reads(stmts, reads, writes);
            }
            Stmt::BlockingAssign { lhs, rhs, .. }
            | Stmt::NonBlockingAssign { lhs, rhs, .. }
            | Stmt::StmtAssign { lhs, rhs } => {
                scan_expr_reads(rhs, reads, writes);
                if let Some(root) = lvalue_root(lhs) {
                    writes.insert(root);
                }
            }
            Stmt::Force { lhs, rhs } => {
                scan_expr_reads(rhs, reads, writes);
                if let Some(root) = lvalue_root(lhs) {
                    writes.insert(root);
                }
            }
            Stmt::Release { expr } => {
                if let Some(root) = lvalue_root(expr) {
                    writes.insert(root);
                }
            }
            Stmt::Deassign { expr } => {
                if let Some(root) = lvalue_root(expr) {
                    writes.insert(root);
                }
            }
            Stmt::Expr { expr } => scan_expr_reads(expr, reads, writes),
            Stmt::SysCall { name, args } => {
                reads.insert(*name);
                for a in args {
                    scan_expr_reads(a, reads, writes);
                }
            }
            Stmt::Delay { delay, stmt } => {
                scan_expr_reads(delay, reads, writes);
                scan_stmt_reads(std::slice::from_ref(stmt), reads, writes);
            }
            Stmt::Wait { cond, stmt } => {
                scan_expr_reads(cond, reads, writes);
                if let Some(s) = stmt {
                    scan_stmt_reads(std::slice::from_ref(s), reads, writes);
                }
            }
            Stmt::EventControl { events, stmt } => {
                for ev in events {
                    match ev {
                        crate::ast::stmt::SensitivityEvent::PosEdge(e)
                        | crate::ast::stmt::SensitivityEvent::NegEdge(e)
                        | crate::ast::stmt::SensitivityEvent::Level(e) => {
                            scan_expr_reads(e, reads, writes);
                        }
                        crate::ast::stmt::SensitivityEvent::Wildcard => {}
                    }
                }
                if let Some(s) = stmt {
                    scan_stmt_reads(std::slice::from_ref(s), reads, writes);
                }
            }
            Stmt::Assert {
                cond,
                pass_stmt,
                fail_stmt,
                clock_event,
                disable_iff,
            }
            | Stmt::Assume {
                cond,
                pass_stmt,
                fail_stmt,
                clock_event,
                disable_iff,
            } => {
                scan_expr_reads(cond, reads, writes);
                if let Some(ce) = clock_event {
                    if let crate::ast::types::ClockEvent::Posedge(s) | crate::ast::types::ClockEvent::Negedge(s) | crate::ast::types::ClockEvent::Edge(s) = ce {
                        reads.insert(*s);
                    }
                }
                if let Some(d) = disable_iff {
                    scan_expr_reads(d, reads, writes);
                }
                if let Some(ps) = pass_stmt {
                    scan_stmt_reads(std::slice::from_ref(ps), reads, writes);
                }
                if let Some(fs) = fail_stmt {
                    scan_stmt_reads(std::slice::from_ref(fs), reads, writes);
                }
            }
            Stmt::Cover {
                cond,
                pass_stmt,
                clock_event,
                disable_iff,
            } => {
                scan_expr_reads(cond, reads, writes);
                if let Some(ce) = clock_event {
                    if let crate::ast::types::ClockEvent::Posedge(s) | crate::ast::types::ClockEvent::Negedge(s) | crate::ast::types::ClockEvent::Edge(s) = ce {
                        reads.insert(*s);
                    }
                }
                if let Some(d) = disable_iff {
                    scan_expr_reads(d, reads, writes);
                }
                if let Some(ps) = pass_stmt {
                    scan_stmt_reads(std::slice::from_ref(ps), reads, writes);
                }
            }
            Stmt::Expect {
                cond,
                pass_stmt,
                fail_stmt,
            } => {
                scan_expr_reads(cond, reads, writes);
                if let Some(ps) = pass_stmt {
                    scan_stmt_reads(std::slice::from_ref(ps), reads, writes);
                }
                if let Some(fs) = fail_stmt {
                    scan_stmt_reads(std::slice::from_ref(fs), reads, writes);
                }
            }
            Stmt::WaitOrder { events, fail_stmt } => {
                for e in events {
                    reads.insert(*e);
                }
                if let Some(fs) = fail_stmt {
                    scan_stmt_reads(std::slice::from_ref(fs), reads, writes);
                }
            }
            Stmt::UniqueIf { cond, true_branch, false_branch }
            | Stmt::PriorityIf { cond, true_branch, false_branch } => {
                scan_expr_reads(cond, reads, writes);
                scan_stmt_reads(std::slice::from_ref(true_branch), reads, writes);
                if let Some(fb) = false_branch {
                    scan_stmt_reads(std::slice::from_ref(fb), reads, writes);
                }
            }
            Stmt::Fork { processes, .. } => {
                for p in processes {
                    scan_stmt_reads(std::slice::from_ref(p), reads, writes);
                }
            }
            Stmt::RandCase { items } => {
                for it in items {
                    scan_stmt_reads(std::slice::from_ref(&it.stmt), reads, writes);
                }
            }
            Stmt::RandSequence { productions } => {
                for p in productions {
                    for it in &p.items {
                        scan_stmt_reads(std::slice::from_ref(&it.value), reads, writes);
                    }
                }
            }
            Stmt::Return(Some(e)) => scan_expr_reads(e, reads, writes),
            Stmt::Disable { name } => {
                reads.insert(*name);
            }
            Stmt::EventTrigger { name } => {
                writes.insert(*name);
            }
            Stmt::ForeachLoop { array_var, index_vars, stmts } => {
                reads.insert(*array_var);
                for iv in index_vars {
                    reads.insert(*iv);
                }
                scan_stmt_reads(stmts, reads, writes);
            }
            Stmt::Break | Stmt::Continue | Stmt::Null | Stmt::SysFinish | Stmt::Return(None) => {}
        }
    }
}

/// ── Latch ──

fn find_incomplete_if(scope: &str, stmts: &[Stmt], out: &mut Vec<Finding>) {
    for stmt in stmts {
        match stmt {
            Stmt::IfElse { false_branch, .. } if false_branch.is_none() => {
                out.push(Finding {
                    module: scope.to_string(),
                    check: "latch",
                    severity: "W",
                    message: "if tanpa else di blok kombinasional → potensi latch".to_string(),
                });
            }
            Stmt::IfElse { true_branch, false_branch, .. } => {
                find_incomplete_if(scope, std::slice::from_ref(true_branch), out);
                if let Some(fb) = false_branch {
                    find_incomplete_if(scope, std::slice::from_ref(fb), out);
                }
            }
            Stmt::Block { stmts } | Stmt::NamedBlock { stmts, .. } | Stmt::LoopForever { stmts } => {
                find_incomplete_if(scope, stmts, out);
            }
            _ => {}
        }
    }
}

/// ── FSM ──

fn find_fsm(scope: &str, stmts: &[Stmt], out: &mut Vec<Finding>) {
    for stmt in stmts {
        match stmt {
            Stmt::Case { expr, .. }
            | Stmt::CaseX { expr, .. }
            | Stmt::CaseZ { expr, .. }
            | Stmt::UniqueCase { expr, .. }
            | Stmt::PriorityCase { expr, .. }
            | Stmt::CaseInside { expr, .. } => {
                if let Expr::Ident { name, .. } = expr {
                    if name.as_str().contains("state") || name.as_str().contains("fsm") {
                        out.push(Finding {
                            module: scope.to_string(),
                            check: "fsm",
                            severity: "I",
                            message: format!(
                                "deteksi FSM: register state '{}' ({} case item)",
                                name.as_str(),
                                case_item_count(stmt)
                            ),
                        });
                    }
                }
            }
            Stmt::Block { stmts } | Stmt::NamedBlock { stmts, .. } | Stmt::LoopForever { stmts } => {
                find_fsm(scope, stmts, out);
            }
            Stmt::IfElse { true_branch, false_branch, .. } => {
                find_fsm(scope, std::slice::from_ref(true_branch), out);
                if let Some(fb) = false_branch {
                    find_fsm(scope, std::slice::from_ref(fb), out);
                }
            }
            _ => {}
        }
    }
}

fn case_item_count(stmt: &Stmt) -> usize {
    match stmt {
        Stmt::Case { items, .. }
        | Stmt::CaseX { items, .. }
        | Stmt::CaseZ { items, .. }
        | Stmt::UniqueCase { items, .. }
        | Stmt::PriorityCase { items, .. }
        | Stmt::CaseInside { items, .. } => items.len(),
        _ => 0,
    }
}
