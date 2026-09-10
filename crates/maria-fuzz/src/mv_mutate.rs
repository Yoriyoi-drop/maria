//! Mutasi AST Maria-MV (Level 2–4: DSL AST / semantic / structural).
//!
//! Backend MvMediated: mutasi di level AST `MvFile`, lalu di-canonical-kan
//! lewat `maria_mv::print::print_file` → teks `.mv` → `mv_lower`. Mutation
//! history dicatat utk reproducibility (task §9).
//!
//! 1 file = 1 tanggung jawab: mutasi AST .mv.

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::Rng;

use maria_api::mv::ast::{Dir, Expr, MItem, MvFile, MvType, Stmt};

/// Jumlah operator mutasi MV.
pub const NUM_MV_OPS: usize = 12;

/// Statistik & bobot adaptif per operator MV.
#[derive(Debug, Clone)]
pub struct MvOpStats {
    pub attempts: [u64; NUM_MV_OPS],
    pub novels: [u64; NUM_MV_OPS],
    weights: [f64; NUM_MV_OPS],
}

impl Default for MvOpStats {
    fn default() -> Self {
        MvOpStats {
            attempts: [0; NUM_MV_OPS],
            novels: [0; NUM_MV_OPS],
            // Bobot awal seragam 1.0 — derive Default memberi 0.0 → pick_op
            // `gen_range(0.0..0.0)` panic (rand).
            weights: [1.0; NUM_MV_OPS],
        }
    }
}

impl MvOpStats {
    pub fn record_attempt(&mut self, op: usize) {
        if op < NUM_MV_OPS {
            self.attempts[op] += 1;
        }
    }
    pub fn record_outcome(&mut self, op: usize, novel: bool) {
        if op >= NUM_MV_OPS {
            return;
        }
        if novel {
            self.weights[op] = (self.weights[op] * 1.25).min(20.0);
        } else {
            self.weights[op] = (self.weights[op] * 0.995).max(0.2);
        }
    }
    pub fn merge(&mut self, other: &MvOpStats) {
        for i in 0..NUM_MV_OPS {
            self.attempts[i] += other.attempts[i];
            self.novels[i] += other.novels[i];
            self.weights[i] = self.weights[i].max(other.weights[i]);
        }
    }
}

fn int(v: i64) -> Expr {
    Expr::Int(v)
}

// ──────────────────────────────────────────────────────────────────────
// Traversal (recursive, mutable, stop-on-true)
// ──────────────────────────────────────────────────────────────────────

/// Kunjungi SEMUA ekspresi dalam satu ekspresi (f tiap node; true → stop).
fn foreach_expr<F>(e: &mut Expr, f: &mut F) -> bool
where
    F: FnMut(&mut Expr) -> bool,
{
    if f(e) {
        return true;
    }
    match e {
        Expr::Unary(_, x)
        | Expr::Paren(x)
        | Expr::IncDec { expr: x, .. }
        | Expr::Cast { expr: x, .. } => foreach_expr(x, f),
        Expr::Binary(_, l, r) => foreach_expr(l, f) || foreach_expr(r, f),
        Expr::Ternary(c, t, el) => foreach_expr(c, f) || foreach_expr(t, f) || foreach_expr(el, f),
        Expr::Call(_, args) => args.iter_mut().any(|a| foreach_expr(a, f)),
        Expr::MethodCall { obj, args, .. } => {
            foreach_expr(obj, f) || args.iter_mut().any(|a| foreach_expr(a, f))
        }
        Expr::Member(o, _, _, _) => foreach_expr(o, f),
        Expr::Index(o, i) => foreach_expr(o, f) || foreach_expr(i, f),
        Expr::Range(o, a, b) => foreach_expr(o, f) || foreach_expr(a, f) || foreach_expr(b, f),
        Expr::Concat(xs) | Expr::ArrayLit(xs) => xs.iter_mut().any(|x| foreach_expr(x, f)),
        Expr::Replicate(n, x) => foreach_expr(n, f) || foreach_expr(x, f),
        Expr::Inside { expr, items } => {
            if foreach_expr(expr, f) {
                return true;
            }
            for it in items {
                let hit = match it {
                    maria_api::mv::ast::InsideItem::Value(e) => foreach_expr(e, f),
                    maria_api::mv::ast::InsideItem::Range(a, b) => {
                        foreach_expr(a, f) || foreach_expr(b, f)
                    }
                };
                if hit {
                    return true;
                }
            }
            false
        }
        Expr::Dist { expr, items } => {
            if foreach_expr(expr, f) {
                return true;
            }
            for d in items {
                if let Some((lo, hi)) = &mut d.range {
                    if foreach_expr(lo, f) || foreach_expr(hi, f) {
                        return true;
                    }
                }
                if foreach_expr(&mut d.value, f) || foreach_expr(&mut d.weight, f) {
                    return true;
                }
            }
            false
        }
        Expr::NamedArg { expr, .. } => foreach_expr(expr, f),
        _ => false,
    }
}

