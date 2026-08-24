//! SDF (Standard Delay Format) parser for timing annotation.
//!
//! Supports IEEE 1497 SDF constructs including:
//! - DELAYFILE header
//! - CELL/DELAYCELL with IOPATH delays
//! - NET/DELAYNET with ABSDELAY
//! - TIMINGCHECK (SETUP, HOLD, WIDTH, PERIOD, etc.)
//! - min:typ:max triple values
//! - Conditional delays (COND)
//!
//! # Min:Typ:Max
//!
//! SDF delays can be specified as triples: (min:typ:max).
//! The `TimingMode` enum selects which value to use:
//! - `Min`: Best-case / fastest
//! - `Typ`: Typical
//! - `Max`: Worst-case / slowest

use std::collections::HashMap;
use std::fs;

/// Timing mode for selecting min:typ:max values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimingMode {
    Min,
    Typ,
    Max,
}

impl TimingMode {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "min" => Some(TimingMode::Min),
            "typ" | "typical" => Some(TimingMode::Typ),
            "max" => Some(TimingMode::Max),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            TimingMode::Min => "min",
            TimingMode::Typ => "typ",
            TimingMode::Max => "max",
        }
    }
}

/// A min:typ:max triple value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MinTypMax {
    pub min: f64,
    pub typ: f64,
    pub max: f64,
}

impl MinTypMax {
    pub fn new(min: f64, typ: f64, max: f64) -> Self {
        MinTypMax { min, typ, max }
    }

    pub fn single(val: f64) -> Self {
        MinTypMax {
            min: val,
            typ: val,
            max: val,
        }
    }

    /// Get the value for the given timing mode.
    pub fn get(&self, mode: TimingMode) -> f64 {
        match mode {
            TimingMode::Min => self.min,
            TimingMode::Typ => self.typ,
            TimingMode::Max => self.max,
        }
    }
}

/// Cell delay with per-path rise/fall and min:typ:max support.
#[derive(Debug, Clone)]
pub struct CellDelay {
    /// IOPATH delays keyed by "from->to"
    pub io_paths: HashMap<String, IoPathDelay>,
    /// Conditional delays keyed by "from->to" with COND expression
    pub cond_paths: Vec<(String, String, IoPathDelay)>,
}

#[derive(Debug, Clone)]
pub struct IoPathDelay {
    pub rise: MinTypMax,
    pub fall: MinTypMax,
}

/// Net delay with min:typ:max support.
#[derive(Debug, Clone)]
pub struct NetDelay {
    pub rise: MinTypMax,
    pub fall: MinTypMax,
}

/// Timing check with min:typ:max delay support.
#[derive(Debug, Clone)]
pub enum TimingCheck {
    Setup {
        signal: String,
        ref_signal: String,
        delay: MinTypMax,
    },
    Hold {
        signal: String,
        ref_signal: String,
        delay: MinTypMax,
    },
    Setuphold {
        signal: String,
        ref_signal: String,
        setup: MinTypMax,
        hold: MinTypMax,
    },
    Width {
        signal: String,
        delay: MinTypMax,
        threshold: Option<MinTypMax>,
    },
    Period {
        signal: String,
        delay: MinTypMax,
    },
    Recovery {
        signal: String,
        ref_signal: String,
        delay: MinTypMax,
    },
    Removal {
        signal: String,
        ref_signal: String,
        delay: MinTypMax,
    },
    Skew {
        signal: String,
        ref_signal: String,
        delay: MinTypMax,
    },
}

impl TimingCheck {
    /// Get the delay value for the given timing mode.
    pub fn delay_value(&self, mode: TimingMode) -> f64 {
        match self {
            TimingCheck::Setup { delay, .. } => delay.get(mode),
            TimingCheck::Hold { delay, .. } => delay.get(mode),
            TimingCheck::Setuphold { setup, .. } => setup.get(mode),
            TimingCheck::Width { delay, .. } => delay.get(mode),
            TimingCheck::Period { delay, .. } => delay.get(mode),
            TimingCheck::Recovery { delay, .. } => delay.get(mode),
            TimingCheck::Removal { delay, .. } => delay.get(mode),
            TimingCheck::Skew { delay, .. } => delay.get(mode),
        }
    }

    /// Get the name of this timing check type.
    pub fn type_name(&self) -> &'static str {
        match self {
            TimingCheck::Setup { .. } => "$setup",
            TimingCheck::Hold { .. } => "$hold",
            TimingCheck::Setuphold { .. } => "$setuphold",
            TimingCheck::Width { .. } => "$width",
            TimingCheck::Period { .. } => "$period",
            TimingCheck::Recovery { .. } => "$recovery",
            TimingCheck::Removal { .. } => "$removal",
            TimingCheck::Skew { .. } => "$skew",
        }
    }
}

/// Pulse control (SIM-09): `(PULSE (PULSE_WIDTH (PORT "x") (1.0:2.0:3.0)))` —
/// lebar pulse minimum yang diterima pada port. Pulse lebih pendek dari
/// batas di-reject (di-filter) bila pulse control aktif — implementasi
/// engine: nilai sinyal di-rollback ke nilai sebelum pulse (bukan violation).
#[derive(Debug, Clone)]
pub struct PulseControl {
    pub signal: String,
    pub width: MinTypMax,
}

/// SDF (Standard Delay Format) data container.
#[derive(Debug, Clone)]
pub struct SdfData {
    pub cell_delays: HashMap<String, CellDelay>,
    pub net_delays: HashMap<String, NetDelay>,
    pub timing_checks: Vec<TimingCheck>,
    pub pulse_controls: HashMap<String, PulseControl>,
    pub sdf_version: Option<String>,
    pub design_name: Option<String>,
    pub date: Option<String>,
    pub vendor: Option<String>,
    pub program_name: Option<String>,
    pub program_version: Option<String>,
    pub hier_divider: Option<String>,
    pub voltage: Option<f64>,
    pub process: Option<f64>,
    pub temperature: Option<f64>,
    pub timescale: Option<String>,
}

impl SdfData {
    pub fn new() -> Self {
        SdfData {
            cell_delays: HashMap::new(),
            net_delays: HashMap::new(),
            timing_checks: Vec::new(),
            pulse_controls: HashMap::new(),
            sdf_version: None,
            design_name: None,
            date: None,
            vendor: None,
            program_name: None,
            program_version: None,
            hier_divider: None,
            voltage: None,
            process: None,
            temperature: None,
            timescale: None,
        }
    }

