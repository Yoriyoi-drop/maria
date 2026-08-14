//! Liberty (`.lib`) parser — subset (SYNTHESIS.md §12/§6 — phase 6).
//!
//! Liberty adalah format library sel standar industri (Synopsys). Maria
//! mem-parse subset yang cukup untuk ASIC mapping & timing/area:
//!
//! ```text
//! library (sky130_fd_sc_hd) {
//!   delay_model : table_lookup;
//!   time_unit : "1ns";
//!   cell (NAND2_X1) {
//!     area : 1.2;
//!     pin (A)   { direction : input; capacitance : 0.003; }
//!     pin (Y)   {
//!       direction : output;
//!       function : "(A & B)";
//!       timing () {
//!         related_pin : "A";
//!         rise_propagation_delay (scalar) { values ("0.5"); }
//!         fall_propagation_delay (scalar) { values ("0.6"); }
//!       }
//!     }
//!   }
//! }
//! ```
//!
//! Didukung: `library` (name, delay_model, time_unit), `cell` (area,
//! footprint, pin), `pin` (direction, capacitance, function, timing),
//! `timing` (related_pin, rise/fall propagation delay scalar). Grup/atribut
//! lain di-skip toleran. Delay dikonversi ke ns sesuai `time_unit`.
//!
//! Hasil bisa disimpan ke `.libmdb` (format teks deterministik — database
//! sederhana ala MICD, bisa di-commit/di-diff) via `save_mdb`/`load_mdb`.

use std::fmt::Write as _;
use std::path::Path;

/// Arah pin sel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinDir {
    Input,
    Output,
    Inout,
}

impl PinDir {
    pub fn name(&self) -> &'static str {
        match self {
            PinDir::Input => "in",
            PinDir::Output => "out",
            PinDir::Inout => "inout",
        }
    }
}

/// Satu arc timing: delay dari `related_pin` ke pin ini.
#[derive(Debug, Clone, PartialEq)]
pub struct TimingArc {
    pub related_pin: String,
    /// Delay rise (ns, dikonversi dari time_unit).
    pub rise_delay_ns: Option<f64>,
    /// Delay fall (ns).
    pub fall_delay_ns: Option<f64>,
}

/// Pin sel.
#[derive(Debug, Clone, PartialEq)]
pub struct LibertyPin {
    pub name: String,
    pub direction: PinDir,
    pub capacitance: f64,
    /// Fungsi boolean (mis. `"(A & B)"`) — dipakai mapping ASIC.
    pub function: Option<String>,
    pub timings: Vec<TimingArc>,
}

/// Sel library.
#[derive(Debug, Clone, PartialEq)]
pub struct LibertyCell {
    pub name: String,
    pub area: f64,
    pub footprint: Option<String>,
    pub pins: Vec<LibertyPin>,
}

impl LibertyCell {
    /// Pin dengan nama tertentu.
    pub fn pin(&self, name: &str) -> Option<&LibertyPin> {
        self.pins.iter().find(|p| p.name == name)
    }

    /// Delay (ns) arc `related → pin` (rise/fall — pakai maksimum).
    pub fn arc_delay_ns(&self, from: &str, to: &str) -> Option<f64> {
        let pin = self.pin(to)?;
        let arc = pin
            .timings
            .iter()
            .find(|t| t.related_pin == from)
            .or_else(|| pin.timings.first())?;
        Some(
            arc.rise_delay_ns
                .unwrap_or(0.0)
                .max(arc.fall_delay_ns.unwrap_or(0.0)),
        )
    }
}

/// Library hasil parse.
#[derive(Debug, Clone, PartialEq)]
pub struct LibertyLibrary {
    pub name: String,
    pub delay_model: String,
    /// Unit waktu (mis. `1ns`, `1ps`).
    pub time_unit: String,
    pub cells: Vec<LibertyCell>,
}

impl LibertyLibrary {
    /// Sel dengan nama tertentu.
    pub fn cell(&self, name: &str) -> Option<&LibertyCell> {
        self.cells.iter().find(|c| c.name == name)
    }
}