/// Kunjungi semua statement dalam satu statement tree (f tiap node; true → stop).
fn foreach_stmt<F>(s: &mut Stmt, f: &mut F) -> bool
where
    F: FnMut(&mut Stmt) -> bool,
{
    if f(s) {
        return true;
    }
    match s {
        Stmt::Block(xs) => xs.iter_mut().any(|x| foreach_stmt(x, f)),
        Stmt::If { then, els, .. } => {
            foreach_stmt(then, f) || els.as_mut().map(|e| foreach_stmt(e, f)).unwrap_or(false)
        }
        Stmt::Case { items, default, .. } => {
            for (_, body) in items {
                if foreach_stmt(body, f) {
                    return true;
                }
            }
            default.as_mut().map(|d| foreach_stmt(d, f)).unwrap_or(false)
        }
        Stmt::For { body, .. } => foreach_stmt(body, f),
        Stmt::While { body, .. } | Stmt::Wait { body, .. } => foreach_stmt(body, f),
        Stmt::DoWhile { body, .. } => foreach_stmt(body, f),
        Stmt::Repeat { body, .. } => foreach_stmt(body, f),
        Stmt::Forever(body) => foreach_stmt(body, f),
        Stmt::Event { body, .. } => foreach_stmt(body, f),
        Stmt::Delay { body, .. } => foreach_stmt(body, f),
        Stmt::Fork { branches, .. } => branches.iter_mut().any(|b| foreach_stmt(b, f)),
        Stmt::Foreach { body, .. } => foreach_stmt(body, f),
        Stmt::Assert { pass, fail, .. } => {
            pass.as_mut().map(|p| foreach_stmt(p, f)).unwrap_or(false)
                || fail.as_mut().map(|x| foreach_stmt(x, f)).unwrap_or(false)
        }
        _ => false,
    }
}

/// Kunjungi semua statement module → item.
fn foreach_stmt_file<F>(file: &mut MvFile, f: &mut F) -> bool
where
    F: FnMut(&mut Stmt) -> bool,
{
    for m in &mut file.modules {
        for it in &mut m.items {
            if let Some(st) = item_stmt(it) {
                if foreach_stmt(st, f) {
                    return true;
                }
            }
        }
    }
    false
}

/// Kunjungi semua ekspresi module (item block-stmt).
fn foreach_expr_file<F>(file: &mut MvFile, f: &mut F) -> bool
where
    F: FnMut(&mut Expr) -> bool,
{
    for m in &mut file.modules {
        for it in &mut m.items {
            if let Some(st) = item_stmt(it) {
                if expr_in_stmt(st, f) {
                    return true;
                }
            }
        }
    }
    false
}