    pub fn parse_file(path: &str) -> Result<Self, String> {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("cannot read SDF file '{}': {}", path, e))?;
        Self::parse(&content)
    }

    pub fn parse(content: &str) -> Result<Self, String> {
        let mut sdf = SdfData::new();
        let tokens = tokenize(content);
        let mut pos = 0;

        while pos < tokens.len() {
            if tokens[pos] == "(" && pos + 1 < tokens.len() {
                match tokens[pos + 1].as_str() {
                    "DELAYFILE" => {
                        pos = parse_delayfile_header(&tokens, pos, &mut sdf);
                    }
                    "CELL" | "DELAYCELL" => {
                        pos = match parse_cell(&tokens, pos, &mut sdf.pulse_controls) {
                            Ok((name, delay, cell_checks, new_pos)) => {
                                sdf.cell_delays.insert(name, delay);
                                sdf.timing_checks.extend(cell_checks);
                                new_pos
                            }
                            Err(_) => {
                                // Skip to matching )
                                let mut depth = 0;
                                while pos < tokens.len() {
                                    if tokens[pos] == "(" {
                                        depth += 1;
                                    }
                                    if tokens[pos] == ")" {
                                        depth -= 1;
                                    }
                                    if depth < 0 {
                                        break;
                                    }
                                    pos += 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                pos
                            }
                        };
                    }
                    "PULSE" => {
                        pos = parse_pulse_construct(&tokens, pos, &mut sdf.pulse_controls);
                    }
                    "NET" | "DELAYNET" => {
                        pos = match parse_net(&tokens, pos) {
                            Ok((name, delay, new_pos)) => {
                                sdf.net_delays.insert(name, delay);
                                new_pos
                            }
                            Err(_) => {
                                let mut depth = 0;
                                while pos < tokens.len() {
                                    if tokens[pos] == "(" {
                                        depth += 1;
                                    }
                                    if tokens[pos] == ")" {
                                        depth -= 1;
                                    }
                                    if depth < 0 {
                                        break;
                                    }
                                    pos += 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                pos
                            }
                        };
                    }
                    "TIMINGCHECK" => {
                        pos = match parse_timing_checks(&tokens, pos) {
                            Ok((checks, new_pos)) => {
                                sdf.timing_checks.extend(checks);
                                new_pos
                            }
                            Err(_) => {
                                let mut depth = 0;
                                while pos < tokens.len() {
                                    if tokens[pos] == "(" {
                                        depth += 1;
                                    }
                                    if tokens[pos] == ")" {
                                        depth -= 1;
                                    }
                                    if depth < 0 {
                                        break;
                                    }
                                    pos += 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                pos
                            }
                        };
                    }
                    "INTERCONNECT" => {
                        // Skip INTERCONNECT statements (not yet supported)
                        let mut inner = 0;
                        while pos < tokens.len() {
                            if tokens[pos] == "(" {
                                inner += 1;
                            }
                            if tokens[pos] == ")" {
                                inner -= 1;
                            }
                            if inner < 0 {
                                break;
                            }
                            pos += 1;
                            if inner == 0 {
                                break;
                            }
                        }
                    }
                    _ => {
                        pos += 1;
                    }
                }
            } else {
                pos += 1;
            }
        }

        Ok(sdf)
    }
}

/// Thread-local timing mode for SDF annotation.
use std::cell::RefCell;
thread_local! {
    static CURRENT_TIMING_MODE: RefCell<TimingMode> = const { RefCell::new(TimingMode::Typ) };
}

/// Set the current timing mode for delay selection.
pub fn set_timing_mode(mode: TimingMode) {
    CURRENT_TIMING_MODE.with(|cell| *cell.borrow_mut() = mode);
}

/// Get the current timing mode.
pub fn get_timing_mode() -> TimingMode {
    CURRENT_TIMING_MODE.with(|cell| *cell.borrow())
}

// ─── Tokenizer ───

fn tokenize(content: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_block_comment = false;
    let mut in_line_comment = false;
    let mut in_string = false;
    let mut chars = content.chars().peekable();

    while let Some(c) = chars.next() {
        if in_block_comment {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_block_comment = false;
            }
            continue;
        }
        if in_line_comment {
            if c == '\n' {
                in_line_comment = false;
            }
            continue;
        }
        if in_string {
            if c == '"' {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
                in_string = false;
            } else {
                current.push(c);
            }
            continue;
        }
        match c {
            '/' if chars.peek() == Some(&'*') => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
                chars.next();
                in_block_comment = true;
            }
            '/' if chars.peek() == Some(&'/') => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
                in_line_comment = true;
            }
            '(' | ')' => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
                tokens.push(c.to_string());
            }
            '"' => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
                in_string = true;
            }
            ':' | '\\' => {
                current.push(c);
            }
            ' ' | '\t' | '\r' | '\n' => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
            }
            _ => {
                current.push(c);
            }
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

// ─── Parser Helpers ───

/// Parse a min:typ:max triple or single value.
fn parse_min_typ_max(tokens: &[String], pos: &mut usize) -> Result<MinTypMax, String> {
    let start = *pos;
    if *pos >= tokens.len() {
        return Err("unexpected end of tokens".to_string());
    }

    // Try to parse as triple (min:typ:max)
    let token = &tokens[*pos];
    if let Some(triple) = parse_triple_token(token) {
        *pos += 1;
        return Ok(triple);
    }

    // Try to parse as single float
    if let Ok(val) = token.parse::<f64>() {
        *pos += 1;
        // Check for colon prefix (after a value)
        return Ok(MinTypMax::single(val));
    }

    Err(format!(
        "expected numeric value at token {}: '{}'",
        start, token
    ))
}

/// Parse a "min:typ:max" triple from a single token (after backslash continuation).
fn parse_triple_token(token: &str) -> Option<MinTypMax> {
    let parts: Vec<String> = if token.contains('\\') {
        // Handle continuation lines
        let cleaned: String = token
            .chars()
            .filter(|&c| c != '\\' && c != '\n' && c != '\r')
            .collect();
        cleaned
            .split(':')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        token.split(':').map(|s| s.trim().to_string()).collect()
    };

    if parts.len() == 3 {
        let min = parts[0].parse::<f64>().ok()?;
        let typ = parts[1].parse::<f64>().ok()?;
        let max = parts[2].parse::<f64>().ok()?;
        Some(MinTypMax { min, typ, max })
    } else if parts.len() == 1 {
        parts[0].parse::<f64>().ok().map(MinTypMax::single)
    } else {
        None
    }
}

/// Parse rise/fall delay values.
/// SDF format: (rise_value) (fall_value) — each in its own parenthesized group.
fn parse_rise_fall(tokens: &[String], pos: &mut usize) -> Result<(MinTypMax, MinTypMax), String> {
    if *pos >= tokens.len() {
        return Err("unexpected end of tokens".to_string());
    }

    if tokens[*pos] == "(" {
        let rise = parse_value_in_parens(tokens, pos)?;
        let fall = parse_value_in_parens(tokens, pos)?;
        Ok((rise, fall))
    } else {
        let val = parse_min_typ_max(tokens, pos)?;
        Ok((val, val))
    }
}