// ────────────────────────────────────────────────────────────────
// Tokenizer
// ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Num(f64),
    LParen,
    RParen,
    LBrace,
    RBrace,
    Colon,
    Semi,
    Comma,
    Star,
}

fn tokenize(text: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    let n = chars.len();
    while i < n {
        let c = chars[i];
        // komentar /* ... */
        if c == '/' && i + 1 < n && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < n && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(n);
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        match c {
            '(' => {
                toks.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                toks.push(Tok::RParen);
                i += 1;
            }
            '{' => {
                toks.push(Tok::LBrace);
                i += 1;
            }
            '}' => {
                toks.push(Tok::RBrace);
                i += 1;
            }
            ':' => {
                toks.push(Tok::Colon);
                i += 1;
            }
            ';' => {
                toks.push(Tok::Semi);
                i += 1;
            }
            ',' => {
                toks.push(Tok::Comma);
                i += 1;
            }
            '*' => {
                toks.push(Tok::Star);
                i += 1;
            }
            '"' => {
                // string
                let mut s = String::new();
                i += 1;
                while i < n && chars[i] != '"' {
                    s.push(chars[i]);
                    i += 1;
                }
                i += 1; // tutup "
                toks.push(Tok::Str(s));
            }
            c if c.is_ascii_digit() || c == '-' || c == '+' || c == '.' => {
                // angka
                let start = i;
                if c == '-' || c == '+' {
                    i += 1;
                }
                while i < n && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let s: String = chars[start..i].iter().collect();
                let v: f64 = s
                    .parse()
                    .map_err(|_| format!("angka tak valid: {s}"))?;
                toks.push(Tok::Num(v));
            }
            c if c.is_alphanumeric() || c == '_' || c == '/' || c == '\\' || c == ':' => {
                // ident (bisa berisi / seperti nama sel skylake_stdcell)
                let start = i;
                while i < n
                    && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '/' | '\\' | ':' | '-' | '+' | '.'))
                {
                    i += 1;
                }
                let s: String = chars[start..i].iter().collect();
                toks.push(Tok::Ident(s));
            }
            other => return Err(format!("karakter tak dikenal: {other}")),
        }
    }
    Ok(toks)
}

// ────────────────────────────────────────────────────────────────
// Parser (recursive descent ringan atas token)
// ────────────────────────────────────────────────────────────────

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn advance(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    /// Parse argumen grup `name ( ... ) { ... }` — kembalikan token args
    /// (di antara parens) sebagai string sumber.
    fn parse_group_args(&mut self) -> Result<Vec<Tok>, String> {
        self.expect(&Tok::LParen)?;
        let mut args = Vec::new();
        loop {
            match self.peek() {
                Some(Tok::RParen) => {
                    self.advance();
                    break;
                }
                Some(_) => args.push(self.advance().unwrap()),
                None => return Err("unexpected EOF di grup args".into()),
            }
        }
        Ok(args)
    }

    fn expect(&mut self, t: &Tok) -> Result<(), String> {
        match self.peek() {
            Some(x) if x == t => {
                self.advance();
                Ok(())
            }
            other => Err(format!("expected {t:?}, found {other:?}")),
        }
    }
}

/// Item body: atribut `name : value ;` atau grup `name (...) { ... }`.
#[derive(Debug, Clone)]
enum Item {
    /// `name : value ;` — value disimpan sebagai string.
    Attr(String, String),
    /// `name (args) { body }`.
    Group {
        name: String,
        args: Vec<Tok>,
        body: Vec<Item>,
    },
}

impl Item {
    fn attr_name(&self) -> Option<&str> {
        match self {
            Item::Attr(n, _) => Some(n),
            _ => None,
        }
    }
}

