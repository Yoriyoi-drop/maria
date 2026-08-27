//! CLI tools untuk maria — backend tunggal untuk tools terminal
//! (minspect, mlint, melab, msim, mcov, mwave, mfmt, mprof, mcheck, mbench).
//!
//! Semua tools memakai infrastruktur Maria yang sama (lexer/parser/elaborator/
//! simulator) agar tidak ada duplikasi logika antara CLI dan GUI.

pub mod bench;
pub mod check;
pub mod ci;
pub mod compile_partition;
pub mod cov;
pub mod crypto;
pub mod elab;
pub mod license;
pub mod fmt;
pub mod gen;
pub mod inspect;
pub mod ipxact;
pub mod lint;
pub mod memcheck;
pub mod plugin_market;
pub mod prof;
pub mod project;
pub mod requirements;
pub mod sim;
pub mod synth;
pub mod testdb;
pub mod wave;

use std::path::{Path, PathBuf};

use maria_ast::types::Design;
use maria_compiler::frontend::compile_session::{CompileSession, SessionConfig};
use maria_core::diagnostics::DiagCode;
use maria_core::error::SimError;
use maria_elaboration::elaborator::ElaborateMode;

fn diag_io(msg: impl Into<String>) -> SimError {
    SimError::with_diag(DiagCode::IoError, msg)
}

