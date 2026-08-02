use std::collections::{HashMap, HashSet};

use super::expr::Expr;
use super::expr::Value;
use super::stmt::Stmt;
use super::types::{CaseGenerateItem, FunctionDecl, GenerateBlock, GenerateItem, Module, ModuleItem};
use crate::ast::inline_util::*;
use crate::intern::Symbol;
pub fn inline_func_calls_in_module(module: &mut Module) -> Result<Vec<(Symbol, usize)>, String> {
    let funcs: HashMap<Symbol, FunctionDecl> = module
        .items
        .iter()
        .filter_map(|item| {
            if let ModuleItem::Func(f) = item {
                Some((f.name, f.clone()))
            } else {
                None
            }
        })
        .collect();

    if funcs.is_empty() {
        return Ok(Vec::new());
    }

    // Detect recursive functions — these must NOT be inlined; they'll be called at runtime
    let recursive_funcs = detect_recursive_functions(&funcs);

    let mut counter = 0usize;
    let prefix = module.name;
    let mut temp_signals: Vec<(Symbol, usize)> = Vec::new();

    let old_items = std::mem::take(&mut module.items);
    let mut new_items: Vec<ModuleItem> = Vec::new();
    for item in old_items {
        match item {
            ModuleItem::Always(mut always) => {
                always.stmts = always
                    .stmts
                    .drain(..)
                    .map(|s| {
                        inline_funcs_in_stmt(
                            s,
                            &funcs,
                            prefix,
                            &mut counter,
                            &mut temp_signals,
                            &recursive_funcs,
                        )
                    })
                    .collect();
                new_items.push(ModuleItem::Always(always));
            }
            ModuleItem::Initial(mut initial) => {
                initial.stmts = initial
                    .stmts
                    .drain(..)
                    .map(|s| {
                        inline_funcs_in_stmt(
                            s,
                            &funcs,
                            prefix,
                            &mut counter,
                            &mut temp_signals,
                            &recursive_funcs,
                        )
                    })
                    .collect();
                new_items.push(ModuleItem::Initial(initial));
            }
            ModuleItem::Final(mut final_block) => {
                final_block.stmts = final_block
                    .stmts
                    .drain(..)
                    .map(|s| {
                        inline_funcs_in_stmt(
                            s,
                            &funcs,
                            prefix,
                            &mut counter,
                            &mut temp_signals,
                            &recursive_funcs,
                        )
                    })
                    .collect();
                new_items.push(ModuleItem::Final(final_block));
            }
            ModuleItem::Assign(assign) => {
                let mut preamble = Vec::new();
                let old_rhs = assign.rhs;
                let new_rhs = replace_func_calls_in_expr(
                    old_rhs,
                    &funcs,
                    prefix,
                    &mut counter,
                    &mut preamble,
                    &mut temp_signals,
                    &recursive_funcs,
                );
                if preamble.is_empty() {
                    new_items.push(ModuleItem::Assign(super::types::ContinuousAssign {
                        lhs: assign.lhs,
                        rhs: new_rhs,
                        delay: assign.delay,
                    }));
                } else {
                    preamble.push(Stmt::BlockingAssign {
                        lhs: assign.lhs,
                        rhs: new_rhs,
                        delay: None,
                    });
                    let wc = super::stmt::SensitivityList {
                        events: vec![super::stmt::SensitivityEvent::Wildcard],
                    };
                    new_items.push(ModuleItem::Always(super::stmt::AlwaysBlock {
                        kind: super::stmt::AlwaysKind::AlwaysComb,
                        sensitivity: Some(wc),
                        stmts: preamble,
                    }));
                }
            }
            ModuleItem::Func(f) => {
                if recursive_funcs.contains(&f.name) {
                    // Keep recursive function declarations in module items for runtime evaluation
                    new_items.push(ModuleItem::Func(f));
                }
                // Non-recursive functions are removed (they've been inlined)
            }
            ModuleItem::Generate(gen) => {
                let new_gen = inline_funcs_in_generate(
                    gen,
                    &funcs,
                    prefix,
                    &mut counter,
                    &mut temp_signals,
                    &recursive_funcs,
                );
                new_items.push(ModuleItem::Generate(new_gen));
            }
            other => {
                new_items.push(other);
            }
        }
    }
    module.items = new_items;

    // Remove non-recursive function declarations from module items
    module.items.retain(|item| {
        if let ModuleItem::Func(f) = item {
            recursive_funcs.contains(&f.name)
        } else {
            true
        }
    });

    Ok(temp_signals)
}

/// Inline function calls di dalam generate block. Generate berisi daftar
/// ModuleItem (Items/If/For/Case body) yang bisa memanggil function package
/// (pola OpenTitan: `for (genvar i ...) assign x[i] = aes_mul2(x[i]);`).
/// Tanpa traversal ini, pemanggilan function di dalam generate tidak pernah
/// di-inline → elaborator mencoba mem-proses body function (dengan variabel
/// lokal seperti `out`) sebagai sinyal module → error E2001.
fn inline_funcs_in_generate(
    gen: GenerateBlock,
    funcs: &HashMap<Symbol, FunctionDecl>,
    prefix: Symbol,
    counter: &mut usize,
    temp_signals: &mut Vec<(Symbol, usize)>,
    recursive_funcs: &HashSet<Symbol>,
) -> GenerateBlock {
    let items = gen
        .items
        .into_iter()
        .map(|gi| inline_generate_item(gi, funcs, prefix, counter, temp_signals, recursive_funcs))
        .collect();
    GenerateBlock { items }
}