/// Ekspresi dalam SATU statement (via traversal ekspresi di tiap node stmt).
fn expr_in_stmt<F>(s: &mut Stmt, f: &mut F) -> bool
where
    F: FnMut(&mut Expr) -> bool,
{
    match s {
        Stmt::Block(xs) => xs.iter_mut().any(|x| expr_in_stmt(x, f)),
        Stmt::Assign { lhs, rhs, .. } => foreach_expr(lhs, f) || foreach_expr(rhs, f),
        Stmt::CompoundAssign { lhs, rhs, .. } => foreach_expr(lhs, f) || foreach_expr(rhs, f),
        Stmt::IncDec { lhs, .. } => foreach_expr(lhs, f),
        Stmt::If { cond, then, els, .. } => {
            if foreach_expr(cond, f) {
                return true;
            }
            if expr_in_stmt(then, f) {
                return true;
            }
            els.as_mut().map(|e| expr_in_stmt(e, f)).unwrap_or(false)
        }
        Stmt::Case { expr, items, default, .. } => {
            if foreach_expr(expr, f) {
                return true;
            }
            for (vals, body) in items {
                for v in vals {
                    if foreach_expr(v, f) {
                        return true;
                    }
                }
                if expr_in_stmt(body, f) {
                    return true;
                }
            }
            default.as_mut().map(|d| expr_in_stmt(d, f)).unwrap_or(false)
        }
        Stmt::For { from, to, step, body, .. } => {
            if foreach_expr(from, f) || foreach_expr(to, f) {
                return true;
            }
            if step.as_mut().map(|e| foreach_expr(e, f)).unwrap_or(false) {
                return true;
            }
            expr_in_stmt(body, f)
        }
        Stmt::While { cond, body } | Stmt::Wait { cond, body } => {
            if foreach_expr(cond, f) {
                return true;
            }
            expr_in_stmt(body, f)
        }
        Stmt::DoWhile { cond, body } => {
            if expr_in_stmt(body, f) {
                return true;
            }
            foreach_expr(cond, f)
        }
        Stmt::EventTrigger(e) => foreach_expr(e, f),
        Stmt::Repeat { count, body } => {
            if foreach_expr(count, f) {
                return true;
            }
            expr_in_stmt(body, f)
        }
        Stmt::Forever(body) => expr_in_stmt(body, f),
        Stmt::Event { expr, body } => {
            if foreach_expr(expr, f) {
                return true;
            }
            expr_in_stmt(body, f)
        }
        Stmt::Delay { amt, body } => {
            if foreach_expr(amt, f) {
                return true;
            }
            expr_in_stmt(body, f)
        }
        Stmt::ExprStmt(e) => foreach_expr(e, f),
        Stmt::VarDecl { init, .. } => init.as_mut().map(|e| foreach_expr(e, f)).unwrap_or(false),
        Stmt::Return(v) => v.as_mut().map(|e| foreach_expr(e, f)).unwrap_or(false),
        Stmt::Fork { branches, .. } => branches.iter_mut().any(|b| expr_in_stmt(b, f)),
        Stmt::Foreach { body, .. } => expr_in_stmt(body, f),
        Stmt::Assert { cond, pass, fail } => {
            if foreach_expr(cond, f) {
                return true;
            }
            if pass.as_mut().map(|p| expr_in_stmt(p, f)).unwrap_or(false) {
                return true;
            }
            fail.as_mut().map(|x| expr_in_stmt(x, f)).unwrap_or(false)
        }
        _ => false,
    }
}

/// `&mut Stmt` dari item module (blok).
fn item_stmt(it: &mut MItem) -> Option<&mut Stmt> {
    match it {
        MItem::Seq(_, s) | MItem::Comb(s) | MItem::Always(s) | MItem::Latch(s)
        | MItem::Initial(s) | MItem::Final(s) => Some(s),
        _ => None,
    }
}

// ──────────────────────────────────────────────────────────────────────
// Operator mutasi
// ──────────────────────────────────────────────────────────────────────

/// Op 0: swap operator biner pertama dalam golongan arith/bitwise.
fn op_swap_binop(file: &mut MvFile, rng: &mut StdRng) -> bool {
    foreach_expr_file(file, &mut |e: &mut Expr| {
        if let Expr::Binary(op, _, _) = e {
            let group: &[&str] = &["+", "-", "&", "|", "^", "<<", ">>"];
            let alt = *group.choose(rng).unwrap();
            if alt != op.as_str() {
                *op = alt.to_string();
                return true;
            }
        }
        false
    })
}

/// Op 1: flip fill literal (`'0`↔`'1`↔`'x`↔`'z`).
fn op_flip_fill(file: &mut MvFile, rng: &mut StdRng) -> bool {
    foreach_expr_file(file, &mut |e: &mut Expr| {
        if let Expr::Fill(c) = e {
            let alt = *['0', '1', 'x', 'z'].choose(rng).unwrap();
            if alt != *c {
                *c = alt;
                return true;
            }
        }
        false
    })
}