/// Parse a single delay value enclosed in parentheses: (value).
fn parse_value_in_parens(tokens: &[String], pos: &mut usize) -> Result<MinTypMax, String> {
    if *pos < tokens.len() && tokens[*pos] == "(" {
        *pos += 1;
    }
    let val = parse_min_typ_max(tokens, pos)?;
    if *pos < tokens.len() && tokens[*pos] == ")" {
        *pos += 1;
    }
    Ok(val)
}

/// Parse DELAYFILE header items.
/// NOTE: Outer depth ONLY decrements for `)`. Children `(` are NOT counted
/// because inner skip loops consume child `)` tokens.
fn parse_delayfile_header(tokens: &[String], mut pos: usize, sdf: &mut SdfData) -> usize {
    pos += 2; // skip (DELAYFILE
    let mut depth = 1;
    while pos < tokens.len() && depth > 0 {
        // Only decrement depth for ) — child ( are tracked + consumed by inner skip
        if tokens[pos] == ")" {
            depth -= 1;
            if depth == 0 {
                break;
            }
            pos += 1;
            continue;
        }

        if tokens[pos] == "(" && pos + 2 < tokens.len() {
            let keyword = &tokens[pos + 1];
            let label = &tokens[pos + 2];
            let known = match keyword.as_str() {
                "SDFVERSION" => {
                    sdf.sdf_version = Some(label.trim_matches('"').to_string());
                    true
                }
                "DESIGN" => {
                    sdf.design_name = Some(label.trim_matches('"').to_string());
                    true
                }
                "DATE" => {
                    sdf.date = Some(label.trim_matches('"').to_string());
                    true
                }
                "VENDOR" => {
                    sdf.vendor = Some(label.trim_matches('"').to_string());
                    true
                }
                "PROGRAM" => {
                    sdf.program_name = Some(label.trim_matches('"').to_string());
                    true
                }
                "VERSION" => {
                    sdf.program_version = Some(label.trim_matches('"').to_string());
                    true
                }
                "DIVIDER" => {
                    sdf.hier_divider = Some(label.trim_matches('"').to_string());
                    true
                }
                "VOLTAGE" => {
                    let val = label.trim_matches('"').parse::<f64>().ok();
                    sdf.voltage = val;
                    true
                }
                "PROCESS" => {
                    let val = label.trim_matches('"').parse::<f64>().ok();
                    sdf.process = val;
                    true
                }
                "TEMPERATURE" => {
                    let val = label.trim_matches('"').parse::<f64>().ok();
                    sdf.temperature = val;
                    true
                }
                "TIMESCALE" => {
                    sdf.timescale = Some(label.trim_matches('"').to_string());
                    true
                }
                _ => false,
            };
            if known {
                // Skip to matching ) — consume the (keyword value) construct
                let mut inner = 0;
                while pos < tokens.len() {
                    if tokens[pos] == "(" {
                        inner += 1;
                    }
                    if tokens[pos] == ")" {
                        inner -= 1;
                    }
                    if inner < 0 {
                        break;
                    }
                    pos += 1;
                    if inner == 0 {
                        break;
                    }
                }
                continue;
            }
        }
        pos += 1;
    }
    if pos < tokens.len() {
        pos += 1;
    }
    pos
}

/// Parse a CELL/DELAYCELL construct.
/// Returns (cell_name, cell_delay, timing_checks, new_pos).
/// Parse `(PULSE (PULSE_WIDTH (PORT "x") (1.0:2.0:3.0)))` — SIM-09.
/// Port bisa dikutip (`"x"`) atau bare; delay skalar/triple. Satu PULSE
/// construct bisa berisi banyak PULSE_WIDTH (per-port).
fn parse_pulse_construct(
    tokens: &[String],
    mut pos: usize,
    out: &mut HashMap<String, PulseControl>,
) -> usize {
    // Struktur: `( PULSE ( PULSE_WIDTH ( PORT "q" ) ( 1.0:2.0:3.0 ) ) )`.
    // Scan semua PULSE_WIDTH child; kembalikan pos SETELAH `)` penutup PULSE
    // (depth balance eksplisit — hindari menelan construct berikutnya).
    pos += 2; // skip ( PULSE
    let mut depth = 1;
    while pos < tokens.len() && depth > 0 {
        let tok = tokens[pos].clone();
        if tok == "(" {
            depth += 1;
            if pos + 1 < tokens.len() && tokens[pos + 1] == "PULSE_WIDTH" {
                // Scan isi PULSE_WIDTH sampai paren penutupnya.
                let mut p = pos + 2;
                let mut inner = 1;
                let mut signal = String::new();
                let mut width = MinTypMax::single(0.0);
                while p < tokens.len() && inner > 0 {
                    if tokens[p] == "(" {
                        inner += 1;
                    }
                    if tokens[p] == ")" {
                        inner -= 1;
                    }
                    if inner == 0 {
                        break;
                    }
                    let trimmed = tokens[p].trim_matches('"');
                    if trimmed == "PORT" {
                        // Struktur `( PORT q )` — nama signal ada di p+1.
                        if p + 1 < tokens.len() {
                            signal = tokens[p + 1].trim_matches('"').to_string();
                        }
                    } else if let Some(mtm) = parse_triple_token(&tokens[p]) {
                        width = mtm;
                    } else if tokens[p].parse::<f64>().is_ok()
                        && width.min == 0.0
                        && width.typ == 0.0
                    {
                        width = MinTypMax::single(tokens[p].parse::<f64>().unwrap_or(0.0));
                    }
                    p += 1;
                }
                if !signal.is_empty() {
                    out.insert(signal.clone(), PulseControl { signal, width });
                }
                // p menunjuk `)` penutup PULSE_WIDTH (inner==0 saat decrement).
                // p+1 = token setelahnya; depth sudah naik utk `(` — turunkan utk `)`.
                pos = p + 1;
                depth -= 1;
                continue;
            }
            pos += 1;
            continue;
        }
        if tok == ")" {
            depth -= 1;
            if depth == 0 {
                break; // pos di `)` penutup PULSE
            }
            pos += 1;
            continue;
        }
        pos += 1;
    }
    if pos < tokens.len() {
        pos += 1;
    }
    pos
}

