//! ──────────────────────────────────────────────────────────────────────────────
//! CATATAN: File ini adalah bagian dari pemisahan parser.rs (SRP Refactoring).
//! Tanggung jawab: Parsing clocking block & specify block (timing checks).
//!
//! Fungsi:
//!   - parse_clocking_block()  — parsing clocking ... endclocking
//!   - parse_specify_item()    — parsing specify item ($setup, $hold, dll.)
//!   - parse_specify_block()   — parsing specify ... endspecify
//!
//! ──────────────────────────────────────────────────────────────────────────────

use super::Parser;
use crate::lexer::*;
use maria_ast::*;
use maria_core::error::SimError;
use maria_core::intern::Symbol;

impl Parser {
    pub(crate) fn parse_clocking_block(&mut self) -> Result<ClockingBlock, SimError> {
        self.advance(); // consume 'clocking'
        let name = self.expect_ident()?;

        // Parse clock event: @(posedge clk) or @(negedge clk) or @(clk)
        self.expect(Token::At)?;
        self.expect(Token::LParen)?;
        let clock_event = if self.peek() == &Token::PosEdge {
            self.advance();
            let mut sig = self.expect_ident()?;
            while self.peek() == &Token::Dot {
                self.advance();
                let seg = self.expect_ident()?;
                sig = Symbol::intern(&format!("{}.{}", sig, seg));
            }
            ClockEvent::Posedge(sig)
        } else if self.peek() == &Token::NegEdge {
            self.advance();
            let mut sig = self.expect_ident()?;
            while self.peek() == &Token::Dot {
                self.advance();
                let seg = self.expect_ident()?;
                sig = Symbol::intern(&format!("{}.{}", sig, seg));
            }
            ClockEvent::Negedge(sig)
        } else {
            let mut sig = self.expect_ident()?;
            while self.peek() == &Token::Dot {
                self.advance();
                let seg = self.expect_ident()?;
                sig = Symbol::intern(&format!("{}.{}", sig, seg));
            }
            ClockEvent::Edge(sig)
        };
        // Clock event boleh berisi beberapa edge dipisah `or`:
        // `clocking cb @(posedge clk or negedge rst);` — skip event lanjutan.
        while self.peek() == &Token::Or {
            self.advance();
            if self.peek() == &Token::PosEdge || self.peek() == &Token::NegEdge {
                self.advance();
            }
            let _ = self.expect_ident()?;
            while self.peek() == &Token::Dot {
                self.advance();
                let _ = self.expect_ident()?;
            }
        }
        self.expect(Token::RParen)?;
        self.skip_semi();

        let mut default_input_skew = None;
        let mut default_output_skew = None;
        let mut items = Vec::new();

        loop {
            match self.peek() {
                Token::EndClocking => {
                    self.advance();
                    if self.peek() == &Token::Colon {
                        self.advance();
                        if matches!(self.peek(), Token::Ident(_)) {
                            self.advance();
                        }
                    }
                    break;
                }
                Token::Default => {
                    // default input/output #skew;
                    self.advance();
                    if self.peek() == &Token::Input {
                        self.advance();
                        if self.peek() == &Token::Hash {
                            self.advance();
                            if let Token::Number { value, .. } = self.peek().clone() {
                                self.advance();
                                default_input_skew = value.as_str().parse::<u64>().ok();
                            }
                        }
                        self.skip_semi();
                    } else if self.peek() == &Token::Output {
                        self.advance();
                        if self.peek() == &Token::Hash {
                            self.advance();
                            if let Token::Number { value, .. } = self.peek().clone() {
                                self.advance();
                                default_output_skew = value.as_str().parse::<u64>().ok();
                            }
                        }
                        self.skip_semi();
                    } else {
                        self.skip_semi();
                    }
                }
                Token::Input => {
                    self.advance();
                    let mut signals = Vec::new();
                    loop {
                        if self.peek() == &Token::Semi || self.peek() == &Token::Eof {
                            break;
                        }
                        if self.peek() == &Token::Hash {
                            // skew override
                            self.advance();
                            if let Token::Number { value, .. } = self.peek().clone() {
                                self.advance();
                                let _skew = value.as_str().parse::<u64>().ok();
                            }
                        }
                        if let Token::Ident(s) = self.peek().clone() {
                            self.advance();
                            signals.push(s);
                        } else {
                            break;
                        }
                        if self.peek() == &Token::Comma {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    self.skip_semi();
                    items.push(ClockingItem::Input {
                        signals,
                        skew: None,
                    });
                }
                Token::Output => {
                    self.advance();
                    let mut signals = Vec::new();
                    loop {
                        if self.peek() == &Token::Semi || self.peek() == &Token::Eof {
                            break;
                        }
                        if self.peek() == &Token::Hash {
                            self.advance();
                            if let Token::Number { value, .. } = self.peek().clone() {
                                self.advance();
                                let _skew = value.as_str().parse::<u64>().ok();
                            }
                        }
                        if let Token::Ident(s) = self.peek().clone() {
                            self.advance();
                            signals.push(s);
                        } else {
                            break;
                        }
                        if self.peek() == &Token::Comma {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    self.skip_semi();
                    items.push(ClockingItem::Output {
                        signals,
                        skew: None,
                    });
                }
                Token::Inout => {
                    self.advance();
                    let mut signals = Vec::new();
                    loop {
                        if self.peek() == &Token::Semi || self.peek() == &Token::Eof {
                            break;
                        }
                        if let Token::Ident(s) = self.peek().clone() {
                            self.advance();
                            signals.push(s);
                        } else {
                            break;
                        }
                        if self.peek() == &Token::Comma {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    self.skip_semi();
                    items.push(ClockingItem::InputOutput { signals });
                }
                _ => {
                    self.advance();
                }
            }
        }

        Ok(ClockingBlock {
            name,
            clock_event,
            default_input_skew,
            default_output_skew,
            items,
        })
    }

    /// Parse a timing-check reference event: optional `posedge`/`negedge` prefix
    /// followed by the signal expression. `parse_expr` tidak menerima token
    /// PosEdge/NegEdge, jadi edge prefix dikonsumsi di sini (fix SIM-24).
    /// Mengembalikan `(expr, edge_kind)` — edge kind dipakai runtime setup/hold
    /// agar tidak memicu false positive pada edge yang salah arah.
    fn parse_ref_event(&mut self) -> Result<(Expr, Option<EdgeKind>), SimError> {
        match self.peek() {
            Token::PosEdge => {
                self.advance();
                Ok((self.parse_expr(0)?, Some(EdgeKind::PosEdge)))
            }
            Token::NegEdge => {
                self.advance();
                Ok((self.parse_expr(0)?, Some(EdgeKind::NegEdge)))
            }
            _ => Ok((self.parse_expr(0)?, None)),
        }
    }

    pub(crate) fn parse_specify_item(&mut self) -> Result<Option<SpecifyItem>, SimError> {
        // Check for $setup, $hold, $setuphold system function calls
        if self.peek() == &Token::Dollar {
            // Read the system function name
            let saved = self.pos.get();
            self.advance(); // consume $
            if let Token::Ident(fname) = self.peek().clone() {
                self.advance();
                // NOTE: tokenizer memisahkan `$` menjadi Token::Dollar, sehingga
                // fname adalah "setup" (TANPA `$`). Fix SIM-24: match arms dan
                // flag is_* harus dibandingkan tanpa `$` — sebelumnya pakai
                // "$setup" sehingga specify_items SELALU kosong (bug pre-existing).
                match fname.as_str() {
                    "setup" | "hold" | "setuphold" | "recovery" | "removal" | "recrem"
                    | "period" | "width" | "nochange" | "skew" | "timeskew" => {
                        let is_setup = fname == "setup";
                        let is_hold = fname == "hold";
                        let is_setuphold = fname == "setuphold";
                        let is_recovery = fname == "recovery";
                        let is_removal = fname == "removal";
                        let is_recrem = fname == "recrem";
                        let is_period = fname == "period";
                        let is_width = fname == "width";
                        let is_nochange = fname == "nochange";
                        let is_skew = fname == "skew";
                        let is_timeskew = fname == "timeskew";
                        self.expect(Token::LParen)?;
                        // Parse based on timing check type
                        if is_period {
                            // $period(ref_event, limit);
                            let (ref_event, ref_edge) = self.parse_ref_event()?;
                            self.expect(Token::Comma)?;
                            let limit = self.parse_expr(0)?;
                            self.expect(Token::RParen)?;
                            if self.peek() == &Token::Semi {
                                self.advance();
                            }
                            return Ok(Some(SpecifyItem::PeriodCheck {
                                ref_event,
                                ref_edge,
                                limit,
                            }));
                        } else if is_width {
                            // $width(ref_event, limit [, threshold]);
                            let (ref_event, ref_edge) = self.parse_ref_event()?;
                            self.expect(Token::Comma)?;
                            let limit = self.parse_expr(0)?;
                            let threshold = if self.peek() == &Token::Comma {
                                self.advance();
                                Some(self.parse_expr(0)?)
                            } else {
                                None
                            };
                            self.expect(Token::RParen)?;
                            if self.peek() == &Token::Semi {
                                self.advance();
                            }
                            return Ok(Some(SpecifyItem::WidthCheck {
                                ref_event,
                                ref_edge,
                                limit,
                                threshold,
                            }));
                        } else if is_skew {
                            // $skew(ref_event, data, limit);
                            let (ref_event, ref_edge) = self.parse_ref_event()?;
                            self.expect(Token::Comma)?;
                            let data = self.parse_expr(0)?;
                            self.expect(Token::Comma)?;
                            let limit = self.parse_expr(0)?;
                            self.expect(Token::RParen)?;
                            if self.peek() == &Token::Semi {
                                self.advance();
                            }
                            return Ok(Some(SpecifyItem::SkewCheck {
                                ref_event,
                                ref_edge,
                                data,
                                limit,
                            }));
                        } else if is_timeskew {
                            // $timeskew(ref_event, data, limit [, threshold]);
                            let (ref_event, ref_edge) = self.parse_ref_event()?;
                            self.expect(Token::Comma)?;
                            let data = self.parse_expr(0)?;
                            self.expect(Token::Comma)?;
                            let limit = self.parse_expr(0)?;
                            let threshold = if self.peek() == &Token::Comma {
                                self.advance();
                                Some(self.parse_expr(0)?)
                            } else {
                                None
                            };
                            self.expect(Token::RParen)?;
                            if self.peek() == &Token::Semi {
                                self.advance();
                            }
                            return Ok(Some(SpecifyItem::TimeskewCheck {
                                ref_event,
                                ref_edge,
                                data,
                                limit,
                                threshold,
                            }));
                        } else if is_nochange {
                            // $nochange(ref_event, data, start_limit, end_limit);
                            let (ref_event, ref_edge) = self.parse_ref_event()?;
                            self.expect(Token::Comma)?;
                            let data = self.parse_expr(0)?;
                            self.expect(Token::Comma)?;
                            let start_limit = self.parse_expr(0)?;
                            self.expect(Token::Comma)?;
                            let end_limit = self.parse_expr(0)?;
                            self.expect(Token::RParen)?;
                            if self.peek() == &Token::Semi {
                                self.advance();
                            }
                            return Ok(Some(SpecifyItem::NochangeCheck {
                                ref_event,
                                ref_edge,
                                data,
                                start_limit,
                                end_limit,
                            }));
                        }
                        // Signature order (IEEE 1800):
                        //   $setup(data, ref, limit)     — data dulu
                        //   $hold(ref, data, limit)      — ref dulu
                        //   $setuphold(ref, data, su,h)  — ref dulu
                        //   $recovery(ref, data, limit)  — ref dulu
                        //   $removal(ref, data, limit)   — ref dulu
                        //   $recrem(ref, data, rec,rem)  — ref dulu
                        // (fix SIM-24: sebelumnya semua diparse data-dulu, sehingga
                        //  $hold(posedge clk, ...) gagal dengan "expected expression,
                        //  found PosEdge")
                        let (data, ref_event, ref_edge) = if is_setup {
                            let data = self.parse_expr(0)?;
                            self.expect(Token::Comma)?;
                            let (ref_event, ref_edge) = self.parse_ref_event()?;
                            (data, ref_event, ref_edge)
                        } else {
                            let (ref_event, ref_edge) = self.parse_ref_event()?;
                            self.expect(Token::Comma)?;
                            let data = self.parse_expr(0)?;
                            (data, ref_event, ref_edge)
                        };
                        let (setup_limit, hold_limit) = if is_setuphold || is_recrem {
                            self.expect(Token::Comma)?;
                            let sl = self.parse_expr(0)?;
                            self.expect(Token::Comma)?;
                            let hl = self.parse_expr(0)?;
                            (Some(sl), Some(hl))
                        } else {
                            self.expect(Token::Comma)?;
                            let limit = self.parse_expr(0)?;
                            if is_setup || is_recovery {
                                (Some(limit), None)
                            } else {
                                (None, Some(limit))
                            }
                        };
                        self.expect(Token::RParen)?;
                        if self.peek() == &Token::Semi {
                            self.advance();
                        }
                        return if is_setuphold {
                            Ok(Some(SpecifyItem::SetupHoldCheck {
                                ref_event,
                                ref_edge,
                                data,
                                setup_limit: setup_limit.unwrap(),
                                hold_limit: hold_limit.unwrap(),
                            }))
                        } else if is_setup {
                            Ok(Some(SpecifyItem::SetupCheck {
                                data,
                                ref_event,
                                ref_edge,
                                limit: setup_limit.unwrap(),
                            }))
                        } else if is_hold {
                            Ok(Some(SpecifyItem::HoldCheck {
                                ref_event,
                                ref_edge,
                                data,
                                limit: hold_limit.unwrap(),
                            }))
                        } else if is_recrem {
                            Ok(Some(SpecifyItem::RecoveryRemovalCheck {
                                ref_event,
                                ref_edge,
                                data,
                                recovery_limit: setup_limit.unwrap(),
                                removal_limit: hold_limit.unwrap(),
                            }))
                        } else if is_recovery {
                            Ok(Some(SpecifyItem::RecoveryCheck {
                                data,
                                ref_event,
                                ref_edge,
                                limit: setup_limit.unwrap(),
                            }))
                        } else if is_removal {
                            Ok(Some(SpecifyItem::RemovalCheck {
                                ref_event,
                                ref_edge,
                                data,
                                limit: hold_limit.unwrap(),
                            }))
                        } else {
                            // Fallback — shouldn't reach here
                            Ok(Some(SpecifyItem::SetupCheck {
                                data,
                                ref_event,
                                ref_edge,
                                limit: setup_limit
                                    .unwrap_or(hold_limit.unwrap_or(Expr::Value(
                                        maria_ast::expr::Value::Decimal(0),
                                    ))),
                            }))
                        };
                    }
                    _ => {}
                }
            }
            // Not recognized, reset position
            self.pos.set(saved);
        }

        // specparam name = value;
        if self.peek() == &Token::SpecParam {
            self.advance();
            let name = self.expect_ident()?;
            self.expect(Token::BlockingAssign)?;
            let value = self.parse_expr(0)?;
            self.skip_semi();
            return Ok(Some(SpecifyItem::SpecParam { name, value }));
        }

        // Simple path delay: (src => dst) = (rise, fall);
        if self.peek() == &Token::LParen {
            let saved = self.pos.get();
            self.advance();
            if let Token::Ident(src) = self.peek().clone() {
                self.advance();
                if self.peek() == &Token::Arrow {
                    self.advance();
                    if let Token::Ident(dst) = self.peek().clone() {
                        self.advance();
                        if self.peek() == &Token::RParen {
                            self.advance();
                            self.expect(Token::BlockingAssign)?;
                            self.expect(Token::LParen)?;
                            let rise = self.parse_expr(0)?;
                            let fall = if self.peek() == &Token::Comma {
                                self.advance();
                                Some(self.parse_expr(0)?)
                            } else {
                                None
                            };
                            self.expect(Token::RParen)?;
                            self.skip_semi();
                            return Ok(Some(SpecifyItem::PathDelay {
                                src,
                                dst,
                                rise: Some(rise),
                                fall,
                            }));
                        }
                    }
                }
            }
            self.pos.set(saved);
        }

        // Skip empty lines or unrecognized items
        Ok(None)
    }

    pub(crate) fn parse_specify_block(&mut self) -> Result<SpecifyBlock, SimError> {
        self.advance(); // consume 'specify'
        let mut items = Vec::new();
        loop {
            if self.peek() == &Token::EndSpecify || self.peek() == &Token::Eof {
                break;
            }
            if let Some(item) = self.parse_specify_item()? {
                items.push(item);
            } else {
                // Unknown item — skip token
                self.advance();
            }
        }
        self.expect(Token::EndSpecify)?;
        Ok(SpecifyBlock { items })
    }
}
