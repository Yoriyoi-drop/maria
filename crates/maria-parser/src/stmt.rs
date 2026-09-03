//! ──────────────────────────────────────────────────────────────────────────────
//! CATATAN: File ini adalah bagian dari pemisahan parser.rs (SRP Refactoring).
//! Tanggung jawab: Parsing statement-level constructs.
//!
//! Fungsi:
//!   - parse_stmt_block()           — parsing begin...end block
//!   - parse_immediate_assertion()  — parsing assert/assume/cover/expect
//!   - parse_clocking_event()       — parsing @(posedge clk) event
//!   - parse_wait_order()          — parsing wait_order(...)
//!   - parse_covergroup()          — parsing covergroup ... endgroup
//!   - parse_dpi_import()          — parsing DPI-C import/export
//!   - skip_dpi_range()            — skip DPI range [N]
//!   - try_parse_dpi_type()        — try parse DPI type
//!   - parse_stmt()                — parsing generic statement (dispatcher)
//!   - parse_if_stmt()             — parsing if/else
//!   - parse_case_stmt()           — parsing case/casex/casez
//!   - parse_for_stmt()            — parsing for loop
//!   - parse_foreach_stmt()        — parsing foreach loop
//!   - parse_while_stmt()          — parsing while loop
//!   - parse_forever_stmt()        — parsing forever loop
//!   - parse_repeat_stmt()         — parsing repeat loop
//!   - parse_fork_join()           — parsing fork...join/join_any/join_none
//!   - parse_syscall()             — parsing $system calls
//!
//! ──────────────────────────────────────────────────────────────────────────────

use super::Parser;
use crate::lexer::*;
use maria_ast::types::const_eval_simple;
use maria_ast::*;
use maria_core::error::SimError;
use maria_core::intern::Symbol;

/// Helper: cek apakah expression adalah lvalue yang valid.
fn is_valid_lvalue(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Ident { .. }
            | Expr::BitSelect { .. }
            | Expr::RangeSelect { .. }
            | Expr::PartSelect { .. }
            | Expr::Concat(_)
            | Expr::MemberAccess { .. }
    )
}

impl Parser {
    pub(crate) fn parse_stmt_block(&mut self) -> Result<Vec<Stmt>, SimError> {
        if self.peek() == &Token::Begin {
            self.advance();
            if self.peek() == &Token::Colon {
                self.advance();
                if let Token::Ident(_) = self.peek() {
                    self.advance();
                }
            }
            let mut stmts = Vec::new();
            loop {
                if self.peek() == &Token::End || self.peek() == &Token::Eof {
                    self.advance();
                    // Konsumsi label opsional setelah `end` (`end : label`) —
                    // tanpa ini `end : recode_st` meninggalkan `: recode_st`
                    // di token stream dan deklarasi module-level SETELAH blok
                    // bernama tidak terdaftar (parser mengira region masih
                    // dalam blok). Contoh nyata: kmac_errchk `end : recode_st`,
                    // spi_tpm `end : hw_reg_mux`.
                    if self.peek() == &Token::Colon {
                        self.advance();
                        if matches!(self.peek(), Token::Ident(_)) {
                            self.advance();
                        }
                    }
                    break;
                }
                stmts.push(self.parse_stmt()?);
            }
            Ok(stmts)
        } else {
            let stmts = match self.parse_stmt() {
                Ok(s) => vec![s],
                Err(e) => {
                    let diag = e.to_diagnostic();
                    self.errors.push(diag);
                    self.skip_to_stmt_boundary();
                    vec![]
                }
            };
            Ok(stmts)
        }
    }

    pub(crate) fn parse_immediate_assertion(&mut self) -> Result<Stmt, SimError> {
        let kind = match self.peek() {
            Token::Assert => {
                self.advance();
                "assert"
            }
            Token::Assume => {
                self.advance();
                "assume"
            }
            Token::Cover => {
                self.advance();
                "cover"
            }
            Token::Expect => {
                self.advance();
                "expect"
            }
            // LANG-11: restrict property — properti constraint (asumsi),
            // diperlakukan seperti assume (violation → fail metric).
            Token::Restrict => {
                self.advance();
                "assume"
            }
            _ => return Err(self.err("expected assert/assume/cover/expect")),
        };
        if self.peek() == &Token::Property {
            self.advance();
            self.expect(Token::LParen)?;
            let clock_event = if self.peek() == &Token::At {
                self.advance();
                self.expect(Token::LParen)?;
                let ce = if self.peek() == &Token::PosEdge {
                    self.advance();
                    let sig = self.expect_ident()?;
                    Some(ClockEvent::Posedge(sig))
                } else if self.peek() == &Token::NegEdge {
                    self.advance();
                    let sig = self.expect_ident()?;
                    Some(ClockEvent::Negedge(sig))
                } else {
                    let sig = self.expect_ident()?;
                    Some(ClockEvent::Edge(sig))
                };
                self.expect(Token::RParen)?;
                ce
            } else {
                None
            };
            let disable_iff = if self.peek() == &Token::Disable {
                self.advance();
                match self.peek() {
                    Token::Ident(s) if s == "iff" => {
                        self.advance();
                    }
                    _ => return Err(self.err("expected 'iff' after 'disable'")),
                }
                self.expect(Token::LParen)?;
                let expr = self.parse_expr(0)?;
                self.expect(Token::RParen)?;
                Some(Box::new(expr))
            } else {
                None
            };
            // LANG-06 (SVA temporal): properti ber-urutan `a ##1 b` /
            // `a ##[2:5] b` / `##1 a` — parse sequence bila token berikutnya
            // adalah `##` (delay operator). Kalau tidak ada `##`, fallback ke
            // boolean biasa (perilaku ROUND 71).
            let mut sequence: Option<super::types::Sequence> = None;
            let mut expr_opt: Option<Expr> = None;
            if self.peek() == &Token::HashHash {
                // bentuk `##N expr` / `##[min:max] expr` — sequence dimulai
                // dengan delay.
                self.advance();
                let delay = self.parse_sequence_delay()?;
                let first = self.parse_expr(0)?;
                let mut seq = super::types::Sequence::Concat(
                    Box::new(delay),
                    Box::new(super::types::Sequence::Expr(first)),
                );
                while self.peek() == &Token::HashHash {
                    self.advance();
                    let delay = self.parse_sequence_delay()?;
                    let next = self.parse_expr(0)?;
                    seq = super::types::Sequence::Concat(
                        Box::new(seq),
                        Box::new(super::types::Sequence::Concat(
                            Box::new(delay),
                            Box::new(super::types::Sequence::Expr(next)),
                        )),
                    );
                }
                sequence = Some(seq);
            } else {
                let expr = self.parse_expr(0)?;
                if self.peek() == &Token::PipeArrow {
                    // `expr |-> consequent` — overlap implication (SVA §16.9.2)
                    self.advance(); // consume |->
                    let cons = self.parse_sequence_after_consequent()?;
                    let ante = super::types::Sequence::Expr(expr);
                    let seq = super::types::Sequence::Implication(Box::new(ante), Box::new(cons));
                    sequence = Some(seq);
                } else if self.peek() == &Token::HashHash {
                    // bentuk `expr ##N expr ...`
                    let mut seq = super::types::Sequence::Expr(expr);
                    while self.peek() == &Token::HashHash {
                        self.advance();
                        let delay = self.parse_sequence_delay()?;
                        let next = self.parse_expr(0)?;
                        seq = super::types::Sequence::Concat(
                            Box::new(seq),
                            Box::new(super::types::Sequence::Concat(
                                Box::new(delay),
                                Box::new(super::types::Sequence::Expr(next)),
                            )),
                        );
                    }
                    sequence = Some(seq);
                } else {
                    expr_opt = Some(expr);
                }
            }
            self.expect(Token::RParen)?;
            let fail_stmt = if self.peek() == &Token::Else {
                self.advance();
                Some(Box::new(self.parse_stmt()?))
            } else {
                None
            };
            self.skip_semi();
            if let Some(seq) = sequence {
                // assert/assume/cover temporal semuanya memakai PropertySeq
                // (engine mencatat pass/fail sama via SequenceAttempt).
                return Ok(Stmt::PropertySeq {
                    sequence: seq,
                    pass_stmt: None,
                    fail_stmt,
                    clock_event,
                    disable_iff,
                });
            }
            let expr = expr_opt.expect("expr or sequence harus ter-set");
            let cond = Expr::TernaryOp {
                cond: Box::new(expr),
                true_expr: Box::new(Expr::Value(Value::Decimal(1))),
                false_expr: Box::new(Expr::Value(Value::Decimal(0))),
            };
            return match kind {
                "assert" => Ok(Stmt::Assert {
                    cond,
                    pass_stmt: None,
                    fail_stmt,
                    clock_event,
                    disable_iff,
                }),
                "assume" => Ok(Stmt::Assume {
                    cond,
                    pass_stmt: None,
                    fail_stmt,
                    clock_event,
                    disable_iff,
                }),
                "cover" => Ok(Stmt::Cover {
                    cond,
                    pass_stmt: None,
                    clock_event,
                    disable_iff,
                }),
                _ => unreachable!(),
            };
        }
        // LANG-03 PSL (IEEE 1850): `assert always (expr) @(posedge clk);` /
        // `assert never (expr) @(posedge clk);` — bentuk boolean PSL
        // (tanpa operator temporal). `always` = properti harus true tiap
        // cycle; `never` = properti tidak boleh true (cond dibalik dengan
        // !). Operator temporal PSL (`|->`, `until`, `before`, `next` dll)
        // membuat parse_expr gagal → caller module-level rollback + skip.
        if self.peek() == &Token::Always
            || matches!(self.peek(), Token::Ident(s) if s.as_str() == "never")
        {
            let is_never = matches!(self.peek(), Token::Ident(s) if s.as_str() == "never");
            self.advance(); // always / never
            self.expect(Token::LParen)?;
            let expr = self.parse_expr(0)?;
            self.expect(Token::RParen)?;
            let clock_event = if self.peek() == &Token::At {
                self.advance();
                self.expect(Token::LParen)?;
                let ce = if self.peek() == &Token::PosEdge {
                    self.advance();
                    let sig = self.expect_ident()?;
                    Some(ClockEvent::Posedge(sig))
                } else if self.peek() == &Token::NegEdge {
                    self.advance();
                    let sig = self.expect_ident()?;
                    Some(ClockEvent::Negedge(sig))
                } else {
                    let sig = self.expect_ident()?;
                    Some(ClockEvent::Edge(sig))
                };
                self.expect(Token::RParen)?;
                ce
            } else {
                None
            };
            self.skip_semi();
            let body = if is_never {
                Expr::UnaryOp {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                }
            } else {
                expr
            };
            let cond = Expr::TernaryOp {
                cond: Box::new(body),
                true_expr: Box::new(Expr::Value(Value::Decimal(1))),
                false_expr: Box::new(Expr::Value(Value::Decimal(0))),
            };
            return Ok(Stmt::Assert {
                cond,
                pass_stmt: None,
                fail_stmt: None,
                clock_event,
                disable_iff: None,
            });
        }
        self.expect(Token::LParen)?;
        let cond = self.parse_expr(0)?;
        self.expect(Token::RParen)?;
        let pass_stmt = if kind == "cover" {
            None
        } else if self.peek() != &Token::Semi && self.peek() != &Token::Else {
            let stmt = self.parse_stmt()?;
            Some(Box::new(stmt))
        } else {
            None
        };
        let fail_stmt = if self.peek() == &Token::Else {
            self.advance();
            Some(Box::new(self.parse_stmt()?))
        } else {
            None
        };
        self.skip_semi();
        match kind {
            "assert" => Ok(Stmt::Assert {
                cond,
                pass_stmt,
                fail_stmt,
                clock_event: None,
                disable_iff: None,
            }),
            "assume" => Ok(Stmt::Assume {
                cond,
                pass_stmt,
                fail_stmt,
                clock_event: None,
                disable_iff: None,
            }),
            "cover" => Ok(Stmt::Cover {
                cond,
                pass_stmt,
                clock_event: None,
                disable_iff: None,
            }),
            "expect" => Ok(Stmt::Expect {
                cond,
                pass_stmt,
                fail_stmt,
            }),
            _ => unreachable!(),
        }
    }