/// Expand input target menjadi daftar file:
/// - direktori  → scan recursive (`.sv`/`.svh`/`.v`/`.vh`)
/// - file `.f` → daftar file (relatif ke direktori file list)
/// - lainnya → file tunggal
///
/// File di-unique-kan dan diurutkan agar output deterministik.
pub fn collect_targets(paths: &[String]) -> Result<Vec<PathBuf>, SimError> {
    use maria_compiler::frontend::discovery::DiscoveryOptions;
    use maria_compiler::frontend::FileDiscovery;
    let mut out: Vec<PathBuf> = Vec::new();
    for p in paths {
        let path = Path::new(p);
        if !path.exists() {
            return Err(diag_io(format!("path tidak ditemukan: '{}'", p)));
        }
        if path.is_dir() {
            // F10: sertakan `.mv` (Maria HDL) di scan direktori — tool yang
            // memakai open_project/open_elaborated otomatis men-transpile-nya.
            let mut opts = DiscoveryOptions::default();
            opts.extensions.push("mv".into());
            let res = FileDiscovery::scan_dir(path, &opts);
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

/// F10: bangun `SessionConfig` dari daftar file + CLI options, sekaligus
/// transpile semua `.mv` ke buffer inline. Satu-satunya cara membuat config
/// dengan dukungan `.mv` — dipakai open_project / open_elaborated / prof /
/// bench (DRY, agar tidak ada call site yang lupa memasang inline_sources).
fn make_session_config_with_mv(
    files: Vec<PathBuf>,
    incdirs: &[String],
    defines: &[String],
    top: Option<String>,
) -> Result<SessionConfig, SimError> {
    let inline = transpile_mv_to_inline(&files)?;
    let mut cfg = make_session_config(files, incdirs, defines, top);
    cfg.inline_sources = inline;
    Ok(cfg)
}

/// F10: transpile semua file `.mv` dalam daftar ke buffer SV inline (svh+sv
/// digabung, baris `` `include `` di-strip — definisi bersama sudah ada di
/// atasnya). File non-`.mv` tidak disentuh. Hasil: peta path → buffer.
///
/// Memakai `transpile_many` (konteks gabungan F9) sehingga package lintas-file
/// (`types.mv` → `counter.mv`) bekerja di tool manapun.
pub fn transpile_mv_to_inline(
    files: &[PathBuf],
) -> Result<std::collections::HashMap<PathBuf, Vec<u8>>, SimError> {
    let mv_idx: Vec<usize> = files
        .iter()
        .enumerate()
        .filter(|(_, p)| p.extension().map(|e| e == "mv").unwrap_or(false))
        .map(|(i, _)| i)
        .collect();
    if mv_idx.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    let mut items: Vec<(String, String)> = Vec::with_capacity(mv_idx.len());
    for &i in &mv_idx {
        let p = &files[i];
        let base = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("design")
            .to_string();
        let src =
            std::fs::read_to_string(p).map_err(|e| diag_io(format!("{}: {}", p.display(), e)))?;
        items.push((src, base));
    }
    let results = maria_mv::transpile_many(&items)
        .map_err(|(i, e)| diag_io(format!("{}: {}", files[mv_idx[i]].display(), e)))?;

    let mut inline = std::collections::HashMap::new();
    for (&i, tr) in mv_idx.iter().zip(results.iter()) {
        let mut buf = tr.svh.clone();
        buf.push('\n');
        for line in tr.sv.lines() {
            let t = line.trim_start();
            if t.starts_with("`include") {
                continue;
            }
            buf.push_str(line);
            buf.push('\n');
        }
        inline.insert(files[i].clone(), buf.into_bytes());
    }
    Ok(inline)
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
    let cfg = make_session_config_with_mv(files, incdirs, defines, top.map(|s| s.to_string()))?;
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
) -> Result<(CompileSession, Design, maria_ir::IrDesign), SimError> {
    let files = collect_targets(targets)?;
    let cfg = make_session_config_with_mv(files, incdirs, defines, top.map(|s| s.to_string()))?;
    let mut session = CompileSession::new(cfg);
    let (design, ir, _len) = session.compile_and_elaborate_with_mode(top, mode)?;
    Ok((session, design, ir))
}

/// Buka lapisan cache pipeline (`cache/<pid>/`) untuk project yang sama dengan
/// targets/incdirs/defines — tanpa compile. Dipakai tool read-back
/// (mprof --cached, melab --from-cache, minspect cache): membaca hasil build
/// sebelumnya (db.md cache/ "tidak perlu dielaborasi ulang").
///
/// Mengembalikan (CacheLayer, project id). Gagal bila targets tidak ada atau
/// database root tidak bisa dibuka.
pub fn open_cache_layer(
    targets: &[String],
    incdirs: &[String],
    defines: &[String],
) -> Result<(maria_compiler::micd::CacheLayer, String), SimError> {
    use maria_compiler::micd::MicdDatabase;
    let files = collect_targets(targets)?;
    let root = MicdDatabase::default_root();
    let proot = std::env::current_dir().unwrap_or_default();
    let inc: Vec<PathBuf> = incdirs.iter().map(PathBuf::from).collect();
    let defs: Vec<(String, String)> = defines
        .iter()
        .filter_map(|d| d.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let pid = MicdDatabase::project_id(&proot, &files, &inc, &defs);
    let db = MicdDatabase::open_project_with_context(&root, &pid, &proot, &files);
    let layer = db.cache_layer.ok_or_else(|| {
        diag_io("lapisan cache pipeline tidak tersedia (database root tak bisa ditulis)")
    })?;
    Ok((layer, pid))
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
    println!("\n── selesai: {} ok, {} warning ──", ok, warn);
}

/// Render ekspresi AST menjadi string ringkas (untuk output tool).
pub fn expr_to_string(e: &maria_ast::expr::Expr) -> String {
    use maria_ast::expr::{Expr, Value};
    match e {
        Expr::Value(v) => match v {
            Value::Binary { bits, .. } | Value::Hex { bits, .. } | Value::Octal { bits, .. } => {
                format!("{}{}", base_prefix(v), bits)
            }
            Value::Decimal(d) => d.to_string(),
            Value::Real(r) => r.to_string(),
        },
        Expr::FillLit(v) => format!(
            "'{}",
            match v {
                maria_ir::LogicVal::Zero => "0",
                maria_ir::LogicVal::One => "1",
                maria_ir::LogicVal::X => "x",
                maria_ir::LogicVal::Z => "z",
            }
        ),
        Expr::Ident { name, .. } => name.as_str().to_string(),
        Expr::FuncCall { name, args, .. } => {
            let a: Vec<String> = args.iter().map(expr_to_string).collect();
            format!("{}({})", name.as_str(), a.join(", "))
        }
        Expr::RangeSelect { expr, msb, lsb, .. } => {
            format!(
                "{}[{}:{}]",
                expr_to_string(expr),
                expr_to_string(msb),
                expr_to_string(lsb)
            )
        }
        Expr::BitSelect { expr, index, .. } => {
            format!("{}[{}]", expr_to_string(expr), expr_to_string(index))
        }
        Expr::PartSelect {
            expr, base, width, ..
        } => {
            format!(
                "{}[{} +: {}]",
                expr_to_string(expr),
                expr_to_string(base),
                expr_to_string(width)
            )
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
            format!(
                "{} {} {}",
                expr_to_string(lhs),
                bin_op_str(op),
                expr_to_string(rhs)
            )
        }
        Expr::TernaryOp {
            cond,
            true_expr,
            false_expr,
        } => format!(
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
        Expr::MethodCall {
            obj, method, args, ..
        } => {
            let a: Vec<String> = args.iter().map(expr_to_string).collect();
            format!(
                "{}.{}({})",
                expr_to_string(obj),
                method.as_str(),
                a.join(", ")
            )
        }
        Expr::ScopedIdent { package, item, .. } => {
            format!("{}::{}", package.as_str(), item.as_str())
        }
        Expr::Cast { dtype, expr } => format!("{}({})", dtype.as_str(), expr_to_string(expr)),
        Expr::CastWidth { width, expr } => {
            format!("{}'({})", expr_to_string(width), expr_to_string(expr))
        }
        Expr::Null => "null".to_string(),
        Expr::Inside { expr, range_list } => {
            let r: Vec<String> = range_list.iter().map(expr_to_string).collect();
            format!("{} inside {{ {} }}", expr_to_string(expr), r.join(", "))
        }
        Expr::StreamingConcat {
            op,
            slice_size,
            slices,
        } => {
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
                    maria_ast::expr::DistItem::Value(v, _) => expr_to_string(v),
                    maria_ast::expr::DistItem::Range(a, b, _) => {
                        format!("[{}:{}]", expr_to_string(a), expr_to_string(b))
                    }
                })
                .collect();
            format!("{} dist {{ {} }}", expr_to_string(expr), its.join(", "))
        }
        Expr::StructLit { members } => {
            let inner: Vec<String> = members
                .iter()
                .map(|m| match m {
                    maria_ast::expr::StructLitMember::Named(n, e) => {
                        format!("{}: {}", n.as_str(), expr_to_string(e))
                    }
                    maria_ast::expr::StructLitMember::Positional(e) => expr_to_string(e),
                    maria_ast::expr::StructLitMember::Default(e) => {
                        format!("default: {}", expr_to_string(e))
                    }
                })
                .collect();
            format!("'{{{}}}", inner.join(", "))
        }
    }
}

fn base_prefix(v: &maria_ast::expr::Value) -> String {
    use maria_ast::expr::Value;
    match v {
        Value::Binary { width, .. } => width
            .map(|w| format!("{}'b", w))
            .unwrap_or_else(|| "'b".into()),
        Value::Hex { width, .. } => width
            .map(|w| format!("{}'h", w))
            .unwrap_or_else(|| "'h".into()),
        Value::Octal { width, .. } => width
            .map(|w| format!("{}'o", w))
            .unwrap_or_else(|| "'o".into()),
        _ => "".into(),
    }
}

fn unary_op_str(op: &maria_ast::expr::UnaryOp) -> &'static str {
    use maria_ast::expr::UnaryOp::*;
    match op {
        Plus => "+",
        Minus => "-",
        BitNot => "~",
        Not => "!",
        ReductionAnd => "&",
        ReductionNand => "~&",
        ReductionOr => "|",
        ReductionNor => "~|",
        ReductionXor => "^",
        ReductionXnor => "~^",
    }
}

fn bin_op_str(op: &maria_ast::expr::BinaryOp) -> &'static str {
    use maria_ast::expr::BinaryOp::*;
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