fn inline_generate_item(
    gi: GenerateItem,
    funcs: &HashMap<Symbol, FunctionDecl>,
    prefix: Symbol,
    counter: &mut usize,
    temp_signals: &mut Vec<(Symbol, usize)>,
    recursive_funcs: &HashSet<Symbol>,
) -> GenerateItem {
    match gi {
        GenerateItem::Items(items) => GenerateItem::Items(inline_module_items(
            items,
            funcs,
            prefix,
            counter,
            temp_signals,
            recursive_funcs,
        )),
        GenerateItem::If {
            cond,
            true_items,
            false_items,
            label,
        } => GenerateItem::If {
            cond,
            true_items: inline_module_items(
                true_items,
                funcs,
                prefix,
                counter,
                temp_signals,
                recursive_funcs,
            ),
            false_items: inline_module_items(
                false_items,
                funcs,
                prefix,
                counter,
                temp_signals,
                recursive_funcs,
            ),
            label,
        },
        GenerateItem::For {
            var,
            init,
            cond,
            step,
            body_items,
            label,
        } => GenerateItem::For {
            var,
            init,
            cond,
            step,
            body_items: inline_module_items(
                body_items,
                funcs,
                prefix,
                counter,
                temp_signals,
                recursive_funcs,
            ),
            label,
        },
        GenerateItem::Case {
            case_type,
            expr,
            items,
            default,
        } => GenerateItem::Case {
            case_type,
            expr,
            items: items
                .into_iter()
                .map(|ci| CaseGenerateItem {
                    labels: ci.labels,
                    body: inline_module_items(
                        ci.body,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    ),
                })
                .collect(),
            default: default.map(|d| {
                inline_module_items(d, funcs, prefix, counter, temp_signals, recursive_funcs)
            }),
        },
    }
}

/// Inline function calls di dalam daftar ModuleItem (dipakai generate body).
fn inline_module_items(
    items: Vec<ModuleItem>,
    funcs: &HashMap<Symbol, FunctionDecl>,
    prefix: Symbol,
    counter: &mut usize,
    temp_signals: &mut Vec<(Symbol, usize)>,
    recursive_funcs: &HashSet<Symbol>,
) -> Vec<ModuleItem> {
    let mut out = Vec::new();
    for item in items {
        match item {
            ModuleItem::Assign(assign) => {
                let mut preamble = Vec::new();
                let old_rhs = assign.rhs;
                let new_rhs = replace_func_calls_in_expr(
                    old_rhs,
                    funcs,
                    prefix,
                    counter,
                    &mut preamble,
                    temp_signals,
                    recursive_funcs,
                );
                if preamble.is_empty() {
                    out.push(ModuleItem::Assign(super::types::ContinuousAssign {
                        lhs: assign.lhs,
                        rhs: new_rhs,
                        delay: assign.delay,
                    }));
                } else {
                    preamble.push(Stmt::BlockingAssign {
                        lhs: assign.lhs,
                        rhs: new_rhs,
                        delay: None,
                    });
                    let wc = super::stmt::SensitivityList {
                        events: vec![super::stmt::SensitivityEvent::Wildcard],
                    };
                    out.push(ModuleItem::Always(super::stmt::AlwaysBlock {
                        kind: super::stmt::AlwaysKind::AlwaysComb,
                        sensitivity: Some(wc),
                        stmts: preamble,
                    }));
                }
            }
            ModuleItem::Always(mut always) => {
                always.stmts = always
                    .stmts
                    .drain(..)
                    .map(|s| {
                        inline_funcs_in_stmt(
                            s,
                            funcs,
                            prefix,
                            counter,
                            temp_signals,
                            recursive_funcs,
                        )
                    })
                    .collect();
                out.push(ModuleItem::Always(always));
            }
            ModuleItem::Initial(mut initial) => {
                initial.stmts = initial
                    .stmts
                    .drain(..)
                    .map(|s| {
                        inline_funcs_in_stmt(
                            s,
                            funcs,
                            prefix,
                            counter,
                            temp_signals,
                            recursive_funcs,
                        )
                    })
                    .collect();
                out.push(ModuleItem::Initial(initial));
            }
            ModuleItem::Final(mut final_block) => {
                final_block.stmts = final_block
                    .stmts
                    .drain(..)
                    .map(|s| {
                        inline_funcs_in_stmt(
                            s,
                            funcs,
                            prefix,
                            counter,
                            temp_signals,
                            recursive_funcs,
                        )
                    })
                    .collect();
                out.push(ModuleItem::Final(final_block));
            }
            ModuleItem::Generate(gen) => {
                out.push(ModuleItem::Generate(inline_funcs_in_generate(
                    gen,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                )));
            }
            other => out.push(other),
        }
    }
    out
}