    /// LANG-06 (SVA temporal): parse delay sequence setelah `##` — bentuk
    /// `##N` (Delay konstan) atau `##[min:max]` (DelayRange). Caller sudah
    /// meng-consume `##`.
    fn parse_sequence_delay(&mut self) -> Result<super::types::Sequence, SimError> {
        if self.peek() == &Token::LBrack {
            // ##[min:max]
            self.advance();
            let min = self.parse_sequence_number()?;
            self.expect(Token::Colon)?;
            let max = self.parse_sequence_number()?;
            self.expect(Token::RBrack)?;
            Ok(super::types::Sequence::DelayRange(min, max))
        } else {
            let n = self.parse_sequence_number()?;
            Ok(super::types::Sequence::Delay(n))
        }
    }

    /// Parse bilangan bulat untuk delay sequence (`##3` → 3).
    fn parse_sequence_number(&mut self) -> Result<u64, SimError> {
        match self.peek().clone() {
            Token::Number { value, base, .. } => {
                let s = value.as_str().to_string();
                self.advance();
                match base {
                    None | Some(10) => s
                        .parse::<u64>()
                        .map_err(|_| self.err("invalid sequence delay number")),
                    Some(b) => u64::from_str_radix(&s, b as u32)
                        .map_err(|_| self.err("invalid sequence delay number")),
                }
            }
            _ => Err(self.err("expected number after '##'")),
        }
    }

    /// Parse consequent sequence setelah `|->`. Bisa `##N expr`,
    /// `##[min:max] expr`, atau `expr` (tanpa delay = same-cycle).
    fn parse_sequence_after_consequent(&mut self) -> Result<super::types::Sequence, SimError> {
        if self.peek() == &Token::HashHash {
            self.advance(); // consume ##
            let delay = self.parse_sequence_delay()?;
            let expr = self.parse_expr(0)?;
            Ok(super::types::Sequence::Concat(
                Box::new(delay),
                Box::new(super::types::Sequence::Expr(expr)),
            ))
        } else {
            let expr = self.parse_expr(0)?;
            Ok(super::types::Sequence::Expr(expr))
        }
    }

    pub(crate) fn parse_clocking_event(&mut self) -> Result<Expr, SimError> {
        self.expect(Token::At)?;
        self.expect(Token::LParen)?;
        if self.peek() == &Token::PosEdge || self.peek() == &Token::NegEdge {
            self.advance();
        }
        let signal = self.parse_expr(0)?;
        self.expect(Token::RParen)?;
        Ok(signal)
    }