/// Parse body `{ ... }` sampai `}`.
fn parse_body(p: &mut Parser) -> Result<Vec<Item>, String> {
    let mut items = Vec::new();
    p.expect(&Tok::LBrace)?;
    loop {
        match p.peek() {
            None => return Err("unexpected EOF di body".into()),
            Some(Tok::RBrace) => {
                p.advance();
                break;
            }
            Some(_) => {
                // nama grup/atribut
                let name = match p.advance() {
                    Some(Tok::Ident(n)) | Some(Tok::Str(n)) => n,
                    Some(Tok::Star) => "*".to_string(),
                    other => return Err(format!("expected ident di body, found {other:?}")),
                };
                match p.peek() {
                    // atribut `name : value ;`
                    Some(Tok::Colon) => {
                        p.advance();
                        let value = parse_value(p)?;
                        p.expect(&Tok::Semi).ok(); // toleran tanpa ;
                        items.push(Item::Attr(name, value));
                    }
                    // grup `name ( ... ) { ... }` — atau statement `name (...);`
                    Some(Tok::LParen) => {
                        let args = p.parse_group_args()?;
                        match p.peek() {
                            Some(Tok::LBrace) => {
                                let body = parse_body(p)?;
                                items.push(Item::Group { name, args, body });
                            }
                            // `technology (cmos);` / `values ("0.5");` — statement
                            // tanpa body: simpan args agar nilai tetap terbaca
                            // (dipakai `values` di timing arc).
                            Some(Tok::Semi) => {
                                p.advance();
                                items.push(Item::Group {
                                    name,
                                    args,
                                    body: vec![],
                                });
                            }
                            _ => {
                                p.expect(&Tok::Semi).ok();
                            }
                        }
                    }
                    // grup tanpa args `name { ... }`? Liberty selalu pakai ().
                    Some(Tok::LBrace) => {
                        let body = parse_body(p)?;
                        items.push(Item::Group { name, args: vec![], body });
                    }
                    _ => {
                        // toleran: skip sampai `;`
                        while !matches!(p.peek(), Some(Tok::Semi) | None) {
                            p.advance();
                        }
                        p.advance();
                    }
                }
            }
        }
    }
    Ok(items)
}

/// Parse nilai atribut sampai `;` (string/angka/ident/list) → string sumber.
fn parse_value(p: &mut Parser) -> Result<String, String> {
    let mut s = String::new();
    loop {
        match p.peek() {
            Some(Tok::Semi) | None => break,
            Some(t) => {
                let part = match t {
                    Tok::Ident(v) | Tok::Str(v) => v.clone(),
                    Tok::Num(v) => v.to_string(),
                    Tok::LParen => "(".to_string(),
                    Tok::RParen => ")".to_string(),
                    Tok::LBrace => "{".to_string(),
                    Tok::RBrace => "}".to_string(),
                    Tok::Colon => ":".to_string(),
                    Tok::Comma => ",".to_string(),
                    Tok::Star => "*".to_string(),
                    Tok::Semi => break,
                };
                if !s.is_empty() {
                    s.push(' ');
                }
                s.push_str(&part);
                p.advance();
            }
        }
    }
    Ok(s)
}

// ────────────────────────────────────────────────────────────────
// Semantik: Items → LibertyLibrary
// ────────────────────────────────────────────────────────────────

/// Parse teks `.lib` → `LibertyLibrary`.
pub fn parse_liberty(text: &str) -> Result<LibertyLibrary, String> {
    let toks = tokenize(text)?;
    let mut p = Parser { toks, pos: 0 };
    let mut lib: Option<LibertyLibrary> = None;

    // Top-level: satu grup `library (name) { ... }`.
    while let Some(item) = parse_top_item(&mut p)? {
        if let Item::Group { name, args, body } = item {
            if name == "library" {
                let lib_name = match args.first() {
                    Some(Tok::Str(s)) | Some(Tok::Ident(s)) => s.clone(),
                    _ => "library".to_string(),
                };
                lib = Some(build_library(lib_name, &body)?);
            }
        }
    }
    lib.ok_or_else(|| "tidak ada grup `library` di file .lib".to_string())
}