/// Op 2: ubah nilai literal Int pertama (lebar/const/delay/init).
fn op_change_literal(file: &mut MvFile, rng: &mut StdRng) -> bool {
    foreach_expr_file(file, &mut |e: &mut Expr| {
        if let Expr::Int(v) = e {
            if *v > 0 {
                *v = rng.gen_range(1..=16);
                return true;
            }
        }
        false
    })
}

/// Op 3: toggle signed pada tipe sinyal/port pertama.
fn op_toggle_signed(file: &mut MvFile, _rng: &mut StdRng) -> bool {
    for m in &mut file.modules {
        for it in &mut m.items {
            let ty = match it {
                MItem::Port(p) => Some(&mut p.ty),
                MItem::Sig { ty, .. } | MItem::Reg { ty, .. } => Some(ty),
                _ => None,
            };
            if let Some(ty) = ty {
                if matches!(ty, MvType::Signed(_)) {
                    let inner = match ty {
                        MvType::Signed(i) => i.as_ref().clone(),
                        _ => unreachable!(),
                    };
                    *ty = inner;
                } else {
                    *ty = MvType::Signed(Box::new(ty.clone()));
                }
                return true;
            }
        }
    }
    false
}

/// Op 4: toggle NBA/blocking pada assignment pertama.
fn op_toggle_nba(file: &mut MvFile, _rng: &mut StdRng) -> bool {
    foreach_stmt_file(file, &mut |s: &mut Stmt| {
        if let Stmt::Assign { nba, .. } = s {
            *nba = !*nba;
            return true;
        }
        false
    })
}

/// Op 5: duplikasi statement acak dalam blok pertama.
fn op_dup_stmt(file: &mut MvFile, rng: &mut StdRng) -> bool {
    foreach_stmt_file(file, &mut |s: &mut Stmt| {
        if let Stmt::Block(stmts) = s {
            if stmts.len() >= 2 {
                let pos = rng.gen_range(0..stmts.len());
                let dup = stmts[pos].clone();
                stmts.insert(pos + 1, dup);
                return true;
            }
        }
        false
    })
}

/// Op 6: tambah sinyal `sig` baru.
fn op_add_sig(file: &mut MvFile, rng: &mut StdRng) -> bool {
    if let Some(m) = file.modules.iter_mut().next() {
        let w = [2usize, 4, 8, 16].choose(rng).copied().unwrap_or(8);
        let n = format!("fz_sq{}", rng.gen_range(0..99_999));
        m.items.push(MItem::Sig {
            names: vec![n],
            ty: MvType::Logic(Some((sub1(w), int(0)))),
            init: Some(Expr::Fill('0')),
            line: 0,
            col: 0,
        });
        true
    } else {
        false
    }
}

/// Op 7: tambah port input baru.
fn op_add_port(file: &mut MvFile, rng: &mut StdRng) -> bool {
    if let Some(m) = file.modules.iter_mut().next() {
        let w = [2usize, 4, 8, 16].choose(rng).copied().unwrap_or(8);
        let n = format!("fz_pi{}", rng.gen_range(0..99_999));
        m.items.insert(
            0,
            MItem::Port(maria_api::mv::ast::Port {
                dir: Dir::In,
                names: vec![n],
                ty: MvType::Logic(Some((sub1(w), int(0)))),
                line: 0,
                col: 0,
            }),
        );
        true
    } else {
        false
    }
}

/// `[w-1:0]` — bentuk Logic range.
fn sub1(w: usize) -> Expr {
    Expr::Binary("-".into(), Box::new(int(w as i64)), Box::new(int(1)))
}

/// Op 8: ubah nilai konstanta pertama.
fn op_change_const(file: &mut MvFile, rng: &mut StdRng) -> bool {
    for m in &mut file.modules {
        for it in &mut m.items {
            if let MItem::Const { value, .. } = it {
                if let Expr::Int(v) = value {
                    *v = rng.gen_range(1..=16);
                    return true;
                }
            }
        }
    }
    false
}

