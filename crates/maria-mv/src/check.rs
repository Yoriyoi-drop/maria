//! Maria HDL (.mv) — Type-check & semantic analysis (Fase 4).
//! Validasi SEBELUM emisi (MARIA-HDL.md §9, prinsip desain #4): error
//! lebar-bit, arah port, sinyal/tipe tak dikenal, dan aturan assignment
//! muncul di level `.mv` — bukan di SV hasil generate.
//!
//! Kode error (MARIA-HDL.md §9):
//! - E2001: undefined signal
//! - E2002: width mismatch pada assignment (RHS lebih lebar dari LHS)
//! - E2003: cannot drive input port (hanya diizinkan di body `initial`/`final`)
//! - E2004: assignment operator di konteks salah (`<=` di luar `seq` / `=` di dalam `seq`)
//! - E2005: unknown type
//! - E2006: literal overflow
//! - E2007: duplicate name
//!
//! Aturan konservatif (hindari false positive pada kode multi-file):
//! - Package/tipe/modul yang TIDAK didefinisikan di file ini dianggap
//!   eksternal (file `.mv` lain) → dilewati, bukan error.
//! - Nama function pada pemanggilan `f(...)` tidak divalidasi (bisa
//!   eksternal); hanya argumennya yang diperiksa.
//! - Port `out`/`inout` boleh di-drive; port `in` hanya boleh di-drive di
//!   body `initial`/`final` (testbench).
//!
//! 1 file = 1 tanggung jawab: hanya type-check, tanpa lexing/parsing/codegen.

use crate::ast::*;
use crate::MvError;
use std::collections::{HashMap, HashSet};

/// Indeks global satu file `.mv`.
struct Ctx<'a> {
    /// nama tipe (file level + semua package) → Typedef
    types: HashMap<&'a str, &'a Typedef>,
    /// nama package → Package
    packages: HashMap<&'a str, &'a Package>,
    /// nama member enum → lebar enum asal (bit)
    enum_members: HashMap<&'a str, i64>,
    /// nama konstanta package (terlihat di lebar/nilai enum & ekspresi)
    consts: HashSet<&'a str>,
    /// nama function/task level file
    funcs: HashSet<&'a str>,
    tasks: HashSet<&'a str>,
    /// nama class — sah sebagai tipe user-defined (lebar tidak diketahui,
    /// lihat type_width): `var it : item = item::new()` (F12).
    classes: HashSet<&'a str>,
    /// nama interface (F26) — sah sebagai tipe port module (`in if : axi_lite`).
    interfaces: HashSet<&'a str>,
    /// nama module + program di file ini/gabungan (F29) — target instansiasi
    /// yang dikenal; nama lain dianggap eksternal (file .mv lain) → dilewati
    /// (pola konservatif E2001/E2005).
    modules: HashSet<&'a str>,
    /// nama target (module/interface/program) → daftar port (utk validasi
    /// koneksi `.port(expr)` dan jumlah positional). Value owned (String)
    /// agar tidak perlu leak; key meminjam AST (&'a str).
    module_ports: HashMap<&'a str, Vec<String>>,
    /// nama target module/program → daftar parameter (F31 — validasi
    /// override `inst foo u #(.W(4))`): nama parameter harus ada; value
    /// owned (String) konsisten dengan module_ports.
    module_params: HashMap<&'a str, Vec<String>>,
    /// F32: nama target module/program → daftar nama TYPE parameter
    /// (`T : type = ...`). Override param ini divalidasi sbg TIPE (bukan
    /// ekspresi nilai) — nilai harus nama tipe yang dikenal.
    module_type_params: HashMap<&'a str, Vec<String>>,
}

/// Nilai parameter yang berhasil di-fold ke konstanta integer.
type Params<'a> = HashMap<&'a str, i64>;

/// Lingkungan module untuk pesan error + aturan E2003.
#[derive(Clone)]
struct Env<'a> {
    mname: &'a str,
    /// arah port (nama port → Dir)
    ports: HashMap<&'a str, Dir>,
}

/// Scope deklarasi yang terlihat pada titik check.
#[derive(Clone)]
struct Scope<'a> {
    /// nama sinyal/port/reg/const/genvar/loop-var/local/instance
    sigs: HashSet<&'a str>,
    /// nama function/task (module + file level)
    funcs: HashSet<&'a str>,
    /// parameter module + nilai konstannya (untuk const-fold)
    params: Params<'a>,
    /// F32: type parameter module (`T : type = logic[7:0]`) — nama → default
    /// tipe (None bila tanpa default). Sah sbg tipe deklarasi di dalam module.
    type_params: HashMap<&'a str, Option<&'a MvType>>,
    /// tipe deklarasi sinyal/port (untuk lebar bit)
    types: HashMap<&'a str, &'a MvType>,
    enum_members: &'a HashMap<&'a str, i64>,
    /// konstanta package (terlihat sebagai ident di ekspresi)
    consts: &'a HashSet<&'a str>,
    env: Env<'a>,
    /// kedalaman loop (untuk validasi break/continue)
    loop_depth: usize,
    /// apakah sedang di dalam task (return expr di task = error)
    in_task: bool,
}

impl Scope<'_> {
    fn known(&self, name: &str) -> bool {
        self.sigs.contains(name)
            || self.funcs.contains(name)
            || self.params.contains_key(name)
            || self.enum_members.contains_key(name)
            || self.consts.contains(name)
    }
}

/// Konteks blok statement untuk aturan assignment (E2004).
#[derive(Clone, Copy, PartialEq)]
enum BlockKind {
    /// `seq` — hanya non-blocking `<=`
    Seq,
    /// comb/always/latch/func/task — hanya blocking `=`; E2003 aktif
    Always,
    /// `initial`/`final` (testbench) — blocking `=`; boleh drive port input
    Tb,
}

/// Error type-check BERPOSISI (F11): `err_at(line, col, code, msg)`.
/// `MvError::format()` menampilkan `line:col: pesan` bila posisi terisi
/// (selama ini semua error check memakai (0,0) — AST tanpa span).
fn err_at(line: usize, col: usize, code: &str, msg: impl Into<String>) -> MvError {
    MvError::new(line, col, format!("[{code}] {}", msg.into()))
}

/// Posisi (line, col) nama typedef — dipakai error E2007/E2005 (F11).
fn td_pos(td: &Typedef) -> (usize, usize) {
    match td {
        Typedef::Alias { line, col, .. }
        | Typedef::Struct { line, col, .. }
        | Typedef::Union { line, col, .. }
        | Typedef::Enum { line, col, .. } => (*line, *col),
    }
}

// ── Entrypoint ──

/// Type-check satu file `.mv` (konteks hanya dari file ini). Error pertama
/// di-return (fail-fast). Untuk batch lintas-file pakai `check_many`.
pub fn check(file: &MvFile) -> Result<(), MvError> {
    let ctx = build_ctx(file);
    check_with_ctx(file, &ctx)
}

/// Type-check beberapa file `.mv` dengan KONTEKS GABUNGAN (F9 — multi-file):
/// tipe/package/enum-member/konstanta/function dari SEMUA file terlihat oleh
/// semua file. Ini yang membuat `use pkg::*` antar-file bekerja (`types.mv`
/// mendefinisikan `Addr`, `counter.mv` memakainya). Error pertama di-return
/// bersama indeks file asalnya (untuk pesan error yang menyertakan path).
pub fn check_many(files: &[&MvFile]) -> Result<(), (usize, MvError)> {
    let ctx = build_ctx_many(files);

    // ── duplikat LINTAS-FILE (E2007) ──
    // Di pipeline buffer gabungan, package/tipe-level-file/class dengan nama
    // sama dari DUA file akan di-emit dua kali → error SV yang membingungkan.
    // Deteksi di sini (prinsip desain #4: error di level `.mv`, bukan hasil
    // generate). Duplikat dalam-satu-file tetap dideteksi check_with_ctx.
    let mut pkg_owner: HashMap<&str, usize> = HashMap::new();
    let mut type_owner: HashMap<&str, usize> = HashMap::new();
    let mut cls_owner: HashMap<&str, usize> = HashMap::new();
    let mut mod_owner: HashMap<&str, usize> = HashMap::new();
    // F26: interface berbagi namespace dengan package (definisi bersama di
    // .svh) — duplikat lintas-file juga error (E2007).
    let mut ifc_owner: HashMap<&str, usize> = HashMap::new();
    for (i, f) in files.iter().enumerate() {
        for p in &f.packages {
            if let Some(prev) = pkg_owner.insert(p.name.as_str(), i) {
                if prev != i {
                    return Err((
                        i,
                        err_at(
                            p.line,
                            p.col,
                            "E2007",
                            format!(
                                "package '{}' dideklarasikan di file #{} dan #{}",
                                p.name,
                                prev + 1,
                                i + 1
                            ),
                        ),
                    ));
                }
            }
        }
        for ifc in &f.interfaces {
            if let Some(prev) = ifc_owner.insert(ifc.name.as_str(), i) {
                if prev != i {
                    return Err((
                        i,
                        err_at(
                            ifc.line,
                            ifc.col,
                            "E2007",
                            format!(
                                "interface '{}' dideklarasikan di file #{} dan #{}",
                                ifc.name,
                                prev + 1,
                                i + 1
                            ),
                        ),
                    ));
                }
            }
        }
        for td in &f.typedefs {
            let n = td_name(td);
            if let Some(prev) = type_owner.insert(n, i) {
                if prev != i {
                    let (l, c) = td_pos(td);
                    return Err((
                        i,
                        err_at(
                            l,
                            c,
                            "E2007",
                            format!(
                                "tipe '{n}' dideklarasikan di file #{} dan #{}",
                                prev + 1,
                                i + 1
                            ),
                        ),
                    ));
                }
            }
        }
        for c in &f.classes {
            if let Some(prev) = cls_owner.insert(c.name.as_str(), i) {
                if prev != i {
                    return Err((
                        i,
                        err_at(
                            c.line,
                            c.col,
                            "E2007",
                            format!(
                                "class '{}' dideklarasikan di file #{} dan #{}",
                                c.name,
                                prev + 1,
                                i + 1
                            ),
                        ),
                    ));
                }
            }
        }
        // Module & program berbagi namespace SV — duplikat LINTAS-file juga
        // error (E2007), posisi memakai line/col nama deklarasi (F11).
        for m in f.modules.iter().chain(f.programs.iter()) {
            if let Some(prev) = mod_owner.insert(m.name.as_str(), i) {
                if prev != i {
                    return Err((
                        i,
                        err_at(
                            m.line,
                            m.col,
                            "E2007",
                            format!(
                                "module '{}' dideklarasikan di file #{} dan #{}",
                                m.name,
                                prev + 1,
                                i + 1
                            ),
                        ),
                    ));
                }
            }
        }
        // F29 fix review: module/interface berbagi namespace tipe di SV —
        // nama yang sama antara module (di file ini) dan interface (file
        // lain) juga error (E2007). `insert` di atas sudah menolak module
        // vs module; cek silang interface vs module di sini.
        for ifc in &f.interfaces {
            if let Some(prev) = mod_owner.get(ifc.name.as_str()) {
                if *prev != i {
                    return Err((
                        i,
                        err_at(
                            ifc.line,
                            ifc.col,
                            "E2007",
                            format!(
                                "interface '{}' bentrok dengan module di file #{}",
                                ifc.name,
                                prev + 1
                            ),
                        ),
                    ));
                }
            }
        }
    }

    for (i, f) in files.iter().enumerate() {
        if let Err(e) = check_with_ctx(f, &ctx) {
            return Err((i, e));
        }
    }
    Ok(())
}