/// Parse satu item top-level (abaikan yang bukan library).
fn parse_top_item(p: &mut Parser) -> Result<Option<Item>, String> {
    match p.peek() {
        None => Ok(None),
        Some(Tok::RBrace) => {
            // } ekstra di top-level — toleran
            p.advance();
            Ok(None)
        }
        Some(_) => {
            let name = match p.advance() {
                Some(Tok::Ident(n)) | Some(Tok::Str(n)) => n,
                other => return Err(format!("expected ident top-level, found {other:?}")),
            };
            match p.peek() {
                Some(Tok::LParen) => {
                    let args = p.parse_group_args()?;
                    match p.peek() {
                        Some(Tok::LBrace) => {
                            let body = parse_body(p)?;
                            Ok(Some(Item::Group { name, args, body }))
                        }
                        _ => {
                            p.expect(&Tok::Semi).ok();
                            Ok(None)
                        }
                    }
                }
                Some(Tok::Colon) => {
                    p.advance();
                    let _ = parse_value(p)?;
                    p.expect(&Tok::Semi).ok();
                    Ok(None)
                }
                _ => {
                    p.expect(&Tok::Semi).ok();
                    Ok(None)
                }
            }
        }
    }
}

fn build_library(name: String, body: &[Item]) -> Result<LibertyLibrary, String> {
    let mut lib = LibertyLibrary {
        name,
        delay_model: String::new(),
        time_unit: "1ns".to_string(),
        cells: Vec::new(),
    };
    for item in body {
        match item {
            Item::Attr(n, v) => {
                if n == "delay_model" {
                    lib.delay_model = v.trim_matches('"').to_string();
                } else if n == "time_unit" {
                    lib.time_unit = v.trim_matches('"').to_string();
                }
            }
            Item::Group { name: gn, body, .. } => {
                if gn == "cell" {
                    let cell_name = item
                        .args_str()
                        .unwrap_or_default();
                    if let Some(c) = build_cell(&cell_name, body)? {
                        lib.cells.push(c);
                    }
                }
            }
        }
    }
    Ok(lib)
}

impl Item {
    /// Argumen grup sebagai string (dipakai nama cell).
    fn args_str(&self) -> Option<String> {
        match self {
            Item::Group { args, .. } => {
                let mut s = String::new();
                for t in args {
                    match t {
                        Tok::Ident(v) | Tok::Str(v) => s.push_str(v),
                        Tok::Num(v) => {
                            let _ = write!(s, "{v}");
                        }
                        _ => {}
                    }
                }
                Some(s)
            }
            _ => None,
        }
    }
}

fn build_cell(name: &str, body: &[Item]) -> Result<Option<LibertyCell>, String> {
    let mut cell = LibertyCell {
        name: name.to_string(),
        area: 0.0,
        footprint: None,
        pins: Vec::new(),
    };
    for item in body {
        match item {
            Item::Attr(n, v) => {
                if n == "area" {
                    cell.area = v
                        .trim()
                        .parse::<f64>()
                        .map_err(|_| format!("area tak valid: {v}"))?;
                } else if n == "cell_footprint" {
                    cell.footprint = Some(v.trim_matches('"').to_string());
                }
            }
            Item::Group { name: gn, body, .. } => {
                if gn == "pin" {
                    let pin_name = item.args_str().unwrap_or_default();
                    if let Some(pin) = build_pin(&pin_name, body)? {
                        cell.pins.push(pin);
                    }
                }
            }
        }
    }
    Ok(Some(cell))
}

fn build_pin(name: &str, body: &[Item]) -> Result<Option<LibertyPin>, String> {
    let mut pin = LibertyPin {
        name: name.to_string(),
        direction: PinDir::Inout,
        capacitance: 0.0,
        function: None,
        timings: Vec::new(),
    };
    for item in body {
        match item {
            Item::Attr(n, v) => {
                match n.as_str() {
                    "direction" => {
                        let d = v.trim_matches('"');
                        pin.direction = match d {
                            "input" => PinDir::Input,
                            "output" => PinDir::Output,
                            "inout" => PinDir::Inout,
                            other => return Err(format!("direction tak dikenal: {other}")),
                        };
                    }
                    "capacitance" => {
                        pin.capacitance = v
                            .trim()
                            .parse::<f64>()
                            .unwrap_or(0.0);
                    }
                    "function" => {
                        pin.function = Some(v.trim().to_string());
                    }
                    _ => {}
                }
            }
            Item::Group { name: gn, body, .. } => {
                if gn == "timing" {
                    if let Some(arc) = build_timing(body) {
                        pin.timings.push(arc);
                    }
                }
            }
        }
    }
    Ok(Some(pin))
}

