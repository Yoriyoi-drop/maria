//! CLI tools untuk maria — backend tunggal untuk tools terminal
//! (minspect, mlint, melab, msim, mcov, mwave, mfmt, mprof, mcheck, mbench).
//!
//! Semua tools memakai infrastruktur Maria yang sama (lexer/parser/elaborator/
//! simulator) agar tidak ada duplikasi logika antara CLI dan GUI.

pub mod bench;
pub mod check;
pub mod cov;
pub mod elab;
pub mod fmt;
pub mod inspect;
pub mod lint;
pub mod prof;
pub mod sim;
pub mod wave;

use std::path::{Path, PathBuf};

use crate::ast::types::Design;
use crate::diagnostics::DiagCode;
use crate::elaboration::elaborator::ElaborateMode;
use crate::error::SimError;
use crate::frontend::compile_session::CompileSession;
use crate::SessionConfig;

fn diag_io(msg: impl Into<String>) -> SimError {
    SimError::with_diag(DiagCode::IoError, msg)
}

/// Expand input target menjadi daftar file:
/// - direktori  → scan recursive (`.sv`/`.svh`/`.v`/`.vh`)
/// - file `.f`/`.maria` → daftar file (relatif ke direktori file list)
/// - lainnya → file tunggal
///
/// File di-unique-kan dan diurutkan agar output deterministik.
pub fn collect_targets(paths: &[String]) -> Result<Vec<PathBuf>, SimError> {
    use crate::frontend::discovery::DiscoveryOptions;
    use crate::frontend::FileDiscovery;
    let mut out: Vec<PathBuf> = Vec::new();
    for p in paths {
        let path = Path::new(p);
        if !path.exists() {
            return Err(diag_io(format!("path tidak ditemukan: '{}'", p)));
        }
        if path.is_dir() {
            let res = FileDiscovery::scan_dir(path, &DiscoveryOptions::default());
            out.extend(res.files.into_iter().map(|f| f.path));
        } else {
            let is_list = path
                .extension()
                .map(|e| e == "f" || e == "maria")
                .unwrap_or(false);
            if is_list {
                let files = FileDiscovery::scan_file_list(path)
                    .map_err(|e| diag_io(format!("{}: {}", p, e)))?;
                out.extend(files);
            } else {
                out.push(path.to_path_buf());
            }
        }
    }
    out.sort();
    out.dedup();
    if out.is_empty() {
        return Err(diag_io(
            "tidak ada file ditemukan — berikan direktori, file list, atau file .sv",
        ));
    }
    Ok(out)
}

/// Bangun `SessionConfig` dari daftar file + CLI options.
fn make_session_config(
    files: Vec<PathBuf>,
    incdirs: &[String],
    defines: &[String],
    top: Option<String>,
) -> SessionConfig {
    let mut cfg = SessionConfig::default();
    cfg.sources = files;
    cfg.incdirs = incdirs.iter().map(PathBuf::from).collect();
    cfg.defines = defines
        .iter()
        .filter_map(|d| d.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    cfg.top_module = top;
    cfg
}

/// Buka project: expand target → CompileSession → parse semua file secara
/// paralel (dengan cache MICD bila tersedia) → merged `Design` + session.
///
/// Session dipertahankan agar tool bisa mengakses `module_index` (peta
/// module → file) untuk output yang butuh lokasi source.
pub fn open_project(
    targets: &[String],
    incdirs: &[String],
    defines: &[String],
    top: Option<&str>,
) -> Result<(Design, CompileSession), SimError> {
    let files = collect_targets(targets)?;
    let cfg = make_session_config(files, incdirs, defines, top.map(|s| s.to_string()));
    let mut session = CompileSession::new(cfg);
    let (design, _index) = session.compile()?;
    Ok((design, session))
}

/// Buka project + elaborasi penuh → (session, design, ir_design).
/// Dipakai melab/msim/mcov/mprof.
pub fn open_elaborated(
    targets: &[String],
    incdirs: &[String],
    defines: &[String],
    top: Option<&str>,
    mode: ElaborateMode,
) -> Result<(CompileSession, Design, crate::ir::IrDesign), SimError> {
    let files = collect_targets(targets)?;
    let cfg = make_session_config(files, incdirs, defines, top.map(|s| s.to_string()));
    let mut session = CompileSession::new(cfg);
    let (design, ir, _len) = session.compile_and_elaborate_with_mode(top, mode)?;
    Ok((session, design, ir))
}

/// Format jumlah byte menjadi string yang mudah dibaca.
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{} B", n)
    } else {
        format!("{:.1} {}", v, UNITS[u])
    }
}

/// Cetak header section yang konsisten.
pub fn section(title: &str) {
    println!("\n── {} ──", title);
}

/// Cetak pasangan label/value rata kiri.
pub fn kv(label: &str, value: impl std::fmt::Display) {
    println!("  {:<26} {}", label, value);
}

/// Cetak footer dengan pesan sukses/error count.
pub fn footer(ok: usize, warn: usize) {
    println!(
        "\n── selesai: {} ok, {} warning ──",
        ok, warn
    );
}