/// Validasi satu file terhadap konteks yang diberikan (bisa gabungan).
fn check_with_ctx<'a>(file: &'a MvFile, ctx: &'a Ctx<'a>) -> Result<(), MvError> {
    // ── level file: duplikasi tipe + validasi isi ──
    let mut seen = HashSet::new();
    for td in &file.typedefs {
        let n = td_name(td);
        if !seen.insert(n) {
            let (l, c) = td_pos(td);
            return Err(err_at(
                l,
                c,
                "E2007",
                format!("tipe '{n}' dideklarasikan dua kali di level file"),
            ));
        }
        check_typedef(td, &ctx)?;
    }
    // duplikat nama function/task level file (E2007)
    let mut fnames = HashSet::new();
    for f in &file.funcs {
        if !fnames.insert(f.name.as_str()) {
            return Err(err_at(
                f.line,
                f.col,
                "E2007",
                format!(
                    "function '{}' dideklarasikan dua kali di level file",
                    f.name
                ),
            ));
        }
    }
    let mut tnames = HashSet::new();
    for t in &file.tasks {
        if !tnames.insert(t.name.as_str()) {
            return Err(err_at(
                t.line,
                t.col,
                "E2007",
                format!("task '{}' dideklarasikan dua kali di level file", t.name),
            ));
        }
    }

    // ── interface (F26) ──
    // F29 fix review: interface & module berbagi namespace tipe di SV —
    // nama yang sama dalam satu file = error (E2007), bukan hanya duplikat
    // sesama interface.
    let mut mod_names: HashSet<&str> = HashSet::new();
    for m in &file.modules {
        mod_names.insert(m.name.as_str());
    }
    for p in &file.programs {
        mod_names.insert(p.name.as_str());
    }
    let mut ifc_seen = HashSet::new();
    for i in &file.interfaces {
        if !ifc_seen.insert(i.name.as_str()) {
            return Err(err_at(
                i.line,
                i.col,
                "E2007",
                format!("interface '{}' dideklarasikan dua kali", i.name),
            ));
        }
        if mod_names.contains(i.name.as_str()) {
            return Err(err_at(
                i.line,
                i.col,
                "E2007",
                format!(
                    "interface '{}' bentrok dengan module/program bernama sama",
                    i.name
                ),
            ));
        }
        check_interface(i, &ctx)?;
    }

    let mut pkg_seen = HashSet::new();
    for p in &file.packages {
        if !pkg_seen.insert(p.name.as_str()) {
            return Err(err_at(
                p.line,
                p.col,
                "E2007",
                format!("package '{}' dideklarasikan dua kali", p.name),
            ));
        }
        let mut td_seen = HashSet::new();
        let mut c_seen = HashSet::new();
        for td in &p.typedefs {
            let n = td_name(td);
            if !td_seen.insert(n) {
                let (l, c) = td_pos(td);
                return Err(err_at(
                    l,
                    c,
                    "E2007",
                    format!("tipe '{n}' dideklarasikan dua kali di package '{}'", p.name),
                ));
            }
            check_typedef(td, &ctx)?;
        }
        for (cn, _, _) in &p.consts {
            if !c_seen.insert(cn.as_str()) {
                return Err(err_at(
                    p.line,
                    p.col,
                    "E2007",
                    format!(
                        "konstanta '{cn}' dideklarasikan dua kali di package '{}'",
                        p.name
                    ),
                ));
            }
        }
    }

    // ── module ──
    // Duplikat nama module (E2007, F11): module & program berbagi namespace
    // SV — dua nama sama dalam satu file = error. Posisi memakai line/col
    // nama deklarasi (sebelumnya tidak dicek sama sekali).
    let mut mod_seen = HashSet::new();
    for m in &file.modules {
        if !mod_seen.insert(m.name.as_str()) {
            return Err(err_at(
                m.line,
                m.col,
                "E2007",
                format!("module '{}' dideklarasikan dua kali", m.name),
            ));
        }
        check_module(m, &ctx)?;
    }
    // `program` (MARIA-HDL.md §7.3) — body testbench tetap di-type-check
    // (E2001–E2007). Program memakai struktur Module yang sama.
    for p in &file.programs {
        if !mod_seen.insert(p.name.as_str()) {
            return Err(err_at(
                p.line,
                p.col,
                "E2007",
                format!("module/program '{}' dideklarasikan dua kali", p.name),
            ));
        }
        check_module(p, &ctx)?;
    }

    // ── class (MARIA-HDL.md §8) ──
    let mut cls_seen = HashSet::new();
    for c in &file.classes {
        if !cls_seen.insert(c.name.as_str()) {
            return Err(err_at(
                c.line,
                c.col,
                "E2007",
                format!("class '{}' dideklarasikan dua kali", c.name),
            ));
        }
        check_class(c, &ctx)?;
    }

    // ── function/task level file ──
    for f in &file.funcs {
        let mut scope = new_scope(&ctx, &f.name);
        check_func(f, &ctx, &mut scope)?;
    }
    for t in &file.tasks {
        let mut scope = new_scope(&ctx, &t.name);
        check_task(t, &ctx, &mut scope)?;
    }
    Ok(())
}

// ── Indeks ──

/// Kumpulkan indeks global dari satu atau banyak file. `or_insert` → nama
/// yang duplikat antar-file memakai definisi pertama (urutan deterministik).
fn collect_ctx<'a>(files: impl IntoIterator<Item = &'a MvFile>) -> Ctx<'a> {
    let mut types = HashMap::new();
    let mut packages = HashMap::new();
    let mut enum_members = HashMap::new();
    let mut funcs = HashSet::new();
    let mut tasks = HashSet::new();
    let mut consts = HashSet::new();
    let mut classes = HashSet::new();
    let mut interfaces = HashSet::new();
    let mut modules = HashSet::new();
    let mut module_ports = HashMap::new();
    let mut module_params = HashMap::new();
    let mut module_type_params = HashMap::new();
    for file in files {
        for td in &file.typedefs {
            types.entry(td_name(td)).or_insert(td);
            collect_enum_members(td, &mut enum_members);
        }
        for i in &file.interfaces {
            interfaces.insert(i.name.as_str());
            module_ports.insert(
                i.name.as_str(),
                i.ports.iter().map(|p| p.names.clone()).flatten().collect(),
            );
        }
        for m in &file.modules {
            modules.insert(m.name.as_str());
            module_ports.insert(
                m.name.as_str(),
                m.items
                    .iter()
                    .filter_map(|it| match it {
                        MItem::Port(p) => Some(p.names.clone()),
                        _ => None,
                    })
                    .flatten()
                    .collect(),
            );
            module_params.insert(
                m.name.as_str(),
                m.params.iter().map(|p| p.name.clone()).collect(),
            );
            module_type_params.insert(
                m.name.as_str(),
                m.params
                    .iter()
                    .filter(|p| {
                        p.type_default.is_some()
                            || matches!(&p.ty, Some(MvType::Named(s, ..)) if s == "type")
                    })
                    .map(|p| p.name.clone())
                    .collect(),
            );
        }
        for prg in &file.programs {
            modules.insert(prg.name.as_str());
            module_ports.insert(
                prg.name.as_str(),
                prg.items
                    .iter()
                    .filter_map(|it| match it {
                        MItem::Port(p) => Some(p.names.clone()),
                        _ => None,
                    })
                    .flatten()
                    .collect(),
            );
            module_params.insert(
                prg.name.as_str(),
                prg.params.iter().map(|p| p.name.clone()).collect(),
            );
            module_type_params.insert(
                prg.name.as_str(),
                prg.params
                    .iter()
                    .filter(|p| {
                        p.type_default.is_some()
                            || matches!(&p.ty, Some(MvType::Named(s, ..)) if s == "type")
                    })
                    .map(|p| p.name.clone())
                    .collect(),
            );
        }
        for p in &file.packages {
            packages.entry(p.name.as_str()).or_insert(p);
            for td in &p.typedefs {
                types.entry(td_name(td)).or_insert(td);
                collect_enum_members(td, &mut enum_members);
            }
        }
        for p in &file.packages {
            for (cn, _, _) in &p.consts {
                consts.insert(cn.as_str());
            }
        }
        for f in &file.funcs {
            funcs.insert(f.name.as_str());
        }
        for t in &file.tasks {
            tasks.insert(t.name.as_str());
        }
        for c in &file.classes {
            classes.insert(c.name.as_str());
        }
    }
    Ctx {
        types,
        packages,
        enum_members,
        consts,
        funcs,
        tasks,
        classes,
        interfaces,
        modules,
        module_ports,
        module_params,
        module_type_params,
    }
}

fn build_ctx<'a>(file: &'a MvFile) -> Ctx<'a> {
    collect_ctx(std::iter::once(file))
}

fn build_ctx_many<'a>(files: &'a [&'a MvFile]) -> Ctx<'a> {
    collect_ctx(files.iter().copied())
}

fn td_name(td: &Typedef) -> &str {
    match td {
        Typedef::Alias { name, .. } => name,
        Typedef::Struct { name, .. } => name,
        Typedef::Union { name, .. } => name,
        Typedef::Enum { name, .. } => name,
    }
}

fn collect_enum_members<'a>(td: &'a Typedef, out: &mut HashMap<&'a str, i64>) {
    if let Typedef::Enum { width, members, .. } = td {
        let w = match width {
            Some(Expr::Int(n)) => *n,
            _ => enum_bits(members.len()),
        };
        for m in members {
            out.entry(m.name.as_str()).or_insert(w);
        }
    }
}

/// Lebar enum implisit: clog2(n), minimal 1 (sinkron dengan codegen).
fn enum_bits(n: usize) -> i64 {
    if n <= 2 {
        1
    } else {
        ((n - 1) as f64).log2().ceil() as i64
    }
}

fn resolve_typedef<'a>(name: &str, ctx: &'a Ctx<'a>) -> Option<&'a Typedef> {
    if let Some((pkg, item)) = name.split_once("::") {
        let p = ctx.packages.get(pkg)?;
        p.typedefs.iter().find(|td| td_name(td) == item)
    } else {
        ctx.types.get(name).copied()
    }
}

// ── Typedef ──

fn check_typedef<'a>(td: &'a Typedef, ctx: &'a Ctx<'a>) -> Result<(), MvError> {
    match td {
        Typedef::Alias { ty, .. } => check_type(ty, ctx, 0),
        Typedef::Struct { name, fields, .. } | Typedef::Union { name, fields, .. } => {
            let mut fseen = HashSet::new();
            for f in fields {
                for n in &f.names {
                    if !fseen.insert(n.as_str()) {
                        return Err(err_at(
                            f.line,
                            f.col,
                            "E2007",
                            format!("field '{n}' dideklarasikan dua kali di struct/union '{name}'"),
                        ));
                    }
                }
                check_type(&f.ty, ctx, 0)?;
            }
            Ok(())
        }
        Typedef::Enum {
            name,
            width,
            members,
            ..
        } => {
            let scope = new_scope(ctx, name);
            if let Some(w) = width {
                check_expr(w, ctx, &scope, 0)?;
            }
            let mut mseen = HashSet::new();
            for m in members {
                if !mseen.insert(m.name.as_str()) {
                    return Err(err_at(
                        m.line,
                        m.col,
                        "E2007",
                        format!(
                            "member '{}' dideklarasikan dua kali di enum '{name}'",
                            m.name
                        ),
                    ));
                }
                if let Some(v) = &m.value {
                    check_expr(v, ctx, &scope, 0)?;
                }
            }
            Ok(())
        }
    }
}

// ── Interface (F26) ──