/// Parse satu `timing () { ... }` → TimingArc (delay scalar).
fn build_timing(body: &[Item]) -> Option<TimingArc> {
    let mut arc = TimingArc {
        related_pin: String::new(),
        rise_delay_ns: None,
        fall_delay_ns: None,
    };
    for item in body {
        match item {
            Item::Attr(n, v) => {
                if n == "related_pin" {
                    arc.related_pin = v.trim_matches('"').to_string();
                }
            }
            Item::Group { name: gn, body, .. } => {
                // rise_propagation_delay (scalar) { values ("0.5"); }
                if gn == "rise_propagation_delay" || gn == "fall_propagation_delay" {
                    if let Some(v) = first_value(body) {
                        let slot = if gn == "rise_propagation_delay" {
                            &mut arc.rise_delay_ns
                        } else {
                            &mut arc.fall_delay_ns
                        };
                        *slot = Some(v);
                    }
                }
            }
        }
    }
    Some(arc)
}

/// Nilai pertama dari `values ("0.5", ...)` di body grup.
fn first_value(body: &[Item]) -> Option<f64> {
    for item in body {
        // `values ("0.5");` — grup tanpa body (args berisi nilai).
        if let Item::Group { name, args, .. } = item {
            if name == "values" {
                for t in args {
                    if let Tok::Num(v) = t {
                        return Some(*v);
                    }
                    if let Tok::Str(s) = t {
                        if let Ok(v) = s.trim().parse::<f64>() {
                            return Some(v);
                        }
                    }
                }
            }
        }
        // atribut langsung: `values : "0.5" ;`
        if let Item::Attr(n, v) = item {
            if n == "values" {
                return extract_first_number(v);
            }
        }
    }
    None
}

/// Ambil angka pertama dari string nilai (`"0.5"`, `"0.5" "0.7"`, `(0.5)`).
fn extract_first_number(s: &str) -> Option<f64> {
    s.split(|c: char| !c.is_ascii_digit() && c != '.' && c != '-' && c != '+')
        .filter(|t| !t.is_empty())
        .next()
        .and_then(|t| t.parse::<f64>().ok())
}

// ────────────────────────────────────────────────────────────────
// `.libmdb` — database deterministik (save/load)
// ────────────────────────────────────────────────────────────────

/// Simpan library ke format `.libmdb` (teks deterministik, ala `.mvnet`).
pub fn save_mdb(lib: &LibertyLibrary, path: &Path) -> std::io::Result<()> {
    let mut s = String::new();
    s.push_str("MARIA-LIB-MDB v1\n");
    s.push_str(&format!("name: {}\n", lib.name));
    s.push_str(&format!("delay_model: {}\n", lib.delay_model));
    s.push_str(&format!("time_unit: {}\n", lib.time_unit));
    for c in &lib.cells {
        s.push_str(&format!("cell: {}\n", c.name));
        s.push_str(&format!("  area: {}\n", c.area));
        if let Some(fp) = &c.footprint {
            s.push_str(&format!("  footprint: {}\n", fp));
        }
        for p in &c.pins {
            s.push_str(&format!(
                "  pin: {} dir={} cap={}\n",
                p.name,
                p.direction.name(),
                p.capacitance
            ));
            if let Some(f) = &p.function {
                s.push_str(&format!("    fn: {}\n", f));
            }
            for t in &p.timings {
                s.push_str(&format!(
                    "    timing: {} rise={:?} fall={:?}\n",
                    t.related_pin, t.rise_delay_ns, t.fall_delay_ns
                ));
            }
        }
    }
    std::fs::write(path, s)
}