/// Render ekspresi AST menjadi string ringkas (untuk output tool).
pub fn expr_to_string(e: &crate::ast::expr::Expr) -> String {
    use crate::ast::expr::{Expr, Value};
    match e {
        Expr::Value(v) => match v {
            Value::Binary { bits, .. } | Value::Hex { bits, .. } | Value::Octal { bits, .. } => {
                format!("{}{}", base_prefix(v), bits)
            }
            Value::Decimal(d) => d.to_string(),
            Value::Real(r) => r.to_string(),
        },
        Expr::FillLit(v) => format!("'{}", match v {
            crate::ir::LogicVal::Zero => "0",
            crate::ir::LogicVal::One => "1",
            crate::ir::LogicVal::X => "x",
            crate::ir::LogicVal::Z => "z",
        }),
        Expr::Ident { name, .. } => name.as_str().to_string(),
        Expr::FuncCall { name, args } => {
            let a: Vec<String> = args.iter().map(expr_to_string).collect();
            format!("{}({})", name.as_str(), a.join(", "))
        }
        Expr::RangeSelect { expr, msb, lsb, .. } => {
            format!("{}[{}:{}]", expr_to_string(expr), expr_to_string(msb), expr_to_string(lsb))
        }
        Expr::BitSelect { expr, index, .. } => {
            format!("{}[{}]", expr_to_string(expr), expr_to_string(index))
        }
        Expr::PartSelect { expr, base, width, .. } => {
            format!("{}[{} +: {}]", expr_to_string(expr), expr_to_string(base), expr_to_string(width))
        }
        Expr::Concat(parts) => {
            let p: Vec<String> = parts.iter().map(expr_to_string).collect();
            format!("'{{{}}}", p.join(", "))
        }
        Expr::Replicate { count, expr } => {
            format!("{{{}{{{}}}}}", expr_to_string(count), expr_to_string(expr))
        }
        Expr::UnaryOp { op, expr } => format!("{}{}", unary_op_str(op), expr_to_string(expr)),
        Expr::BinaryOp { op, lhs, rhs } => {
            format!("{} {} {}", expr_to_string(lhs), bin_op_str(op), expr_to_string(rhs))
        }
        Expr::TernaryOp { cond, true_expr, false_expr } => format!(
            "{} ? {} : {}",
            expr_to_string(cond),
            expr_to_string(true_expr),
            expr_to_string(false_expr)
        ),
        Expr::Paren(inner) => format!("({})", expr_to_string(inner)),
        Expr::String(s) => format!("\"{}\"", s),
        Expr::MemberAccess { obj, field } => {
            format!("{}.{}", expr_to_string(obj), field.as_str())
        }
        Expr::MethodCall { obj, method, args, .. } => {
            let a: Vec<String> = args.iter().map(expr_to_string).collect();
            format!("{}.{}({})", expr_to_string(obj), method.as_str(), a.join(", "))
        }
        Expr::ScopedIdent { package, item } => {
            format!("{}::{}", package.as_str(), item.as_str())
        }
        Expr::Cast { dtype, expr } => format!("{}({})", dtype.as_str(), expr_to_string(expr)),
        Expr::Null => "null".to_string(),
        Expr::Inside { expr, range_list } => {
            let r: Vec<String> = range_list.iter().map(expr_to_string).collect();
            format!("{} inside {{ {} }}", expr_to_string(expr), r.join(", "))
        }
        Expr::StreamingConcat { op, slice_size, slices } => {
            let s: Vec<String> = slices.iter().map(expr_to_string).collect();
            let w = slice_size
                .as_ref()
                .map(|e| format!("[{}]", expr_to_string(e)))
                .unwrap_or_default();
            format!("{{{} {} {}}}", op, w, s.join(", "))
        }
        Expr::Dist { expr, items } => {
            let its: Vec<String> = items
                .iter()
                .map(|d| match d {
                    crate::ast::expr::DistItem::Value(v, _) => expr_to_string(v),
                    crate::ast::expr::DistItem::Range(a, b, _) => {
                        format!("[{}:{}]", expr_to_string(a), expr_to_string(b))
                    }
                })
                .collect();
            format!("{} dist {{ {} }}", expr_to_string(expr), its.join(", "))
        }
    }
}

fn base_prefix(v: &crate::ast::expr::Value) -> String {
    use crate::ast::expr::Value;
    match v {
        Value::Binary { width, .. } => width.map(|w| format!("{}'b", w)).unwrap_or_else(|| "'b".into()),
        Value::Hex { width, .. } => width.map(|w| format!("{}'h", w)).unwrap_or_else(|| "'h".into()),
        Value::Octal { width, .. } => width.map(|w| format!("{}'o", w)).unwrap_or_else(|| "'o".into()),
        _ => "".into(),
    }
}

fn unary_op_str(op: &crate::ast::expr::UnaryOp) -> &'static str {
    use crate::ast::expr::UnaryOp::*;
    match op {
        Plus => "+",
        Minus => "-",
        BitNot => "~",
        Not => "!",
        And => "&",
        Nand => "~&",
        Or => "|",
        Nor => "~|",
        Xor => "^",
        Xnor => "~^",
    }
}

fn bin_op_str(op: &crate::ast::expr::BinaryOp) -> &'static str {
    use crate::ast::expr::BinaryOp::*;
    match op {
        Add => "+",
        Sub => "-",
        Mul => "*",
        Div => "/",
        Mod => "%",
        Power => "**",
        Eq => "==",
        Neq => "!=",
        CaseEq => "===",
        CaseNeq => "!==",
        EqWild => "==?",
        NeqWild => "!=?",
        Lt => "<",
        Le => "<=",
        Gt => ">",
        Ge => ">=",
        LogicalAnd => "&&",
        LogicalOr => "||",
        BitAnd => "&",
        BitOr => "|",
        BitXor => "^",
        BitXnor => "~^",
        Shl => "<<",
        Shr => ">>",
        Sshl => "<<<",
        Sshr => ">>>",
    }
}