/// Op 9: ubah delay `#N` pertama.
fn op_change_delay(file: &mut MvFile, rng: &mut StdRng) -> bool {
    foreach_stmt_file(file, &mut |s: &mut Stmt| {
        if let Stmt::Delay { amt, .. } = s {
            if let Expr::Int(v) = amt {
                *v = rng.gen_range(0..=50);
                return true;
            }
        }
        false
    })
}

/// Op 10: switch blok src `seq`↔`comb` (fix operator assignment).
fn op_switch_block(file: &mut MvFile, _rng: &mut StdRng) -> bool {
    for m in &mut file.modules {
        for it in &mut m.items {
            let placeholder = MItem::Comb(Stmt::Block(Vec::new()));
            let item = std::mem::replace(it, placeholder);
            match item {
                MItem::Comb(mut st) => {
                    fix_nba_in(&mut st, true);
                    let spec = maria_api::mv::ast::SeqSpec {
                        clk: "clk".to_string(),
                        neg_edge: false,
                        reset: Some(("rst_n".to_string(), true, false)),
                        line: 0,
                        col: 0,
                    };
                    *it = MItem::Seq(spec, st);
                    return true;
                }
                MItem::Seq(_spec, mut st) => {
                    fix_nba_in(&mut st, false);
                    *it = MItem::Comb(st);
                    return true;
                }
                other => *it = other,
            }
        }
    }
    false
}

/// Sesuaikan operator assignment: `to_seq=true` → semua `=`, jadi `<=`.
fn fix_nba_in(s: &mut Stmt, to_seq: bool) {
    foreach_stmt(s, &mut |st: &mut Stmt| {
        if let Stmt::Assign { nba, .. } = st {
            *nba = to_seq;
        }
        false
    });
}

/// Op 11: tanam assert-oracle di `initial` pertama (2 expr identik `===`).
fn op_add_assert(file: &mut MvFile, rng: &mut StdRng) -> bool {
    for m in &mut file.modules {
        for it in &mut m.items {
            if let MItem::Initial(st) = it {
                if let Stmt::Block(stmts) = st {
                    let id = rng.gen_range(0..99_999u32);
                    let a = format!("fz_atA_{id}");
                    let b = format!("fz_atB_{id}");
                    stmts.insert(
                        0,
                        Stmt::RawSvh(format!(
                            "wire [7:0] {a}; assign {a} = y;\nwire [7:0] {b}; assign {b} = y;"
                        )),
                    );
                    stmts.insert(
                        1,
                        Stmt::Assert {
                            cond: Expr::Binary(
                                "===".into(),
                                Box::new(Expr::Ident(a, 0, 0)),
                                Box::new(Expr::Ident(b, 0, 0)),
                            ),
                            pass: None,
                            fail: Some(Box::new(Stmt::ExprStmt(Expr::Call(
                                "$fatal".to_string(),
                                vec![int(0)],
                            )))),
                        },
                    );
                    return true;
                }
            }
        }
    }
    false
}

/// Pilih op — berbobot adaptif.
fn pick_mv_op(rng: &mut StdRng, stats: &MvOpStats) -> usize {
    let total: f64 = stats.weights.iter().sum();
    let mut pick = rng.gen_range(0.0..total);
    for (i, w) in stats.weights.iter().enumerate() {
        if pick < *w {
            return i;
        }
        pick -= *w;
    }
    NUM_MV_OPS - 1
}