/// Muat library dari `.libmdb`.
pub fn load_mdb(path: &Path) -> std::io::Result<LibertyLibrary> {
    let text = std::fs::read_to_string(path)?;
    let mut lib = LibertyLibrary {
        name: String::new(),
        delay_model: String::new(),
        time_unit: "1ns".to_string(),
        cells: Vec::new(),
    };
    let mut cell: Option<LibertyCell> = None;
    let mut pin_name: Option<String> = None;
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with("MARIA-LIB") {
            continue;
        }
        if let Some(v) = t.strip_prefix("name: ") {
            lib.name = v.to_string();
        } else if let Some(v) = t.strip_prefix("delay_model: ") {
            lib.delay_model = v.to_string();
        } else if let Some(v) = t.strip_prefix("time_unit: ") {
            lib.time_unit = v.to_string();
        } else if let Some(v) = t.strip_prefix("cell: ") {
            if let Some(c) = cell.take() {
                lib.cells.push(c);
            }
            cell = Some(LibertyCell {
                name: v.to_string(),
                area: 0.0,
                footprint: None,
                pins: Vec::new(),
            });
            pin_name = None;
        } else if let Some(v) = t.strip_prefix("area: ") {
            if let Some(c) = &mut cell {
                c.area = v.trim().parse::<f64>().unwrap_or(0.0);
            }
        } else if let Some(v) = t.strip_prefix("footprint: ") {
            if let Some(c) = &mut cell {
                c.footprint = Some(v.to_string());
            }
        } else if let Some(v) = t.strip_prefix("pin: ") {
            if let Some(c) = &mut cell {
                let mut parts = v.split_whitespace();
                let name = parts.next().unwrap_or("").to_string();
                let dir = match parts.next() {
                    Some(d) if d.starts_with("dir=") => match &d[4..] {
                        "in" => PinDir::Input,
                        "out" => PinDir::Output,
                        _ => PinDir::Inout,
                    },
                    _ => PinDir::Inout,
                };
                let cap = parts
                    .next()
                    .and_then(|c| c.strip_prefix("cap="))
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                c.pins.push(LibertyPin {
                    name: name.clone(),
                    direction: dir,
                    capacitance: cap,
                    function: None,
                    timings: Vec::new(),
                });
                pin_name = Some(name);
            }
        } else if let Some(v) = t.strip_prefix("fn: ") {
            if let Some(c) = &mut cell {
                if let Some(p) = find_pin_mut(c, &pin_name) {
                    p.function = Some(v.to_string());
                }
            }
        } else if let Some(v) = t.strip_prefix("timing: ") {
            if let Some(c) = &mut cell {
                if let Some(p) = find_pin_mut(c, &pin_name) {
                    let mut parts = v.split_whitespace();
                    let rel = parts.next().unwrap_or("").to_string();
                    let rise = parts
                        .next()
                        .and_then(|t| t.strip_prefix("rise="))
                        .and_then(|t| parse_opt_f64(t));
                    let fall = parts
                        .next()
                        .and_then(|t| t.strip_prefix("fall="))
                        .and_then(|t| parse_opt_f64(t));
                    p.timings.push(TimingArc {
                        related_pin: rel,
                        rise_delay_ns: rise,
                        fall_delay_ns: fall,
                    });
                }
            }
        }
    }
    if let Some(c) = cell.take() {
        lib.cells.push(c);
    }
    Ok(lib)
}

/// Cari pin mutable di cell berdasarkan nama (baris `fn:`/`timing:` di `.libmdb`).
fn find_pin_mut<'a>(c: &'a mut LibertyCell, name: &Option<String>) -> Option<&'a mut LibertyPin> {
    let n = name.as_ref()?;
    c.pins.iter_mut().find(|p| &p.name == n)
}

fn parse_opt_f64(s: &str) -> Option<f64> {
    if s == "None" {
        return None;
    }
    // Terima `0.5` maupun `Some(0.5)` (format Debug `{:?}` dari save_mdb).
    let t = s
        .strip_prefix("Some(")
        .and_then(|t| t.strip_suffix(')'))
        .unwrap_or(s);
    t.parse::<f64>().ok()
}