fn parse_cell(
    tokens: &[String],
    mut pos: usize,
    pulse_controls: &mut HashMap<String, PulseControl>,
) -> Result<(String, CellDelay, Vec<TimingCheck>, usize), String> {
    pos += 2; // skip (CELL or (DELAYCELL
    let mut name = "unknown".to_string();

    let mut delay = CellDelay {
        io_paths: HashMap::new(),
        cond_paths: Vec::new(),
    };
    let mut timing_checks = Vec::new();

    let mut depth = 1;
    while pos < tokens.len() && depth > 0 {
        if tokens[pos] == "(" {
            depth += 1;
        }
        if tokens[pos] == ")" {
            depth -= 1;
        }
        if depth == 0 {
            break;
        }

        if tokens[pos] == "(" && pos + 1 < tokens.len() {
            match tokens[pos + 1].as_str() {
                "PULSE" => {
                    pos = parse_pulse_construct(tokens, pos, pulse_controls);
                    continue;
                }
                "INSTANCE" => {
                    pos += 2;
                    if pos < tokens.len() && tokens[pos] != "(" {
                        name = tokens[pos].trim_matches('"').to_string();
                        pos += 1;
                    }
                    continue;
                }
                "DELAY" | "ABSOLUTE" | "INCREMENT" => {
                    pos += 2;
                    continue;
                }
                "IOPATH" => {
                    pos += 2;
                    let from = if pos < tokens.len() && tokens[pos] != "(" {
                        let s = tokens[pos].trim_matches('"').to_string();
                        pos += 1;
                        s
                    } else {
                        "*".to_string()
                    };
                    let to = if pos < tokens.len() && tokens[pos] != "(" {
                        let s = tokens[pos].trim_matches('"').to_string();
                        pos += 1;
                        s
                    } else {
                        "*".to_string()
                    };
                    let (rise, fall) = parse_rise_fall(tokens, &mut pos)?;
                    delay
                        .io_paths
                        .insert(format!("{}->{}", from, to), IoPathDelay { rise, fall });
                    continue;
                }
                "TIMINGCHECK" => {
                    match parse_timing_checks(tokens, pos) {
                        Ok((checks, new_pos)) => {
                            timing_checks.extend(checks);
                            pos = new_pos;
                        }
                        Err(_) => {
                            // Skip to matching )
                            let mut skip = 0;
                            while pos < tokens.len() {
                                if tokens[pos] == "(" {
                                    skip += 1;
                                }
                                if tokens[pos] == ")" {
                                    skip -= 1;
                                }
                                if skip < 0 {
                                    break;
                                }
                                pos += 1;
                                if skip == 0 {
                                    break;
                                }
                            }
                        }
                    }
                    continue;
                }
                "COND" => {
                    // Parse COND: (COND <expression> <delay_spec>)
                    // Skip (COND and the condition expression tokens
                    pos += 2; // skip (COND
                    let mut cond_depth = 0;
                    // Read until we find the IOPATH (skip condition expression)
                    while pos < tokens.len() {
                        if tokens[pos] == "(" {
                            cond_depth += 1;
                        }
                        if tokens[pos] == ")" {
                            cond_depth -= 1;
                        }
                        pos += 1;
                        if cond_depth < 0 {
                            // Found closing of whole COND block, back up
                            break;
                        }
                        if cond_depth == 1
                            && pos < tokens.len()
                            && tokens[pos] == "("
                            && pos + 1 < tokens.len()
                            && tokens[pos + 1] == "IOPATH"
                        {
                            // Found IOPATH inside COND — parse it
                            pos += 2; // skip (IOPATH
                            let from = if pos < tokens.len() && tokens[pos] != "(" {
                                let s = tokens[pos].trim_matches('"').to_string();
                                pos += 1;
                                s
                            } else {
                                "*".to_string()
                            };
                            let to = if pos < tokens.len() && tokens[pos] != "(" {
                                let s = tokens[pos].trim_matches('"').to_string();
                                pos += 1;
                                s
                            } else {
                                "*".to_string()
                            };
                            let (rise, fall) = parse_rise_fall(tokens, &mut pos)?;
                            delay.cond_paths.push((
                                String::new(),
                                format!("{}->{}", from, to),
                                IoPathDelay { rise, fall },
                            ));
                            // Continue to find closing of COND
                            continue;
                        }
                    }
                    continue;
                }
                _ => {
                    // Unknown construct — let outer depth tracking handle naturally
                    // (no inner skip — the child ) will be seen by outer loop)
                }
            }
        }
        pos += 1;
    }
    if pos < tokens.len() {
        pos += 1;
    } // skip closing )

    Ok((name, delay, timing_checks, pos))
}

/// Parse a NET/DELAYNET construct.
fn parse_net(tokens: &[String], mut pos: usize) -> Result<(String, NetDelay, usize), String> {
    pos += 2; // skip (NET or (DELAYNET
    let name = if pos < tokens.len() && tokens[pos] != "(" {
        let n = tokens[pos].trim_matches('"').to_string();
        pos += 1;
        n
    } else {
        "unknown".to_string()
    };

    let mut net_delay = NetDelay {
        rise: MinTypMax::single(0.0),
        fall: MinTypMax::single(0.0),
    };

    let mut depth = 1;
    while pos < tokens.len() && depth > 0 {
        if tokens[pos] == "(" {
            depth += 1;
        }
        if tokens[pos] == ")" {
            depth -= 1;
        }
        if depth == 0 {
            break;
        }

        if tokens[pos] == "(" && pos + 1 < tokens.len() {
            match tokens[pos + 1].as_str() {
                "ABSDELAY" | "DELAY" => {
                    pos += 2;
                    let (rise, fall) = parse_rise_fall(tokens, &mut pos)?;
                    net_delay.rise = rise;
                    net_delay.fall = fall;
                    continue; // go back to depth check (pos is at child's ))
                }
                _ => {
                    // Unknown construct — let outer depth tracking handle naturally
                }
            }
        }
        pos += 1;
    }
    if pos < tokens.len() {
        pos += 1;
    }

    Ok((name, net_delay, pos))
}