/// Terapkan SATU mutasi AST. Kembalikan (op_id, canonical .mv baru).
/// Parse gagal / target tak ditemukan → source tidak berubah.
pub fn mutate(rng: &mut StdRng, src: &str, stats: &mut MvOpStats) -> (usize, String) {
    let op = pick_mv_op(rng, stats);
    stats.record_attempt(op);
    let mut file = match maria_api::mv::parser::parse(src) {
        Ok(f) => f,
        Err(_) => return (op, src.to_string()),
    };
    let applied = match op {
        0 => op_swap_binop(&mut file, rng),
        1 => op_flip_fill(&mut file, rng),
        2 => op_change_literal(&mut file, rng),
        3 => op_toggle_signed(&mut file, rng),
        4 => op_toggle_nba(&mut file, rng),
        5 => op_dup_stmt(&mut file, rng),
        6 => op_add_sig(&mut file, rng),
        7 => op_add_port(&mut file, rng),
        8 => op_change_const(&mut file, rng),
        9 => op_change_delay(&mut file, rng),
        10 => op_switch_block(&mut file, rng),
        _ => op_add_assert(&mut file, rng),
    };
    if !applied {
        return (op, src.to_string());
    }
    let out = maria_api::mv::print::print_file(&file);
    // Invariant printer: hasil harus tetap ter-parse.
    if maria_api::mv::parser::parse(&out).is_err() {
        return (op, src.to_string());
    }
    (op, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    const SRC: &str = r#"
module m {
    in  clk, rst_n : bit
    in  a, b       : logic[7:0]
    out y          : logic[7:0]
    sig t          : logic[7:0]
    const C = 3
    comb {
        t = a + b
        y = t ^ '0
    }
    seq(clk, rst_n) {
        if (!rst_n) {
            t <= '0
        } else {
            t <= (t + 1) & b
        }
    }
    initial {
        #7 a = 8'd5
        #80
        assert (y !== 'x) $info("PASS")
    }
}
"#;

    #[test]
    fn parse_baseline() {
        maria_api::mv::parser::parse(SRC).expect("sample harus parse");
    }

    #[test]
    fn all_ops_keep_parseable() {
        let mut rng = StdRng::seed_from_u64(3);
        let mut stats = MvOpStats::default();
        let mut s = SRC.to_string();
        for _ in 0..40 {
            let (op, out) = mutate(&mut rng, &s, &mut stats);
            assert!(op < NUM_MV_OPS);
            s = out;
            maria_api::mv::parser::parse(&s).expect("hasil mutasi harus parse");
        }
    }

    #[test]
    fn mutate_changes_source() {
        let mut rng = StdRng::seed_from_u64(9);
        let mut stats = MvOpStats::default();
        let mut s = SRC.to_string();
        let mut different = false;
        for _ in 0..30 {
            let (_, out) = mutate(&mut rng, &s, &mut stats);
            if out != s {
                different = true;
                s = out;
            }
        }
        assert!(different, "mutasi harus mengubah source");
    }

    #[test]
    fn swap_binop_reaches_expr() {
        let mut rng = StdRng::seed_from_u64(5);
        let mut file = maria_api::mv::parser::parse(SRC).unwrap();
        assert!(op_swap_binop(&mut file, &mut rng), "harus ada binary");
    }

    #[test]
    fn add_sig_and_port_parseable() {
        let mut rng = StdRng::seed_from_u64(6);
        let mut file = maria_api::mv::parser::parse(SRC).unwrap();
        assert!(op_add_sig(&mut file, &mut rng));
        assert!(op_add_port(&mut file, &mut rng));
        let text = maria_api::mv::print::print_file(&file);
        assert!(text.contains("fz_sq"));
        assert!(text.contains("fz_pi"));
        maria_api::mv::parser::parse(&text).expect("hasil harus parse");
    }

    #[test]
    fn switch_block_fixes_nba() {
        let mut rng = StdRng::seed_from_u64(7);
        // comb → seq: semua assign jadi non-blocking (E2004 terhindar utk seq).
        let src = "module s {\n    in clk : bit\n    in a : logic[7:0]\n    sig t : logic[7:0]\n    comb {\n        t = a\n    }\n}\n";
        let mut file = maria_api::mv::parser::parse(src).unwrap();
        assert!(op_switch_block(&mut file, &mut rng));
        let text = maria_api::mv::print::print_file(&file);
        assert!(text.contains("seq(clk, rst_n)"));
        assert!(text.contains("<="), "comb→seq harus NBA: {}", text);
        maria_api::mv::parser::parse(&text).expect("parse");
    }

    #[test]
    fn assert_oracle_planted() {
        let mut rng = StdRng::seed_from_u64(8);
        let mut file = maria_api::mv::parser::parse(SRC).unwrap();
        assert!(op_add_assert(&mut file, &mut rng));
        let text = maria_api::mv::print::print_file(&file);
        assert!(text.contains("fz_atA_"));
        assert!(text.contains("==="));
    }
}