fn inline_funcs_in_stmt(
    stmt: Stmt,
    funcs: &HashMap<Symbol, FunctionDecl>,
    prefix: Symbol,
    counter: &mut usize,
    temp_signals: &mut Vec<(Symbol, usize)>,
    recursive_funcs: &HashSet<Symbol>,
) -> Stmt {
    match stmt {
        Stmt::Block { stmts } => {
            let new_stmts = stmts
                .into_iter()
                .map(|s| {
                    inline_funcs_in_stmt(s, funcs, prefix, counter, temp_signals, recursive_funcs)
                })
                .collect();
            Stmt::Block { stmts: new_stmts }
        }
        Stmt::NamedBlock { name, stmts, decls } => {
            let new_stmts = stmts
                .into_iter()
                .map(|s| {
                    inline_funcs_in_stmt(s, funcs, prefix, counter, temp_signals, recursive_funcs)
                })
                .collect();
            Stmt::NamedBlock {
                name,
                stmts: new_stmts,
                decls,
            }
        }
        Stmt::IfElse {
            cond,
            true_branch,
            false_branch,
        } => {
            let mut preamble = Vec::new();
            let new_cond = replace_func_calls_in_expr(
                cond,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let new_true = inline_funcs_in_stmt(
                *true_branch,
                funcs,
                prefix,
                counter,
                temp_signals,
                recursive_funcs,
            );
            let new_false = false_branch.map(|fb| {
                inline_funcs_in_stmt(*fb, funcs, prefix, counter, temp_signals, recursive_funcs)
            });
            let main = Stmt::IfElse {
                cond: new_cond,
                true_branch: Box::new(new_true),
                false_branch: new_false.map(Box::new),
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::Case {
            expr,
            items,
            default,
        } => {
            let mut preamble = Vec::new();
            let new_expr = replace_func_calls_in_expr(
                expr,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let new_items = items
                .into_iter()
                .map(|item| {
                    let new_labels = item
                        .labels
                        .into_iter()
                        .map(|l| {
                            replace_func_calls_in_expr(
                                l,
                                funcs,
                                prefix,
                                counter,
                                &mut Vec::new(),
                                temp_signals,
                                recursive_funcs,
                            )
                        })
                        .collect();
                    let new_stmt = inline_funcs_in_stmt(
                        *item.stmt,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    );
                    super::stmt::CaseItem {
                        labels: new_labels,
                        stmt: Box::new(new_stmt),
                    }
                })
                .collect();
            let new_default = default.map(|d| {
                Box::new(inline_funcs_in_stmt(
                    *d,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                ))
            });
            let main = Stmt::Case {
                expr: new_expr,
                items: new_items,
                default: new_default,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::CaseX {
            expr,
            items,
            default,
        } => {
            let mut preamble = Vec::new();
            let new_expr = replace_func_calls_in_expr(
                expr,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let new_items = items
                .into_iter()
                .map(|item| {
                    let new_labels = item
                        .labels
                        .into_iter()
                        .map(|l| {
                            replace_func_calls_in_expr(
                                l,
                                funcs,
                                prefix,
                                counter,
                                &mut Vec::new(),
                                temp_signals,
                                recursive_funcs,
                            )
                        })
                        .collect();
                    let new_stmt = inline_funcs_in_stmt(
                        *item.stmt,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    );
                    super::stmt::CaseItem {
                        labels: new_labels,
                        stmt: Box::new(new_stmt),
                    }
                })
                .collect();
            let new_default = default.map(|d| {
                Box::new(inline_funcs_in_stmt(
                    *d,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                ))
            });
            let main = Stmt::CaseX {
                expr: new_expr,
                items: new_items,
                default: new_default,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::CaseZ {
            expr,
            items,
            default,
        } => {
            let mut preamble = Vec::new();
            let new_expr = replace_func_calls_in_expr(
                expr,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let new_items = items
                .into_iter()
                .map(|item| {
                    let new_labels = item
                        .labels
                        .into_iter()
                        .map(|l| {
                            replace_func_calls_in_expr(
                                l,
                                funcs,
                                prefix,
                                counter,
                                &mut Vec::new(),
                                temp_signals,
                                recursive_funcs,
                            )
                        })
                        .collect();
                    let new_stmt = inline_funcs_in_stmt(
                        *item.stmt,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    );
                    super::stmt::CaseItem {
                        labels: new_labels,
                        stmt: Box::new(new_stmt),
                    }
                })
                .collect();
            let new_default = default.map(|d| {
                Box::new(inline_funcs_in_stmt(
                    *d,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                ))
            });
            let main = Stmt::CaseZ {
                expr: new_expr,
                items: new_items,
                default: new_default,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::LoopForever { stmts } => Stmt::LoopForever {
            stmts: stmts
                .into_iter()
                .map(|s| {
                    inline_funcs_in_stmt(s, funcs, prefix, counter, temp_signals, recursive_funcs)
                })
                .collect(),
        },
        Stmt::LoopWhile { cond, stmts } => {
            let mut preamble = Vec::new();
            let new_cond = replace_func_calls_in_expr(
                cond,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let new_stmts = stmts
                .into_iter()
                .map(|s| {
                    inline_funcs_in_stmt(s, funcs, prefix, counter, temp_signals, recursive_funcs)
                })
                .collect();
            let main = Stmt::LoopWhile {
                cond: new_cond,
                stmts: new_stmts,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::LoopFor {
            init,
            cond,
            step,
            stmts,
        } => {
            let new_init = init.map(|i| {
                Box::new(inline_funcs_in_stmt(
                    *i,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                ))
            });
            let mut preamble = Vec::new();
            let new_cond = cond.map(|c| {
                replace_func_calls_in_expr(
                    c,
                    funcs,
                    prefix,
                    counter,
                    &mut preamble,
                    temp_signals,
                    recursive_funcs,
                )
            });
            let new_step = step.map(|s| {
                Box::new(inline_funcs_in_stmt(
                    *s,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                ))
            });
            let new_stmts = stmts
                .into_iter()
                .map(|s| {
                    inline_funcs_in_stmt(s, funcs, prefix, counter, temp_signals, recursive_funcs)
                })
                .collect();
            let main = Stmt::LoopFor {
                init: new_init,
                cond: new_cond,
                step: new_step,
                stmts: new_stmts,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::Repeat { count, stmts } => {
            let mut preamble = Vec::new();
            let new_count = replace_func_calls_in_expr(
                count,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let new_stmts = stmts
                .into_iter()
                .map(|s| {
                    inline_funcs_in_stmt(s, funcs, prefix, counter, temp_signals, recursive_funcs)
                })
                .collect();
            let main = Stmt::Repeat {
                count: new_count,
                stmts: new_stmts,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::BlockingAssign { lhs, rhs, delay } => {
            let mut preamble = Vec::new();
            let new_rhs = replace_func_calls_in_expr(
                rhs,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let main = Stmt::BlockingAssign {
                lhs,
                rhs: new_rhs,
                delay,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::NonBlockingAssign { lhs, rhs, delay } => {
            let mut preamble = Vec::new();
            let new_rhs = replace_func_calls_in_expr(
                rhs,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let main = Stmt::NonBlockingAssign {
                lhs,
                rhs: new_rhs,
                delay,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::StmtAssign { lhs, rhs } => {
            // Check if LHS is a function/task call (task statement like `my_task(a, b)`)
            if let Expr::FuncCall { name, args } = &lhs {
                if let Some(func) = funcs.get(name) {
                    let c = *counter;
                    *counter += 1;

                    let mut preamble = Vec::new();

                    let new_args: Vec<Expr> = args
                        .iter()
                        .map(|a| {
                            replace_func_calls_in_expr(
                                a.clone(),
                                funcs,
                                prefix,
                                counter,
                                &mut preamble,
                                temp_signals,
                                recursive_funcs,
                            )
                        })
                        .collect();

                    let mut rename_map: HashMap<Symbol, Symbol> = HashMap::new();

                    for (i, arg) in new_args.into_iter().enumerate() {
                        let port = func.ports.get(i).cloned().unwrap_or_else(|| {
                            super::types::FunctionPort {
                                name: Symbol::intern(&format!("_arg{}", i)),
                                range: None,
                                expr_range: None,
                                direction: None,
                            }
                        });                            let temp_arg_name = Symbol::intern(&format!("__func_{}_{}_{}_{}", prefix, name, c, port.name));
                    let port_width = func_port_width(func, port.name);
                    temp_signals.push((temp_arg_name, port_width));
                    rename_map.insert(port.name, temp_arg_name);
                    preamble.push(Stmt::BlockingAssign {
                        lhs: Expr::Ident { name: temp_arg_name, line: 0, col: 0 },
                            rhs: arg,
                            delay: None,
                        });
                    }

                    // Add internal declarations (non-port variables)
                    for decl in &func.decls {
                        for var in &decl.names {
                            if rename_map.contains_key(&var.name) {
                                continue;
                            }                        let new_name_sym = Symbol::intern(&format!("__func_{}_{}_{}_{}", prefix, name, c, var.name));
                        let dtype_width = match &decl.dtype {
                            super::types::DataType::Bit | super::types::DataType::Logic => 1,
                            super::types::DataType::Byte => 8,
                            super::types::DataType::Shortint => 16,
                            super::types::DataType::Int | super::types::DataType::Integer => 32,
                            super::types::DataType::Longint => 64,
                            super::types::DataType::Time => 64,
                            super::types::DataType::Signed(inner) => match inner.as_ref() {
                                super::types::DataType::Bit | super::types::DataType::Logic => {
                                    1
                                }
                                super::types::DataType::Byte => 8,
                                super::types::DataType::Shortint => 16,
                                super::types::DataType::Int
                                | super::types::DataType::Integer => 32,
                                super::types::DataType::Longint => 64,
                                super::types::DataType::Time => 64,
                                _ => 32,
                            },
                            _ => 1,
                        };
                        let decl_width = match &decl.kind {
                            super::types::DeclKind::Wire
                            | super::types::DeclKind::Reg
                            | super::types::DeclKind::Logic => 1,
                            super::types::DeclKind::Int | super::types::DeclKind::Integer => 32,
                            _ => 1,
                        };
                        let width = if let Some(r) = &var.range {
                            r.width()
                        } else if let Some(er) = &var.expr_range {
                            // `logic [7:0] out` menyimpan range di expr_range
                            // bila batasnya ekspresi/konstanta yang belum
                            // di-fold saat parse.
                            if let (Ok(msb), Ok(lsb)) = (
                                super::types::const_eval_simple(&er.msb),
                                super::types::const_eval_simple(&er.lsb),
                            ) {
                                if msb >= lsb {
                                    (msb - lsb + 1) as usize
                                } else {
                                    (lsb - msb + 1) as usize
                                }
                            } else {
                                dtype_width.max(decl_width)
                            }
                        } else {
                            dtype_width.max(decl_width)
                        };
                        temp_signals.push((new_name_sym, width));
                        rename_map.insert(var.name, new_name_sym);
                    }
                }

                    // Insert renamed body statements
                    for func_stmt in &func.stmts {
                        let mut renamed = rename_in_stmt(func_stmt, &rename_map);
                        renamed = rename_func_decls_in_stmt(renamed, &rename_map);
                        // Proses ulang untuk menangkap nested function calls
                        // di dalam body task (sama seperti jalur function).
                        renamed = inline_funcs_in_stmt(
                            renamed,
                            funcs,
                            prefix,
                            counter,
                            temp_signals,
                            recursive_funcs,
                        );
                        preamble.push(renamed);
                    }

                    // Also process the RHS normally (may contain function calls)
                    let preamble2 = &mut Vec::new();
                    let _new_rhs = replace_func_calls_in_expr(
                        rhs,
                        funcs,
                        prefix,
                        counter,
                        preamble2,
                        temp_signals,
                        recursive_funcs,
                    );
                    preamble.append(preamble2);

                    if preamble.len() == 1 {
                        preamble.into_iter().next().unwrap()
                    } else {
                        Stmt::Block { stmts: preamble }
                    }
                } else {
                    let mut preamble = Vec::new();
                    let new_rhs = replace_func_calls_in_expr(
                        rhs,
                        funcs,
                        prefix,
                        counter,
                        &mut preamble,
                        temp_signals,
                        recursive_funcs,
                    );
                    let main = Stmt::StmtAssign { lhs, rhs: new_rhs };
                    if preamble.is_empty() {
                        main
                    } else {
                        preamble.push(main);
                        Stmt::Block { stmts: preamble }
                    }
                }
            } else {
                let mut preamble = Vec::new();
                let new_rhs = replace_func_calls_in_expr(
                    rhs,
                    funcs,
                    prefix,
                    counter,
                    &mut preamble,
                    temp_signals,
                    recursive_funcs,
                );
                let main = Stmt::StmtAssign { lhs, rhs: new_rhs };
                if preamble.is_empty() {
                    main
                } else {
                    preamble.push(main);
                    Stmt::Block { stmts: preamble }
                }
            }
        }
        Stmt::StmtCase {
            expr,
            items,
            default,
        } => {
            let mut preamble = Vec::new();
            let new_expr = replace_func_calls_in_expr(
                expr,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let new_items = items
                .into_iter()
                .map(|item| {
                    let new_labels = item
                        .labels
                        .into_iter()
                        .map(|l| {
                            replace_func_calls_in_expr(
                                l,
                                funcs,
                                prefix,
                                counter,
                                &mut Vec::new(),
                                temp_signals,
                                recursive_funcs,
                            )
                        })
                        .collect();
                    let new_stmt = inline_funcs_in_stmt(
                        *item.stmt,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    );
                    super::stmt::CaseItem {
                        labels: new_labels,
                        stmt: Box::new(new_stmt),
                    }
                })
                .collect();
            let new_default = default.map(|d| {
                Box::new(inline_funcs_in_stmt(
                    *d,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                ))
            });
            let main = Stmt::StmtCase {
                expr: new_expr,
                items: new_items,
                default: new_default,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::SysCall { name, args } => {
            let mut preamble = Vec::new();
            let new_args = args
                .into_iter()
                .map(|a| {
                    replace_func_calls_in_expr(
                        a,
                        funcs,
                        prefix,
                        counter,
                        &mut preamble,
                        temp_signals,
                        recursive_funcs,
                    )
                })
                .collect();
            let main = Stmt::SysCall {
                name,
                args: new_args,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::SysFinish => Stmt::SysFinish,
        Stmt::Delay { delay, stmt } => {
            let mut preamble = Vec::new();
            let new_delay = replace_func_calls_in_expr(
                delay,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let new_stmt =
                inline_funcs_in_stmt(*stmt, funcs, prefix, counter, temp_signals, recursive_funcs);
            let main = Stmt::Delay {
                delay: new_delay,
                stmt: Box::new(new_stmt),
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::Disable { name } => Stmt::Disable { name },
        Stmt::Force { lhs, rhs } => {
            let mut preamble = Vec::new();
            let new_rhs = replace_func_calls_in_expr(
                rhs,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let main = Stmt::Force { lhs, rhs: new_rhs };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::Release { expr } => Stmt::Release { expr },
        Stmt::Deassign { expr } => Stmt::Deassign { expr },
        Stmt::Wait { cond, stmt } => {
            let new_cond = replace_func_calls_in_expr(
                cond,
                funcs,
                prefix,
                counter,
                &mut vec![],
                temp_signals,
                recursive_funcs,
            );
            Stmt::Wait {
                cond: new_cond,
                stmt: stmt.map(|s| {
                    Box::new(inline_funcs_in_stmt(
                        *s,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    ))
                }),
            }
        }
        Stmt::EventControl { events, stmt } => Stmt::EventControl {
            events: events.clone(),
            stmt: stmt.map(|s| {
                Box::new(inline_funcs_in_stmt(
                    *s,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                ))
            }),
        },
        Stmt::EventTrigger { name } => Stmt::EventTrigger { name },
        Stmt::Expr { expr } => {
            let mut preamble = Vec::new();
            let new_expr = replace_func_calls_in_expr(
                expr,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let main = Stmt::Expr { expr: new_expr };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::Null => Stmt::Null,
        Stmt::Return(expr) => {
            // `return <expr>` bisa mengandung nested function calls (pola
            // OpenTitan: `aes_mul4` → `return aes_mul2(aes_mul2(in));`).
            // Tanpa proses ulang, panggilan di dalam return tidak di-inline
            // → elaborator mencoba resolve nama lokal function → E2001.
            let mut preamble = Vec::new();
            let new_expr = expr.map(|e| {
                replace_func_calls_in_expr(
                    *e,
                    funcs,
                    prefix,
                    counter,
                    &mut preamble,
                    temp_signals,
                    recursive_funcs,
                )
            });
            let main = Stmt::Return(new_expr.map(Box::new));
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::ForeachLoop {
            array_var,
            index_vars,
            stmts,
        } => {
            let stmts = stmts
                .into_iter()
                .map(|s| {
                    inline_funcs_in_stmt(s, funcs, prefix, counter, temp_signals, recursive_funcs)
                })
                .collect();
            Stmt::ForeachLoop {
                array_var,
                index_vars,
                stmts,
            }
        }
        Stmt::Break => Stmt::Break,
        Stmt::Continue => Stmt::Continue,
        Stmt::DoWhile { cond, stmts } => {
            let new_stmts = stmts
                .into_iter()
                .map(|s| {
                    inline_funcs_in_stmt(s, funcs, prefix, counter, temp_signals, recursive_funcs)
                })
                .collect();
            let new_cond = replace_func_calls_in_expr(
                cond,
                funcs,
                prefix,
                counter,
                &mut Vec::new(),
                temp_signals,
                recursive_funcs,
            );
            Stmt::DoWhile {
                cond: new_cond,
                stmts: new_stmts,
            }
        }
        Stmt::Fork {
            processes,
            join_type,
        } => Stmt::Fork {
            processes: processes
                .into_iter()
                .map(|s| {
                    inline_funcs_in_stmt(s, funcs, prefix, counter, temp_signals, recursive_funcs)
                })
                .collect(),
            join_type,
        },
        Stmt::RandCase { items } => Stmt::RandCase {
            items: items
                .into_iter()
                .map(|rc| crate::ast::stmt::RandCaseItem {
                    weight: rc.weight,
                    stmt: Box::new(inline_funcs_in_stmt(
                        *rc.stmt,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    )),
                })
                .collect(),
        },
        Stmt::RandSequence { productions } => Stmt::RandSequence {
            productions: productions
                .into_iter()
                .map(|p| crate::ast::stmt::RandSeqProduction {
                    name: p.name,
                    items: p
                        .items
                        .into_iter()
                        .map(|item| crate::ast::stmt::RandSeqItem {
                            value: Box::new(inline_funcs_in_stmt(
                                *item.value,
                                funcs,
                                prefix,
                                counter,
                                temp_signals,
                                recursive_funcs,
                            )),
                            weight: item.weight,
                        })
                        .collect(),
                })
                .collect(),
        },
        Stmt::UniqueCase {
            expr,
            items,
            default,
        } => {
            let mut preamble = Vec::new();
            let new_expr = replace_func_calls_in_expr(
                expr,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let new_items = items
                .into_iter()
                .map(|item| {
                    let new_labels = item
                        .labels
                        .into_iter()
                        .map(|l| {
                            replace_func_calls_in_expr(
                                l,
                                funcs,
                                prefix,
                                counter,
                                &mut Vec::new(),
                                temp_signals,
                                recursive_funcs,
                            )
                        })
                        .collect();
                    let new_stmt = inline_funcs_in_stmt(
                        *item.stmt,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    );
                    super::stmt::CaseItem {
                        labels: new_labels,
                        stmt: Box::new(new_stmt),
                    }
                })
                .collect();
            let new_default = default.map(|d| {
                Box::new(inline_funcs_in_stmt(
                    *d,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                ))
            });
            let main = Stmt::UniqueCase {
                expr: new_expr,
                items: new_items,
                default: new_default,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::PriorityCase {
            expr,
            items,
            default,
        } => {
            let mut preamble = Vec::new();
            let new_expr = replace_func_calls_in_expr(
                expr,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let new_items = items
                .into_iter()
                .map(|item| {
                    let new_labels = item
                        .labels
                        .into_iter()
                        .map(|l| {
                            replace_func_calls_in_expr(
                                l,
                                funcs,
                                prefix,
                                counter,
                                &mut Vec::new(),
                                temp_signals,
                                recursive_funcs,
                            )
                        })
                        .collect();
                    let new_stmt = inline_funcs_in_stmt(
                        *item.stmt,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    );
                    super::stmt::CaseItem {
                        labels: new_labels,
                        stmt: Box::new(new_stmt),
                    }
                })
                .collect();
            let new_default = default.map(|d| {
                Box::new(inline_funcs_in_stmt(
                    *d,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                ))
            });
            let main = Stmt::PriorityCase {
                expr: new_expr,
                items: new_items,
                default: new_default,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::CaseInside {
            expr,
            items,
            default,
        } => {
            let mut preamble = Vec::new();
            let new_expr = replace_func_calls_in_expr(
                expr,
                funcs,
                prefix,
                counter,
                &mut preamble,
                temp_signals,
                recursive_funcs,
            );
            let new_items = items
                .into_iter()
                .map(|item| {
                    let new_labels = item
                        .labels
                        .into_iter()
                        .map(|l| {
                            replace_func_calls_in_expr(
                                l,
                                funcs,
                                prefix,
                                counter,
                                &mut Vec::new(),
                                temp_signals,
                                recursive_funcs,
                            )
                        })
                        .collect();
                    let new_stmt = inline_funcs_in_stmt(
                        *item.stmt,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    );
                    super::stmt::CaseItem {
                        labels: new_labels,
                        stmt: Box::new(new_stmt),
                    }
                })
                .collect();
            let new_default = default.map(|d| {
                Box::new(inline_funcs_in_stmt(
                    *d,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                ))
            });
            let main = Stmt::CaseInside {
                expr: new_expr,
                items: new_items,
                default: new_default,
            };
            if preamble.is_empty() {
                main
            } else {
                preamble.push(main);
                Stmt::Block { stmts: preamble }
            }
        }
        Stmt::Assert {
            cond,
            pass_stmt,
            fail_stmt,
            clock_event: _ce,
            disable_iff: _di,
        } => {
            let new_cond = replace_func_calls_in_expr(
                cond,
                funcs,
                prefix,
                counter,
                &mut vec![],
                temp_signals,
                recursive_funcs,
            );
            Stmt::Assert {
                cond: new_cond,
                pass_stmt: pass_stmt.map(|s| {
                    Box::new(inline_funcs_in_stmt(
                        *s,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    ))
                }),
                fail_stmt: fail_stmt.map(|s| {
                    Box::new(inline_funcs_in_stmt(
                        *s,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    ))
                }),
                clock_event: None,
                disable_iff: None,
            }
        }
        Stmt::Assume {
            cond,
            pass_stmt,
            fail_stmt,
            clock_event: _ce,
            disable_iff: _di,
        } => {
            let new_cond = replace_func_calls_in_expr(
                cond,
                funcs,
                prefix,
                counter,
                &mut vec![],
                temp_signals,
                recursive_funcs,
            );
            Stmt::Assume {
                cond: new_cond,
                pass_stmt: pass_stmt.map(|s| {
                    Box::new(inline_funcs_in_stmt(
                        *s,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    ))
                }),
                fail_stmt: fail_stmt.map(|s| {
                    Box::new(inline_funcs_in_stmt(
                        *s,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    ))
                }),
                clock_event: None,
                disable_iff: None,
            }
        }
        Stmt::Cover {
            cond,
            pass_stmt,
            clock_event: _ce,
            disable_iff: _di,
        } => {
            let new_cond = replace_func_calls_in_expr(
                cond,
                funcs,
                prefix,
                counter,
                &mut vec![],
                temp_signals,
                recursive_funcs,
            );
            Stmt::Cover {
                cond: new_cond,
                pass_stmt: pass_stmt.map(|s| {
                    Box::new(inline_funcs_in_stmt(
                        *s,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    ))
                }),
                clock_event: None,
                disable_iff: None,
            }
        }
        Stmt::Expect {
            cond,
            pass_stmt,
            fail_stmt,
        } => {
            let new_cond = replace_func_calls_in_expr(
                cond,
                funcs,
                prefix,
                counter,
                &mut vec![],
                temp_signals,
                recursive_funcs,
            );
            Stmt::Expect {
                cond: new_cond,
                pass_stmt: pass_stmt.map(|s| {
                    Box::new(inline_funcs_in_stmt(
                        *s,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    ))
                }),
                fail_stmt: fail_stmt.map(|s| {
                    Box::new(inline_funcs_in_stmt(
                        *s,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    ))
                }),
            }
        }
        Stmt::WaitOrder { events, fail_stmt } => Stmt::WaitOrder {
            events,
            fail_stmt: fail_stmt.map(|s| {
                Box::new(inline_funcs_in_stmt(
                    *s,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                ))
            }),
        },
        Stmt::UniqueIf {
            cond,
            true_branch,
            false_branch,
        } => {
            let new_cond = replace_func_calls_in_expr(
                cond,
                funcs,
                prefix,
                counter,
                &mut vec![],
                temp_signals,
                recursive_funcs,
            );
            Stmt::UniqueIf {
                cond: new_cond,
                true_branch: Box::new(inline_funcs_in_stmt(
                    *true_branch,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                )),
                false_branch: false_branch.map(|s| {
                    Box::new(inline_funcs_in_stmt(
                        *s,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    ))
                }),
            }
        }
        Stmt::PriorityIf {
            cond,
            true_branch,
            false_branch,
        } => {
            let new_cond = replace_func_calls_in_expr(
                cond,
                funcs,
                prefix,
                counter,
                &mut vec![],
                temp_signals,
                recursive_funcs,
            );
            Stmt::PriorityIf {
                cond: new_cond,
                true_branch: Box::new(inline_funcs_in_stmt(
                    *true_branch,
                    funcs,
                    prefix,
                    counter,
                    temp_signals,
                    recursive_funcs,
                )),
                false_branch: false_branch.map(|s| {
                    Box::new(inline_funcs_in_stmt(
                        *s,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    ))
                }),
            }
        }
    }
}

fn replace_func_calls_in_expr(
    expr: Expr,
    funcs: &HashMap<Symbol, FunctionDecl>,
    prefix: Symbol,
    counter: &mut usize,
    preamble: &mut Vec<Stmt>,
    temp_signals: &mut Vec<(Symbol, usize)>,
    recursive_funcs: &HashSet<Symbol>,
) -> Expr {
    match expr {
        Expr::FuncCall { name, args } => {
            if recursive_funcs.contains(&name) {
                // Recursive function call — keep as FuncCall for runtime evaluation
                let new_args: Vec<Expr> = args
                    .into_iter()
                    .map(|a| {
                        replace_func_calls_in_expr(
                            a,
                            funcs,
                            prefix,
                            counter,
                            preamble,
                            temp_signals,
                            recursive_funcs,
                        )
                    })
                    .collect();
                return Expr::FuncCall {
                    name,
                    args: new_args,
                };
            }
            if let Some(func) = funcs.get(&name) {
                let c = *counter;
                *counter += 1;

                let ret_width = func_return_width(func);
                let is_void = ret_width == 0;
                let ret_name = if !is_void {
                    let rn = Symbol::intern(&format!("__func_{}_{}_{}_result", prefix, name, c));
                    temp_signals.push((rn, ret_width));
                    Some(rn)
                } else {
                    None
                };

                let new_args: Vec<Expr> = args
                    .into_iter()
                    .map(|a| {
                        replace_func_calls_in_expr(
                            a,
                            funcs,
                            prefix,
                            counter,
                            preamble,
                            temp_signals,
                            recursive_funcs,
                        )
                    })
                    .collect();

                let mut rename_map: HashMap<Symbol, Symbol> = HashMap::new();
                if let Some(ref rn) = ret_name {
                    rename_map.insert(name, *rn);
                }

                let orig_args: Vec<Expr> = new_args.clone();

                for (i, arg) in new_args.into_iter().enumerate() {
                    let port =
                        func.ports
                            .get(i)
                            .cloned()
                            .unwrap_or_else(||                            super::types::FunctionPort {
                                name: Symbol::intern(&format!("_arg{}", i)),
                                range: None,
                                expr_range: None,
                                direction: None,
                            });                    let temp_arg_name = Symbol::intern(&format!("__func_{}_{}_{}_{}", prefix, name, c, port.name));
                        let port_width = func_port_width(func, port.name);
                        temp_signals.push((temp_arg_name, port_width));
                        rename_map.insert(port.name, temp_arg_name);
                        preamble.push(Stmt::BlockingAssign {
                            lhs: Expr::Ident { name: temp_arg_name, line: 0, col: 0 },
                        rhs: arg,
                        delay: None,
                    });
                }

                // Add internal function declarations (non-port variables)
                for decl in &func.decls {
                    for var in &decl.names {
                        if rename_map.contains_key(&var.name) {
                            continue;
                        }
                        let new_name_sym = Symbol::intern(&format!("__func_{}_{}_{}_{}", prefix, name, c, var.name));
                        let dtype_width = match &decl.dtype {
                            super::types::DataType::Bit | super::types::DataType::Logic => 1,
                            super::types::DataType::Byte => 8,
                            super::types::DataType::Shortint => 16,
                            super::types::DataType::Int | super::types::DataType::Integer => 32,
                            super::types::DataType::Longint => 64,
                            super::types::DataType::Time => 64,
                            super::types::DataType::Signed(inner) => match inner.as_ref() {
                                super::types::DataType::Bit | super::types::DataType::Logic => 1,
                                super::types::DataType::Byte => 8,
                                super::types::DataType::Shortint => 16,
                                super::types::DataType::Int | super::types::DataType::Integer => 32,
                                super::types::DataType::Longint => 64,
                                super::types::DataType::Time => 64,
                                _ => 32,
                            },
                            _ => 1,
                        };
                        let decl_width = match &decl.kind {
                            super::types::DeclKind::Wire
                            | super::types::DeclKind::Reg
                            | super::types::DeclKind::Logic => 1,
                            super::types::DeclKind::Int | super::types::DeclKind::Integer => 32,
                            _ => 1,
                        };
                        let width = if let Some(r) = &var.range {
                            r.width()
                        } else if let Some(er) = &var.expr_range {
                            // `logic [7:0] out` menyimpan range di expr_range
                            // bila batasnya ekspresi/konstanta yang belum
                            // di-fold saat parse.
                            if let (Ok(msb), Ok(lsb)) = (
                                super::types::const_eval_simple(&er.msb),
                                super::types::const_eval_simple(&er.lsb),
                            ) {
                                if msb >= lsb {
                                    (msb - lsb + 1) as usize
                                } else {
                                    (lsb - msb + 1) as usize
                                }
                            } else {
                                dtype_width.max(decl_width)
                            }
                        } else {
                            dtype_width.max(decl_width)
                        };
                        temp_signals.push((new_name_sym, width));
                        rename_map.insert(var.name, new_name_sym);
                    }
                }

                for func_stmt in &func.stmts {
                    let mut renamed = rename_in_stmt(func_stmt, &rename_map);
                    // Convert Return(expr) to assignment to result signal
                    if let Some(ref rn) = ret_name {
                        if let Stmt::Return(Some(expr)) = &renamed {
                            renamed = Stmt::BlockingAssign {
                                lhs: Expr::Ident { name: *rn, line: 0, col: 0 },
                                rhs: *expr.clone(),
                                delay: None,
                            };
                        }
                    }
                    renamed = rename_func_decls_in_stmt(renamed, &rename_map);
                    // Proses ulang statement body untuk menangkap nested
                    // function calls (pola `aes_mul4` memanggil `aes_mul2`
                    // di dalam body-nya). Tanpa ini, FuncCall nested tetap
                    // tersisa → elaborator tidak bisa resolve nama lokal
                    // function → E2001 'out' not found.
                    renamed = inline_funcs_in_stmt(
                        renamed,
                        funcs,
                        prefix,
                        counter,
                        temp_signals,
                        recursive_funcs,
                    );
                    preamble.push(renamed);
                }

                // Write-back output/inout port values to caller's signals
                for (i, orig_arg) in orig_args.into_iter().enumerate() {
                    let port =
                        func.ports
                            .get(i)
                            .cloned()
                            .unwrap_or_else(||                            super::types::FunctionPort {
                                name: Symbol::intern(&format!("_arg{}", i)),
                                range: None,
                                expr_range: None,
                                direction: None,
                            });
                    let temp_arg_sym = Symbol::intern(&format!("__func_{}_{}_{}_{}", prefix, name, c, port.name));
                    if let Expr::Ident { .. } = &orig_arg {
                        preamble.push(Stmt::BlockingAssign {
                            lhs: orig_arg,
                            rhs: Expr::Ident { name: temp_arg_sym, line: 0, col: 0 },
                            delay: None,
                        });
                    }
                }

                if let Some(rn) = ret_name {
                    Expr::Ident { name: rn, line: 0, col: 0 }
                } else {
                    Expr::Value(Value::Decimal(0))
                }
            } else {
                Expr::FuncCall { name, args }
            }
        }
        Expr::BinaryOp { op, lhs, rhs } => Expr::BinaryOp {
            op,
            lhs: Box::new(replace_func_calls_in_expr(
                *lhs,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
            rhs: Box::new(replace_func_calls_in_expr(
                *rhs,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
        },
        Expr::UnaryOp { op, expr: inner } => Expr::UnaryOp {
            op,
            expr: Box::new(replace_func_calls_in_expr(
                *inner,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
        },
        Expr::TernaryOp {
            cond,
            true_expr,
            false_expr,
        } => Expr::TernaryOp {
            cond: Box::new(replace_func_calls_in_expr(
                *cond,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
            true_expr: Box::new(replace_func_calls_in_expr(
                *true_expr,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
            false_expr: Box::new(replace_func_calls_in_expr(
                *false_expr,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
        },
        Expr::Concat(exprs) => Expr::Concat(
            exprs
                .into_iter()
                .map(|e| {
                    replace_func_calls_in_expr(
                        e,
                        funcs,
                        prefix,
                        counter,
                        preamble,
                        temp_signals,
                        recursive_funcs,
                    )
                })
                .collect(),
        ),
        Expr::Replicate { count, expr: inner } => Expr::Replicate {
            count,
            expr: Box::new(replace_func_calls_in_expr(
                *inner,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
        },
        Expr::Paren(inner) => Expr::Paren(Box::new(replace_func_calls_in_expr(
            *inner,
            funcs,
            prefix,
            counter,
            preamble,
            temp_signals,
            recursive_funcs,
        ))),
        Expr::RangeSelect {
            expr: inner,
            msb,
            lsb,
        } => Expr::RangeSelect {
            expr: Box::new(replace_func_calls_in_expr(
                *inner,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
            msb: Box::new(replace_func_calls_in_expr(
                *msb,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
            lsb: Box::new(replace_func_calls_in_expr(
                *lsb,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
        },
        Expr::BitSelect { expr: inner, index } => Expr::BitSelect {
            expr: Box::new(replace_func_calls_in_expr(
                *inner,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
            index: Box::new(replace_func_calls_in_expr(
                *index,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
        },
        Expr::PartSelect {
            expr: inner,
            base,
            width,
        } => Expr::PartSelect {
            expr: Box::new(replace_func_calls_in_expr(
                *inner,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
            base: Box::new(replace_func_calls_in_expr(
                *base,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
            width: Box::new(replace_func_calls_in_expr(
                *width,
                funcs,
                prefix,
                counter,
                preamble,
                temp_signals,
                recursive_funcs,
            )),
        },
        other => other,
    }
}