    pub(crate) fn parse_wait_order(&mut self) -> Result<Stmt, SimError> {
        self.advance();
        self.expect(Token::LParen)?;
        let mut events = Vec::new();
        if self.peek() != &Token::RParen {
            loop {
                events.push(self.expect_ident()?);
                if self.peek() == &Token::Comma {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        self.expect(Token::RParen)?;
        let fail_stmt = if self.peek() == &Token::Else {
            self.advance();
            Some(Box::new(self.parse_stmt()?))
        } else {
            None
        };
        self.skip_semi();
        Ok(Stmt::WaitOrder { events, fail_stmt })
    }

    pub(crate) fn parse_covergroup(&mut self) -> Result<CovergroupDecl, SimError> {
        self.advance();
        let name = self.expect_ident()?;
        let clocking_event = if self.peek() == &Token::At {
            Some(self.parse_clocking_event()?)
        } else {
            None
        };
        if let Token::Ident(s) = self.peek() {
            if *s == Symbol::intern("with") {
                self.advance();
                let mut depth = 0;
                loop {
                    match self.peek() {
                        Token::Semi if depth == 0 => {
                            self.advance();
                            break;
                        }
                        Token::LParen => {
                            depth += 1;
                            self.advance();
                        }
                        Token::RParen if depth > 0 => {
                            depth -= 1;
                            self.advance();
                        }
                        Token::Eof => break,
                        _ => {
                            self.advance();
                        }
                    }
                }
            } else {
                self.skip_semi();
            }
        } else {
            self.skip_semi();
        }
        let mut coverpoints = Vec::new();
        let mut crosses = Vec::new();
        // VERIF-28: `type_option.weight = N` / `type_option.per_instance = 1`
        // (juga varian `option.*`) — dibaca saat body loop; default weight 1,
        // per_instance false.
        let mut cg_weight: Option<u64> = None;
        let mut cg_per_instance: bool = false;
        loop {
            match self.peek() {
                Token::EndGroup | Token::Eof => {
                    self.advance();
                    if self.peek() == &Token::Colon {
                        self.advance();
                        if matches!(self.peek(), Token::Ident(_)) {
                            self.advance();
                        }
                    }
                    break;
                }
                Token::Ident(_) => {
                    let ident = self.expect_ident()?;
                    // VERIF-28: `type_option.weight = N` / `type_option.per_instance =
                    // 0|1` / `type_option.merge_instances = 0|1` — opsi tipe
                    // covergroup. Sebelumnya di-skip sampai ';' (opsi diabaikan).
                    if ident == Symbol::intern("type_option") && self.peek() == &Token::Dot {
                        self.advance(); // '.'
                        let opt = self.expect_ident()?;
                        self.expect(Token::BlockingAssign)?;
                        if opt == Symbol::intern("weight") {
                            let w = self.parse_expr(0)?;
                            cg_weight = const_eval_simple(&w).ok().map(|v| v as u64);
                        } else if opt == Symbol::intern("per_instance")
                            || opt == Symbol::intern("merge_instances")
                        {
                            let v = self.parse_expr(0)?;
                            let v = const_eval_simple(&v).ok().map(|x| x as u64);
                            if opt == Symbol::intern("per_instance") {
                                // per_instance=1 → per-instance; 0 → merge
                                cg_per_instance = v == Some(1);
                            } else {
                                // merge_instances=1 → merge; 0 → per-instance
                                cg_per_instance = v == Some(0);
                            }
                        } else {
                            let _ = self.parse_expr(0)?;
                        }
                        self.skip_semi();
                        continue;
                    }
                    if self.peek() == &Token::Colon {
                        self.advance();
                        match self.peek() {
                            Token::Coverpoint => {
                                self.advance();
                                let expr = self.parse_expr(0)?;
                                let mut bins = Vec::new();
                                if self.peek() == &Token::LBrace {
                                    self.advance();
                                    loop {
                                        match self.peek() {
                                            Token::RBrace => {
                                                self.advance();
                                                break;
                                            }
                                            Token::Bins
                                            | Token::IllegalBins
                                            | Token::IgnoreBins => {
                                                let bin_type = match self.peek() {
                                                    Token::IllegalBins => BinType::Illegal,
                                                    Token::IgnoreBins => BinType::Ignore,
                                                    _ => BinType::Normal,
                                                };
                                                self.advance();
                                                // Wildcard modifier opsional sebelum nama bin
                                                if let Token::Ident(s) = self.peek() {
                                                    if *s == Symbol::intern("wildcard") {
                                                        self.advance();
                                                    }
                                                }
                                                let bin_name = self.expect_ident()?;
                                                // Opsional: `[N]` atau `[]` setelah nama (array bins)
                                                if self.peek() == &Token::LBrack {
                                                    self.advance();
                                                    if self.peek() != &Token::RBrack {
                                                        let _ = self.parse_expr(0);
                                                    }
                                                    let _ = self.expect(Token::RBrack);
                                                }
                                                self.expect(Token::BlockingAssign)?;
                                                let mut range_list: Vec<BinRange> = Vec::new();
                                                let mut transitions: Vec<Vec<Expr>> = Vec::new();
                                                // Cek apakah rhs adalah binsof(...) atau {range_list}
                                                let is_binsof = if let Token::Ident(s) = self.peek()
                                                {
                                                    s.as_str() == "binsof"
                                                        || s.as_str() == "default"
                                                } else {
                                                    false
                                                };
                                                if is_binsof {
                                                    // binsof(coverpoint) intersect {values} — skip sampai ';'
                                                    // Implementasi nyata: konsumsi expression binsof
                                                    let mut depth = 0i32;
                                                    loop {
                                                        match self.peek() {
                                                            Token::Semi if depth == 0 => break,
                                                            Token::LParen | Token::LBrace => {
                                                                depth += 1;
                                                                self.advance();
                                                            }
                                                            Token::RParen | Token::RBrace
                                                                if depth > 0 =>
                                                            {
                                                                depth -= 1;
                                                                self.advance();
                                                            }
                                                            Token::RBrace if depth == 0 => break,
                                                            Token::Eof => break,
                                                            _ => {
                                                                self.advance();
                                                            }
                                                        }
                                                    }
                                                } else if self.peek() == &Token::LParen {
                                                    // Transition bin (VERIF-31). Setiap `(...)` = SATU
                                                    // sekuens transisi; beberapa sekuens dipisah koma
                                                    // di luar paren: `(a => b)`, `(0 => 1), (1 => 0)`.
                                                    // Di dalam sekuens, `=>` memisahkan level dan
                                                    // koma memisahkan nilai/range dalam satu level:
                                                    //   `(a => b => c)`  `(a, b => c, d)`  `([lo:hi] => v)`.
                                                    // Representasi: transitions[i] = satu Vec<Expr>
                                                    // level ter-flatten (panjang 2 = `prev => curr`).
                                                    // Jika paren TANPA `=>` → value-list; tiap nilai
                                                    // masuk range_list (bukan transisi).
                                                    self.advance(); // '('
                                                    loop {
                                                        // Level pertama.
                                                        let mut level1 = Vec::new();
                                                        loop {
                                                            level1.push(self.parse_expr(0)?);
                                                            if self.peek() == &Token::Comma {
                                                                self.advance();
                                                            } else {
                                                                break;
                                                            }
                                                        }
                                                        if self.peek() == &Token::FatArrow {
                                                            // Transition: kumpulkan level tersisa.
                                                            let mut seq = level1;
                                                            while self.peek() == &Token::FatArrow {
                                                                self.advance(); // '=>'
                                                                loop {
                                                                    seq.push(self.parse_expr(0)?);
                                                                    if self.peek() == &Token::Comma {
                                                                        self.advance();
                                                                    } else {
                                                                        break;
                                                                    }
                                                                }
                                                            }
                                                            self.expect(Token::RParen)?;
                                                            transitions.push(seq);
                                                        } else {
                                                            // Value-list: `(0,1,2)` — nilai tunggal.
                                                            self.expect(Token::RParen)?;
                                                            for v in level1 {
                                                                range_list.push(BinRange {
                                                                    low: v,
                                                                    high: None,
                                                                });
                                                            }
                                                        }
                                                        if self.peek() == &Token::Comma
                                                            && self.peek_ahead(1) == &Token::LParen
                                                        {
                                                            self.advance(); // ','
                                                            self.advance(); // '('
                                                            continue;
                                                        }
                                                        break;
                                                    }
                                                } else if self.peek() == &Token::LBrace {
                                                    self.advance();
                                                    loop {
                                                        if self.peek() == &Token::RBrace {
                                                            self.advance();
                                                            break;
                                                        }
                                                        if self.peek() == &Token::LBrack {
                                                            self.advance();
                                                            let low = self.parse_expr(0)?;
                                                            self.expect(Token::Colon)?;
                                                            let high = self.parse_expr(0)?;
                                                            self.expect(Token::RBrack)?;
                                                            range_list.push(BinRange {
                                                                low,
                                                                high: Some(high),
                                                            });
                                                        } else {
                                                            let low = self.parse_expr(0)?;
                                                            range_list
                                                                .push(BinRange { low, high: None });
                                                        }
                                                        if self.peek() == &Token::Comma {
                                                            let ahead = self.peek_ahead(1).clone();
                                                            let is_new_port = ahead == Token::Input
                                                                || ahead == Token::Output
                                                                || ahead == Token::Inout
                                                                || (matches!(
                                                                    &ahead,
                                                                    Token::Ident(_)
                                                                ) && matches!(
                                                                    self.peek_ahead(2),
                                                                    Token::Scope
                                                                ));
                                                            if !is_new_port {
                                                                self.advance();
                                                            } else {
                                                                break;
                                                            }
                                                        } else {
                                                            // BUG VERIF-30: range-list `{...}` ditutup `}` — loop sebelumnya
                                                            // break TANPA mengonsumsi `}` sehingga skip_semi tidak melihat
                                                            // `;` dan bins loop break di `}` → hanya bin PERTAMA yang
                                                            // ter-parse (bin kedua+ di-skip diam-diam).
                                                            if self.peek() == &Token::RBrace {
                                                                self.advance();
                                                            }
                                                            break;
                                                        }
                                                    }
                                                }
                                                self.skip_semi();
                                                bins.push(BinDef {
                                                    name: bin_name,
                                                    range_list,
                                                    bin_type,
                                                    transitions,
                                                });
                                            }
                                            // `default` bin — skip sampai ';'
                                            Token::Default => {
                                                self.advance();
                                                self.skip_until_semi_or_end()?;
                                            }
                                            _ => break,
                                        }
                                    }
                                }
                                self.skip_semi();
                                coverpoints.push(CoverpointDef {
                                    name: ident,
                                    expr,
                                    bins,
                                });
                            }
                            Token::Cross => {
                                self.advance();
                                let mut cps = Vec::new();
                                loop {
                                    cps.push(self.expect_ident()?);
                                    if self.peek() == &Token::Comma {
                                        self.advance();
                                    } else {
                                        break;
                                    }
                                }
                                self.skip_semi();
                                crosses.push(CrossDef {
                                    name: ident,
                                    coverpoints: cps,
                                });
                            }
                            _ => {
                                // Konstruk SV tidak dikenal setelah ':' — skip sampai ';'
                                // Contoh: `name : assume ...` atau `name : restrict ...`
                                self.skip_until_semi_or_end()?;
                            }
                        }
                    } else {
                        // Identifier tanpa ':' — kemungkinan `option.weight = 0` atau
                        // `type_option.name = "..."`. Skip sampai ';'.
                        self.skip_until_semi_or_end()?;
                    }
                }
                Token::Option_ => {
                    self.advance();
                    // VERIF-28: `option.weight = N` / `option.per_instance = 0|1` —
                    // opsi instance covergroup. Sebelumnya di-skip sampai ';'.
                    if self.peek() == &Token::Dot {
                        self.advance(); // '.'
                        let opt = self.expect_ident()?;
                        self.expect(Token::BlockingAssign)?;
                        if opt == Symbol::intern("weight") {
                            let w = self.parse_expr(0)?;
                            cg_weight = const_eval_simple(&w).ok().map(|v| v as u64);
                        } else if opt == Symbol::intern("per_instance") {
                            let v = self.parse_expr(0)?;
                            let v = const_eval_simple(&v).ok().map(|x| x as u64);
                            cg_per_instance = v == Some(1);
                        } else {
                            let _ = self.parse_expr(0)?;
                        }
                    } else {
                        self.skip_until_semi_or_end()?;
                    }
                    self.skip_semi();
                }
                _ => {
                    // Token tidak dikenal di body covergroup — skip sampai ';' atau '}'
                    // agar parsing tidak berhenti di sini. Contoh: `type_option.weight = 0;`
                    // atau konstruk SV coverage lain yang belum diimplementasikan.
                    self.advance();
                    // Skip jika ada expression atau assignment setelah token ini
                    let mut depth = 0i32;
                    loop {
                        match self.peek() {
                            Token::Semi if depth == 0 => {
                                self.advance();
                                break;
                            }
                            Token::EndGroup | Token::Eof => break,
                            Token::LBrace | Token::LParen => {
                                depth += 1;
                                self.advance();
                            }
                            Token::RBrace if depth > 0 => {
                                depth -= 1;
                                self.advance();
                            }
                            Token::RBrace if depth == 0 => break,
                            Token::RParen if depth > 0 => {
                                depth -= 1;
                                self.advance();
                            }
                            _ => {
                                self.advance();
                            }
                        }
                    }
                }
            }
        }
        Ok(CovergroupDecl {
            name,
            clocking_event,
            coverpoints,
            crosses,
            weight: cg_weight,
            per_instance: cg_per_instance,
        })
    }

    pub(crate) fn parse_dpi_import(&mut self) -> Result<DpiImport, SimError> {
        self.advance();
        // `import "DPI-C" [context|pure] function/task` (LRM 1800 §35.5).
        while matches!(self.peek(), Token::Ident(s) if s == "context" || s == "pure") {
            self.advance();
        }
        let is_task = if self.peek() == &Token::Task {
            self.advance();
            true
        } else if self.peek() == &Token::Function {
            self.advance();
            false
        } else {
            return Err(self.err("expected 'function' or 'task' after import \"DPI-C\""));
        };
        if matches!(self.peek(), Token::Auto | Token::Static) {
            self.advance();
        }
        let return_type = if is_task {
            None
        } else if self.peek() == &Token::Void {
            self.advance();
            None
        } else if let Some(dt) = self.try_parse_dpi_type() {
            self.skip_dpi_range();
            Some(Box::new(dt))
        } else {
            None
        };
        let name = self.expect_ident()?;
        let mut args = Vec::new();
        if self.peek() != &Token::LParen {
            // DPI heading boleh tanpa port list: `export "DPI-C" function foo;`
            self.skip_semi();
            return Ok(DpiImport {
                name,
                return_type,
                args,
                is_task,
            });
        }
        self.expect(Token::LParen)?;
        if self.peek() != &Token::RParen {
            loop {
                let direction = if self.peek() == &Token::Input {
                    self.advance();
                    PortDirection::Input
                } else if self.peek() == &Token::Output {
                    self.advance();
                    PortDirection::Output
                } else if self.peek() == &Token::Inout {
                    self.advance();
                    PortDirection::Inout
                } else {
                    PortDirection::Input
                };
                let dtype = self.try_parse_dpi_type().unwrap_or(DataType::Logic);
                self.skip_dpi_range();
                // Modifier signed/unsigned sebelum nama: `int unsigned cycle_count`.
                if self.peek() == &Token::Signed || self.peek() == &Token::Unsigned {
                    self.advance();
                }
                let arg_name = self.expect_ident()?;
                args.push(DpiArg {
                    direction,
                    dtype,
                    name: arg_name,
                });
                if self.peek() == &Token::Comma {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        self.expect(Token::RParen)?;
        self.skip_semi();
        Ok(DpiImport {
            name,
            return_type,
            args,
            is_task,
        })
    }

    pub(crate) fn skip_dpi_range(&mut self) {
        if self.peek() == &Token::LBrack {
            self.advance();
            let mut depth = 1;
            while depth > 0 && self.peek() != &Token::Eof {
                match self.peek() {
                    Token::LBrack => depth += 1,
                    Token::RBrack => {
                        depth -= 1;
                        if depth == 0 {
                            self.advance();
                            break;
                        }
                    }
                    _ => {}
                }
                self.advance();
            }
        }
    }

    pub(crate) fn try_parse_dpi_type(&mut self) -> Option<DataType> {
        let dt = match self.peek() {
            Token::Byte => {
                self.advance();
                DataType::Byte
            }
            Token::Shortint => {
                self.advance();
                DataType::Shortint
            }
            Token::Int => {
                self.advance();
                DataType::Int
            }
            Token::Longint => {
                self.advance();
                DataType::Longint
            }
            Token::Integer => {
                self.advance();
                DataType::Integer
            }
            Token::Real => {
                self.advance();
                DataType::Real
            }
            Token::RealTime => {
                self.advance();
                DataType::Realtime
            }
            Token::Bit => {
                self.advance();
                DataType::Bit
            }
            Token::Logic => {
                self.advance();
                DataType::Logic
            }
            Token::String => {
                self.advance();
                DataType::String
            }
            Token::Ident(s) if s == "chandle" => {
                self.advance();
                DataType::Longint
            }
            _ => return None,
        };
        Some(dt)
    }

    pub(crate) fn parse_stmt(&mut self) -> Result<Stmt, SimError> {
        self.push_depth()?;
        let result = self.parse_stmt_impl();
        self.pop_depth();
        result
    }

    /// Apakah token sekarang memulai deklarasi variabel dalam statement block
    /// (tipe builtin, atau user-defined type yang diikuti nama/::).
    fn is_decl_stmt_start(&self) -> bool {
        match self.peek() {
            Token::Wire
            | Token::Wand
            | Token::Wor
            | Token::Tri
            | Token::Tri0
            | Token::Tri1
            | Token::TriAnd
            | Token::TriOr
            | Token::Supply0
            | Token::Supply1
            | Token::Reg
            | Token::Logic
            | Token::Int
            | Token::Integer
            | Token::Bit
            | Token::Byte
            | Token::Shortint
            | Token::Longint
            | Token::Time
            | Token::String
            | Token::Real
            | Token::Enum
            | Token::Struct
            | Token::Union
            | Token::Const
            | Token::Var => true,
            Token::Ident(_) => match self.peek_ahead(1) {
                Token::Ident(_) => true,
                // pkg::type varname;  (bukan pkg::func(...))
                Token::Scope => matches!(self.peek_ahead(3), Token::Ident(_) | Token::LBrack),
                _ => false,
            },
            _ => false,
        }
    }

    fn parse_stmt_impl(&mut self) -> Result<Stmt, SimError> {
        if self.peek() == &Token::LParen && self.peek_ahead(1) == &Token::Star {
            self.skip_attribute();
            return self.parse_stmt();
        }
        // Procedural static/automatic variable declaration:
        // `static SomeType cfg = pkg::type_id::create("x");` — pola umum DV
        // UVM di dalam blok initial. Sebelumnya `static` jatuh ke fallthrough
        // parse_primary_expr → "expected expression, found Static" → blok
        // desync dan statement berikutnya (mis. `uvm_config_db::set`) salah
        // di-parse sebagai instance. Static/auto hanyalah modifier; deklarasi
        // sisanya di-parse via parse_decl (termasuk `Type v = expr`).
        if matches!(self.peek(), Token::Static | Token::Auto) {
            self.advance();
            let decl = self.parse_decl()?;
            return Ok(Stmt::NamedBlock {
                name: Symbol::EMPTY,
                stmts: vec![],
                decls: vec![decl],
            });
        }
        // Declaration statement in procedural block (e.g. `int index_x1;` or
        // `logic unused;` inside an always/initial block). Sebelumnya dibuang
        // sebagai `Stmt::Null`, sehingga variabel lokal di dalam loop body yang
        // di-unroll tidak pernah terdaftar (error "signal 'x' not found").
        // Sekarang disimpan sebagai `Stmt::NamedBlock` dengan `decls`, dan
        // elaborator mengumpulkan decls tersebut ke signal_map (lihat
        // `collect_procedural_decls` di elaborator/mod.rs).
        if self.is_decl_stmt_start() {
            let decl = self.parse_decl()?;
            return Ok(Stmt::NamedBlock {
                name: Symbol::EMPTY,
                stmts: vec![],
                decls: vec![decl],
            });
        }
        // Procedural localparam (e.g. `localparam logic [4:0] X = ...;` inside a
        // block). Disimpan sebagai `Stmt::NamedBlock` dengan `decls` (bukan
        // dibuang) agar `collect_procedural_decls` di elaborator mendaftarkan
        // nama tersebut — tanpa ini referensi ke localparam block-scoped
        // (`NumRounds` di otp_ctrl_scrmbl, pola `localparam` di dalam for-loop
        // body) menjadi "signal not found" (E2001). Nilai const dievaluasi
        // elaborator ke param context.
        if matches!(
            self.peek(),
            Token::Param | Token::Parameter | Token::LocalParam
        ) {
            return self.parse_procedural_localparam();
        }
        // Named assertion di level STATEMENT: `label: assert (prop);` —
        // ASSERT_* macros (prim_assert.sv) memperluas ke bentuk ini di dalam
        // blok initial/always (mis. `ASSERT_I(accelerate_regulators_power_up_time,
        // dv_hook inside {[0:3]})` di rglts_pdm_3p3v). Tanpa ini parser error
        // "expected expression, found Colon" dan file RTL tidak ter-parse.
        // Label assertion dibuang (nama tidak dipakai engine); isi assertion
        // di-parse normal sebagai Stmt::Assert/Assume/Cover.
        if matches!(self.peek(), Token::Ident(_))
            && self.peek_ahead(1) == &Token::Colon
            && matches!(
                self.peek_ahead(2),
                Token::Assert | Token::Assume | Token::Cover | Token::Expect
            )
        {
            self.advance(); // label
            self.advance(); // ':'
            return self.parse_immediate_assertion();
        }
        match self.peek() {
            Token::Assert | Token::Assume | Token::Cover | Token::Expect => {
                self.parse_immediate_assertion()
            }
            Token::Unique | Token::Priority | Token::Unique0 => {
                let qualifier = self.peek().clone();
                self.advance();
                match self.peek() {
                    Token::Case | Token::CaseX | Token::CaseZ => {
                        let stmt = self.parse_case_stmt()?;
                        match stmt {
                            Stmt::Case {
                                expr,
                                items,
                                default,
                            } => {
                                if qualifier == Token::Unique0 {
                                    Ok(Stmt::Unique0Case {
                                        expr,
                                        items,
                                        default,
                                    })
                                } else if qualifier == Token::Unique {
                                    Ok(Stmt::UniqueCase {
                                        expr,
                                        items,
                                        default,
                                    })
                                } else {
                                    Ok(Stmt::PriorityCase {
                                        expr,
                                        items,
                                        default,
                                    })
                                }
                            }
                            Stmt::CaseX {
                                expr,
                                items,
                                default,
                            } => {
                                if qualifier == Token::Unique0 {
                                    Ok(Stmt::Unique0Case {
                                        expr,
                                        items,
                                        default,
                                    })
                                } else if qualifier == Token::Unique {
                                    Ok(Stmt::UniqueCase {
                                        expr,
                                        items,
                                        default,
                                    })
                                } else {
                                    Ok(Stmt::PriorityCase {
                                        expr,
                                        items,
                                        default,
                                    })
                                }
                            }
                            Stmt::CaseZ {
                                expr,
                                items,
                                default,
                            } => {
                                if qualifier == Token::Unique0 {
                                    Ok(Stmt::Unique0Case {
                                        expr,
                                        items,
                                        default,
                                    })
                                } else if qualifier == Token::Unique {
                                    Ok(Stmt::UniqueCase {
                                        expr,
                                        items,
                                        default,
                                    })
                                } else {
                                    Ok(Stmt::PriorityCase {
                                        expr,
                                        items,
                                        default,
                                    })
                                }
                            }
                            _ => Ok(stmt),
                        }
                    }
                    Token::If => {
                        let stmt = self.parse_if_stmt()?;
                        match stmt {
                            Stmt::IfElse {
                                cond,
                                true_branch,
                                false_branch,
                            } => Ok(Stmt::IfElse {
                                cond,
                                true_branch,
                                false_branch,
                            }),
                            _ => Ok(stmt),
                        }
                    }
                    _ => {
                        let stmt = self.parse_if_stmt()?;
                        Ok(Stmt::IfElse {
                            cond: Expr::Value(Value::Decimal(1)),
                            true_branch: Box::new(stmt),
                            false_branch: None,
                        })
                    }
                }
            }
            Token::If => self.parse_if_stmt(),
            Token::Case | Token::CaseX | Token::CaseZ => self.parse_case_stmt(),
            Token::For => self.parse_for_stmt(),
            Token::Foreach => self.parse_foreach_stmt(),
            Token::While => self.parse_while_stmt(),
            Token::Forever => self.parse_forever_stmt(),
            Token::Repeat => self.parse_repeat_stmt(),
            Token::Fork => self.parse_fork_join(),
            Token::Dollar => self.parse_syscall(),
            Token::Return => {
                self.advance();
                if self.peek() == &Token::Semi {
                    // `return;` tanpa nilai (task / void function)
                    self.skip_semi();
                    Ok(Stmt::Return(None))
                } else {
                    let expr = self.parse_expr(0)?;
                    self.skip_semi();
                    Ok(Stmt::Return(Some(Box::new(expr))))
                }
            }
            Token::Break => {
                self.advance();
                self.skip_semi();
                Ok(Stmt::Break)
            }
            Token::Continue => {
                self.advance();
                self.skip_semi();
                Ok(Stmt::Continue)
            }
            Token::Disable => {
                self.advance();
                // `fork` adalah keyword (Token::Fork), bukan Ident — sama
                // seperti `wait fork` (LANG-29).
                if matches!(self.peek(), Token::Fork) {
                    self.advance();
                    self.skip_semi();
                    Ok(Stmt::Disable {
                        name: Symbol::intern("fork"),
                    })
                } else {
                    let name = self.expect_ident()?;
                    self.skip_semi();
                    Ok(Stmt::Disable { name })
                }
            }
            Token::Wait => {
                self.advance();
                // `fork` adalah keyword (Token::Fork), bukan Ident.
                if matches!(self.peek(), Token::Fork) {
                    self.advance();
                    self.skip_semi();
                    // LANG-29: `wait fork;` — varian khusus (bukan `wait (0)`
                    // yang juga menghasilkan Wait{cond:0,stmt:None}).
                    Ok(Stmt::WaitFork)
                } else {
                    self.expect(Token::LParen)?;
                    let cond = self.parse_expr(0)?;
                    self.expect(Token::RParen)?;
                    let single_stmt = self.parse_stmt_block()?.into_iter().next();
                    Ok(Stmt::Wait {
                        cond,
                        stmt: single_stmt.map(Box::new),
                    })
                }
            }
            Token::Begin => {
                let stmts = self.parse_stmt_block()?;
                Ok(Stmt::Block { stmts })
            }
            Token::Force => {
                self.advance();
                // L: latch boleh hierarkis: `force tb.dut.u_sig = value;`
                let lhs = self.parse_expr(0)?;
                self.expect(Token::BlockingAssign)?;
                let rhs = self.parse_expr(0)?;
                self.skip_semi();
                Ok(Stmt::Force { lhs, rhs })
            }
            Token::Release => {
                self.advance();
                // `release <hier>;` — ref hierarkis (Ident/Member/select).
                let expr = self.parse_expr(0)?;
                self.skip_semi();
                match &expr {
                    Expr::Ident { .. }
                    | Expr::MemberAccess { .. }
                    | Expr::BitSelect { .. }
                    | Expr::RangeSelect { .. }
                    | Expr::PartSelect { .. } => Ok(Stmt::Release { expr }),
                    _ => Err(self.err("expected signal name after release")),
                }
            }
            Token::Deassign => {
                self.advance();
                let expr = self.parse_primary_expr()?;
                self.skip_semi();
                Ok(Stmt::Deassign { expr })
            }

            Token::At => {
                self.advance();
                // `@(...)` dan bare `@signal` (LRM 1800 §9.4.2): event control
                // boleh menyebut signal/ref hierarkis tanpa kurung —
                // `forever @cfg.in_reset begin ... end`.
                if self.peek() != &Token::LParen {
                    let expr = self.parse_expr(0)?;
                    let stmt = self.parse_stmt()?;
                    return Ok(Stmt::EventControl {
                        events: vec![SensitivityEvent::Level(expr)],
                        stmt: Some(Box::new(stmt)),
                    });
                }
                self.expect(Token::LParen)?;
                let events = self.parse_sensitivity_events()?;
                self.expect(Token::RParen)?;
                let stmt = self.parse_stmt()?;
                Ok(Stmt::EventControl {
                    events,
                    stmt: Some(Box::new(stmt)),
                })
            }

            Token::Do => {
                self.advance();
                let stmts = self.parse_stmt_block()?;
                self.expect(Token::While)?;
                self.expect(Token::LParen)?;
                let cond = self.parse_expr(0)?;
                self.expect(Token::RParen)?;
                self.skip_semi();
                Ok(Stmt::DoWhile { cond, stmts })
            }
            // F37: prefix `++lhs` / `--lhs` di level statement — setara postfix
            // `lhs++` (arm di bawah): assign `lhs = lhs ± 1`.
            Token::Increment | Token::Decrement => {
                let is_inc = matches!(self.peek(), Token::Increment);
                self.advance();
                let mut lhs = self.parse_primary_expr()?;
                // postfix lvalue: `++arr[i]`, `++obj.field`
                loop {
                    match self.peek() {
                        Token::LBrack => {
                            self.advance();
                            let first = self.parse_expr(0)?;
                            if self.peek() == &Token::Colon {
                                self.advance();
                                let second = self.parse_expr(0)?;
                                self.expect(Token::RBrack)?;
                                lhs = Expr::RangeSelect {
                                    expr: Box::new(lhs),
                                    msb: Box::new(first),
                                    lsb: Box::new(second),
                                };
                            } else {
                                self.expect(Token::RBrack)?;
                                lhs = Expr::BitSelect {
                                    expr: Box::new(lhs),
                                    index: Box::new(first),
                                };
                            }
                        }
                        Token::Dot => {
                            self.advance();
                            let member = self.expect_ident()?;
                            lhs = Expr::MemberAccess {
                                obj: Box::new(lhs),
                                field: member,
                            };
                        }
                        _ => break,
                    }
                }
                self.skip_semi();
                let rhs = Expr::BinaryOp {
                    op: if is_inc { BinaryOp::Add } else { BinaryOp::Sub },
                    lhs: Box::new(lhs.clone()),
                    rhs: Box::new(Expr::Value(Value::Decimal(1))),
                };
                Ok(Stmt::BlockingAssign {
                    lhs,
                    rhs,
                    delay: None,
                })
            }
            Token::Semi => {
                self.advance();
                Ok(Stmt::Null)
            }
            Token::Hash => {
                // Delay statement: #delay_stmt ... or #delay_val;
                self.advance();
                let delay = self.parse_expr(0)?;
                let stmts = self.parse_stmt_block()?;
                if stmts.len() == 1 {
                    Ok(Stmt::Delay {
                        delay,
                        stmt: Box::new(stmts.into_iter().next().unwrap()),
                    })
                } else {
                    Ok(Stmt::Delay {
                        delay,
                        stmt: Box::new(Stmt::Block { stmts }),
                    })
                }
            }
            Token::WaitOrder => self.parse_wait_order(),
            Token::Arrow => {
                self.advance();
                let name = self.expect_ident()?;
                self.skip_semi();
                Ok(Stmt::EventTrigger { name })
            }
            Token::Ident(ref s) if s == "randcase" => {
                self.advance();
                let mut items = Vec::new();
                loop {
                    let weight = self.parse_primary_expr()?;
                    self.expect(Token::Colon)?;
                    let then_stmt = self.parse_stmt()?;
                    let w = const_eval_simple(&weight).unwrap_or(1) as u64;
                    items.push(RandCaseItem {
                        weight: w,
                        stmt: Box::new(then_stmt),
                    });
                    if self.peek() == &Token::Endcase {
                        self.advance();
                        break;
                    }
                    if self.peek() == &Token::Eof || self.peek() == &Token::RBrace {
                        break;
                    }
                    if self.peek() == &Token::Semi {
                        self.advance();
                    }
                    // Selain itu cursor sudah di awal item berikutnya (semi
                    // konsumsi internal) — lanjut iterasi berikutnya.
                }
                Ok(Stmt::RandCase { items })
            }
            Token::Ident(ref s) if s == "randsequence" => {
                self.advance();
                let mut productions = Vec::new();
                loop {
                    let is_endseq = matches!(self.peek(), Token::Ident(s) if s == "endsequence");
                    if is_endseq || self.peek() == &Token::Eof {
                        if matches!(self.peek(), Token::Ident(s) if s == "endsequence") {
                            self.advance();
                        }
                        break;
                    }
                    let prod_name = self.expect_ident()?;
                    self.expect(Token::Colon)?;
                    let mut items = Vec::new();
                    loop {
                        let stmt = self.parse_stmt()?;
                        let weight = if self.peek() == &Token::BlockingAssign {
                            self.advance();
                            let w_expr = self.parse_expr(0)?;
                            Some(const_eval_simple(&w_expr).unwrap_or(1) as u64)
                        } else {
                            None
                        };
                        items.push(RandSeqItem {
                            value: Box::new(stmt),
                            weight,
                        });
                        if self.peek() == &Token::Pipe {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    self.skip_semi();
                    productions.push(RandSeqProduction {
                        name: prod_name,
                        items,
                    });
                }
                Ok(Stmt::RandSequence { productions })
            }
            _ => {
                let mut lhs = self.parse_primary_expr()?;
                loop {
                    match self.peek() {
                        Token::LBrack => {
                            self.advance();
                            let first = self.parse_expr(0)?;
                            if self.peek() == &Token::Colon {
                                self.advance();
                                let second = self.parse_expr(0)?;
                                self.expect(Token::RBrack)?;
                                lhs = Expr::RangeSelect {
                                    expr: Box::new(lhs),
                                    msb: Box::new(first),
                                    lsb: Box::new(second),
                                };
                            } else if self.peek() == &Token::PlusColon {
                                self.advance();
                                let width = self.parse_expr(0)?;
                                self.expect(Token::RBrack)?;
                                lhs = Expr::PartSelect {
                                    expr: Box::new(lhs),
                                    base: Box::new(first),
                                    width: Box::new(width),
                                };
                            } else if self.peek() == &Token::MinusColon {
                                self.advance();
                                let width = self.parse_expr(0)?;
                                self.expect(Token::RBrack)?;
                                lhs = Expr::PartSelect {
                                    expr: Box::new(lhs),
                                    base: Box::new(Expr::BinaryOp {
                                        op: BinaryOp::Sub,
                                        lhs: Box::new(first.clone()),
                                        rhs: Box::new(Expr::BinaryOp {
                                            op: BinaryOp::Sub,
                                            lhs: Box::new(width.clone()),
                                            rhs: Box::new(Expr::Value(Value::Decimal(1))),
                                        }),
                                    }),
                                    width: Box::new(width),
                                };
                            } else {
                                self.expect(Token::RBrack)?;
                                lhs = Expr::BitSelect {
                                    expr: Box::new(lhs),
                                    index: Box::new(first),
                                };
                            }
                        }
                        Token::Dot => {
                            self.advance();
                            let member = self.expect_ident()?;
                            if self.peek() == &Token::LParen {
                                self.advance();
                                let mut args = Vec::new();
                                if self.peek() != &Token::RParen {
                                    loop {
                                        if self.peek() == &Token::Comma {
                                            // Empty positional arg (macro DV).
                                            args.push(Expr::Value(Value::Decimal(1)));
                                            self.advance();
                                            if self.peek() == &Token::RParen {
                                                break;
                                            }
                                            continue;
                                        }
                                        if self.peek() == &Token::Dot {
                                            // Named arg `.name(expr)` (LRM 1800 §10.6.1) —
                                            // pola UVM `obj.method(.arg(val), ...)`. Nama
                                            // dibuang, ekspresi dipertahankan (urutan).
                                            self.advance();
                                            self.expect_ident()?;
                                            self.expect(Token::LParen)?;
                                            let e = self.parse_expr(0)?;
                                            self.expect(Token::RParen)?;
                                            args.push(e);
                                        } else {
                                            args.push(self.parse_expr(0)?);
                                        }
                                        if self.peek() == &Token::Comma {
                                            let ahead = self.peek_ahead(1).clone();
                                            let is_new_port = ahead == Token::Input
                                                || ahead == Token::Output
                                                || ahead == Token::Inout
                                                || (matches!(&ahead, Token::Ident(_))
                                                    && matches!(self.peek_ahead(2), Token::Scope));
                                            if !is_new_port {
                                                self.advance();
                                            } else {
                                                break;
                                            }
                                        } else {
                                            break;
                                        }
                                    }
                                }
                                self.expect(Token::RParen)?;
                                let mut with_clause = None;
                                // `obj.method(...) with { ... }` — constraint-style
                                // clause (randomize). Parse body lalu ikat ke MethodCall.
                                if matches!(self.peek(), Token::Ident(s) if s == "with") {
                                    self.advance();
                                    let w = if self.peek() == &Token::LBrace {
                                        self.advance();
                                        let mut exprs: Vec<Expr> = Vec::new();
                                        while self.peek() != &Token::RBrace
                                            && self.peek() != &Token::Eof
                                        {
                                            exprs.push(self.parse_expr(0)?);
                                            if self.peek() == &Token::Semi {
                                                self.advance();
                                            }
                                        }
                                        self.expect(Token::RBrace)?;
                                        exprs
                                            .into_iter()
                                            .reduce(|acc, e| Expr::BinaryOp {
                                                op: BinaryOp::LogicalAnd,
                                                lhs: Box::new(acc),
                                                rhs: Box::new(e),
                                            })
                                            .unwrap_or(Expr::Value(Value::Decimal(1)))
                                    } else {
                                        self.expect(Token::LParen)?;
                                        let e = self.parse_expr(0)?;
                                        self.expect(Token::RParen)?;
                                        e
                                    };
                                    with_clause = Some(Box::new(w));
                                }
                                lhs = Expr::MethodCall {
                                    obj: Box::new(lhs),
                                    method: member,
                                    args,
                                    with_clause,
                                };
                            } else {
                                lhs = Expr::MemberAccess {
                                    obj: Box::new(lhs),
                                    field: member,
                                };
                            }
                        }
                        _ => break,
                    }
                }
                match self.peek() {
                    Token::Increment => {
                        self.advance();
                        self.skip_semi();
                        let rhs = Expr::BinaryOp {
                            op: BinaryOp::Add,
                            lhs: Box::new(lhs.clone()),
                            rhs: Box::new(Expr::Value(Value::Decimal(1))),
                        };
                        Ok(Stmt::BlockingAssign {
                            lhs,
                            rhs,
                            delay: None,
                        })
                    }
                    Token::Decrement => {
                        self.advance();
                        self.skip_semi();
                        let rhs = Expr::BinaryOp {
                            op: BinaryOp::Sub,
                            lhs: Box::new(lhs.clone()),
                            rhs: Box::new(Expr::Value(Value::Decimal(1))),
                        };
                        Ok(Stmt::BlockingAssign {
                            lhs,
                            rhs,
                            delay: None,
                        })
                    }
                    Token::BlockingAssign => {
                        self.advance();
                        let delay = self.parse_intra_assign_delay()?;
                        // Chained assign: `a = b = val` → parse RHS expression,
                        // then check if next token is also `=` (chained).
                        let rhs = self.parse_expr(0)?;
                        if self.peek() == &Token::BlockingAssign {
                            // Chained: rhs is inner LHS, `= val` is inner assign.
                            // Parse inner assign manually (can't use parse_stmt_impl
                            // because cursor is past the inner LHS).
                            self.advance();
                            let inner_delay = self.parse_intra_assign_delay()?;
                            let inner_rhs = self.parse_expr(0)?;
                            self.skip_semi();
                            Ok(Stmt::Block {
                                stmts: vec![
                                    Stmt::BlockingAssign {
                                        lhs: rhs,
                                        rhs: inner_rhs.clone(),
                                        delay: inner_delay,
                                    },
                                    Stmt::BlockingAssign {
                                        lhs,
                                        rhs: inner_rhs,
                                        delay,
                                    },
                                ],
                            })
                        } else {
                            self.skip_semi();
                            Ok(Stmt::BlockingAssign { lhs, rhs, delay })
                        }
                    }
                    Token::NonBlockingAssign => {
                        if is_valid_lvalue(&lhs) {
                            self.advance();
                            let delay = self.parse_intra_assign_delay()?;
                            let rhs = self.parse_expr(0)?;
                            self.skip_semi();
                            Ok(Stmt::NonBlockingAssign { lhs, rhs, delay })
                        } else {
                            self.advance();
                            let rhs = self.parse_expr(8)?;
                            self.skip_semi();
                            Ok(Stmt::Expr {
                                expr: Expr::BinaryOp {
                                    op: BinaryOp::Le,
                                    lhs: Box::new(lhs),
                                    rhs: Box::new(rhs),
                                },
                            })
                        }
                    }
                    Token::PlusAssign => {
                        self.advance();
                        let rhs = self.parse_expr(0)?;
                        self.skip_semi();
                        let lhs_copy = lhs.clone();
                        Ok(Stmt::BlockingAssign {
                            lhs,
                            rhs: Expr::BinaryOp {
                                op: BinaryOp::Add,
                                lhs: Box::new(lhs_copy),
                                rhs: Box::new(rhs),
                            },
                            delay: None,
                        })
                    }
                    Token::MinusAssign => {
                        self.advance();
                        let rhs = self.parse_expr(0)?;
                        self.skip_semi();
                        let lhs_copy = lhs.clone();
                        Ok(Stmt::BlockingAssign {
                            lhs,
                            rhs: Expr::BinaryOp {
                                op: BinaryOp::Sub,
                                lhs: Box::new(lhs_copy),
                                rhs: Box::new(rhs),
                            },
                            delay: None,
                        })
                    }
                    Token::XorAssign => {
                        self.advance();
                        let rhs = self.parse_expr(0)?;
                        self.skip_semi();
                        let lhs_copy = lhs.clone();
                        Ok(Stmt::BlockingAssign {
                            lhs,
                            rhs: Expr::BinaryOp {
                                op: BinaryOp::BitXor,
                                lhs: Box::new(lhs_copy),
                                rhs: Box::new(rhs),
                            },
                            delay: None,
                        })
                    }
                    Token::AndAssign => {
                        self.advance();
                        let rhs = self.parse_expr(0)?;
                        self.skip_semi();
                        let lhs_copy = lhs.clone();
                        Ok(Stmt::BlockingAssign {
                            lhs,
                            rhs: Expr::BinaryOp {
                                op: BinaryOp::BitAnd,
                                lhs: Box::new(lhs_copy),
                                rhs: Box::new(rhs),
                            },
                            delay: None,
                        })
                    }
                    Token::OrAssign => {
                        self.advance();
                        let rhs = self.parse_expr(0)?;
                        self.skip_semi();
                        let lhs_copy = lhs.clone();
                        Ok(Stmt::BlockingAssign {
                            lhs,
                            rhs: Expr::BinaryOp {
                                op: BinaryOp::BitOr,
                                lhs: Box::new(lhs_copy),
                                rhs: Box::new(rhs),
                            },
                            delay: None,
                        })
                    }
                    Token::MulAssign => {
                        self.advance();
                        let rhs = self.parse_expr(0)?;
                        self.skip_semi();
                        let lhs_copy = lhs.clone();
                        Ok(Stmt::BlockingAssign {
                            lhs,
                            rhs: Expr::BinaryOp {
                                op: BinaryOp::Mul,
                                lhs: Box::new(lhs_copy),
                                rhs: Box::new(rhs),
                            },
                            delay: None,
                        })
                    }
                    Token::DivAssign => {
                        self.advance();
                        let rhs = self.parse_expr(0)?;
                        self.skip_semi();
                        let lhs_copy = lhs.clone();
                        Ok(Stmt::BlockingAssign {
                            lhs,
                            rhs: Expr::BinaryOp {
                                op: BinaryOp::Div,
                                lhs: Box::new(lhs_copy),
                                rhs: Box::new(rhs),
                            },
                            delay: None,
                        })
                    }
                    Token::ModAssign => {
                        self.advance();
                        let rhs = self.parse_expr(0)?;
                        self.skip_semi();
                        let lhs_copy = lhs.clone();
                        Ok(Stmt::BlockingAssign {
                            lhs,
                            rhs: Expr::BinaryOp {
                                op: BinaryOp::Mod,
                                lhs: Box::new(lhs_copy),
                                rhs: Box::new(rhs),
                            },
                            delay: None,
                        })
                    }
                    Token::ShlAssign => {
                        self.advance();
                        let rhs = self.parse_expr(0)?;
                        self.skip_semi();
                        let lhs_copy = lhs.clone();
                        Ok(Stmt::BlockingAssign {
                            lhs,
                            rhs: Expr::BinaryOp {
                                op: BinaryOp::Shl,
                                lhs: Box::new(lhs_copy),
                                rhs: Box::new(rhs),
                            },
                            delay: None,
                        })
                    }
                    Token::ShrAssign => {
                        self.advance();
                        let rhs = self.parse_expr(0)?;
                        self.skip_semi();
                        let lhs_copy = lhs.clone();
                        Ok(Stmt::BlockingAssign {
                            lhs,
                            rhs: Expr::BinaryOp {
                                op: BinaryOp::Shr,
                                lhs: Box::new(lhs_copy),
                                rhs: Box::new(rhs),
                            },
                            delay: None,
                        })
                    }
                    _ => {
                        self.skip_semi();
                        Ok(Stmt::Expr { expr: lhs })
                    }
                }
            }
        }
    }

    /// Parsing deklarasi localparam di dalam procedural block.
    /// Mendukung `localparam <type> [range] name = expr, name2 = expr2;`.
    fn parse_procedural_localparam(&mut self) -> Result<Stmt, SimError> {
        self.advance(); // consume localparam/parameter/param
        let mut dtype = DataType::Logic;
        let mut kind = DeclKind::Logic;
        // Optional type keyword
        match self.peek() {
            Token::Int => {
                self.advance();
                dtype = DataType::Int;
                kind = DeclKind::Int;
            }
            Token::Integer => {
                self.advance();
                dtype = DataType::Integer;
                kind = DeclKind::Integer;
            }
            Token::Logic => {
                self.advance();
                dtype = DataType::Logic;
                kind = DeclKind::Logic;
            }
            Token::Bit => {
                self.advance();
                dtype = DataType::Bit;
            }
            Token::Byte => {
                self.advance();
                dtype = DataType::Byte;
            }
            Token::Shortint => {
                self.advance();
                dtype = DataType::Shortint;
            }
            Token::Longint => {
                self.advance();
                dtype = DataType::Longint;
            }
            Token::Time => {
                self.advance();
                dtype = DataType::Time;
            }
            Token::Reg => {
                self.advance();
                kind = DeclKind::Reg;
            }
            Token::Signed | Token::Unsigned => {
                self.advance();
            }
            _ => {}
        }
        // Optional packed range [msb:lsb]
        let mut expr_range = None;
        if self.peek() == &Token::LBrack {
            self.advance();
            let msb = self.parse_expr(0)?;
            self.expect(Token::Colon)?;
            let lsb = self.parse_expr(0)?;
            self.expect(Token::RBrack)?;
            expr_range = Some(ExprRange { msb, lsb });
        }
        // Parameter name(s) with optional default
        let mut names = Vec::new();
        loop {
            let name = self.expect_ident()?;
            let mut expr = None;
            if self.peek() == &Token::BlockingAssign {
                self.advance();
                expr = Some(self.parse_expr(0)?);
            }
            names.push(DeclVar {
                name,
                range: None,
                expr_range: expr_range.clone(),
                array_range: None,
                array_size_expr: None,
                extra_packed_dims: vec![],
                is_dynamic: false,
                is_queue: false,
                is_associative: false,
                assoc_key_type: None,
                is_rand: false,
                is_const: true,
                expr,
            });
            if self.peek() == &Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        self.skip_semi();
        Ok(Stmt::NamedBlock {
            name: Symbol::EMPTY,
            stmts: vec![],
            decls: vec![Decl { dtype, kind, names }],
        })
    }

    pub(crate) fn parse_if_stmt(&mut self) -> Result<Stmt, SimError> {
        self.advance();
        self.expect(Token::LParen)?;
        let cond = self.parse_expr(0)?;
        self.expect(Token::RParen)?;
        let true_branch = self.parse_stmt_block()?;
        let true_stmt = if true_branch.len() == 1 {
            true_branch.into_iter().next().unwrap()
        } else {
            Stmt::Block { stmts: true_branch }
        };
        let false_branch = if self.peek() == &Token::Else {
            self.advance();
            let fb = self.parse_stmt_block()?;
            Some(Box::new(if fb.len() == 1 {
                fb.into_iter().next().unwrap()
            } else {
                Stmt::Block { stmts: fb }
            }))
        } else {
            None
        };
        Ok(Stmt::IfElse {
            cond,
            true_branch: Box::new(true_stmt),
            false_branch,
        })
    }

    pub(crate) fn parse_case_stmt(&mut self) -> Result<Stmt, SimError> {
        let is_casex = self.peek() == &Token::CaseX;
        let is_casez = self.peek() == &Token::CaseZ;
        // `inside` dapat muncul dalam dua bentuk:
        //   `case (x) inside`            — keyword setelah `(expr)`
        //   `unique/priority case (x) inside` — dipanggil setelah qualifier
        // Deteksi di depan `case` hanya menangkap bentuk tanpa qualifier;
        // deteksi setelah `(expr)` menangkap SEMUA bentuk termasuk yang
        // di-qualifier `unique`/`priority` (mis. dm_csrs OpenTitan).
        let mut is_case_inside = if self.peek() == &Token::Case {
            let saved = self.pos.get();
            self.advance();
            let is_inside = self.peek() == &Token::Inside;
            self.pos.set(saved);
            is_inside
        } else {
            false
        };
        if is_case_inside {
            self.advance();
        } else {
            self.advance();
        }
        self.expect(Token::LParen)?;
        let expr = self.parse_expr(0)?;
        self.expect(Token::RParen)?;
        // Bentuk `case (x) inside` dan `unique case (x) inside`: token `inside`
        // muncul tepat setelah `(expr)` — konsumsi di sini.
        if !is_case_inside && self.peek() == &Token::Inside {
            self.advance();
            is_case_inside = true;
        }
        let mut items = Vec::new();
        let mut default = None;
        loop {
            if self.peek() == &Token::Endcase || self.peek() == &Token::Eof {
                break;
            }
            if self.peek() == &Token::Default {
                self.advance();
                // Bentuk `default;` (pemisah `;` tanpa kolon) valid di SV:
                // berarti default dengan statement kosong.
                if self.peek() == &Token::Semi {
                    self.advance();
                    default = Some(Box::new(Stmt::Block { stmts: Vec::new() }));
                } else {
                    self.expect(Token::Colon)?;
                    let stmts = self.parse_stmt_block()?;
                    default = Some(Box::new(Stmt::Block { stmts }));
                }
            } else {
                let mut labels = Vec::new();
                loop {
                    if is_case_inside && self.peek() == &Token::LBrack {
                        // Label range dalam case-inside: `[lo:hi]` (bukan bit-select).
                        // Representasikan sebagai RangeSelect dengan base 0 —
                        // elaborator CaseInside mengenali pola ini dan mengubahnya
                        // menjadi IrExpr::InsideRange.
                        self.advance();
                        let lo = self.parse_expr(0)?;
                        self.expect(Token::Colon)?;
                        let hi = self.parse_expr(0)?;
                        self.expect(Token::RBrack)?;
                        labels.push(Expr::RangeSelect {
                            expr: Box::new(Expr::Value(Value::Decimal(0))),
                            msb: Box::new(lo),
                            lsb: Box::new(hi),
                        });
                    } else {
                        labels.push(self.parse_expr(0)?);
                    }
                    if self.peek() == &Token::Comma {
                        self.advance();
                    } else {
                        break;
                    }
                }
                self.expect(Token::Colon)?;
                let stmts = self.parse_stmt_block()?;
                items.push(CaseItem {
                    labels,
                    stmt: Box::new(Stmt::Block { stmts }),
                });
            }
        }
        self.expect(Token::Endcase)?;
        if is_case_inside {
            Ok(Stmt::CaseInside {
                expr,
                items,
                default,
            })
        } else if is_casex {
            Ok(Stmt::CaseX {
                expr,
                items,
                default,
            })
        } else if is_casez {
            Ok(Stmt::CaseZ {
                expr,
                items,
                default,
            })
        } else {
            Ok(Stmt::Case {
                expr,
                items,
                default,
            })
        }
    }

    pub(crate) fn parse_for_stmt(&mut self) -> Result<Stmt, SimError> {
        self.advance();
        self.expect(Token::LParen)?;
        let init = if self.peek() != &Token::Semi {
            if matches!(
                self.peek(),
                Token::Int
                    | Token::Integer
                    | Token::Bit
                    | Token::Logic
                    | Token::Reg
                    | Token::Longint
                    | Token::Shortint
                    | Token::Byte
                    | Token::Time
            ) || matches!(self.peek(), Token::Ident(s) if s == "uint" || s == "sint")
            {
                self.advance();
                if self.peek() == &Token::Signed {
                    self.advance();
                }
                if self.peek() == &Token::Unsigned {
                    self.advance();
                }
                let var = self.expect_ident()?;
                let init_val = if self.peek() == &Token::BlockingAssign {
                    self.advance();
                    Some(self.parse_expr(0)?)
                } else {
                    None
                };
                let stmt = if let Some(val) = init_val {
                    Stmt::BlockingAssign {
                        lhs: Expr::Ident {
                            name: var,
                            line: 0,
                            col: 0,
                        },
                        rhs: val,
                        delay: None,
                    }
                } else {
                    Stmt::Null
                };
                Some(Box::new(stmt))
            } else {
                let expr = self.parse_expr(0)?;
                let init_stmt = match self.peek() {
                    Token::BlockingAssign => {
                        self.advance();
                        let rhs = self.parse_expr(0)?;
                        Stmt::BlockingAssign {
                            lhs: expr,
                            rhs,
                            delay: None,
                        }
                    }
                    _ => Stmt::Null,
                };
                Some(Box::new(init_stmt))
            }
        } else {
            None
        };
        self.expect(Token::Semi)?;
        let cond = if self.peek() != &Token::Semi {
            Some(self.parse_expr(0)?)
        } else {
            None
        };
        self.expect(Token::Semi)?;
        let step = if self.peek() != &Token::RParen {
            let expr = self.parse_expr(0)?;
            if self.peek() == &Token::Increment {
                self.advance();
                if let Expr::Ident { name: var, .. } = expr {
                    Some(Box::new(Stmt::BlockingAssign {
                        lhs: Expr::Ident {
                            name: var,
                            line: 0,
                            col: 0,
                        },
                        rhs: Expr::BinaryOp {
                            op: BinaryOp::Add,
                            lhs: Box::new(Expr::Ident {
                                name: var,
                                line: 0,
                                col: 0,
                            }),
                            rhs: Box::new(Expr::Value(Value::Decimal(1))),
                        },
                        delay: None,
                    }))
                } else {
                    None
                }
            } else if self.peek() == &Token::Decrement {
                self.advance();
                if let Expr::Ident { name: var, .. } = expr {
                    Some(Box::new(Stmt::BlockingAssign {
                        lhs: Expr::Ident {
                            name: var,
                            line: 0,
                            col: 0,
                        },
                        rhs: Expr::BinaryOp {
                            op: BinaryOp::Sub,
                            lhs: Box::new(Expr::Ident {
                                name: var,
                                line: 0,
                                col: 0,
                            }),
                            rhs: Box::new(Expr::Value(Value::Decimal(1))),
                        },
                        delay: None,
                    }))
                } else {
                    None
                }
            } else {
                let step_stmt = match self.peek() {
                    Token::BlockingAssign => {
                        self.advance();
                        let rhs = self.parse_expr(0)?;
                        Stmt::BlockingAssign {
                            lhs: expr,
                            rhs,
                            delay: None,
                        }
                    }
                    Token::PlusAssign | Token::MinusAssign => {
                        let is_plus = self.peek() == &Token::PlusAssign;
                        let op = if is_plus {
                            BinaryOp::Add
                        } else {
                            BinaryOp::Sub
                        };
                        self.advance();
                        let rhs = self.parse_expr(0)?;
                        match expr {
                            Expr::Ident { name, .. } => Stmt::BlockingAssign {
                                lhs: Expr::Ident {
                                    name,
                                    line: 0,
                                    col: 0,
                                },
                                rhs: Expr::BinaryOp {
                                    op,
                                    lhs: Box::new(Expr::Ident {
                                        name,
                                        line: 0,
                                        col: 0,
                                    }),
                                    rhs: Box::new(rhs),
                                },
                                delay: None,
                            },
                            other => {
                                // Compound assign ke non-ident jarang di for-step.
                                drop(other);
                                Stmt::Null
                            }
                        }
                    }
                    _ => Stmt::Null,
                };
                Some(Box::new(step_stmt))
            }
        } else {
            None
        };
        self.expect(Token::RParen)?;
        let stmts = self.parse_stmt_block()?;
        Ok(Stmt::LoopFor {
            init,
            cond,
            step,
            stmts,
        })
    }

    pub(crate) fn parse_foreach_stmt(&mut self) -> Result<Stmt, SimError> {
        self.advance();
        self.expect(Token::LParen)?;
        let mut array_var = self.expect_ident()?;
        // Member path sebagai iterable: `foreach(cfg.ral_models[i] ...)` —
        // gabung segmen path dengan '.' menjadi satu identifier simbolik.
        while self.peek() == &Token::Dot {
            self.advance();
            let seg = self.expect_ident()?;
            array_var = Symbol::intern(&format!("{}.{}", array_var, seg));
        }
        self.expect(Token::LBrack)?;
        let mut index_vars = Vec::new();
        loop {
            index_vars.push(self.expect_ident()?);
            if self.peek() == &Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        self.expect(Token::RBrack)?;
        self.expect(Token::RParen)?;
        let stmts = self.parse_stmt_block()?;
        Ok(Stmt::ForeachLoop {
            array_var,
            index_vars,
            stmts,
        })
    }

    pub(crate) fn parse_while_stmt(&mut self) -> Result<Stmt, SimError> {
        self.advance();
        self.expect(Token::LParen)?;
        let cond = self.parse_expr(0)?;
        self.expect(Token::RParen)?;
        let stmts = self.parse_stmt_block()?;
        Ok(Stmt::LoopWhile { cond, stmts })
    }

    pub(crate) fn parse_forever_stmt(&mut self) -> Result<Stmt, SimError> {
        self.advance();
        let stmts = self.parse_stmt_block()?;
        Ok(Stmt::LoopForever { stmts })
    }

    pub(crate) fn parse_repeat_stmt(&mut self) -> Result<Stmt, SimError> {
        self.advance();
        self.expect(Token::LParen)?;
        let count = self.parse_expr(0)?;
        self.expect(Token::RParen)?;
        let stmts = self.parse_stmt_block()?;
        Ok(Stmt::Repeat { count, stmts })
    }

    pub(crate) fn parse_fork_join(&mut self) -> Result<Stmt, SimError> {
        self.advance();
        // Named fork: `fork : label_name begin ... end` — skip label
        if self.peek() == &Token::Colon {
            self.advance(); // consume ':'
            if matches!(self.peek(), Token::Ident(_)) {
                self.advance(); // consume label name
            }
        }
        let mut processes = Vec::new();
        loop {
            match self.peek() {
                Token::Join => {
                    self.advance();
                    return Ok(Stmt::Fork {
                        processes,
                        join_type: JoinType::Join,
                    });
                }
                Token::JoinAny => {
                    self.advance();
                    return Ok(Stmt::Fork {
                        processes,
                        join_type: JoinType::JoinAny,
                    });
                }
                Token::JoinNone => {
                    self.advance();
                    return Ok(Stmt::Fork {
                        processes,
                        join_type: JoinType::JoinNone,
                    });
                }
                Token::Eof => return Err(self.err("unexpected EOF in fork block")),
                _ => {
                    processes.push(self.parse_stmt()?);
                }
            }
        }
    }

    pub(crate) fn parse_syscall(&mut self) -> Result<Stmt, SimError> {
        // F20: posisi token `$` — dipakai diagnostic file:line:col.
        let sc_line = self.peek_line();
        let sc_col = self.peek_col();
        self.advance();
        let name_tok = self.peek().clone();
        let name = match &name_tok {
            Token::Ident(s) => {
                self.advance();
                *s
            }
            // Keyword tokens yang valid sebagai system call name ($assign, $deassign)
            Token::Assign => {
                self.advance();
                Symbol::intern("assign")
            }
            Token::Deassign => {
                self.advance();
                Symbol::intern("deassign")
            }
            _ => return Err(self.err("expected system call name after $")),
        };
        match name.as_str() {
            "finish" | "stop" => {
                if self.peek() == &Token::LParen {
                    self.advance();
                    if self.peek() != &Token::RParen {
                        self.parse_expr(0)?;
                    }
                    self.expect(Token::RParen)?;
                }
                self.skip_semi();
                Ok(Stmt::SysFinish)
            }
            "time" | "printtimescale" | "showscopes" | "get_randcount" | "get_randstate" => {
                // $time / $printtimescale / $showscopes / $get_randcount / $get_randstate
                // dipanggil tanpa kurung (atau dengan kurung kosong).
                if self.peek() == &Token::LParen {
                    self.advance();
                    let mut args = Vec::new();
                    if self.peek() != &Token::RParen {
                        loop {
                            args.push(self.parse_expr(0)?);
                            if self.peek() == &Token::Comma {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(Token::RParen)?;
                    Ok(Stmt::SysCall {
                        name,
                        args,
                        line: sc_line,
                        col: sc_col,
                    })
                } else {
                    Ok(Stmt::SysCall {
                        name,
                        args: vec![],
                        line: sc_line,
                        col: sc_col,
                    })
                }
            }
            _ => {
                self.expect(Token::LParen)?;
                let mut args = Vec::new();
                if self.peek() != &Token::RParen {
                    loop {
                        args.push(self.parse_expr(0)?);
                        if self.peek() == &Token::Comma
                            || matches!(
                                self.peek(),
                                Token::Input | Token::Output | Token::Inout | Token::Dot
                            )
                        {
                            if self.peek() == &Token::Comma {
                                self.advance();
                            }
                        } else {
                            break;
                        }
                    }
                }
                self.expect(Token::RParen)?;
                self.skip_semi();
                Ok(Stmt::SysCall {
                    name,
                    args,
                    line: sc_line,
                    col: sc_col,
                })
            }
        }
    }
}