/// Validasi interface: duplikat signal (port+sig), tipe dikenal (E2005),
/// dan setiap modport hanya merujuk signal yang ada (E2001).
fn check_interface<'a>(i: &'a Interface, ctx: &'a Ctx<'a>) -> Result<(), MvError> {
    let mut sig_names: HashSet<&'a str> = HashSet::new();
    let mut port_names: HashSet<&'a str> = HashSet::new();

    // port + sig sama-sama signal interface (lebar/nama namespace sama)
    for p in &i.ports {
        check_type(&p.ty, ctx, 0)?;
        for n in &p.names {
            if !port_names.insert(n.as_str()) {
                return Err(err_at(
                    p.line,
                    p.col,
                    "E2007",
                    format!(
                        "port '{}' dideklarasikan dua kali di interface '{}'",
                        n, i.name
                    ),
                ));
            }
            sig_names.insert(n.as_str());
        }
    }
    for (names, ty, line, col) in &i.sigs {
        check_type(ty, ctx, 0)?;
        for n in names {
            if !sig_names.insert(n.as_str()) {
                return Err(err_at(
                    *line,
                    *col,
                    "E2007",
                    format!(
                        "signal '{}' dideklarasikan dua kali di interface '{}'",
                        n, i.name
                    ),
                ));
            }
        }
    }

    // modport: nama unik + hanya merujuk signal yang dideklarasikan
    let mut mp_seen = HashSet::new();
    for mp in &i.modports {
        if !mp_seen.insert(mp.name.as_str()) {
            return Err(err_at(
                mp.line,
                mp.col,
                "E2007",
                format!(
                    "modport '{}' dideklarasikan dua kali di interface '{}'",
                    mp.name, i.name
                ),
            ));
        }
        for (_, names) in &mp.dirs {
            for n in names {
                if !sig_names.contains(n.as_str()) {
                    return Err(err_at(
                        mp.line,
                        mp.col,
                        "E2001",
                        format!(
                            "modport '{}' merujuk signal '{}' yang tidak ada di interface '{}'",
                            mp.name, n, i.name
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

// ── Module ──

fn check_module<'a>(m: &'a Module, ctx: &'a Ctx<'a>) -> Result<(), MvError> {
    let mut port_names: HashSet<&'a str> = HashSet::new();
    let mut decl_names: HashSet<&'a str> = HashSet::new();
    let mut scope = new_scope(ctx, &m.name);

    // ── parameter: duplikat (E2007) + fold nilai + validasi tipe ──
    for p in &m.params {
        if scope.params.contains_key(p.name.as_str()) {
            return Err(err_at(
                p.line,
                p.col,
                "E2007",
                format!(
                    "parameter '{}' dideklarasikan dua kali di module '{}'",
                    p.name, m.name
                ),
            ));
        }
        // F32: type param — marker `Named("type")` (`T : type = ...`) ATAU
        // bentuk kata kunci `type T = ...` (ty=None, type_default terisi).
        // Marker bukan tipe nyata: skip check_type(marker), validasi
        // type_default, daftarkan ke scope.type_params agar `sig x : T` sah.
        let is_tp =
            p.type_default.is_some() || matches!(&p.ty, Some(MvType::Named(s, ..)) if s == "type");
        if is_tp {
            if let Some(td) = &p.type_default {
                check_type(td, ctx, 0)?;
                scope.type_params.insert(p.name.as_str(), Some(td));
            } else {
                scope.type_params.insert(p.name.as_str(), None);
            }
        } else {
            if let Some(t) = &p.ty {
                check_type(t, ctx, 0)?;
            }
            if let Some(v) = p
                .default
                .as_ref()
                .and_then(|e| fold_const(e, &scope.params, 0))
            {
                scope.params.insert(p.name.as_str(), v);
            }
        }
    }

    // nama function/task module → terlihat sebagai ident
    for item in &m.items {
        match item {
            MItem::Func(f) => {
                scope.funcs.insert(f.name.as_str());
            }
            MItem::Task(t) => {
                scope.funcs.insert(t.name.as_str());
            }
            _ => {}
        }
    }

    // ── pass 1: kumpulkan deklarasi, deteksi duplikat (E2007), validasi tipe (E2005) ──
    for item in &m.items {
        match item {
            MItem::Port(p) => {
                let iface_ty = is_iface_type(&p.ty, ctx);
                for n in &p.names {
                    if !port_names.insert(n.as_str()) {
                        return Err(err_at(
                            p.line,
                            p.col,
                            "E2007",
                            format!("port '{n}' dideklarasikan dua kali di module '{}'", m.name),
                        ));
                    }
                    // F26: port interface tidak punya arah drive (field-nya
                    // diatur via modport) — jangan daftar ke env.ports agar
                    // E2003 tidak terpicu saat drive `axi_if.awready`.
                    if !iface_ty {
                        scope.env.ports.insert(n.as_str(), p.dir);
                    }
                }
                check_type_scope(&p.ty, ctx, Some(&scope), 0)?;
                for n in &p.names {
                    scope.sigs.insert(n.as_str());
                    scope.types.insert(n.as_str(), &p.ty);
                }
            }
            MItem::Sig {
                names,
                ty,
                line,
                col,
                ..
            }
            | MItem::Reg {
                names,
                ty,
                line,
                col,
                ..
            } => {
                check_type_scope(ty, ctx, Some(&scope), 0)?;
                for n in names {
                    if !decl_names.insert(n.as_str()) {
                        return Err(err_at(
                            *line,
                            *col,
                            "E2007",
                            format!(
                                "sinyal '{n}' dideklarasikan dua kali di module '{}'",
                                m.name
                            ),
                        ));
                    }
                    if scope.params.contains_key(n.as_str()) {
                        return Err(err_at(
                            *line,
                            *col,
                            "E2007",
                            format!(
                                "sinyal '{n}' bentrok dengan parameter di module '{}'",
                                m.name
                            ),
                        ));
                    }
                }
                for n in names {
                    scope.sigs.insert(n.as_str());
                    scope.types.insert(n.as_str(), ty);
                }
            }
            MItem::Const {
                name,
                ty,
                line,
                col,
                ..
            } => {
                if let Some(t) = ty {
                    check_type_scope(t, ctx, Some(&scope), 0)?;
                }
                if !decl_names.insert(name.as_str()) {
                    return Err(err_at(
                        *line,
                        *col,
                        "E2007",
                        format!(
                            "konstanta '{name}' dideklarasikan dua kali di module '{}'",
                            m.name
                        ),
                    ));
                }
                if scope.params.contains_key(name.as_str()) {
                    return Err(err_at(
                        *line,
                        *col,
                        "E2007",
                        format!(
                            "konstanta '{name}' bentrok dengan parameter di module '{}'",
                            m.name
                        ),
                    ));
                }
                scope.sigs.insert(name.as_str());
            }
            MItem::Initial(_) | MItem::Final(_) => {}
            _ => {}
        }
    }

    // ── pass 2: statement (seq/comb/always/latch/initial/final/inst/gen/…) ──
    for item in &m.items {
        check_module_item(item, ctx, &mut scope)?;
    }
    Ok(())
}

/// True bila tipe adalah nama interface (F26) — port bertipe interface
/// di-emit tanpa arah dan field-nya bebas di-drive (sesuai modport).
fn is_iface_type(ty: &MvType, ctx: &Ctx) -> bool {
    matches!(ty, MvType::Named(n, ..) if ctx.interfaces.contains(n.as_str()))
}

fn new_scope<'a>(ctx: &'a Ctx<'a>, mname: &'a str) -> Scope<'a> {
    let mut funcs: HashSet<&'a str> = ctx.funcs.clone();
    funcs.extend(ctx.tasks.iter().copied());
    Scope {
        sigs: HashSet::new(),
        funcs,
        params: HashMap::new(),
        type_params: HashMap::new(),
        types: HashMap::new(),
        enum_members: &ctx.enum_members,
        consts: &ctx.consts,
        env: Env {
            mname,
            ports: HashMap::new(),
        },
        loop_depth: 0,
        in_task: false,
    }
}

/// Item module (termasuk di dalam blok generate).
fn check_module_item<'a>(
    item: &'a MItem,
    ctx: &'a Ctx<'a>,
    scope: &mut Scope<'a>,
) -> Result<(), MvError> {
    match item {
        MItem::Port(_) => Ok(()), // port tidak valid di dalam generate — abaikan
        MItem::Sig {
            names, ty, init, ..
        }
        | MItem::Reg {
            names, ty, init, ..
        } => {
            check_type_scope(ty, ctx, Some(scope), 0)?;
            if let Some(i) = init {
                check_expr(i, ctx, scope, 0)?;
            }
            for n in names {
                scope.sigs.insert(n.as_str());
                scope.types.insert(n.as_str(), ty);
            }
            Ok(())
        }
        MItem::Const {
            name, ty, value, ..
        } => {
            if let Some(t) = ty {
                check_type_scope(t, ctx, Some(scope), 0)?;
            }
            check_expr(value, ctx, scope, 0)?;
            scope.sigs.insert(name.as_str());
            Ok(())
        }
        MItem::Use { .. } => Ok(()),
        MItem::Seq(spec, body) => {
            // F26: clock bisa `iface.clk` — validasi base ident-nya saja
            // (field interface tidak di-verifikasi per-nama di sini).
            let clk_base = spec.clk.split('.').next().unwrap_or(&spec.clk);
            if !scope.known(clk_base) {
                return Err(err_at(
                    spec.line,
                    spec.col,
                    "E2001",
                    format!(
                        "undefined signal '{}' (clock seq) — di module '{}'",
                        spec.clk, scope.env.mname
                    ),
                ));
            }
            if let Some((rname, _, _)) = &spec.reset {
                if !scope.known(rname) {
                    return Err(err_at(
                        spec.line,
                        spec.col,
                        "E2001",
                        format!(
                            "undefined signal '{rname}' (reset seq) — di module '{}'",
                            scope.env.mname
                        ),
                    ));
                }
            }
            check_stmt(body, ctx, scope, BlockKind::Seq)
        }
        MItem::Comb(body) => check_stmt(body, ctx, scope, BlockKind::Always),
        MItem::Always(body) => check_stmt(body, ctx, scope, BlockKind::Always),
        MItem::Latch(body) => check_stmt(body, ctx, scope, BlockKind::Always),
        MItem::Initial(body) => check_stmt(body, ctx, scope, BlockKind::Tb),
        MItem::Final(body) => check_stmt(body, ctx, scope, BlockKind::Tb),
        MItem::Inst {
            module,
            name,
            dims,
            params,
            conns,
            line,
            col,
        } => {
            if let Some(d) = dims {
                check_expr(d, ctx, scope, 0)?;
            }
            // F31: validasi override parameter `#(.NAME(expr))` BILA target
            // dikenali: nama parameter harus ada (E2001) + tidak boleh
            // di-override dua kali (E2007). Nama eksternal dilewati.
            // Catatan: interface `.mv` belum punya params (ast Interface tanpa
            // field params) — target interface dgn override → E2001 benar
            // (override param pada interface = memang tidak valid di .mv).
            let target_known =
                ctx.modules.contains(module.as_str()) || ctx.interfaces.contains(module.as_str());
            let kind = if ctx.interfaces.contains(module.as_str()) {
                "interface"
            } else {
                "module"
            };
            let pnames = ctx
                .module_params
                .get(module.as_str())
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let tpnames = ctx
                .module_type_params
                .get(module.as_str())
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let mut seen_params: HashSet<&str> = HashSet::new();
            for (pn, e) in params {
                // F32: override TYPE param (`.T(Word16)`) — nilai adalah TIPE
                // (nama tipe yang dikenal), bukan ekspresi nilai. Dikenali
                // saat: (a) target dikenal & nama param adalah type param;
                // (b) target eksternal & nilainya berbentuk tipe (konservatif
                // lintas-file — kita tidak tahu daftar type param file lain).
                let tp_known = target_known && tpnames.iter().any(|s| s == pn);
                let tp_like = !target_known && is_type_like(e, ctx);
                if tp_known || tp_like {
                    match e {
                        Expr::Ident(tn, _, _) => {
                            let t = MvType::Named(tn.clone(), 0, 0);
                            check_type_scope(&t, ctx, Some(scope), 0)?;
                        }
                        // F32 fix review: override type param dgn tipe scoped
                        // `#(.T(chip_types::Word16))` — Expr::Scoped valid sbg
                        // tipe (`pkg::item`), bukan hanya Expr::Ident polos.
                        Expr::Scoped(pkg, item, ..) => {
                            let t = MvType::Named(format!("{}::{}", pkg, item), 0, 0);
                            check_type_scope(&t, ctx, Some(scope), 0)?;
                        }
                        other => {
                            if tp_known {
                                return Err(err_at(
                                    *line,
                                    *col,
                                    "E2005",
                                    format!(
                                        "nilai override type param '{}' harus nama tipe (dapatkan: {:?})",
                                        pn, other
                                    ),
                                ));
                            }
                            check_expr(e, ctx, scope, 0)?;
                        }
                    }
                    if !seen_params.insert(pn.as_str()) {
                        return Err(err_at(
                            *line,
                            *col,
                            "E2007",
                            format!(
                                "parameter '{}' di-override dua kali di {} '{}'",
                                pn, kind, module
                            ),
                        ));
                    }
                    continue;
                }
                check_expr(e, ctx, scope, 0)?;
                if target_known && !pnames.iter().any(|p| p == pn) {
                    return Err(err_at(
                        *line,
                        *col,
                        "E2001",
                        format!("parameter '{}' tidak ada di {} '{}'", pn, kind, module),
                    ));
                }
                if !seen_params.insert(pn.as_str()) {
                    return Err(err_at(
                        *line,
                        *col,
                        "E2007",
                        format!(
                            "parameter '{}' di-override dua kali di {} '{}'",
                            pn, kind, module
                        ),
                    ));
                }
            }
            // F29: validasi koneksi port BILA target dikenali (module/interface/
            // program di file ini/gabungan): nama port harus ada + jumlah
            // positional tak boleh melebihi port. Nama eksternal (file .mv
            // lain) dilewati — pola konservatif E2001/E2005.
            let ports = ctx
                .module_ports
                .get(module.as_str())
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let mut pos_count = 0usize;
            let mut seen_ports: HashSet<&str> = HashSet::new();
            for c in conns {
                match c {
                    Conn::Named { port, expr } => {
                        if let Some(e) = expr {
                            check_expr(e, ctx, scope, 0)?;
                        }
                        if target_known && !ports.iter().any(|p| p == port) {
                            return Err(err_at(
                                *line,
                                *col,
                                "E2001",
                                format!("port '{}' tidak ada di {} '{}'", port, kind, module),
                            ));
                        }
                        // F29 fix review: port dikoneksikan dua kali (`.a(x), .a(y)`)
                        // → SV ganda; error E2007 (duplicate connection).
                        if !seen_ports.insert(port.as_str()) {
                            return Err(err_at(
                                *line,
                                *col,
                                "E2007",
                                format!(
                                    "port '{}' dikoneksikan dua kali di {} '{}'",
                                    port, kind, module
                                ),
                            ));
                        }
                    }
                    Conn::Positional(e) => {
                        pos_count += 1;
                        check_expr(e, ctx, scope, 0)?;
                    }
                }
            }
            if target_known && pos_count > ports.len() {
                return Err(err_at(
                    *line,
                    *col,
                    "E2001",
                    format!(
                        "terlalu banyak koneksi positional: {} koneksi untuk {} port di {} '{}'",
                        pos_count,
                        ports.len(),
                        kind,
                        module
                    ),
                ));
            }
            // nama instance bisa direferensikan (mis. `u_mem.data`)
            scope.sigs.insert(name.as_str());
            Ok(())
        }
        MItem::GenFor {
            var,
            from,
            to,
            body,
        } => {
            check_expr(from, ctx, scope, 0)?;
            check_expr(to, ctx, scope, 0)?;
            let mut inner = scope.clone();
            inner.sigs.insert(var.as_str());
            for it in body {
                check_module_item(it, ctx, &mut inner)?;
            }
            Ok(())
        }
        MItem::GenIf { cond, then, els } => {
            check_expr(cond, ctx, scope, 0)?;
            let mut inner = scope.clone();
            for it in then {
                check_module_item(it, ctx, &mut inner)?;
            }
            let mut inner2 = scope.clone();
            for it in els {
                check_module_item(it, ctx, &mut inner2)?;
            }
            Ok(())
        }
        MItem::Func(f) => check_func(f, ctx, scope),
        MItem::Task(t) => check_task(t, ctx, scope),
    }
}

// ── Function / Task ──

fn check_func<'a>(f: &'a MFunc, ctx: &'a Ctx<'a>, base: &mut Scope<'a>) -> Result<(), MvError> {
    let mut scope = base.clone();
    for (n, t, _) in &f.args {
        check_type_scope(t, ctx, Some(&scope), 0)?;
        scope.sigs.insert(n.as_str());
        scope.types.insert(n.as_str(), t);
    }
    if let Some(ret) = &f.ret {
        check_type_scope(ret, ctx, Some(&scope), 0)?;
    }
    for s in &f.body {
        check_stmt(s, ctx, &mut scope, BlockKind::Always)?;
    }
    Ok(())
}

fn check_task<'a>(t: &'a MTask, ctx: &'a Ctx<'a>, base: &mut Scope<'a>) -> Result<(), MvError> {
    let mut scope = base.clone();
    scope.in_task = true;
    for (n, ty, _) in &t.args {
        check_type_scope(ty, ctx, Some(&scope), 0)?;
        scope.sigs.insert(n.as_str());
        scope.types.insert(n.as_str(), ty);
    }
    for s in &t.body {
        check_stmt(s, ctx, &mut scope, BlockKind::Always)?;
    }
    Ok(())
}

/// Class (MARIA-HDL.md §8): fields terlihat di scope method/constraint;
/// `this`/`super`/`rand` adalah kata kunci konteks — bukan sinyal.
fn check_class<'a>(c: &'a MClass, ctx: &'a Ctx<'a>) -> Result<(), MvError> {
    let mut scope = new_scope(ctx, &c.name);

    // ── field: duplikat (E2007) + tipe (E2005) ──
    let mut fseen = HashSet::new();
    for (n, ty, _) in &c.fields {
        if !fseen.insert(n.as_str()) {
            return Err(err_at(
                c.line,
                c.col,
                "E2007",
                format!("field '{n}' dideklarasikan dua kali di class '{}'", c.name),
            ));
        }
        check_type(ty, ctx, 0)?;
        scope.sigs.insert(n.as_str());
        scope.types.insert(n.as_str(), ty);
    }

    // ── method: duplikat nama (E2007) ──
    let mut mseen = HashSet::new();
    for f in &c.funcs {
        if !mseen.insert(f.name.as_str()) {
            return Err(err_at(
                f.line,
                f.col,
                "E2007",
                format!(
                    "method '{}' dideklarasikan dua kali di class '{}'",
                    f.name, c.name
                ),
            ));
        }
    }
    for t in &c.tasks {
        if !mseen.insert(t.name.as_str()) {
            return Err(err_at(
                t.line,
                t.col,
                "E2007",
                format!(
                    "method '{}' dideklarasikan dua kali di class '{}'",
                    t.name, c.name
                ),
            ));
        }
    }

    // ── constraint: duplikat (E2007) + item divalidasi (F12: if/solve/expr) ──
    let mut cseen = HashSet::new();
    for (cname, items) in &c.constraints {
        if !cseen.insert(cname.as_str()) {
            return Err(err_at(
                c.line,
                c.col,
                "E2007",
                format!(
                    "constraint '{cname}' dideklarasikan dua kali di class '{}'",
                    c.name
                ),
            ));
        }
        check_constraint_items(items, ctx, &scope)?;
    }

    for f in &c.funcs {
        check_func(f, ctx, &mut scope)?;
    }
    for t in &c.tasks {
        check_task(t, ctx, &mut scope)?;
    }
    Ok(())
}

// ── Constraint items (F12) ──

/// Validasi item constraint: ekspresi (termasuk inside/dist), if/else
/// (rekursif ke cabang), dan `solve var before a, b` (var harus dikenal).
fn check_constraint_items<'a>(
    items: &'a [ConstraintItem],
    ctx: &'a Ctx<'a>,
    scope: &Scope<'a>,
) -> Result<(), MvError> {
    for item in items {
        match item {
            ConstraintItem::Expr(e) => check_expr(e, ctx, scope, 0)?,
            ConstraintItem::If { cond, then, els } => {
                check_expr(cond, ctx, scope, 0)?;
                check_constraint_items(then, ctx, scope)?;
                check_constraint_items(els, ctx, scope)?;
            }
            ConstraintItem::Solve {
                var,
                before,
                line,
                col,
            } => {
                if !scope.known(var) {
                    return Err(err_at(
                        *line,
                        *col,
                        "E2001",
                        format!(
                            "undefined signal '{var}' (solve) — di '{}'",
                            scope.env.mname
                        ),
                    ));
                }
                for b in before {
                    if !scope.known(b) {
                        return Err(err_at(
                            *line,
                            *col,
                            "E2001",
                            format!(
                                "undefined signal '{b}' (solve before) — di '{}'",
                                scope.env.mname
                            ),
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

// ── Statements ──

fn check_stmt<'a>(
    stmt: &'a Stmt,
    ctx: &'a Ctx<'a>,
    scope: &mut Scope<'a>,
    kind: BlockKind,
) -> Result<(), MvError> {
    match stmt {
        Stmt::Block(stmts) => {
            for s in stmts {
                check_stmt(s, ctx, scope, kind)?;
            }
            Ok(())
        }
        Stmt::Assign {
            lhs,
            rhs,
            nba,
            line,
            col,
        } => {
            // E2004: operator assignment di konteks salah
            if kind == BlockKind::Seq && !*nba {
                return Err(err_at(
                    *line,
                    *col,
                    "E2004",
                    format!(
                        "blocking assign '=' tidak boleh di dalam seq (pakai '<=') — di '{}'",
                        scope.env.mname
                    ),
                ));
            }
            if kind != BlockKind::Seq && *nba {
                return Err(err_at(
                    *line,
                    *col,
                    "E2004",
                    format!(
                        "non-blocking assign '<=' hanya boleh di dalam seq — di '{}'",
                        scope.env.mname
                    ),
                ));
            }
            // E2003: drive port input hanya diizinkan di body `initial`/`final`
            // (testbench). Di seq/comb/always/latch/func/task → error.
            if kind != BlockKind::Tb {
                if let Some(base) = base_ident(lhs) {
                    if let Some(Dir::In) = scope.env.ports.get(base) {
                        return Err(err_at(
                            *line,
                            *col,
                            "E2003",
                            format!(
                                "cannot drive input port '{base}' — di '{}'",
                                scope.env.mname
                            ),
                        ));
                    }
                }
            }
            check_expr(lhs, ctx, scope, 0)?;
            check_expr(rhs, ctx, scope, 0)?;
            // E2002: RHS lebih lebar dari LHS → truncation
            let wl = expr_width(lhs, ctx, scope, 0);
            let wr = expr_width(rhs, ctx, scope, 0);
            if let (Some(l), Some(r)) = (wl, wr) {
                if r > l {
                    return Err(err_at(
                        *line,
                        *col,
                        "E2002",
                        format!(
                            "lebar {r} bit ke sinyal {l}-bit '{}' — di '{}'",
                            describe_lhs(lhs),
                            scope.env.mname
                        ),
                    ));
                }
            }
            Ok(())
        }
        // F36: `lhs += rhs` — compound assignment (blocking, seperti `=`).
        Stmt::CompoundAssign {
            lhs,
            op,
            rhs,
            line,
            col,
        } => {
            if kind == BlockKind::Seq {
                return Err(err_at(
                    *line,
                    *col,
                    "E2004",
                    format!(
                        "compound assign '{op}' tidak boleh di dalam seq (pakai '<=') — di '{}'",
                        scope.env.mname
                    ),
                ));
            }
            if kind != BlockKind::Tb {
                if let Some(base) = base_ident(lhs) {
                    if let Some(Dir::In) = scope.env.ports.get(base) {
                        return Err(err_at(
                            *line,
                            *col,
                            "E2003",
                            format!(
                                "cannot drive input port '{base}' — di '{}'",
                                scope.env.mname
                            ),
                        ));
                    }
                }
            }
            check_expr(lhs, ctx, scope, 0)?;
            check_expr(rhs, ctx, scope, 0)?;
            // E2002: RHS lebih lebar dari LHS → truncation
            let wl = expr_width(lhs, ctx, scope, 0);
            let wr = expr_width(rhs, ctx, scope, 0);
            if let (Some(l), Some(r)) = (wl, wr) {
                if r > l {
                    return Err(err_at(
                        *line,
                        *col,
                        "E2002",
                        format!(
                            "lebar {r} bit ke sinyal {l}-bit '{}' — di '{}'",
                            describe_lhs(lhs),
                            scope.env.mname
                        ),
                    ));
                }
            }
            Ok(())
        }
        // F36: `lhs++` / `lhs--` — increment (blocking, seperti `=`).
        Stmt::IncDec { lhs, line, col, .. } => {
            if kind == BlockKind::Seq {
                return Err(err_at(
                    *line,
                    *col,
                    "E2004",
                    format!(
                        "increment/decrement tidak boleh di dalam seq (pakai '<=') — di '{}'",
                        scope.env.mname
                    ),
                ));
            }
            if kind != BlockKind::Tb {
                if let Some(base) = base_ident(lhs) {
                    if let Some(Dir::In) = scope.env.ports.get(base) {
                        return Err(err_at(
                            *line,
                            *col,
                            "E2003",
                            format!(
                                "cannot drive input port '{base}' — di '{}'",
                                scope.env.mname
                            ),
                        ));
                    }
                }
            }
            check_expr(lhs, ctx, scope, 0)?;
            Ok(())
        }
        Stmt::If { cond, then, els } => {
            check_expr(cond, ctx, scope, 0)?;
            check_stmt(then, ctx, scope, kind)?;
            if let Some(e) = els {
                check_stmt(e, ctx, scope, kind)?;
            }
            Ok(())
        }
        Stmt::Case {
            expr,
            items,
            default,
            qual: _,
            kind: _,
        } => {
            check_expr(expr, ctx, scope, 0)?;
            for (vals, body) in items {
                for v in vals {
                    check_expr(v, ctx, scope, 0)?;
                }
                check_stmt(body, ctx, scope, kind)?;
            }
            if let Some(d) = default {
                check_stmt(d, ctx, scope, kind)?;
            }
            Ok(())
        }
        Stmt::For {
            var,
            from,
            to,
            body,
        } => {
            check_expr(from, ctx, scope, 0)?;
            check_expr(to, ctx, scope, 0)?;
            let mut inner = scope.clone();
            inner.sigs.insert(var.as_str());
            inner.loop_depth += 1;
            check_stmt(body, ctx, &mut inner, kind)
        }
        Stmt::While { cond, body } => {
            check_expr(cond, ctx, scope, 0)?;
            let mut inner = scope.clone();
            inner.loop_depth += 1;
            check_stmt(body, ctx, &mut inner, kind)
        }
        // F38: `do { body } while (cond)` — post-test, validasi body+cond biasa.
        Stmt::DoWhile { cond, body } => {
            let mut inner = scope.clone();
            inner.loop_depth += 1;
            check_stmt(body, ctx, &mut inner, kind)?;
            check_expr(cond, ctx, scope, 0)
        }
        // F38: event trigger `->ev` — target harus signal/event yang dikenal.
        Stmt::EventTrigger(ev) => {
            // Event bisa berupa signal biasa (`->ev`) — E2001 sudah ditangani
            // check_expr (Ident tak dikenal).
            check_expr(ev, ctx, scope, 0)
        }
        // F39: fork/join — tiap branch divalidasi di scope yang sama (konteks
        // blok sama dengan induknya; E2004 seq dst. diterapkan per branch).
        Stmt::Fork { branches, .. } => {
            for b in branches {
                check_stmt(b, ctx, scope, kind)?;
            }
            Ok(())
        }
        Stmt::Repeat { count, body } => {
            check_expr(count, ctx, scope, 0)?;
            let mut inner = scope.clone();
            inner.loop_depth += 1;
            check_stmt(body, ctx, &mut inner, kind)
        }
        Stmt::Forever(body) => {
            let mut inner = scope.clone();
            inner.loop_depth += 1;
            check_stmt(body, ctx, &mut inner, kind)
        }
        Stmt::Wait { cond, body } => {
            check_expr(cond, ctx, scope, 0)?;
            check_stmt(body, ctx, scope, kind)
        }
        Stmt::Event { expr, body } => {
            check_expr(expr, ctx, scope, 0)?;
            check_stmt(body, ctx, scope, kind)
        }
        Stmt::Delay { amt, body } => {
            check_expr(amt, ctx, scope, 0)?;
            check_stmt(body, ctx, scope, kind)
        }
        Stmt::ExprStmt(e) => check_expr(e, ctx, scope, 0),
        Stmt::VarDecl { names, ty, init } => {
            check_type_scope(ty, ctx, Some(scope), 0)?;
            if let Some(i) = init {
                check_expr(i, ctx, scope, 0)?;
            }
            for n in names {
                if scope.sigs.contains(n.as_str()) {
                    return Err(MvError::new(
                        0,
                        0,
                        format!("E2007: variabel '{}' sudah dideklarasikan", n),
                    ));
                }
                scope.sigs.insert(n.as_str());
                scope.types.insert(n.as_str(), ty);
            }
            Ok(())
        }
        Stmt::Return(v) => {
            if let Some(v) = v {
                if scope.in_task {
                    return Err(MvError::new(
                        0,
                        0,
                        "E2008: task tidak boleh mengembalikan nilai (return expr)".to_string(),
                    ));
                }
                check_expr(v, ctx, scope, 0)?;
            }
            Ok(())
        }
        Stmt::Break | Stmt::Continue => {
            if scope.loop_depth == 0 {
                return Err(MvError::new(
                    0,
                    0,
                    "E2009: break/continue hanya boleh di dalam loop".to_string(),
                ));
            }
            Ok(())
        }
        Stmt::Assert { cond, pass, fail } => {
            check_expr(cond, ctx, scope, 0)?;
            if let Some(p) = pass {
                check_stmt(p, ctx, scope, kind)?;
            }
            if let Some(f) = fail {
                check_stmt(f, ctx, scope, kind)?;
            }
            Ok(())
        }
        // `assert property (...)` — body RAW (operator SVA `|->`/`##` bukan
        // token .mv), konservatif: isi tidak dianalisis, hanya dilewati.
        Stmt::AssertProperty(_) => Ok(()),
    }
}

fn base_ident(e: &Expr) -> Option<&str> {
    match e {
        Expr::Ident(s, ..) => Some(s),
        Expr::Member(o, _, ..) => base_ident(o),
        Expr::Index(o, _) => base_ident(o),
        Expr::Range(o, _, _) => base_ident(o),
        _ => None,
    }
}

/// F32: ekspresi yang BERBENTUK tipe (bukan nilai) — ident yang merupakan
/// typedef/enum-member/class/interface, atau `pkg::item` scoped. Dipakai utk
/// override type param pada target EKSTERNAL (daftar type param file lain tak
/// diketahui): `.T(Word16)` harus dianggap override tipe, bukan ekspresi nilai.
fn is_type_like(e: &Expr, ctx: &Ctx) -> bool {
    match e {
        Expr::Ident(s, ..) => {
            ctx.types.contains_key(s.as_str())
                || ctx.enum_members.contains_key(s.as_str())
                || ctx.classes.contains(s.as_str())
                || ctx.interfaces.contains(s.as_str())
        }
        Expr::Scoped(..) => true,
        _ => false,
    }
}

fn describe_lhs(e: &Expr) -> String {
    match e {
        Expr::Ident(s, ..) => s.clone(),
        Expr::Member(o, f, ..) => format!("{}.{}", describe_lhs(o), f),
        Expr::Index(o, i) => format!("{}[{}]", describe_lhs(o), describe_lhs(i)),
        Expr::Range(o, a, b) => format!(
            "{}[{}:{}]",
            describe_lhs(o),
            describe_lhs(a),
            describe_lhs(b)
        ),
        other => format!("{other:?}"),
    }
}

// ── Expressions ──

fn check_expr<'a>(
    e: &'a Expr,
    ctx: &'a Ctx<'a>,
    scope: &Scope<'a>,
    depth: usize,
) -> Result<(), MvError> {
    if depth > 24 {
        return Ok(()); // guard rekursi dalam
    }
    match e {
        Expr::Int(_) | Expr::Real(_) | Expr::Fill(_) | Expr::Str(_) => {}
        Expr::Sized(Some(w), base, digits, l, c) => {
            // E2006: literal melebihi lebar
            if let Some(v) = sized_value(*base, digits) {
                if v >= (1i64 << (*w).min(62)) {
                    return Err(err_at(
                        *l,
                        *c,
                        "E2006",
                        format!(
                            "literal {w}'{base}{digits} melebihi {w} bit — di '{}'",
                            scope.env.mname
                        ),
                    ));
                }
            }
        }
        Expr::Sized(None, _, _, ..) => {}
        Expr::Ident(s, l, c) => {
            // `$finish`/`$display`/`$past`/`$clog2` — system task/function
            // (di-lex sebagai Ident dengan awalan `$`), bukan sinyal.
            if s.starts_with('$') {
                return Ok(());
            }
            // `this`/`super` — kata kunci konteks di method class (F7)
            if s == "this" || s == "super" {
                return Ok(());
            }
            if !scope.known(s) {
                return Err(err_at(
                    *l,
                    *c,
                    "E2001",
                    format!("undefined signal '{s}' — di '{}'", scope.env.mname),
                ));
            }
        }
        Expr::Scoped(p, i, l, c) => {
            if let Some(pkg) = ctx.packages.get(p.as_str()) {
                let in_pkg = pkg.typedefs.iter().any(|td| td_name(td) == i)
                    || pkg.consts.iter().any(|(n, _, _)| n == i)
                    || pkg
                        .typedefs
                        .iter()
                        .any(|td| matches!(td, Typedef::Enum { members, .. } if members.iter().any(|mm| mm.name == *i)));
                if !in_pkg {
                    return Err(err_at(
                        *l,
                        *c,
                        "E2001",
                        format!(
                            "item '{i}' tidak ada di package '{p}' — di '{}'",
                            scope.env.mname
                        ),
                    ));
                }
            }
        }
        // F33: type cast `T'(x)` — tipe target harus dikenal (E2005),
        // ekspresi dalam divalidasi biasa.
        Expr::Cast {
            ty,
            expr,
            line,
            col,
        } => {
            // F33 fix review: cast target tidak boleh punya range/array
            // (`logic[7:0]'(x)` → emit SV invalid `logic [7:0]'(x)`).
            if matches!(ty.as_ref(), MvType::Logic(Some(_)) | MvType::Array(_, _)) {
                return Err(err_at(
                    *line,
                    *col,
                    "E2005",
                    "cast target tidak boleh punya range/array — pakai tipe sederhana (mis. `Word16'(x)`)",
                ));
            }
            // F33 fix review: size cast via parameter/konstanta (`WIDTH'(x)`)
            // — lebar = nilai param, bukan tipe. SV backend mendukungnya
            // (resolve_cast_name_width step 0/1); jangan E2005 di sini.
            if let MvType::Named(n, ..) = ty.as_ref() {
                if scope.params.contains_key(n.as_str()) {
                    check_expr(expr, ctx, scope, depth + 1)?;
                    return Ok(());
                }
            }
            check_type_scope(ty, ctx, Some(scope), 0)?;
            check_expr(expr, ctx, scope, depth + 1)?;
        }
        Expr::Unary(_, inner) => check_expr(inner, ctx, scope, depth + 1)?,
        // F37: `++x` / `x++` / `--x` / `x--` di level ekspresi (RHS) —
        // operand harus lvalue yang dikenal (validasi rekursif biasa).
        Expr::IncDec { expr, .. } => check_expr(expr, ctx, scope, depth + 1)?,
        Expr::Binary(_, l, r) => {
            check_expr(l, ctx, scope, depth + 1)?;
            check_expr(r, ctx, scope, depth + 1)?;
        }
        Expr::Ternary(c, t, f) => {
            check_expr(c, ctx, scope, depth + 1)?;
            check_expr(t, ctx, scope, depth + 1)?;
            check_expr(f, ctx, scope, depth + 1)?;
        }
        Expr::Call(_, args) => {
            // nama pemanggilan tidak divalidasi (bisa function eksternal/$system)
            for a in args {
                check_expr(a, ctx, scope, depth + 1)?;
            }
        }
        Expr::MethodCall {
            obj,
            method: _,
            args,
        } => {
            // objek (`this`/`super`/var/instance) + argumen; nama method tidak
            // divalidasi (method class/eksternal — konservatif)
            check_expr(obj, ctx, scope, depth + 1)?;
            for a in args {
                check_expr(a, ctx, scope, depth + 1)?;
            }
        }
        Expr::Member(obj, f, l, c) => {
            check_expr(obj, ctx, scope, depth + 1)?;
            // field struct: jika objek adalah Ident dengan tipe struct in-file
            if let Expr::Ident(base, ..) = obj.as_ref() {
                if let Some(ty) = scope.types.get(base.as_str()) {
                    if let Some(fields) = resolve_fields(ty, ctx, 0) {
                        if !fields.iter().any(|fl| fl.names.iter().any(|n| n == f)) {
                            return Err(err_at(
                                *l,
                                *c,
                                "E2001",
                                format!(
                                    "field '{f}' tidak ada di struct — di '{}'",
                                    scope.env.mname
                                ),
                            ));
                        }
                    }
                }
            }
        }
        Expr::Index(obj, i) => {
            check_expr(obj, ctx, scope, depth + 1)?;
            check_expr(i, ctx, scope, depth + 1)?;
        }
        Expr::Range(obj, a, b) => {
            check_expr(obj, ctx, scope, depth + 1)?;
            check_expr(a, ctx, scope, depth + 1)?;
            check_expr(b, ctx, scope, depth + 1)?;
        }
        Expr::Concat(parts) => {
            for p in parts {
                check_expr(p, ctx, scope, depth + 1)?;
            }
        }
        Expr::Replicate(n, inner) => {
            check_expr(n, ctx, scope, depth + 1)?;
            check_expr(inner, ctx, scope, depth + 1)?;
        }
        Expr::Paren(i) => check_expr(i, ctx, scope, depth + 1)?,
        // F12: `x inside {a, b, [lo:hi]}` — validasi ekspresi dan semua anggota
        Expr::Inside { expr, items } => {
            check_expr(expr, ctx, scope, depth + 1)?;
            for it in items {
                match it {
                    InsideItem::Value(v) => check_expr(v, ctx, scope, depth + 1)?,
                    InsideItem::Range(lo, hi) => {
                        check_expr(lo, ctx, scope, depth + 1)?;
                        check_expr(hi, ctx, scope, depth + 1)?;
                    }
                }
            }
        }
        // F12: `x dist { v := w, [lo:hi] :/ w }` — validasi ekspresi & bobot
        Expr::Dist { expr, items } => {
            check_expr(expr, ctx, scope, depth + 1)?;
            for it in items {
                check_expr(&it.value, ctx, scope, depth + 1)?;
                if let Some((lo, hi)) = &it.range {
                    check_expr(lo, ctx, scope, depth + 1)?;
                    check_expr(hi, ctx, scope, depth + 1)?;
                }
                check_expr(&it.weight, ctx, scope, depth + 1)?;
            }
        }
    }
    Ok(())
}

fn sized_value(base: char, digits: &str) -> Option<i64> {
    let radix = match base {
        'd' => 10,
        'h' => 16,
        'o' => 8,
        'b' => 2,
        _ => return None,
    };
    i64::from_str_radix(digits, radix).ok()
}

// ── Tipe: validasi (E2005) + lebar bit ──

/// Validasi tipe: `Named` harus ter-resolve (E2005). Rekursif ke dalam.
/// Tanpa scope — dipakai di level file/package/typedef (tanpa type param).
fn check_type(ty: &MvType, ctx: &Ctx, depth: usize) -> Result<(), MvError> {
    check_type_scope(ty, ctx, None, depth)
}

/// Validasi tipe dgn scope module (F32): `Named` yang merupakan type
/// parameter module (`T`) sah — tidak perlu ada di ctx.types.
fn check_type_scope(
    ty: &MvType,
    ctx: &Ctx,
    scope: Option<&Scope>,
    depth: usize,
) -> Result<(), MvError> {
    if depth > 8 {
        return Ok(());
    }
    match ty {
        MvType::Named(n, l, c) => {
            // F32: type param module sah sbg tipe di dalam module
            if let Some(sc) = scope {
                if sc.type_params.contains_key(n.as_str()) {
                    return Ok(());
                }
            }
            if let Some((pkg, item)) = n.split_once("::") {
                if let Some(p) = ctx.packages.get(pkg) {
                    if !p.typedefs.iter().any(|td| td_name(td) == item) {
                        return Err(err_at(
                            *l,
                            *c,
                            "E2005",
                            format!(
                                "tipe tak dikenal '{n}' (package '{pkg}' tidak punya '{item}')"
                            ),
                        ));
                    }
                }
                // package eksternal (file lain) → dilewati
            } else if !ctx.types.contains_key(n.as_str())
                && !ctx.classes.contains(n.as_str())
                && !ctx.interfaces.contains(n.as_str())
            {
                return Err(err_at(*l, *c, "E2005", format!("tipe tak dikenal '{n}'")));
            }
        }
        MvType::Signed(inner) => check_type_scope(inner, ctx, scope, depth + 1)?,
        MvType::Array(inner, _) => check_type_scope(inner, ctx, scope, depth + 1)?,
        _ => {}
    }
    Ok(())
}

/// Lebar bit tipe; None bila tidak diketahui (eksternal / ekspresi non-konstanta).
fn type_width(ty: &MvType, ctx: &Ctx, scope: &Scope, depth: usize) -> Option<i64> {
    if depth > 8 {
        return None;
    }
    match ty {
        MvType::Bit => Some(1),
        MvType::Logic(r) => match r {
            None => Some(1),
            Some((a, b)) => {
                let x = fold_const(a, &scope.params, 0);
                let y = fold_const(b, &scope.params, 0);
                match (x, y) {
                    (Some(ai), Some(bi)) => Some((ai - bi).abs() + 1),
                    _ => None,
                }
            }
        },
        MvType::Signed(inner) => type_width(inner, ctx, scope, depth + 1),
        MvType::Int => Some(32),
        MvType::Uint => Some(32),
        MvType::LongInt => Some(64),
        MvType::ULongInt => Some(64),
        MvType::ShortInt => Some(16),
        MvType::Byte => Some(8),
        MvType::Time => Some(64),
        MvType::Real | MvType::Str => None,
        MvType::Named(n, ..) => {
            // F32: type param module — lebar mengikuti default tipe-nya
            // (`T : type = logic[7:0]` → 8). None bila tanpa default.
            if let Some(Some(td)) = scope.type_params.get(n.as_str()) {
                return type_width(td, ctx, scope, depth + 1);
            }
            let td = resolve_typedef(n, ctx)?;
            match td {
                Typedef::Alias { ty, .. } => type_width(ty, ctx, scope, depth + 1),
                Typedef::Struct {
                    packed: true,
                    fields,
                    ..
                } => {
                    let mut total = 0i64;
                    for f in fields {
                        total += type_width(&f.ty, ctx, scope, depth + 1)?;
                    }
                    Some(total)
                }
                Typedef::Struct { packed: false, .. } => None,
                Typedef::Union {
                    packed: true,
                    fields,
                    ..
                } => {
                    let mut max_w = 0i64;
                    for f in fields {
                        let w = type_width(&f.ty, ctx, scope, depth + 1)?;
                        if w > max_w {
                            max_w = w;
                        }
                    }
                    Some(max_w)
                }
                Typedef::Union { packed: false, .. } => None,
                Typedef::Enum { width, members, .. } => match width {
                    Some(Expr::Int(n)) => Some(*n),
                    _ => Some(enum_bits(members.len())),
                },
            }
        }
        MvType::Array(inner, _) => type_width(inner, ctx, scope, depth + 1),
    }
}

/// Lebar elemen hasil select `x[i]`: vektor → 1 bit; array → lebar elemen.
fn element_width(ty: &MvType, ctx: &Ctx, scope: &Scope, depth: usize) -> Option<i64> {
    if depth > 8 {
        return None;
    }
    match ty {
        MvType::Logic(_) | MvType::Bit => Some(1),
        MvType::Array(inner, _) => type_width(inner, ctx, scope, depth + 1),
        MvType::Named(n, ..) => {
            if let Some(td) = resolve_typedef(n, ctx) {
                match td {
                    Typedef::Alias { ty, .. } => element_width(ty, ctx, scope, depth + 1),
                    _ => Some(1),
                }
            } else {
                None
            }
        }
        _ => type_width(ty, ctx, scope, depth + 1),
    }
}

/// Field struct bila tipe ter-resolve ke struct in-file.
fn resolve_fields<'a>(ty: &'a MvType, ctx: &'a Ctx<'a>, depth: usize) -> Option<Vec<&'a Field>> {
    if depth > 8 {
        return None;
    }
    match ty {
        MvType::Named(n, ..) => {
            let td = resolve_typedef(n, ctx)?;
            match td {
                Typedef::Struct { fields, .. } | Typedef::Union { fields, .. } => {
                    Some(fields.iter().collect())
                }
                Typedef::Alias { ty, .. } => resolve_fields(ty, ctx, depth + 1),
                Typedef::Enum { .. } => None,
            }
        }
        _ => None,
    }
}

/// Lebar bit ekspresi; None bila tidak dapat dihitung secara konstan.
fn expr_width(e: &Expr, ctx: &Ctx, scope: &Scope, depth: usize) -> Option<i64> {
    if depth > 16 {
        return None;
    }
    match e {
        Expr::Int(v) => Some(bit_len(*v)),
        Expr::Sized(Some(w), _, _, ..) => Some(*w),
        Expr::Sized(None, _, _, ..) => None,
        Expr::Real(_) | Expr::Fill(_) | Expr::Str(_) => None,
        Expr::Ident(s, ..) => {
            if let Some(ty) = scope.types.get(s.as_str()) {
                type_width(ty, ctx, scope, depth + 1)
            } else if let Some(v) = scope.params.get(s.as_str()) {
                Some(bit_len(*v))
            } else if let Some(w) = scope.enum_members.get(s.as_str()) {
                Some(*w)
            } else {
                None
            }
        }
        Expr::Scoped(..) => None,
        Expr::Unary(op, inner) => match op.as_str() {
            "&" | "|" | "^" | "~&" | "~|" | "~^" | "!" => Some(1),
            "~" | "-" | "+" => expr_width(inner, ctx, scope, depth + 1),
            _ => None,
        },
        // F37: `++x`/`x++`/`--x`/`x--` — lebar mengikuti operand.
        Expr::IncDec { expr, .. } => expr_width(expr, ctx, scope, depth + 1),
        Expr::Binary(op, l, r) => {
            let wl = expr_width(l, ctx, scope, depth + 1);
            let wr = expr_width(r, ctx, scope, depth + 1);
            match op.as_str() {
                "==" | "!=" | "===" | "!==" | "<" | "<=" | ">" | ">=" | "&&" | "||" => Some(1),
                "<<" | ">>" | "<<<" | ">>>" => wl,
                "*" => match (wl, wr) {
                    (Some(a), Some(b)) => Some(a + b),
                    _ => None,
                },
                "**" => None,
                _ => match (wl, wr) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    _ => None,
                },
            }
        }
        Expr::Ternary(_, t, f) => {
            let wt = expr_width(t, ctx, scope, depth + 1);
            let wf = expr_width(f, ctx, scope, depth + 1);
            match (wt, wf) {
                (Some(a), Some(b)) => Some(a.max(b)),
                _ => None,
            }
        }
        Expr::Call(..) | Expr::MethodCall { .. } => None,
        // F33: lebar cast = lebar tipe target (`Word16'(x)` → 16) — dipakai
        // E2002 (RHS cast lebih lebar dari LHS).
        Expr::Cast { ty, .. } => {
            // F33 fix review: size cast via parameter — lebar = NILAI param
            // (`WIDTH'(x)` dgn WIDTH=8 → 8-bit), bukan bit_len(nilai).
            if let MvType::Named(n, ..) = ty.as_ref() {
                if let Some(v) = scope.params.get(n.as_str()) {
                    return Some(*v);
                }
            }
            type_width(ty, ctx, scope, depth + 1)
        }
        Expr::Member(obj, f, ..) => {
            if let Expr::Ident(base, ..) = obj.as_ref() {
                if let Some(ty) = scope.types.get(base.as_str()) {
                    if let Some(fields) = resolve_fields(ty, ctx, 0) {
                        for fl in fields {
                            if fl.names.iter().any(|n| n == f) {
                                return type_width(&fl.ty, ctx, scope, depth + 1);
                            }
                        }
                    }
                }
            }
            None
        }
        Expr::Index(obj, _) => {
            if let Expr::Ident(base, ..) = obj.as_ref() {
                if let Some(ty) = scope.types.get(base.as_str()) {
                    return element_width(ty, ctx, scope, depth + 1);
                }
            }
            None
        }
        Expr::Range(_obj, a, b) => {
            let wa = fold_const(a, &scope.params, 0);
            let wb = fold_const(b, &scope.params, 0);
            match (wa, wb) {
                (Some(x), Some(y)) => Some((x - y).abs() + 1),
                _ => None,
            }
        }
        Expr::Concat(parts) => {
            let mut total = 0i64;
            for p in parts {
                total += expr_width(p, ctx, scope, depth + 1)?;
            }
            Some(total)
        }
        Expr::Replicate(n, inner) => {
            let c = fold_const(n, &scope.params, 0)?;
            Some(c * expr_width(inner, ctx, scope, depth + 1)?)
        }
        Expr::Paren(i) => expr_width(i, ctx, scope, depth + 1),
        // F12: inside/dist adalah predikat boolean → 1 bit
        Expr::Inside { .. } | Expr::Dist { .. } => Some(1),
    }
}

/// Minimal bit untuk nilai integer (negatif → anggap 32-bit).
fn bit_len(v: i64) -> i64 {
    if v == 0 {
        1
    } else if v > 0 {
        64 - (v as u64).leading_zeros() as i64
    } else {
        32
    }
}

/// Const-fold ekspresi integer sederhana. `Ident` hanya dari parameter.
fn fold_const(e: &Expr, params: &Params, depth: usize) -> Option<i64> {
    if depth > 8 {
        return None;
    }
    match e {
        Expr::Int(v) => Some(*v),
        Expr::Paren(i) => fold_const(i, params, depth + 1),
        Expr::Unary(op, i) => {
            let v = fold_const(i, params, depth + 1)?;
            match op.as_str() {
                "-" => Some(-v),
                "+" => Some(v),
                "~" => Some(!v),
                _ => None,
            }
        }
        Expr::Binary(op, l, r) => {
            let a = fold_const(l, params, depth + 1)?;
            let b = fold_const(r, params, depth + 1)?;
            match op.as_str() {
                "+" => Some(a.wrapping_add(b)),
                "-" => Some(a.wrapping_sub(b)),
                "*" => Some(a.wrapping_mul(b)),
                "/" => (b != 0).then(|| a.wrapping_div(b)),
                "%" => (b != 0).then(|| a.wrapping_rem(b)),
                "<<" => Some(a.wrapping_shl(b as u32)),
                ">>" => Some(a.wrapping_shr(b as u32)),
                "&" => Some(a & b),
                "|" => Some(a | b),
                "^" => Some(a ^ b),
                _ => None,
            }
        }
        Expr::Ident(s, ..) => params.get(s.as_str()).copied(),
        _ => None,
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn check_src(src: &str) -> Result<(), MvError> {
        let file = parse(src)?;
        check(&file)
    }

    #[test]
    fn ok_interface() {
        // F26: interface sehat + port module bertipe interface + seq clock
        // field interface
        let src = r#"
interface axi_lite {
    in  clk : bit
    sig awaddr  : logic[31:0]
    sig awvalid : bit
    sig awready : bit
    modport slave {
        in  awaddr, awvalid
        out awready
    }
}
module dut {
    in axi_if : axi_lite
    out done  : bit
    sig cnt : logic[3:0]
    seq(axi_if.clk) {
        if (axi_if.awvalid && axi_if.awready) {
            cnt <= cnt + 1
            done <= 1
        }
    }
}
"#;
        check_src(src).expect("interface sehat harus lolos check");
    }

    #[test]
    fn e2001_interface_modport_ref_unknown() {
        // F26: modport merujuk signal yang tidak dideklarasikan di interface
        let src = "interface bad {\n sig a : bit\n modport m { in a, nope } }\n";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2001"), "msg: {}", e.msg);
        assert!(e.msg.contains("nope"));
    }

    #[test]
    fn e2007_duplicate_interface() {
        // F26: interface nama sama dua kali
        let src = "interface x {}\ninterface x {}\n";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2007"), "msg: {}", e.msg);
        assert!(e.msg.contains("interface"));
    }

    #[test]
    fn ok_interface_port_type_known() {
        // F26: port module bertipe interface = tipe sah (bukan E2005)
        let src = "interface bus {}\nmodule m {\n in b : bus\n out y : bit\n comb { y = 0 } }\n";
        check_src(src).expect("interface sebagai tipe port harus lolos");
    }

    #[test]
    fn ok_counter_like() {
        let src = r#"
module counter #(WIDTH = 8, MOD = 100) {
    in  clk, rst_n : bit
    in  enable     : bit
    out count      : logic[WIDTH-1:0]
    out done       : bit
    seq(clk, rst_n) {
        if (!rst_n) {
            count <= '0
        } else if (enable) {
            count <= (count == MOD-1) ? '0 : count + 1
            done <= 1
        } else {
            done <= 0
        }
    }
}
"#;
        check_src(src).expect("counter harus lolos check");
    }

    #[test]
    fn ok_traffic_like_enum_and_struct() {
        let src = r#"
package pkt {
    enum State { RED, GREEN, YELLOW }
    type Addr = logic[15:0]
    packed struct Packet {
        valid : bit,
        addr  : Addr,
        data  : logic[31:0]
    }
}
module traffic {
    use pkt::*
    in  clk, rst_n : bit
    out state      : State
    out p          : Packet
    reg state      : State
    seq(clk, rst_n) {
        if (!rst_n) {
            state <= RED
        } else {
            state <= GREEN
        }
    }
    comb {
        p.valid = 1
        p.addr  = 8'h00
        p.data  = 32'd0
    }
}
"#;
        check_src(src).expect("enum/struct harus lolos check");
    }

    #[test]
    fn ok_widening_allowed() {
        // RHS lebih sempit dari LHS → bukan error (zero-extension aman)
        let src = "module m {\n in a : logic[3:0]\n out y : logic[7:0]\n comb { y = a } }";
        check_src(src).expect("widening harus diizinkan");
    }

    #[test]
    fn ok_testbench_drives_input() {
        // body initial = testbench → boleh drive port input
        let src = "module tb {\n in clk : bit\n in d   : bit\n initial { clk = 0\n d = 1 } }";
        check_src(src).expect("testbench boleh drive input");
    }

    #[test]
    fn ok_system_task_in_initial() {
        // `$finish`/`$display`/`$past` adalah system task/function —
        // bukan sinyal (F6; regresi: E2001 keliru untuk `$finish`)
        let src = "module tb {\n in clk : bit\n in count : logic[7:0]\n initial {\n $display(\"mulai\")\n $finish\n assert property (@(posedge clk) count == $past(count) + 1)\n } }\n";
        check_src(src).expect("system task bukan sinyal");
    }

    #[test]
    fn ok_assert_property_skipped() {
        // Body `assert property` raw tidak dianalisis (konservatif)
        let src = "module m {\n in clk : bit\n initial {\n assert property (@(posedge clk) some_signal_apa_saja == 1)\n } }\n";
        check_src(src).expect("assert property body dilewati");
    }

    #[test]
    fn ok_package_const_in_enum_width() {
        // konstanta package terlihat di lebar/nilai enum (regresi review)
        let src = "package p {\n const N = 4\n enum(N) Color { RED, GREEN } }\nmodule m { in clk : bit\n out c : p::Color\n comb { c = RED } }";
        check_src(src).expect("const package boleh dipakai di lebar enum");
    }

    #[test]
    fn e2001_in_program_block() {
        // Program block (MARIA-HDL.md §7.3) TIDAK boleh lolos type-check
        // (regresi review: check() hanya iterasi modules, bukan programs)
        let src = "program p {\n in clk : bit\n initial { foo = 1 } }\n";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2001"), "msg: {}", e.msg);
        assert!(e.msg.contains("foo"));
    }

    #[test]
    fn ok_program_drives_input_in_initial() {
        // Program testbench boleh drive port input di body initial
        let src = "program p {\n in clk : bit\n initial { clk = 0 } }\n";
        check_src(src).expect("program initial boleh drive input");
    }

    #[test]
    fn e2003_drive_input_with_final_still_error() {
        // module punya final (assertion) tapi comb tetap tidak boleh drive input
        let src =
            "module m {\n in clk : bit\n out y : bit\n comb { clk = 0 }\n final { y = clk } }";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2003"), "msg: {}", e.msg);
    }

    #[test]
    fn ok_func_locals() {
        let src = r#"
func clog2(x : int) -> int {
    var r : int = 0
    var n : int = x - 1
    while (n > 0) {
        r = r + 1
        n = n >> 1
    }
    return r
}
"#;
        check_src(src).expect("func dengan local harus lolos");
    }

    #[test]
    fn e2001_undefined_signal() {
        let src = "module m {\n in clk : bit\n out y : bit\n comb { y = foo } }";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2001"), "msg: {}", e.msg);
        assert!(e.msg.contains("foo"));
    }

    #[test]
    fn e2005_unknown_type() {
        let src = "module m {\n in clk : bit\n out y : Foo }";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2005"), "msg: {}", e.msg);
        assert!(e.msg.contains("Foo"));
    }

    #[test]
    fn e2007_duplicate_signal() {
        let src = "module m {\n sig a : bit\n sig a : bit }";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2007"), "msg: {}", e.msg);
    }

    #[test]
    fn e2004_nba_outside_seq() {
        let src = "module m {\n in clk : bit\n out y : bit\n comb { y <= clk } }";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2004"), "msg: {}", e.msg);
    }

    #[test]
    fn e2004_blocking_in_seq() {
        let src = "module m {\n in clk : bit\n out y : bit\n seq(clk) { y = clk } }";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2004"), "msg: {}", e.msg);
    }

    #[test]
    fn e2003_drive_input() {
        let src = "module m {\n in clk : bit\n out y : bit\n comb { clk = 0 } }";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2003"), "msg: {}", e.msg);
        assert!(e.msg.contains("clk"));
    }

    #[test]
    fn e2002_width() {
        let src = "module m {\n in a : logic[7:0]\n out y : logic[3:0]\n comb { y = a } }";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2002"), "msg: {}", e.msg);
    }

    #[test]
    fn e2006_overflow() {
        let src = "module m {\n in clk : bit\n out y : logic[7:0]\n comb { y = 8'h1FF } }";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2006"), "msg: {}", e.msg);
    }

    #[test]
    fn e2001_struct_field_unknown() {
        let src = r#"
package pkt {
    packed struct Packet { valid : bit, addr : logic[7:0] }
}
module m {
    use pkt::*
    in clk : bit
    out p  : Packet
    comb {
        p.nonexistent = 1
    }
}
"#;
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2001"), "msg: {}", e.msg);
        assert!(e.msg.contains("nonexistent"));
    }

    #[test]
    fn e2007_duplicate_enum_member() {
        let src = "package p { enum E { A, A } }";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2007"), "msg: {}", e.msg);
    }

    #[test]
    fn ok_class_plain() {
        // Class mandiri (semua tipe internal) → lolos check penuh (F7)
        let src = r#"
class counter_model {
    field value : uint
    constraint c { value > 0 }
    func new() {
        value = 0
    }
    task tick() {
        value = value + 1
    }
}
"#;
        check_src(src).expect("class mandiri harus lolos check");
    }

    #[test]
    fn ok_class_this_super_method_call() {
        // `this`/`super`/method-call bukan sinyal (F7)
        let src = r#"
class c extends base {
    field x : uint
    func new() {
        super.new()
        this.x = 0
    }
    func f() {
        self_helper(x)
    }
}
"#;
        // `self_helper` = pemanggilan function (nama tak divalidasi)
        check_src(src).expect("this/super/method-call harus lolos");
    }

    #[test]
    fn e2001_in_class_method() {
        // E2001 ditegakkan di body method class (F7)
        let src = "class c {\n field x : uint\n func f() {\n y = 1\n }\n}";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2001"), "msg: {}", e.msg);
        assert!(e.msg.contains("y"));
    }

    #[test]
    fn e2007_duplicate_class_field() {
        let src = "class c {\n field x : uint\n field x : uint\n}";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2007"), "msg: {}", e.msg);
    }

    #[test]
    fn e2007_duplicate_class() {
        let src = "class a {}\nclass a {}\n";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2007"), "msg: {}", e.msg);
    }

    // ── F9: multi-file (konteks gabungan lintas file) ──

    #[test]
    fn f9_check_many_cross_file_package() {
        // `types.mv` mendefinisikan `Addr`/`State` di package `types_pkg`;
        // `counter.mv` memakainya via `use types_pkg::*` — harus lolos bila
        // di-check BERSAMA (F9). Solo check tetap E2005 (batasan per-file).
        let types = parse(
            "package types_pkg {\n type Addr = logic[15:0]\n enum State { IDLE, RUN }\n}\nmodule types_dummy {\n in clk : bit\n}\n",
        )
        .unwrap();
        let counter = parse(
            "module counter {\n use types_pkg::*\n in clk, rst_n : bit\n out addr : Addr\n out st : State\n seq(clk, rst_n) {\n if (!rst_n) {\n addr <= '0\n st <= IDLE\n } else {\n addr <= addr + 1\n st <= RUN\n }\n }\n}\n",
        )
        .unwrap();

        // solo: `Addr` tidak dikenal di file counter → E2005
        let e = check(&counter).unwrap_err();
        assert!(e.msg.contains("E2005"), "msg: {}", e.msg);

        // batch: konteks gabungan → lolos
        check_many(&[&types, &counter]).expect("multi-file harus lolos check");
    }

    #[test]
    fn f9_check_many_error_index_and_message() {
        // Error di file kedua harus di-return dengan indeks file yang benar.
        let good = parse("module a {\n in clk : bit\n}\n").unwrap();
        let bad =
            parse("module b {\n in clk : bit\n out y : bit\n comb { y = nope }\n}\n").unwrap();
        let (idx, e) = check_many(&[&good, &bad]).unwrap_err();
        assert_eq!(idx, 1);
        assert!(e.msg.contains("E2001"), "msg: {}", e.msg);
        assert!(e.msg.contains("nope"));
    }

    #[test]
    fn f9_check_many_const_visible_across_files() {
        // Konstanta package di file A terlihat sebagai enum width di file B.
        let a = parse("package cfg {\n const N = 4\n}\n").unwrap();
        let b = parse(
            "package p {\n enum(N) Color { RED, GREEN }\n}\nmodule m {\n in clk : bit\n out c : p::Color\n comb { c = RED }\n}\n",
        )
        .unwrap();
        check_many(&[&a, &b]).expect("konstanta antar-file harus terlihat");
    }

    #[test]
    fn f9_check_many_duplicate_package_cross_file() {
        // Dua file mendefinisikan package dengan nama sama → E2007 lintas-file
        // (temuan review: sebelumnya first-wins silent, error SV yang membingungkan).
        let a = parse("package pkt {\n type Addr = logic[7:0]\n}\n").unwrap();
        let b = parse("package pkt {\n type Data = logic[7:0]\n}\n").unwrap();
        let (idx, e) = check_many(&[&a, &b]).unwrap_err();
        assert_eq!(idx, 1);
        assert!(e.msg.contains("E2007"), "msg: {}", e.msg);
        assert!(e.msg.contains("pkt"));
    }

    #[test]
    fn f9_check_many_duplicate_class_cross_file() {
        // Class dengan nama sama di dua file → E2007 lintas-file.
        let a = parse("class foo {\n field x : uint\n}\n").unwrap();
        let b = parse("class foo {\n field y : uint\n}\n").unwrap();
        let e = check_many(&[&a, &b]).unwrap_err();
        assert!(e.1.msg.contains("E2007"), "msg: {}", e.1.msg);
        assert!(e.1.msg.contains("foo"));
    }

    // ── F11: error type-check BERPOSISI (line:col) ──
    // Sebelumnya semua error check memakai (0,0); sekarang AST membawa posisi
    // dari parser sehingga E2001–E2007 menunjuk baris/kolom persis di .mv.

    #[test]
    fn f11_e2001_undefined_signal_position() {
        // `foo` di baris 4, kolom 16 (`    comb { y = foo }`)
        let src = "module m {\n    in clk : bit\n    out y : bit\n    comb { y = foo }\n}\n";
        let e = check_src(src).unwrap_err();
        assert_eq!(e.line, 4, "line: {e:?}");
        assert_eq!(e.col, 16, "col: {e:?}");
        assert!(e.msg.contains("E2001"));
    }

    #[test]
    fn f11_e2002_width_position() {
        // Statement assignment dimulai di `y` (baris 4, kolom 12)
        let src =
            "module m {\n    in a : logic[7:0]\n    out y : logic[3:0]\n    comb { y = a }\n}\n";
        let e = check_src(src).unwrap_err();
        assert_eq!(e.line, 4, "line: {e:?}");
        assert_eq!(e.col, 12, "col: {e:?}");
        assert!(e.msg.contains("E2002"));
    }

    #[test]
    fn f11_e2003_drive_input_position() {
        let src = "module m {\n    in clk : bit\n    out y : bit\n    comb { clk = 0 }\n}\n";
        let e = check_src(src).unwrap_err();
        assert_eq!(e.line, 4);
        assert_eq!(e.col, 12); // posisi `clk` (awal statement)
        assert!(e.msg.contains("E2003"));
    }

    #[test]
    fn f11_e2004_nba_outside_seq_position() {
        let src = "module m {\n    in clk : bit\n    out y : bit\n    comb { y <= clk }\n}\n";
        let e = check_src(src).unwrap_err();
        assert_eq!(e.line, 4);
        assert_eq!(e.col, 12);
        assert!(e.msg.contains("E2004"));
    }

    #[test]
    fn f11_e2005_unknown_type_position() {
        // `Foo` di baris 3, kolom 13 (`    out y : Foo`)
        let src = "module m {\n    in clk : bit\n    out y : Foo\n}\n";
        let e = check_src(src).unwrap_err();
        assert_eq!(e.line, 3, "line: {e:?}");
        assert_eq!(e.col, 13, "col: {e:?}");
        assert!(e.msg.contains("E2005"));
    }

    #[test]
    fn f11_e2006_overflow_position() {
        // `8'h1FF` di baris 4, kolom 16
        let src =
            "module m {\n    in clk : bit\n    out y : logic[7:0]\n    comb { y = 8'h1FF }\n}\n";
        let e = check_src(src).unwrap_err();
        assert_eq!(e.line, 4, "line: {e:?}");
        assert_eq!(e.col, 16, "col: {e:?}");
        assert!(e.msg.contains("E2006"));
    }

    #[test]
    fn f11_e2007_duplicate_signal_position() {
        // Duplikat `sig a` kedua di baris 3, kolom 5 (`    sig a : bit`)
        let src = "module m {\n    sig a : bit\n    sig a : bit\n}\n";
        let e = check_src(src).unwrap_err();
        assert_eq!(e.line, 3, "line: {e:?}");
        assert_eq!(e.col, 5, "col: {e:?}");
        assert!(e.msg.contains("E2007"));
    }

    #[test]
    fn f11_e2001_clock_undefined_position() {
        // clock `nope` di seq — posisi seq spec (setelah `seq`)
        let src = "module m {\n    in clk : bit\n    out y : bit\n    seq(nope) { y <= 0 }\n}\n";
        let e = check_src(src).unwrap_err();
        assert_eq!(e.line, 4, "line: {e:?}");
        assert!(e.msg.contains("E2001"));
        assert!(e.msg.contains("nope"));
    }

    #[test]
    fn f11_e2001_struct_field_position() {
        // field struct tak dikenal → posisi `.nonexistent` di baris 8
        let src = "package pkt {\n    packed struct P { valid : bit }\n}\nmodule m {\n    use pkt::*\n    in clk : bit\n    out p : P\n    comb { p.nonexistent = 1 }\n}\n";
        let e = check_src(src).unwrap_err();
        assert_eq!(e.line, 8, "line: {e:?}");
        assert!(e.msg.contains("E2001"));
        assert!(e.msg.contains("nonexistent"));
    }

    #[test]
    fn f11_cross_file_duplicate_has_position() {
        // Duplikat package lintas-file (F9) kini membawa posisi deklarasi kedua
        let a = parse("package pkt {\n type A = logic[7:0]\n}\n").unwrap();
        let b = parse("package pkt {\n type B = logic[7:0]\n}\n").unwrap();
        let (idx, e) = check_many(&[&a, &b]).unwrap_err();
        assert_eq!(idx, 1);
        assert_eq!(e.line, 1, "line: {e:?}");
        assert_eq!(e.col, 9, "col: {e:?}"); // `pkt` kedua di kolom 9
        assert!(e.msg.contains("E2007"));
    }

    // ── F12: constraint lanjutan (inside/dist/if-else/solve) ──

    #[test]
    fn f12_ok_constraint_advanced() {
        // Semua fitur constraint F12 dalam satu blok — harus lolos check
        let src = r#"
class item {
    rand field mode : uint[2]
    rand field addr : uint[8]
    rand field data : uint[8]
    constraint c_adv {
        addr inside {[1:10], 20, 30},
        data dist { 0 := 1, [1:5] :/ 9 },
        if (mode == 1) { addr > 5 } else { addr < 100 },
        solve addr before data
    }
}
"#;
        check_src(src).expect("constraint lanjutan harus lolos check");
    }

    #[test]
    fn f12_constraint_solve_unknown_var() {
        // `solve nope before x` — var tak dikenal → E2001 berposisi
        let src = "class c {\n    rand field x : uint\n    constraint c1 {\n        solve nope before x\n    }\n}\n";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2001"), "msg: {}", e.msg);
        assert!(e.msg.contains("nope"));
        assert_eq!(e.line, 4, "line: {e:?}");
    }

    #[test]
    fn f12_constraint_inside_unknown_signal() {
        // `y inside {...}` — y tak dikenal → E2001 di posisi y
        let src = "class c {\n    rand field x : uint\n    constraint c1 {\n        y inside {[1:10]}\n    }\n}\n";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2001"), "msg: {}", e.msg);
        assert!(e.msg.contains("y"));
        assert_eq!(e.line, 4, "line: {e:?}");
    }

    #[test]
    fn f12_constraint_dist_unknown_weight_signal() {
        // bobot dist memakai sinyal tak dikenal → E2001
        let src = "class c {\n    rand field x : uint\n    constraint c1 {\n        x dist { 1 := w }\n    }\n}\n";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2001"), "msg: {}", e.msg);
        assert!(e.msg.contains("w"));
    }

    #[test]
    fn f11_e2007_duplicate_module_in_file() {
        // Temuan review: duplikat nama module dalam satu file TIDAK pernah
        // dicek — kini E2007 dengan posisi nama module kedua (baris 4).
        let src = "module a {\n    in clk : bit\n}\nmodule a {\n    in clk : bit\n}\n";
        let e = check_src(src).unwrap_err();
        assert_eq!(e.line, 4, "line: {e:?}");
        assert_eq!(e.col, 8, "col: {e:?}"); // `a` kedua di kolom 8
        assert!(e.msg.contains("E2007"));
        assert!(e.msg.contains("module 'a'"));
    }

    #[test]
    fn f11_e2007_duplicate_module_cross_file() {
        // Module dengan nama sama di dua file → E2007 lintas-file (baru).
        let a = parse("module top {\n    in clk : bit\n}\n").unwrap();
        let b = parse("module top {\n    in clk : bit\n}\n").unwrap();
        let (idx, e) = check_many(&[&a, &b]).unwrap_err();
        assert_eq!(idx, 1);
        assert_eq!(e.line, 1, "line: {e:?}");
        assert_eq!(e.col, 8, "col: {e:?}"); // `top` kedua di kolom 8
        assert!(e.msg.contains("E2007"));
    }

    #[test]
    fn f11_e2007_module_vs_program_collision() {
        // module `tb` + program `tb` = bentrok namespace SV (F11).
        let src = "module tb {\n    in clk : bit\n}\nprogram tb {\n    in clk : bit\n}\n";
        let e = check_src(src).unwrap_err();
        assert_eq!(e.line, 4, "line: {e:?}");
        assert!(e.msg.contains("E2007"));
    }

    // ── F29: validasi instansiasi + koneksi port ──

    #[test]
    fn f29_inst_named_port_not_exist() {
        // koneksi `.bogus(y)` ke module foo yang tidak punya port bogus →
        // E2001 dengan posisi nama module (baris 5).
        let src = "module foo {\n    in a : bit\n    out b : bit\n}\nmodule tb {\n    sig x : bit\n    sig y : bit\n    inst foo u (.a(x), .bogus(y))\n}\n";
        let e = check_src(src).unwrap_err();
        assert_eq!(e.line, 8, "line: {e:?}");
        assert!(e.msg.contains("E2001"), "msg: {}", e.msg);
        assert!(e.msg.contains("bogus"), "msg: {}", e.msg);
        assert!(e.msg.contains("module 'foo'"), "msg: {}", e.msg);
    }

    #[test]
    fn f29_inst_positional_too_many() {
        // 2 koneksi positional untuk module 1 port → E2001.
        let src = "module foo {\n    in a : bit\n}\nmodule tb {\n    sig x : bit\n    sig y : bit\n    inst foo u (x, y)\n}\n";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2001"), "msg: {}", e.msg);
        assert!(
            e.msg.contains("terlalu banyak koneksi positional"),
            "msg: {}",
            e.msg
        );
    }

    #[test]
    fn f29_inst_external_module_skipped() {
        // module foo3 tidak ada di file → konservatif dilewati (bukan error),
        // contoh kode multi-file di mana target di file .mv lain.
        let src = "module tb {\n    sig x : bit\n    inst foo3 u (.a(x))\n}\n";
        check_src(src).expect("module eksternal harus dilewati, bukan error");
    }

    #[test]
    fn f29_inst_interface_port_check() {
        // instansiasi interface `axi_lite bus(.bogus(x))` — port tak ada → E2001.
        let src = "interface axi_lite {\n    in clk : bit\n}\nmodule tb {\n    sig x : bit\n    inst axi_lite bus (.bogus(x))\n}\n";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2001"), "msg: {}", e.msg);
        assert!(e.msg.contains("interface 'axi_lite'"), "msg: {}", e.msg);
    }

    #[test]
    fn f29_inst_ok_named_and_positional() {
        // koneksi valid: named + positional dalam jumlah pas → tidak error.
        let src = "module foo {\n    in a : bit\n    out b : bit\n}\nmodule tb {\n    sig x : bit\n    sig y : bit\n    inst foo u (x, .b(y))\n}\n";
        check_src(src).expect("koneksi valid harus lolos");
    }

    // ── F29 fix review ──

    #[test]
    fn f29_review_duplicate_port_connection() {
        // `.a(x), .a(y)` — port sama dikoneksikan dua kali → E2007.
        let src = "module foo {\n    in a : bit\n}\nmodule tb {\n    sig x : bit\n    sig y : bit\n    inst foo u (.a(x), .a(y))\n}\n";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2007"), "msg: {}", e.msg);
        assert!(e.msg.contains("dikoneksikan dua kali"), "msg: {}", e.msg);
    }

    #[test]
    fn f29_review_module_interface_name_collision() {
        // module `foo` + interface `foo` = bentrok namespace tipe SV → E2007.
        let src = "interface foo {\n    in clk : bit\n}\nmodule foo {\n    in a : bit\n}\n";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2007"), "msg: {}", e.msg);
        assert!(e.msg.contains("bentrok"), "msg: {}", e.msg);
    }

    #[test]
    fn f30_inst_cross_file_port_check() {
        // F29/F30: module didefinisikan di file A, di-instansiasi di file B.
        // check_many (konteks gabungan) mengenali target lintas-file →
        // koneksi port tetap divalidasi (.nope → E2001), bukan dilewati.
        let a = parse("module foo {\n    in a : bit\n    out b : bit\n}\n").unwrap();
        let b = parse(
            "module tb {\n    sig x : bit\n    sig y : bit\n    inst foo u (.a(x), .nope(y))\n}\n",
        )
        .unwrap();
        let (idx, e) = check_many(&[&a, &b]).unwrap_err();
        assert_eq!(idx, 1);
        assert!(e.msg.contains("E2001"), "msg: {}", e.msg);
        assert!(e.msg.contains("nope"), "msg: {}", e.msg);
        assert!(e.msg.contains("module 'foo'"), "msg: {}", e.msg);
    }

    #[test]
    fn f30_inst_cross_file_ok() {
        // Koneksi lintas-file yang benar → lolos.
        let a = parse("module foo {\n    in a : bit\n    out b : bit\n}\n").unwrap();
        let b =
            parse("module tb {\n    sig x : bit\n    sig y : bit\n    inst foo u (x, .b(y))\n}\n")
                .unwrap();
        check_many(&[&a, &b]).expect("koneksi lintas-file valid harus lolos");
    }

    // ── F31: parameter module + validasi override ──

    #[test]
    fn f31_param_override_unknown() {
        // `#(.NOPE(4))` — parameter tak dikenal di module target → E2001.
        let src = "module counter_w #(W = 4) {\n    in  clk : bit\n    out count : logic[W-1:0]\n}\nmodule tb {\n    sig x : bit\n    inst counter_w u #(.NOPE(4)) (.clk(x), .count(x))\n}\n";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2001"), "msg: {}", e.msg);
        assert!(e.msg.contains("NOPE"), "msg: {}", e.msg);
        assert!(e.msg.contains("module 'counter_w'"), "msg: {}", e.msg);
    }

    #[test]
    fn f31_param_override_duplicate() {
        // `#(.W(4), .W(8))` — parameter sama di-override dua kali → E2007.
        let src = "module counter_w #(W = 4) {\n    in  clk : bit\n    out count : logic[W-1:0]\n}\nmodule tb {\n    sig x : bit\n    inst counter_w u #(.W(4), .W(8)) (.clk(x), .count(x))\n}\n";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2007"), "msg: {}", e.msg);
        assert!(e.msg.contains("di-override dua kali"), "msg: {}", e.msg);
    }

    #[test]
    fn f31_param_override_ok() {
        // Override parameter yang valid + koneksi port sah → lolos.
        let src = "module counter_w #(W = 4) {\n    in  clk : bit\n    out count : logic[W-1:0]\n}\nmodule tb {\n    sig x : bit\n    sig y : logic[7:0]\n    inst counter_w u #(.W(8)) (.clk(x), .count(y))\n}\n";
        check_src(src).expect("override parameter valid harus lolos");
    }

    #[test]
    fn f31_param_override_external_module_skipped() {
        // Module target tidak dikenal (file .mv lain) → override dilewati
        // (pola konservatif E2001, sama seperti koneksi port F29).
        let src = "module tb {\n    sig x : bit\n    sig y : bit\n    inst foo3 u #(.NOPE(4)) (.a(x), .b(y))\n}\n";
        check_src(src).expect("module eksternal harus dilewati, bukan error");
    }

    // ── F32: type parameter (`T : type = logic[7:0]`) ──

    #[test]
    fn f32_type_param_decl_ok() {
        // Type param deklarasi + dipakai sbg tipe sinyal/port → lolos.
        // `sig x : T` — T ter-resolve dari type param (bukan E2005); lebar
        // sinyal ikut default tipe (logic[7:0] → 8).
        let src =
            "module m #(T : type = logic[7:0]) {\n    in  d : T\n    out q : T\n    sig x : T\n}\n";
        check_src(src).expect("type param + sinyal bertipe T harus lolos");
    }

    #[test]
    fn f32_type_param_type_kw_syntax() {
        // Bentuk (B): `type T = logic[7:0]` (kata kunci `type` di awal).
        let src = "module m #(type T = logic[15:0]) {\n    sig x : T\n}\n";
        check_src(src).expect("sintaks `type T = ...` harus lolos");
    }

    #[test]
    fn f32_type_param_bad_default_type() {
        // Type param dgn default TIPE tak dikenal → E2005.
        let src = "module m #(T : type = UnknownTy) {\n    sig x : T\n}\n";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2005"), "msg: {}", e.msg);
        assert!(e.msg.contains("UnknownTy"), "msg: {}", e.msg);
    }

    #[test]
    fn f32_type_param_override_type_ok() {
        // Override type param dgn nama tipe yang dikenal → lolos.
        let src = "type Word16 = logic[15:0]\nmodule m #(T : type = logic[7:0]) {\n    in  d : T\n    out q : T\n}\nmodule tb {\n    sig d16 : logic[15:0]\n    sig q16 : logic[15:0]\n    inst m u #(.T(Word16)) (.d(d16), .q(q16))\n}\n";
        check_src(src).expect("override type param dgn tipe dikenal harus lolos");
    }

    #[test]
    fn f32_type_param_override_non_type_err() {
        // Override type param dgn NILAI (bukan tipe) → E2005.
        let src = "module m #(T : type = logic[7:0]) {\n    in  d : T\n    out q : T\n}\nmodule tb {\n    sig d8 : logic[7:0]\n    sig q8 : logic[7:0]\n    inst m u #(.T(8'd4)) (.d(d8), .q(q8))\n}\n";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2005"), "msg: {}", e.msg);
        assert!(e.msg.contains("harus nama tipe"), "msg: {}", e.msg);
    }

    #[test]
    fn f32_type_param_override_unknown_type_err() {
        // Override type param dgn nama tipe yang TIDAK dikenal → E2005.
        let src = "module m #(T : type = logic[7:0]) {\n    in  d : T\n    out q : T\n}\nmodule tb {\n    sig d8 : logic[7:0]\n    sig q8 : logic[7:0]\n    inst m u #(.T(NopeTy)) (.d(d8), .q(q8))\n}\n";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2005"), "msg: {}", e.msg);
        assert!(e.msg.contains("NopeTy"), "msg: {}", e.msg);
    }

    #[test]
    fn f32_inst_external_type_param_like_skipped() {
        // Module eksternal (file .mv lain) + override `.T(Word16)` dgn nilai
        // berbentuk tipe → dilewati konservatif (bukan E2001 signal).
        let src = "type Word16 = logic[15:0]\nmodule tb {\n    sig d16 : logic[15:0]\n    sig q16 : logic[15:0]\n    inst ext_mod u #(.T(Word16)) (.d(d16), .q(q16))\n}\n";
        check_src(src).expect("override type param eksternal harus dilewati");
    }

    // ── F32 fix review ──

    #[test]
    fn f32_review_const_of_type_param() {
        // `const c : T = ...` — T type param juga sah utk konstanta module
        // (pass-1 check_module harus scope-aware, sama dgn port/sig/reg).
        let src = "module m #(T : type = logic[7:0]) {\n    const C = 4\n    sig x : T\n    comb { x = C[0] }\n}\n";
        check_src(src).expect("const dgn tipe type param harus lolos");
    }

    #[test]
    fn f32_review_override_scoped_type() {
        // Override type param dgn tipe SCOPED `#(.T(pkg::Word16))` → lolos
        // (bukan E2005 false positive — Expr::Scoped sah sbg tipe).
        let src = "package p {\n    type Word16 = logic[15:0]\n}\nmodule m #(T : type = logic[7:0]) {\n    in  d : T\n    out q : T\n}\nmodule tb {\n    sig d16 : logic[15:0]\n    sig q16 : logic[15:0]\n    inst m u #(.T(p::Word16)) (.d(d16), .q(q16))\n}\n";
        check_src(src).expect("override scoped type param harus lolos");
    }

    #[test]
    fn f32_review_type_kw_without_default() {
        // Bentuk (B) `type T` TANPA default — marker tetap ter-set (parser)
        // → `sig x : T` dikenal (bukan E2005), lebar tak diketahui (None).
        let src = "module m #(type T) {\n    sig x : T\n}\n";
        check_src(src).expect("type param tanpa default tetap dikenal sbg tipe");
    }

    // ── F33: type cast `T'(expr)` ──

    #[test]
    fn f33_cast_ok() {
        // Cast ke tipe dasar, typedef, type param, dan scoped → lolos.
        let src = "type Word16 = logic[15:0]\nmodule m #(T : type = logic[7:0]) {\n    in  a : logic[15:0]\n    out w : Word16\n    out t : T\n    out b : bit\n    comb {\n        w = Word16'(a)\n        t = T'(a)\n        b = logic'(a[0])\n    }\n}\n";
        check_src(src).expect("cast ke tipe dikenal harus lolos");
    }

    #[test]
    fn f33_cast_unknown_type_err() {
        // Cast ke tipe TAK dikenal → E2005.
        let src = "module m {\n    in  a : logic[7:0]\n    out q : logic[7:0]\n    comb {\n        q = NopeTy'(a)\n    }\n}\n";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2005"), "msg: {}", e.msg);
        assert!(e.msg.contains("NopeTy"), "msg: {}", e.msg);
    }

    #[test]
    fn f33_cast_width_into_narrow_signal_err() {
        // E2002: cast lebar 16 (`Word16'(a)`) ke sinyal 8-bit → truncation.
        let src = "type Word16 = logic[15:0]\nmodule m {\n    in  a : logic[15:0]\n    out q : logic[7:0]\n    comb {\n        q = Word16'(a)\n    }\n}\n";
        let e = check_src(src).unwrap_err();
        assert!(e.msg.contains("E2002"), "msg: {}", e.msg);
    }

    // ── F33 fix review ──

    #[test]
    fn f33_review_size_cast_via_param_ok() {
        // Size cast via parameter `WIDTH'(a)` (lebar = nilai param) → lolos
        // (bukan E2005 false positive — SV backend mendukung size cast).
        let src = "module m #(WIDTH = 8) {\n    in  a : logic[7:0]\n    out q : logic[7:0]\n    comb {\n        q = WIDTH'(a)\n    }\n}\n";
        check_src(src).expect("size cast via parameter harus lolos");
    }

    #[test]
    fn f33_review_cast_ranged_target_err() {
        // Cast ke tipe ber-range `logic[7:0]'(a)` → emit SV invalid → ditolak
        // (di PARSER: parse_base_type tak makan `[7:0]` → E1005; guard E2005
        // di check hanya defensif bila suatu saat parser berubah).
        let src = "module m {\n    in  a : logic[7:0]\n    out q : logic[7:0]\n    comb {\n        q = logic[7:0]'(a)\n    }\n}\n";
        let e = check_src(src).unwrap_err();
        assert!(
            e.msg.contains("tidak boleh punya range") || e.msg.contains("ekspresi tidak valid"),
            "msg: {}",
            e.msg
        );
    }
}