// ────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
/* skylake generic cell library — subset untuk fase 6 */
library (generic) {
  technology (cmos);
  delay_model : table_lookup;
  time_unit : "1ns";
  voltage_unit : "1V";
  current_unit : "1mA";
  capacitive_load_unit (1, "pf");

  cell (NAND2_X1) {
    area : 1.2;
    cell_footprint : "nand2";
    pin (A) {
      direction : input;
      capacitance : 0.003;
    }
    pin (B) {
      direction : input;
      capacitance : 0.003;
    }
    pin (Y) {
      direction : output;
      function : "(A & B)";
      timing () {
        related_pin : "A";
        timing_sense : negative_unate;
        rise_propagation_delay (scalar) {
          values ("0.5");
        }
        fall_propagation_delay (scalar) {
          values ("0.6");
        }
      }
    }
  }

  cell (INV_X1) {
    area : 0.4;
    pin (A) { direction : input; }
    pin (Y) {
      direction : output;
      function : "(!A)";
      timing () {
        related_pin : "A";
        rise_propagation_delay (scalar) { values ("0.2"); }
        fall_propagation_delay (scalar) { values ("0.3"); }
      }
    }
  }
}
"#;

    #[test]
    fn parse_library_metadata() {
        let lib = parse_liberty(SAMPLE).expect("parse");
        assert_eq!(lib.name, "generic");
        assert_eq!(lib.delay_model, "table_lookup");
        assert_eq!(lib.time_unit, "1ns");
        assert_eq!(lib.cells.len(), 2);
    }

    #[test]
    fn parse_cell_area_and_pins() {
        let lib = parse_liberty(SAMPLE).expect("parse");
        let nand = lib.cell("NAND2_X1").expect("cell NAND2_X1");
        assert!((nand.area - 1.2).abs() < 1e-9, "area = 1.2, dapat {}", nand.area);
        assert_eq!(nand.footprint.as_deref(), Some("nand2"));
        assert_eq!(nand.pins.len(), 3);
        let a = nand.pin("A").expect("pin A");
        assert_eq!(a.direction, PinDir::Input);
        assert!((a.capacitance - 0.003).abs() < 1e-9);
        let y = nand.pin("Y").expect("pin Y");
        assert_eq!(y.direction, PinDir::Output);
        assert_eq!(y.function.as_deref(), Some("(A & B)"));
        // Arc timing A→Y: rise 0.5, fall 0.6.
        assert!((nand.arc_delay_ns("A", "Y").expect("arc") - 0.6).abs() < 1e-9);
        let arc = &y.timings[0];
        assert_eq!(arc.related_pin, "A");
        assert!((arc.rise_delay_ns.expect("rise") - 0.5).abs() < 1e-9);
        assert!((arc.fall_delay_ns.expect("fall") - 0.6).abs() < 1e-9);
    }

    #[test]
    fn parse_inv_negative_function() {
        let lib = parse_liberty(SAMPLE).expect("parse");
        let inv = lib.cell("INV_X1").expect("cell INV_X1");
        assert!((inv.area - 0.4).abs() < 1e-9);
        assert_eq!(inv.pin("Y").expect("Y").function.as_deref(), Some("(!A)"));
        assert!((inv.arc_delay_ns("A", "Y").expect("arc") - 0.3).abs() < 1e-9);
    }

    #[test]
    fn mdb_roundtrip() {
        let lib = parse_liberty(SAMPLE).expect("parse");
        let path = std::env::temp_dir().join("maria_test_generic.libmdb");
        save_mdb(&lib, &path).expect("save mdb");
        let loaded = load_mdb(&path).expect("load mdb");
        assert_eq!(loaded.name, lib.name);
        assert_eq!(loaded.cells.len(), lib.cells.len());
        let n1 = loaded.cell("NAND2_X1").expect("nand");
        assert!((n1.area - 1.2).abs() < 1e-9);
        let y = n1.pin("Y").expect("y");
        assert_eq!(y.function.as_deref(), Some("(A & B)"));
        assert!((y.timings[0].rise_delay_ns.expect("rise") - 0.5).abs() < 1e-9);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn parse_unknown_tolerated() {
        // Grup/atribut di luar subset di-skip tanpa error.
        let lib = parse_liberty(
            "library (x) { delay_model : table_lookup; leakage_power () { } \
             cell (C) { area : 1; pin (A) { direction : input; } } }",
        )
        .expect("parse toleran");
        assert_eq!(lib.cells.len(), 1);
    }
}