/// Parse TIMINGCHECK construct.
fn parse_timing_checks(
    tokens: &[String],
    mut pos: usize,
) -> Result<(Vec<TimingCheck>, usize), String> {
    pos += 2; // skip (TIMINGCHECK
    let mut checks = Vec::new();
    let mut depth = 1;

    while pos < tokens.len() && depth > 0 {
        if tokens[pos] == "(" {
            depth += 1;
        }
        if tokens[pos] == ")" {
            depth -= 1;
        }
        if depth == 0 {
            break;
        }

        if tokens[pos] == "(" && pos + 1 < tokens.len() {
            match tokens[pos + 1].as_str() {
                "SETUP" => {
                    if let Ok((check, new_pos)) = parse_simple_timing_check(
                        tokens,
                        pos,
                        TimingCheck::Setup {
                            signal: String::new(),
                            ref_signal: String::new(),
                            delay: MinTypMax::single(0.0),
                        },
                    ) {
                        if let TimingCheck::Setup {
                            signal,
                            ref_signal,
                            delay,
                        } = check
                        {
                            checks.push(TimingCheck::Setup {
                                signal,
                                ref_signal,
                                delay,
                            });
                        }
                        pos = new_pos;
                        continue;
                    }
                }
                "HOLD" => {
                    if let Ok((check, new_pos)) = parse_simple_timing_check(
                        tokens,
                        pos,
                        TimingCheck::Hold {
                            signal: String::new(),
                            ref_signal: String::new(),
                            delay: MinTypMax::single(0.0),
                        },
                    ) {
                        if let TimingCheck::Hold {
                            signal,
                            ref_signal,
                            delay,
                        } = check
                        {
                            checks.push(TimingCheck::Hold {
                                signal,
                                ref_signal,
                                delay,
                            });
                        }
                        pos = new_pos;
                        continue;
                    }
                }
                "SETUPHOLD" => {
                    if let Ok((check, new_pos)) = parse_setuphold_check(tokens, pos) {
                        checks.push(check);
                        pos = new_pos;
                        continue;
                    }
                }
                "WIDTH" => {
                    if let Ok((check, new_pos)) = parse_simple_timing_check(
                        tokens,
                        pos,
                        TimingCheck::Width {
                            signal: String::new(),
                            delay: MinTypMax::single(0.0),
                            threshold: None,
                        },
                    ) {
                        if let TimingCheck::Width { signal, delay, .. } = check {
                            checks.push(TimingCheck::Width {
                                signal,
                                delay,
                                threshold: None,
                            });
                        }
                        pos = new_pos;
                        continue;
                    }
                }
                "PERIOD" => {
                    if let Ok((check, new_pos)) = parse_simple_timing_check(
                        tokens,
                        pos,
                        TimingCheck::Period {
                            signal: String::new(),
                            delay: MinTypMax::single(0.0),
                        },
                    ) {
                        if let TimingCheck::Period { signal, delay } = check {
                            checks.push(TimingCheck::Period { signal, delay });
                        }
                        pos = new_pos;
                        continue;
                    }
                }
                "RECOVERY" => {
                    if let Ok((check, new_pos)) = parse_simple_timing_check(
                        tokens,
                        pos,
                        TimingCheck::Recovery {
                            signal: String::new(),
                            ref_signal: String::new(),
                            delay: MinTypMax::single(0.0),
                        },
                    ) {
                        if let TimingCheck::Recovery {
                            signal,
                            ref_signal,
                            delay,
                        } = check
                        {
                            checks.push(TimingCheck::Recovery {
                                signal,
                                ref_signal,
                                delay,
                            });
                        }
                        pos = new_pos;
                        continue;
                    }
                }
                "REMOVAL" => {
                    if let Ok((check, new_pos)) = parse_simple_timing_check(
                        tokens,
                        pos,
                        TimingCheck::Removal {
                            signal: String::new(),
                            ref_signal: String::new(),
                            delay: MinTypMax::single(0.0),
                        },
                    ) {
                        if let TimingCheck::Removal {
                            signal,
                            ref_signal,
                            delay,
                        } = check
                        {
                            checks.push(TimingCheck::Removal {
                                signal,
                                ref_signal,
                                delay,
                            });
                        }
                        pos = new_pos;
                        continue;
                    }
                }
                "SKEW" => {
                    if let Ok((check, new_pos)) = parse_simple_timing_check(
                        tokens,
                        pos,
                        TimingCheck::Skew {
                            signal: String::new(),
                            ref_signal: String::new(),
                            delay: MinTypMax::single(0.0),
                        },
                    ) {
                        if let TimingCheck::Skew {
                            signal,
                            ref_signal,
                            delay,
                        } = check
                        {
                            checks.push(TimingCheck::Skew {
                                signal,
                                ref_signal,
                                delay,
                            });
                        }
                        pos = new_pos;
                        continue;
                    }
                }
                _ => {}
            }
        }
        pos += 1;
    }
    if pos < tokens.len() {
        pos += 1;
    }

    Ok((checks, pos))
}

/// Parse a SETUPHOLD timing check — bedanya dari check sederhana: dua delay
/// (setup, hold) dalam satu construct `(SETUPHOLD (posedge clk) (data)
/// (setup_delay hold_delay))`. SIM-06/10: versi lama menyimpan signal/ref_signal
/// kosong dan kedua delay 0 sehingga check tidak pernah bisa dievaluasi.
fn parse_setuphold_check(
    tokens: &[String],
    mut pos: usize,
) -> Result<(TimingCheck, usize), String> {
    pos += 2; // skip (SETUPHOLD
    let mut signal = String::new();
    let mut ref_signal = String::new();
    let mut delays: Vec<MinTypMax> = Vec::new();
    let mut in_delay_construct = false;

    let mut depth = 1;
    while pos < tokens.len() && depth > 0 {
        if tokens[pos] == "(" {
            depth += 1;
        }
        if tokens[pos] == ")" {
            depth -= 1;
        }
        if depth == 0 {
            break;
        }

        // Delay construct `(1.5:2.0:2.5 0.5:1.0:1.5)` — isi sampai tutup paren.
        if tokens[pos] == "(" && pos + 1 < tokens.len() {
            let nxt = &tokens[pos + 1];
            if parse_triple_token(nxt).is_some() || nxt.parse::<f64>().is_ok() {
                in_delay_construct = true;
                let mut inner = 1;
                let mut p = pos + 1;
                while p < tokens.len() {
                    if tokens[p] == "(" {
                        inner += 1;
                    }
                    if tokens[p] == ")" {
                        inner -= 1;
                        if inner == 0 {
                            break;
                        }
                    }
                    if let Some(mtm) = parse_triple_token(&tokens[p]) {
                        delays.push(mtm);
                    } else if let Ok(val) = tokens[p].parse::<f64>() {
                        delays.push(MinTypMax::single(val));
                    }
                    p += 1;
                }
                pos = p + 1;
                continue;
            }
            if !in_delay_construct {
                // Inner signal construct: `(posedge clk)`, `(data)`, `(PORT clk)`.
                // Ambil quoted string (nama signal) di dalamnya.
                let mut p = pos + 2;
                let mut inner = 1;
                while p < tokens.len() && inner > 0 {
                    if tokens[p] == "(" {
                        inner += 1;
                    }
                    if tokens[p] == ")" {
                        inner -= 1;
                    }
                    if inner == 0 {
                        break;
                    }
                    let trimmed = tokens[p].trim_matches('"');
                    if !trimmed.is_empty()
                        && trimmed.parse::<f64>().is_err()
                        && trimmed != "posedge"
                        && trimmed != "negedge"
                        && trimmed != "PORT"
                        && trimmed != "DATA"
                        && trimmed != "COND"
                    {
                        if ref_signal.is_empty() {
                            ref_signal = trimmed.to_string();
                        } else if signal.is_empty() {
                            signal = trimmed.to_string();
                        }
                        break;
                    }
                    p += 1;
                }
                pos = p + 1;
                continue;
            }
        }
        pos += 1;
    }
    if pos < tokens.len() {
        pos += 1;
    }

    let setup = delays.first().copied().unwrap_or(MinTypMax::single(0.0));
    let hold = delays.get(1).copied().unwrap_or(setup);
    Ok((
        TimingCheck::Setuphold {
            signal,
            ref_signal,
            setup,
            hold,
        },
        pos,
    ))
}

/// Parse a simple timing check (SETUP, HOLD, WIDTH, PERIOD, etc.)
///
/// Format: `(CHECKNAME (posedge ref) (data d) (delay))` — nama signal boleh
/// dikutip (`"d"`) atau bare (`d`); delay boleh skalar (`5.0`) atau triple
/// (`1.0:1.5:2.0`). SIM-06: versi lama tidak pernah mengekstrak nilai dari
/// construct `(POSEDGE clk)`/`(DATA d)` (loop inner salah mulai `inner=0`),
/// sehingga signal malah terisi token `(`/`)`.
fn parse_simple_timing_check(
    tokens: &[String],
    mut pos: usize,
    template: TimingCheck,
) -> Result<(TimingCheck, usize), String> {
    pos += 2; // skip (CHECKNAME
    let mut signal = String::new();
    let mut ref_signal = String::new();
    let mut delay = MinTypMax::single(0.0);
    let mut delay_found = false;

    const RESERVED: &[&str] = &["posedge", "negedge", "port", "data", "cond", "edge"];
    let is_reserved = |s: &str| RESERVED.contains(&s.to_lowercase().as_str());
    // Construct nilai: `(POSEDGE clk)`, `(DATA d)`, `(PORT x)` — ambil nama
    // signal pertama di dalamnya.
    let is_value_construct =
        |k: &str| matches!(k, "PORT" | "DATA" | "POSEDGE" | "NEGEDGE" | "EDGE");

    let mut depth = 1;
    while pos < tokens.len() && depth > 0 {
        let tok = tokens[pos].clone();
        if tok == "(" {
            depth += 1;
            if pos + 1 < tokens.len() {
                let keyword = &tokens[pos + 1];
                if keyword == "COND" {
                    // `(COND (reset == 1) (DATA d))` — ekspresi kondisi (bukan
                    // port). Lompati seluruh construct agar token `==`/`1` di
                    // dalamnya tidak terambil sebagai signal/delay (SIM-06).
                    let mut p = pos + 2;
                    let mut inner = 1;
                    while p < tokens.len() && inner > 0 {
                        if tokens[p] == "(" {
                            inner += 1;
                        }
                        if tokens[p] == ")" {
                            inner -= 1;
                        }
                        p += 1;
                    }
                    // Paren penutup COND sudah dikonsumsi — balance depth.
                    pos = p;
                    depth -= 1;
                    continue;
                }
                if is_value_construct(keyword) {
                    // Scan sampai paren penutup construct, ambil token nilai
                    // pertama yang bukan keyword/angka → nama signal.
                    let mut p = pos + 2;
                    let mut inner = 1;
                    while p < tokens.len() && inner > 0 {
                        if tokens[p] == "(" {
                            inner += 1;
                        }
                        if tokens[p] == ")" {
                            inner -= 1;
                            if inner == 0 {
                                break;
                            }
                        }
                        let trimmed = tokens[p].trim_matches('"');
                        if !trimmed.is_empty()
                            && trimmed.parse::<f64>().is_err()
                            && !is_reserved(trimmed)
                        {
                            if signal.is_empty() {
                                signal = trimmed.to_string();
                            } else if ref_signal.is_empty() {
                                ref_signal = trimmed.to_string();
                            }
                            break;
                        }
                        p += 1;
                    }
                    // Loncat ke akhir construct
                    pos = p + 1;
                    continue;
                }
            }
            pos += 1;
            continue;
        }
        if tok == ")" {
            depth -= 1;
            if depth == 0 {
                // Paren penutup check — pos menunjuk `)`, biarkan `pos += 1`
                // di akhir menunjuk token berikutnya (hindari double-increment
                // yang membuat parse_timing_checks melompati check berikutnya).
                break;
            }
            pos += 1;
            continue;
        }

        // Delay: `5.0` atau triple `1.0:1.5:2.0`
        if !delay_found {
            if let Some(mtm) = parse_triple_token(&tok) {
                delay = mtm;
                delay_found = true;
                pos += 1;
                continue;
            }
            if let Ok(val) = tok.parse::<f64>() {
                delay = MinTypMax::single(val);
                delay_found = true;
                pos += 1;
                continue;
            }
        }
        // Nama signal bare (SDF tanpa kutip): `clk`, `d`, `top.sig`
        let trimmed = tok.trim_matches('"');
        if !trimmed.is_empty() && trimmed.parse::<f64>().is_err() && !is_reserved(trimmed) {
            if signal.is_empty() {
                signal = trimmed.to_string();
            } else if ref_signal.is_empty() {
                ref_signal = trimmed.to_string();
            }
        }
        pos += 1;
    }
    if pos < tokens.len() {
        pos += 1;
    }

    // Reconstruct the check based on template type.
    // Catatan SDF (IEEE 1497): SETUP/HOLD/RECOVERY/REMOVAL menulis port REF
    // lebih dulu, lalu port DATA: `(SETUP (posedge clk) (DATA d) (limit))`.
    // Ekstraksi di atas menempatkan nama pertama ke `signal` — TUKAR agar
    // field TimingCheck konsisten dengan specify-block: signal = data,
    // ref_signal = clock/ref (SIM-06: tanpa swap, check membandingkan clk
    // sebagai data dan d sebagai ref → tidak pernah fire).
    use TimingCheck::*;
    let check = match template {
        Setup { .. } => Setup {
            signal: ref_signal,
            ref_signal: signal,
            delay,
        },
        Hold { .. } => Hold {
            signal: ref_signal,
            ref_signal: signal,
            delay,
        },
        Width { .. } => Width {
            signal,
            delay,
            threshold: None,
        },
        Period { .. } => Period { signal, delay },
        Recovery { .. } => Recovery {
            signal: ref_signal,
            ref_signal: signal,
            delay,
        },
        Removal { .. } => Removal {
            signal: ref_signal,
            ref_signal: signal,
            delay,
        },
        Skew { .. } => Skew {
            signal: ref_signal,
            ref_signal: signal,
            delay,
        },
        Setuphold { .. } => Setuphold {
            signal: ref_signal,
            ref_signal: signal,
            setup: delay,
            hold: delay,
        },
    };

    Ok((check, pos))
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_min_typ_max_single() {
        let mtm = MinTypMax::single(1.5);
        assert!((mtm.get(TimingMode::Min) - 1.5).abs() < 1e-9);
        assert!((mtm.get(TimingMode::Typ) - 1.5).abs() < 1e-9);
        assert!((mtm.get(TimingMode::Max) - 1.5).abs() < 1e-9);
    }

    #[test]
    fn test_min_typ_max_triple() {
        let mtm = MinTypMax::new(1.0, 2.0, 3.0);
        assert!((mtm.get(TimingMode::Min) - 1.0).abs() < 1e-9);
        assert!((mtm.get(TimingMode::Typ) - 2.0).abs() < 1e-9);
        assert!((mtm.get(TimingMode::Max) - 3.0).abs() < 1e-9);
    }

    #[test]
    fn test_timing_mode_from_str() {
        assert_eq!(TimingMode::from_str("min"), Some(TimingMode::Min));
        assert_eq!(TimingMode::from_str("typ"), Some(TimingMode::Typ));
        assert_eq!(TimingMode::from_str("max"), Some(TimingMode::Max));
        assert_eq!(TimingMode::from_str("invalid"), None);
    }

    #[test]
    fn test_tokenize_simple() {
        let tokens = tokenize("(DELAYFILE (SDFVERSION \"3.0\") (DESIGN \"test\"))");
        assert!(tokens.contains(&"DELAYFILE".to_string()));
        assert!(tokens.contains(&"SDFVERSION".to_string()));
        assert!(tokens.contains(&"DESIGN".to_string()));
    }

    #[test]
    fn test_sdf_header_parse() {
        let sdf_source = r#"
        (DELAYFILE
            (SDFVERSION "3.0")
            (DESIGN "top")
            (DATE "01/01/2024")
            (VENDOR "TestCorp")
            (PROGRAM "TestSDF")
            (VERSION "1.0")
            (DIVIDER .)
            (VOLTAGE 1.2)
            (PROCESS "1.0")
            (TEMPERATURE "25.0")
            (TIMESCALE 1ns)
        )
        "#;
        let sdf = SdfData::parse(sdf_source).unwrap();
        assert_eq!(sdf.sdf_version, Some("3.0".to_string()));
        assert_eq!(sdf.design_name, Some("top".to_string()));
        assert_eq!(sdf.vendor, Some("TestCorp".to_string()));
        assert!(sdf.voltage.is_some());
    }

    #[test]
    fn test_cell_delay_parse() {
        let sdf_source = r#"
        (DELAYFILE (SDFVERSION "3.0"))
        (CELL (CELLTYPE "AND2")
            (INSTANCE u_and)
            (DELAY
                (ABSOLUTE
                    (IOPATH a z (1.0) (1.5))
                    (IOPATH b z (2.0:2.5:3.0) (2.5:3.0:3.5))
                )
            )
        )
        "#;
        let sdf = SdfData::parse(sdf_source).unwrap();
        assert!(
            sdf.cell_delays.contains_key("u_and"),
            "cell 'u_and' should be present: keys={:?}",
            sdf.cell_delays.keys().collect::<Vec<_>>()
        );
        let cell = &sdf.cell_delays["u_and"];
        assert!(
            cell.io_paths.contains_key("a->z"),
            "IOPATH a->z should be present: paths={:?}",
            cell.io_paths.keys().collect::<Vec<_>>()
        );
        let path_az = &cell.io_paths["a->z"];
        assert!((path_az.rise.get(TimingMode::Typ) - 1.0).abs() < 1e-9);
        assert!((path_az.fall.get(TimingMode::Typ) - 1.5).abs() < 1e-9);

        let path_bz = &cell.io_paths["b->z"];
        assert!((path_bz.rise.get(TimingMode::Min) - 2.0).abs() < 1e-9);
        assert!((path_bz.rise.get(TimingMode::Typ) - 2.5).abs() < 1e-9);
        assert!((path_bz.rise.get(TimingMode::Max) - 3.0).abs() < 1e-9);
        assert!((path_bz.fall.get(TimingMode::Min) - 2.5).abs() < 1e-9);
        assert!((path_bz.fall.get(TimingMode::Typ) - 3.0).abs() < 1e-9);
        assert!((path_bz.fall.get(TimingMode::Max) - 3.5).abs() < 1e-9);
    }

    #[test]
    fn test_net_delay_parse() {
        let sdf_source = r#"
        (DELAYFILE (SDFVERSION "3.0"))
        (NET "top.clk"
            (ABSDELAY (1.0) (1.2))
        )
        (NET "top.data"
            (DELAY (0.5:0.8:1.0) (0.6:0.9:1.2))
        )
        "#;
        let sdf = SdfData::parse(sdf_source).unwrap();
        assert!(sdf.net_delays.contains_key("top.clk"));
        assert!(sdf.net_delays.contains_key("top.data"));

        let clk = &sdf.net_delays["top.clk"];
        assert!((clk.rise.get(TimingMode::Typ) - 1.0).abs() < 1e-9);
        assert!((clk.fall.get(TimingMode::Typ) - 1.2).abs() < 1e-9);

        let data = &sdf.net_delays["top.data"];
        assert!((data.rise.get(TimingMode::Min) - 0.5).abs() < 1e-9);
        assert!((data.rise.get(TimingMode::Max) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_timing_checks_parse() {
        // TIMINGCHECK can now be inside CELL — parse_cell() handles TIMINGCHECK.
        // POSEDGE and COND constructs are safe (known sub-keyword filter + safety guard).
        // Also test top-level TIMINGCHECK for backward compatibility.
        let sdf_source = r#"
        (DELAYFILE (SDFVERSION "3.0"))
        (CELL (CELLTYPE "DFF")
            (INSTANCE u_dff)
            (DELAY (ABSOLUTE (IOPATH clk q (1.0) (1.5))))
            (TIMINGCHECK
                (SETUP (POSEDGE clk) (COND (reset == 1) (DATA d)) (1.0:1.5:2.0))
                (HOLD (POSEDGE clk) (DATA d) (0.5:1.0:1.5))
                (WIDTH (POSEDGE clk) (1.0:2.0:3.0))
                (PERIOD (POSEDGE clk) (5.0:6.0:7.0))
            )
        )
        (TIMINGCHECK
            (RECOVERY (POSEDGE clk) (DATA d) (2.0:3.0:4.0))
            (REMOVAL (POSEDGE clk) (DATA d) (1.0:2.0:3.0))
        )
        "#;
        let sdf = SdfData::parse(sdf_source).unwrap();
        assert!(
            !sdf.timing_checks.is_empty(),
            "timing checks should be parsed"
        );
        // 4 inside CELL + 2 at top level = 6
        assert_eq!(
            sdf.timing_checks.len(),
            6,
            "should have 6 timing checks (4 inside cell + 2 at top level)"
        );
        // Also verify cell parsing still works
        assert!(
            sdf.cell_delays.contains_key("u_dff"),
            "cell 'u_dff' should still be present"
        );
    }

    #[test]
    fn test_sdf_file_parse() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("maria_sdf_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let sdf_path = dir.join("test.sdf");
        let sdf_content = r#"
        (DELAYFILE
            (SDFVERSION "3.0")
            (DESIGN "test")
        )
        (CELL (CELLTYPE "BUF")
            (INSTANCE u_buf)
            (DELAY (ABSOLUTE (IOPATH a y (0.5) (0.7))))
        )
        (NET "test.sig"
            (ABSDELAY (0.1) (0.1))
        )
        "#;
        {
            let mut f = std::fs::File::create(&sdf_path).unwrap();
            f.write_all(sdf_content.as_bytes()).unwrap();
        }

        let sdf = SdfData::parse_file(sdf_path.to_str().unwrap()).unwrap();
        assert_eq!(sdf.sdf_version, Some("3.0".to_string()));
        assert!(sdf.cell_delays.contains_key("u_buf"));
        assert!(sdf.net_delays.contains_key("test.sig"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_timing_mode_thread_local() {
        set_timing_mode(TimingMode::Min);
        assert_eq!(get_timing_mode(), TimingMode::Min);
        set_timing_mode(TimingMode::Max);
        assert_eq!(get_timing_mode(), TimingMode::Max);
    }

    #[test]
    fn test_tokenize_with_comments() {
        let tokens = tokenize("(CELL /* comment */ (IOPATH a z (1.0) (1.5)))");
        assert!(tokens.contains(&"CELL".to_string()));
        assert!(tokens.contains(&"IOPATH".to_string()));
        assert!(tokens.contains(&"1.0".to_string()));
        assert!(tokens.contains(&"1.5".to_string()));
    }

    #[test]
    fn test_tokenize_with_line_comments() {
        let tokens = tokenize("(CELL // line comment\n (IOPATH a z (1.0) (1.5)))");
        assert!(tokens.contains(&"CELL".to_string()));
        assert!(tokens.contains(&"IOPATH".to_string()));
    }

    #[test]
    fn test_parse_conditional_delay() {
        let sdf_source = r#"
        (DELAYFILE (SDFVERSION "3.0"))
        (CELL (CELLTYPE "MUX")
            (INSTANCE u_mux)
            (DELAY (ABSOLUTE
                (COND (sel == 1) (IOPATH a z (2.0) (2.5)))
                (IOPATH a z (3.0) (3.5))
            ))
        )
        "#;
        let sdf = SdfData::parse(sdf_source).unwrap();
        assert!(sdf.cell_delays.contains_key("u_mux"));
    }

    #[test]
    fn test_setuphold_parse_full() {
        // SIM-06/10: SETUPHOLD harus menangkap signal/ref_signal + setup & hold
        // (sebelumnya signal/ref_signal kosong dan kedua delay = 0).
        let sdf_source = r#"
        (DELAYFILE (SDFVERSION "3.0"))
        (CELL (CELLTYPE "DFF")
            (INSTANCE u_dff)
            (TIMINGCHECK
                (SETUPHOLD (POSEDGE clk) (DATA d) (1.5:2.0:2.5) (0.5:1.0:1.5))
            )
        )
        "#;
        let sdf = SdfData::parse(sdf_source).unwrap();
        assert_eq!(sdf.timing_checks.len(), 1, "satu SETUPHOLD ter-parse");
        match &sdf.timing_checks[0] {
            TimingCheck::Setuphold {
                signal,
                ref_signal,
                setup,
                hold,
            } => {
                assert_eq!(
                    signal, "d",
                    "signal data harus ter-ekstrak: got '{:?}'",
                    signal
                );
                assert_eq!(
                    ref_signal, "clk",
                    "ref signal harus ter-ekstrak: got '{:?}'",
                    ref_signal
                );
                assert!((setup.get(TimingMode::Typ) - 2.0).abs() < 1e-9, "setup=2.0");
                assert!(
                    (setup.get(TimingMode::Min) - 1.5).abs() < 1e-9,
                    "setup min=1.5"
                );
                assert!((hold.get(TimingMode::Typ) - 1.0).abs() < 1e-9, "hold=1.0");
                assert!(
                    (hold.get(TimingMode::Max) - 1.5).abs() < 1e-9,
                    "hold max=1.5"
                );
            }
            other => panic!("bukan Setuphold: {}", other.type_name()),
        }
    }

    #[test]
    fn test_setuphold_parse_negative() {
        // SIM-08: delay negatif harus ter-parse (setup -1ns, hold 0.5ns).
        let sdf_source = r#"
        (DELAYFILE (SDFVERSION "3.0"))
        (CELL (CELLTYPE "DFF")
            (INSTANCE u_dff)
            (TIMINGCHECK
                (SETUPHOLD (POSEDGE clk) (DATA d) (-1.0) (0.5))
            )
        )
        "#;
        let sdf = SdfData::parse(sdf_source).unwrap();
        match &sdf.timing_checks[0] {
            TimingCheck::Setuphold {
                signal,
                ref_signal,
                setup,
                hold,
            } => {
                assert_eq!(signal, "d");
                assert_eq!(ref_signal, "clk");
                assert!(
                    setup.get(TimingMode::Typ) < 0.0,
                    "setup negatif: {}",
                    setup.get(TimingMode::Typ)
                );
                assert!(hold.get(TimingMode::Typ) > 0.0);
            }
            other => panic!("bukan Setuphold: {}", other.type_name()),
        }
    }

    #[test]
    fn test_pulse_control_parse() {
        // SIM-09: (PULSE (PULSE_WIDTH (PORT "x") (1.0:2.0:3.0))) harus
        // menghasilkan pulse_controls per-port dengan lebar min/typ/max.
        // Bisa di dalam CELL maupun top-level.
        let sdf_source = r#"
        (DELAYFILE (SDFVERSION "3.0"))
        (CELL (CELLTYPE "DFF")
            (INSTANCE u_dff)
            (PULSE (PULSE_WIDTH (PORT "q") (1.0:2.0:3.0)))
        )
        (PULSE (PULSE_WIDTH (PORT "clk") (2.0)))
        "#;
        let sdf = SdfData::parse(sdf_source).unwrap();
        assert_eq!(sdf.pulse_controls.len(), 2, "dua pulse control ter-parse");
        let q = sdf.pulse_controls.get("q").expect("port q");
        assert!((q.width.get(TimingMode::Typ) - 2.0).abs() < 1e-9, "typ=2.0");
        assert!((q.width.get(TimingMode::Max) - 3.0).abs() < 1e-9, "max=3.0");
        let clk = sdf.pulse_controls.get("clk").expect("port clk");
        assert!(
            (clk.width.get(TimingMode::Typ) - 2.0).abs() < 1e-9,
            "scalar 2.0"
        );
    }

    #[test]
    fn test_pulse_control_parse_quoted_signal() {
        // Nama port dikutip (`"q"`) atau bare (`q`) harus sama-sama ter-parse.
        let sdf_source = r#"
        (DELAYFILE (SDFVERSION "3.0"))
        (PULSE (PULSE_WIDTH (PORT "q" ) (5.0)))
        "#;
        let sdf = SdfData::parse(sdf_source).unwrap();
        assert!(sdf.pulse_controls.contains_key("q"), "port q terdaftar");
    }
}
