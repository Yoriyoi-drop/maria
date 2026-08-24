//! Liberty (.lib) cell library parser.
//!
//! Parses the IEEE Liberty library format used for ASIC cell libraries.
//! Supports:
//! - library/cell/pin hierarchy
//! - Timing arcs with rise/fall delays (IOPATH)
//! - Pin direction (input, output, inout)
//! - Cell area
//! - Delay table templates (lookup table format)
//!
//! # Example Liberty snippet
//!
//! ```text
//! library(test) {
//!   cell(AND2) {
//!     area: 12;
//!     pin(A) { direction: input; }
//!     pin(B) { direction: input; }
//!     pin(Z) {
//!       direction: output;
//!       timing() {
//!         related_pin: "A";
//!         timing_sense: positive_unate;
//!         cell_rise(delay_template_2x2) {
//!           index_1("0.1, 0.5");
//!           index_2("0.1, 0.5");
//!           values("0.05, 0.10", "0.10, 0.20");
//!         }
//!         cell_fall(delay_template_2x2) {
//!           index_1("0.1, 0.5");
//!           index_2("0.1, 0.5");
//!           values("0.04, 0.09", "0.09, 0.18");
//!         }
//!       }
//!     }
//!   }
//! }
//! ```

use std::collections::HashMap;
use std::fs;

// ─── Data Structures ───

/// A complete Liberty library definition.
#[derive(Debug, Clone, PartialEq)]
pub struct LibertyLibrary {
    pub name: String,
    pub cells: HashMap<String, LibertyCell>,
    pub delay_templates: HashMap<String, LibertyDelayTemplate>,
    pub default_operating_conditions: Option<String>,
    pub in_place_swap_mode: Option<bool>,
}

/// A cell (standard cell) definition.
#[derive(Debug, Clone, PartialEq)]
pub struct LibertyCell {
    pub name: String,
    pub area: Option<f64>,
    pub pins: HashMap<String, LibertyPin>,
    pub ff: Option<LibertyFF>,
    pub cell_leakage_power: Option<f64>,
}

/// A pin (input/output/inout) definition.
#[derive(Debug, Clone, PartialEq)]
pub struct LibertyPin {
    pub name: String,
    pub direction: LibertyPinDirection,
    pub capacitance: Option<f64>,
    pub max_capacitance: Option<f64>,
    pub min_capacitance: Option<f64>,
    pub timing_arcs: Vec<LibertyTimingArc>,
    pub function: Option<String>,
    pub clock: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LibertyPinDirection {
    Input,
    Output,
    Inout,
}

/// A timing arc from a related_pin to the output pin.
#[derive(Debug, Clone, PartialEq)]
pub struct LibertyTimingArc {
    pub related_pin: String,
    pub timing_sense: LibertyTimingSense,
    pub cell_rise: Option<LibertyDelayTable>,
    pub cell_fall: Option<LibertyDelayTable>,
    pub rise_transition: Option<LibertyDelayTable>,
    pub fall_transition: Option<LibertyDelayTable>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LibertyTimingSense {
    PositiveUnate,
    NegativeUnate,
    NonUnate,
}

/// A delay lookup table (typically 2D or 1D).
#[derive(Debug, Clone, PartialEq)]
pub struct LibertyDelayTable {
    pub template_name: Option<String>,
    pub index_1: Vec<f64>,
    pub index_2: Vec<f64>,
    pub values: Vec<Vec<f64>>,
}

/// Flip-flop definition inside a cell.
#[derive(Debug, Clone, PartialEq)]
pub struct LibertyFF {
    pub clocked_on: Option<String>,
    pub next_state: Option<String>,
    pub clear: Option<String>,
    pub preset: Option<String>,
}

/// A delay template definition (reusable table shape).
#[derive(Debug, Clone, PartialEq)]
pub struct LibertyDelayTemplate {
    pub name: String,
    pub variable_1: String,
    pub variable_2: Option<String>,
    pub index_1: Vec<f64>,
    pub index_2: Vec<f64>,
}

impl LibertyLibrary {
    pub fn new() -> Self {
        LibertyLibrary {
            name: String::new(),
            cells: HashMap::new(),
            delay_templates: HashMap::new(),
            default_operating_conditions: None,
            in_place_swap_mode: None,
        }
    }

    /// Parse a Liberty library from a file path.
    pub fn parse_file(path: &str) -> Result<Self, String> {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("cannot read Liberty file '{}': {}", path, e))?;
        Self::parse(&content)
    }

    /// Parse a Liberty library from a string.
    pub fn parse(content: &str) -> Result<Self, String> {
        let tokens = tokenize(content);
        let mut parser = LibertyParser { tokens, pos: 0 };
        parser.parse_library()
    }

    /// Look up the rise delay for a given input→output path.
    /// Uses timing_mode to select min/typ/max (uses first value as default).
    /// Simple interpolation: uses first value in table.
    pub fn get_rise_delay(&self, cell_name: &str, from_pin: &str, to_pin: &str) -> Option<f64> {
        let cell = self.cells.get(cell_name)?;
        let pin = cell.pins.get(to_pin)?;
        for arc in &pin.timing_arcs {
            if arc.related_pin == from_pin {
                if let Some(ref table) = arc.cell_rise {
                    // Return the first value in the table (simplified)
                    return table.values.first().and_then(|row| row.first()).copied();
                }
            }
        }
        None
    }

    /// Look up the fall delay for a given input→output path.
    pub fn get_fall_delay(&self, cell_name: &str, from_pin: &str, to_pin: &str) -> Option<f64> {
        let cell = self.cells.get(cell_name)?;
        let pin = cell.pins.get(to_pin)?;
        for arc in &pin.timing_arcs {
            if arc.related_pin == from_pin {
                if let Some(ref table) = arc.cell_fall {
                    return table.values.first().and_then(|row| row.first()).copied();
                }
            }
        }
        None
    }
}

impl Default for LibertyLibrary {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tokenizer ───

#[derive(Debug, Clone, PartialEq)]
enum LibToken {
    Ident(String),
    StringLit(String),
    Number(f64),
    LParen,
    RParen,
    LBrace,
    RBrace,
    Colon,
    Semi,
    Comma,
    At,
    Dot,
    Plus,
    Minus,
    ColonEquals,
}

fn tokenize(content: &str) -> Vec<LibToken> {
    let mut tokens = Vec::new();
    let mut chars = content.chars().peekable();
    let mut current = String::new();
    let mut in_block_comment = false;
    let mut in_line_comment = false;

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

        match c {
            '/' if chars.peek() == Some(&'*') => {
                if !current.is_empty() {
                    tokens.push(parse_number_or_ident(&current));
                    current.clear();
                }
                chars.next();
                in_block_comment = true;
            }
            '/' if chars.peek() == Some(&'/') => {
                if !current.is_empty() {
                    tokens.push(parse_number_or_ident(&current));
                    current.clear();
                }
                in_line_comment = true;
            }
            '"' => {
                if !current.is_empty() {
                    tokens.push(parse_number_or_ident(&current));
                    current.clear();
                }
                // Read string content
                let mut s = String::new();
                while let Some(&ch) = chars.peek() {
                    if ch == '"' {
                        chars.next();
                        break;
                    }
                    s.push(ch);
                    chars.next();
                }
                tokens.push(LibToken::StringLit(s));
            }
            '(' => {
                if !current.is_empty() {
                    tokens.push(parse_number_or_ident(&current));
                    current.clear();
                }
                tokens.push(LibToken::LParen);
            }
            ')' => {
                if !current.is_empty() {
                    tokens.push(parse_number_or_ident(&current));
                    current.clear();
                }
                tokens.push(LibToken::RParen);
            }
            '{' => {
                if !current.is_empty() {
                    tokens.push(parse_number_or_ident(&current));
                    current.clear();
                }
                tokens.push(LibToken::LBrace);
            }
            '}' => {
                if !current.is_empty() {
                    tokens.push(parse_number_or_ident(&current));
                    current.clear();
                }
                tokens.push(LibToken::RBrace);
            }
            ':' => {
                if !current.is_empty() {
                    tokens.push(parse_number_or_ident(&current));
                    current.clear();
                }
                if chars.peek() == Some(&'=') {
                    chars.next();
                    tokens.push(LibToken::ColonEquals);
                } else {
                    tokens.push(LibToken::Colon);
                }
            }
            ';' => {
                if !current.is_empty() {
                    tokens.push(parse_number_or_ident(&current));
                    current.clear();
                }
                tokens.push(LibToken::Semi);
            }
            ',' => {
                if !current.is_empty() {
                    tokens.push(parse_number_or_ident(&current));
                    current.clear();
                }
                tokens.push(LibToken::Comma);
            }
            '@' => {
                if !current.is_empty() {
                    tokens.push(parse_number_or_ident(&current));
                    current.clear();
                }
                tokens.push(LibToken::At);
            }
            '.' => {
                // Check if this is a decimal number (e.g., "0.01")
                // If current contains only digits (or a number prefix), treat this as part of a decimal
                let is_decimal =
                    !current.is_empty() && current.chars().all(|c| c.is_ascii_digit() || c == '-');
                if is_decimal {
                    current.push('.');
                } else {
                    if !current.is_empty() {
                        tokens.push(parse_number_or_ident(&current));
                        current.clear();
                    }
                    tokens.push(LibToken::Dot);
                }
            }
            '+' => {
                if !current.is_empty() {
                    tokens.push(parse_number_or_ident(&current));
                    current.clear();
                }
                tokens.push(LibToken::Plus);
            }
            '-' => {
                if !current.is_empty() {
                    tokens.push(parse_number_or_ident(&current));
                    current.clear();
                }
                tokens.push(LibToken::Minus);
            }
            ' ' | '\t' | '\r' | '\n' => {
                if !current.is_empty() {
                    tokens.push(parse_number_or_ident(&current));
                    current.clear();
                }
            }
            _ => {
                current.push(c);
            }
        }
    }
    if !current.is_empty() {
        tokens.push(parse_number_or_ident(&current));
    }
    tokens
}

fn parse_number_or_ident(s: &str) -> LibToken {
    // Try to parse as number (float or integer)
    if let Ok(val) = s.parse::<f64>() {
        return LibToken::Number(val);
    }
    // Also try with leading minus
    if let Some(rest) = s.strip_prefix('-') {
        if let Ok(val) = rest.parse::<f64>() {
            return LibToken::Number(-val);
        }
    }
    LibToken::Ident(s.to_string())
}

// ─── Parser ───

struct LibertyParser {
    tokens: Vec<LibToken>,
    pos: usize,
}

impl LibertyParser {
    fn peek(&self) -> Option<&LibToken> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) {
        self.pos += 1;
    }

    fn expect(&mut self, expected: &LibToken) -> Result<(), String> {
        if self.peek() == Some(expected) {
            self.advance();
            Ok(())
        } else {
            Err(format!(
                "expected {:?}, found {:?} at token {}",
                expected,
                self.peek(),
                self.pos
            ))
        }
    }

    fn expect_ident(&mut self) -> Result<String, String> {
        match self.peek() {
            Some(LibToken::Ident(s)) => {
                let s = s.clone();
                self.advance();
                Ok(s)
            }
            Some(LibToken::StringLit(s)) => {
                let s = s.clone();
                self.advance();
                Ok(s)
            }
            _ => Err(format!(
                "expected identifier, found {:?} at token {}",
                self.peek(),
                self.pos
            )),
        }
    }

    fn expect_number(&mut self) -> Result<f64, String> {
        match self.peek() {
            Some(LibToken::Number(n)) => {
                let n = *n;
                self.advance();
                Ok(n)
            }
            _ => Err(format!(
                "expected number, found {:?} at token {}",
                self.peek(),
                self.pos
            )),
        }
    }

    /// Skip to the next closing brace at the current depth level.
    fn skip_block(&mut self) {
        let mut depth = 1;
        while let Some(tok) = self.peek() {
            match tok {
                LibToken::LBrace => depth += 1,
                LibToken::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        self.advance();
                        return;
                    }
                }
                _ => {}
            }
            self.advance();
        }
    }

    // ── Top-level parsing ──

    fn parse_library(&mut self) -> Result<LibertyLibrary, String> {
        let ident = self.expect_ident()?;
        if ident.to_lowercase() != "library" {
            return Err(format!("expected 'library', found '{}' at token 0", ident));
        }
        self.expect(&LibToken::LParen)?;
        let name = self.expect_ident()?;
        self.expect(&LibToken::RParen)?;
        self.expect(&LibToken::LBrace)?;

        let mut lib = LibertyLibrary {
            name,
            cells: HashMap::new(),
            delay_templates: HashMap::new(),
            default_operating_conditions: None,
            in_place_swap_mode: None,
        };

        self.parse_library_body(&mut lib)?;

        self.expect(&LibToken::RBrace)?;
        Ok(lib)
    }

    fn parse_library_body(&mut self, lib: &mut LibertyLibrary) -> Result<(), String> {
        loop {
            match self.peek() {
                None | Some(LibToken::RBrace) => break,
                Some(LibToken::Semi) => {
                    self.advance();
                }
                Some(LibToken::Ident(keyword)) => {
                    let kw = keyword.to_lowercase();
                    match kw.as_str() {
                        "cell" => {
                            let cell = self.parse_cell()?;
                            lib.cells.insert(cell.name.clone(), cell);
                        }
                        "delay_template" | "lu_table_template" | "power_lut_template" => {
                            let tmpl = self.parse_delay_template()?;
                            lib.delay_templates.insert(tmpl.name.clone(), tmpl);
                        }
                        "default_operating_conditions" => {
                            self.advance();
                            self.expect(&LibToken::Colon)?;
                            let val = self.expect_ident()?;
                            self.expect(&LibToken::Semi)?;
                            lib.default_operating_conditions = Some(val);
                        }
                        "in_place_swap_mode" => {
                            self.advance();
                            self.expect(&LibToken::Colon)?;
                            let val = self.expect_ident()?;
                            self.expect(&LibToken::Semi)?;
                            lib.in_place_swap_mode = Some(val.to_lowercase() == "true");
                        }
                        _ => {
                            // Unknown/skip: try to parse as key: value; or skip block
                            self.advance();
                            if self.peek() == Some(&LibToken::Colon) {
                                self.advance();
                                while matches!(
                                    self.peek(),
                                    Some(LibToken::Ident(_))
                                        | Some(LibToken::StringLit(_))
                                        | Some(LibToken::Number(_))
                                ) {
                                    self.advance();
                                }
                                if self.peek() == Some(&LibToken::Semi) {
                                    self.advance();
                                }
                            } else if self.peek() == Some(&LibToken::LParen) {
                                // Skip group: keyword(args) { ... }
                                self.advance(); // skip (
                                while self.peek() != Some(&LibToken::RParen)
                                    && self.peek().is_some()
                                {
                                    self.advance();
                                }
                                if self.peek() == Some(&LibToken::RParen) {
                                    self.advance();
                                }
                                if self.peek() == Some(&LibToken::LBrace) {
                                    self.skip_block();
                                }
                            }
                        }
                    }
                }
                _ => {
                    self.advance();
                }
            }
        }
        Ok(())
    }

    // ── Cell parsing ──

    fn parse_cell(&mut self) -> Result<LibertyCell, String> {
        self.advance(); // consume 'cell'
        self.expect(&LibToken::LParen)?;
        let name = self.expect_ident()?;
        self.expect(&LibToken::RParen)?;
        self.expect(&LibToken::LBrace)?;

        let mut cell = LibertyCell {
            name,
            area: None,
            pins: HashMap::new(),
            ff: None,
            cell_leakage_power: None,
        };

        self.parse_cell_body(&mut cell)?;

        self.expect(&LibToken::RBrace)?;
        Ok(cell)
    }

    fn parse_cell_body(&mut self, cell: &mut LibertyCell) -> Result<(), String> {
        loop {
            match self.peek() {
                None | Some(LibToken::RBrace) => break,
                Some(LibToken::Semi) => {
                    self.advance();
                }
                Some(LibToken::Ident(keyword)) => {
                    let kw = keyword.to_lowercase();
                    match kw.as_str() {
                        "pin" => {
                            let pin = self.parse_pin()?;
                            cell.pins.insert(pin.name.clone(), pin);
                        }
                        "area" => {
                            self.advance();
                            self.expect(&LibToken::Colon)?;
                            let val = self.expect_number()?;
                            self.expect(&LibToken::Semi)?;
                            cell.area = Some(val);
                        }
                        "cell_leakage_power" => {
                            self.advance();
                            self.expect(&LibToken::Colon)?;
                            let val = self.expect_number()?;
                            self.expect(&LibToken::Semi)?;
                            cell.cell_leakage_power = Some(val);
                        }
                        "ff" => {
                            cell.ff = Some(self.parse_ff()?);
                        }
                        _ => {
                            // Skip unknown cell attribute
                            self.advance();
                            self.skip_value_or_block();
                        }
                    }
                }
                _ => {
                    self.advance();
                }
            }
        }
        Ok(())
    }

    // ── Pin parsing ──

    fn parse_pin(&mut self) -> Result<LibertyPin, String> {
        self.advance(); // consume 'pin'
        self.expect(&LibToken::LParen)?;
        let name = self.expect_ident()?;
        self.expect(&LibToken::RParen)?;
        self.expect(&LibToken::LBrace)?;

        let mut pin = LibertyPin {
            name,
            direction: LibertyPinDirection::Input,
            capacitance: None,
            max_capacitance: None,
            min_capacitance: None,
            timing_arcs: Vec::new(),
            function: None,
            clock: None,
        };

        self.parse_pin_body(&mut pin)?;

        self.expect(&LibToken::RBrace)?;
        Ok(pin)
    }

    fn parse_pin_body(&mut self, pin: &mut LibertyPin) -> Result<(), String> {
        loop {
            match self.peek() {
                None | Some(LibToken::RBrace) => break,
                Some(LibToken::Semi) => {
                    self.advance();
                }
                Some(LibToken::Ident(keyword)) => {
                    let kw = keyword.to_lowercase();
                    match kw.as_str() {
                        "direction" => {
                            self.advance();
                            self.expect(&LibToken::Colon)?;
                            let dir = self.expect_ident()?;
                            match dir.to_lowercase().as_str() {
                                "input" => pin.direction = LibertyPinDirection::Input,
                                "output" => pin.direction = LibertyPinDirection::Output,
                                "inout" => pin.direction = LibertyPinDirection::Inout,
                                _ => {}
                            }
                            self.expect(&LibToken::Semi)?;
                        }
                        "capacitance" => {
                            self.advance();
                            self.expect(&LibToken::Colon)?;
                            pin.capacitance = Some(self.expect_number()?);
                            self.expect(&LibToken::Semi)?;
                        }
                        "max_capacitance" | "max_cap" => {
                            self.advance();
                            self.expect(&LibToken::Colon)?;
                            pin.max_capacitance = Some(self.expect_number()?);
                            self.expect(&LibToken::Semi)?;
                        }
                        "min_capacitance" | "min_cap" => {
                            self.advance();
                            self.expect(&LibToken::Colon)?;
                            pin.min_capacitance = Some(self.expect_number()?);
                            self.expect(&LibToken::Semi)?;
                        }
                        "function" => {
                            self.advance();
                            self.expect(&LibToken::Colon)?;
                            let func = self.expect_ident()?;
                            self.expect(&LibToken::Semi)?;
                            pin.function = Some(func);
                        }
                        "clock" => {
                            self.advance();
                            self.expect(&LibToken::Colon)?;
                            let val = self.expect_ident()?;
                            self.expect(&LibToken::Semi)?;
                            pin.clock = Some(val.to_lowercase() == "true");
                        }
                        "timing" => {
                            let arc = self.parse_timing_arc()?;
                            pin.timing_arcs.push(arc);
                        }
                        _ => {
                            // Skip unknown pin attribute
                            self.advance();
                            self.skip_value_or_block();
                        }
                    }
                }
                _ => {
                    self.advance();
                }
            }
        }
        Ok(())
    }

    // ── Timing arc parsing ──

    fn parse_timing_arc(&mut self) -> Result<LibertyTimingArc, String> {
        self.advance(); // consume 'timing'
        self.expect(&LibToken::LParen)?;
        // timing() can have optional arguments — skip them
        while self.peek() != Some(&LibToken::RParen) && self.peek().is_some() {
            self.advance();
        }
        self.expect(&LibToken::RParen)?;
        self.expect(&LibToken::LBrace)?;

        let mut arc = LibertyTimingArc {
            related_pin: String::new(),
            timing_sense: LibertyTimingSense::NonUnate,
            cell_rise: None,
            cell_fall: None,
            rise_transition: None,
            fall_transition: None,
        };

        self.parse_timing_body(&mut arc)?;

        self.expect(&LibToken::RBrace)?;
        Ok(arc)
    }

    fn parse_timing_body(&mut self, arc: &mut LibertyTimingArc) -> Result<(), String> {
        loop {
            match self.peek() {
                None | Some(LibToken::RBrace) => break,
                Some(LibToken::Semi) => {
                    self.advance();
                }
                Some(LibToken::Ident(keyword)) => {
                    let kw = keyword.to_lowercase();
                    match kw.as_str() {
                        "related_pin" => {
                            self.advance();
                            self.expect(&LibToken::Colon)?;
                            arc.related_pin = self.expect_ident()?;
                            self.expect(&LibToken::Semi)?;
                        }
                        "timing_sense" => {
                            self.advance();
                            self.expect(&LibToken::Colon)?;
                            let sense = self.expect_ident()?;
                            match sense.to_lowercase().as_str() {
                                "positive_unate" => {
                                    arc.timing_sense = LibertyTimingSense::PositiveUnate
                                }
                                "negative_unate" => {
                                    arc.timing_sense = LibertyTimingSense::NegativeUnate
                                }
                                "non_unate" => arc.timing_sense = LibertyTimingSense::NonUnate,
                                _ => {}
                            }
                            self.expect(&LibToken::Semi)?;
                        }
                        "cell_rise" | "rise_transition" | "cell_fall" | "fall_transition" => {
                            let table = self.parse_delay_table()?;
                            match kw.as_str() {
                                "cell_rise" => arc.cell_rise = Some(table),
                                "rise_transition" => arc.rise_transition = Some(table),
                                "cell_fall" => arc.cell_fall = Some(table),
                                "fall_transition" => arc.fall_transition = Some(table),
                                _ => {}
                            }
                        }
                        _ => {
                            // Skip unknown timing attribute
                            self.advance();
                            self.skip_value_or_block();
                        }
                    }
                }
                _ => {
                    self.advance();
                }
            }
        }
        Ok(())
    }

    // ── Delay table parsing ──

    fn parse_delay_table(&mut self) -> Result<LibertyDelayTable, String> {
        self.advance(); // consume keyword (cell_rise, etc.)

        let template_name = if self.peek() == Some(&LibToken::LParen) {
            self.advance();
            // Handle empty parens: cell_rise() — no template name
            if self.peek() == Some(&LibToken::RParen) {
                self.advance();
                None
            } else {
                let name = self.expect_ident()?;
                self.expect(&LibToken::RParen)?;
                Some(name)
            }
        } else {
            None
        };

        self.expect(&LibToken::LBrace)?;

        let mut table = LibertyDelayTable {
            template_name,
            index_1: Vec::new(),
            index_2: Vec::new(),
            values: Vec::new(),
        };

        loop {
            match self.peek() {
                None | Some(LibToken::RBrace) => break,
                Some(LibToken::Semi) => {
                    self.advance();
                }
                Some(LibToken::Ident(keyword)) => {
                    let kw = keyword.to_lowercase();
                    match kw.as_str() {
                        "index_1" => {
                            self.advance();
                            self.expect(&LibToken::LParen)?;
                            let vals = self.parse_string_list()?;
                            table.index_1 = vals;
                            self.expect(&LibToken::RParen)?;
                            self.expect(&LibToken::Semi)?;
                        }
                        "index_2" => {
                            self.advance();
                            self.expect(&LibToken::LParen)?;
                            let vals = self.parse_string_list()?;
                            table.index_2 = vals;
                            self.expect(&LibToken::RParen)?;
                            self.expect(&LibToken::Semi)?;
                        }
                        "values" => {
                            self.advance();
                            self.expect(&LibToken::LParen)?;
                            table.values = self.parse_value_matrix()?;
                            self.expect(&LibToken::RParen)?;
                            self.expect(&LibToken::Semi)?;
                        }
                        _ => {
                            self.advance();
                            self.skip_value_or_block();
                        }
                    }
                }
                _ => {
                    self.advance();
                }
            }
        }

        self.expect(&LibToken::RBrace)?;
        Ok(table)
    }

    // ── Helper: parse a comma-separated list of floats from a string ──

    fn parse_string_list(&mut self) -> Result<Vec<f64>, String> {
        // index_1("0.1, 0.5, 1.0") → [0.1, 0.5, 1.0]
        match self.peek() {
            Some(LibToken::StringLit(s)) => {
                let vals: Result<Vec<f64>, _> =
                    s.split(',').map(|v| v.trim().parse::<f64>()).collect();
                self.advance();
                vals.map_err(|e| format!("invalid number in list: {}", e))
            }
            _ => {
                // Try comma-separated numbers without quotes
                let mut vals = Vec::new();
                while matches!(
                    self.peek(),
                    Some(LibToken::Number(_)) | Some(LibToken::Comma) | Some(LibToken::Minus)
                ) {
                    if self.peek() == Some(&LibToken::Comma) {
                        self.advance();
                        continue;
                    }
                    if self.peek() == Some(&LibToken::Minus) {
                        self.advance();
                        if let Some(LibToken::Number(n)) = self.peek() {
                            vals.push(-n);
                            self.advance();
                        }
                        continue;
                    }
                    if let Some(LibToken::Number(n)) = self.peek() {
                        vals.push(*n);
                        self.advance();
                    }
                }
                Ok(vals)
            }
        }
    }

    /// Parse a 2D matrix of values: "0.05, 0.10", "0.10, 0.20"
    fn parse_value_matrix(&mut self) -> Result<Vec<Vec<f64>>, String> {
        let mut matrix = Vec::new();
        loop {
            match self.peek() {
                Some(LibToken::RParen) => break,
                Some(LibToken::StringLit(s)) => {
                    let row: Result<Vec<f64>, _> =
                        s.split(',').map(|v| v.trim().parse::<f64>()).collect();
                    matrix.push(row.map_err(|e| format!("invalid number: {}", e))?);
                    self.advance();
                }
                Some(LibToken::Comma) => {
                    self.advance();
                }
                _ => break,
            }
        }
        Ok(matrix)
    }

    // ── FF parsing ──

    fn parse_ff(&mut self) -> Result<LibertyFF, String> {
        self.advance(); // consume 'ff'
        self.expect(&LibToken::LParen)?;

        // Optional arguments
        while self.peek() != Some(&LibToken::RParen) && self.peek().is_some() {
            self.advance();
        }
        self.expect(&LibToken::RParen)?;
        self.expect(&LibToken::LBrace)?;

        let mut ff = LibertyFF {
            clocked_on: None,
            next_state: None,
            clear: None,
            preset: None,
        };

        loop {
            match self.peek() {
                None | Some(LibToken::RBrace) => break,
                Some(LibToken::Semi) => {
                    self.advance();
                }
                Some(LibToken::Ident(keyword)) => {
                    let kw = keyword.to_lowercase();
                    match kw.as_str() {
                        "clocked_on" => {
                            self.advance();
                            self.expect(&LibToken::Colon)?;
                            ff.clocked_on = Some(self.expect_ident()?);
                            self.expect(&LibToken::Semi)?;
                        }
                        "next_state" => {
                            self.advance();
                            self.expect(&LibToken::Colon)?;
                            ff.next_state = Some(self.expect_ident()?);
                            self.expect(&LibToken::Semi)?;
                        }
                        "clear" => {
                            self.advance();
                            self.expect(&LibToken::Colon)?;
                            ff.clear = Some(self.expect_ident()?);
                            self.expect(&LibToken::Semi)?;
                        }
                        "preset" => {
                            self.advance();
                            self.expect(&LibToken::Colon)?;
                            ff.preset = Some(self.expect_ident()?);
                            self.expect(&LibToken::Semi)?;
                        }
                        _ => {
                            self.advance();
                            self.skip_value_or_block();
                        }
                    }
                }
                _ => {
                    self.advance();
                }
            }
        }

        self.expect(&LibToken::RBrace)?;
        Ok(ff)
    }

    // ── Delay template parsing ──

    fn parse_delay_template(&mut self) -> Result<LibertyDelayTemplate, String> {
        let _keyword = self.expect_ident()?; // delay_template or lu_table_template
        self.expect(&LibToken::LParen)?;
        let name = self.expect_ident()?;
        self.expect(&LibToken::RParen)?;
        self.expect(&LibToken::LBrace)?;

        let mut tmpl = LibertyDelayTemplate {
            name,
            variable_1: String::new(),
            variable_2: None,
            index_1: Vec::new(),
            index_2: Vec::new(),
        };

        loop {
            match self.peek() {
                None | Some(LibToken::RBrace) => break,
                Some(LibToken::Semi) => {
                    self.advance();
                }
                Some(LibToken::Ident(keyword)) => {
                    let kw = keyword.to_lowercase();
                    match kw.as_str() {
                        "variable_1" => {
                            self.advance();
                            self.expect(&LibToken::Colon)?;
                            tmpl.variable_1 = self.expect_ident()?;
                            self.expect(&LibToken::Semi)?;
                        }
                        "variable_2" => {
                            self.advance();
                            self.expect(&LibToken::Colon)?;
                            tmpl.variable_2 = Some(self.expect_ident()?);
                            self.expect(&LibToken::Semi)?;
                        }
                        "index_1" => {
                            self.advance();
                            self.expect(&LibToken::LParen)?;
                            let vals = self.parse_string_list()?;
                            tmpl.index_1 = vals;
                            self.expect(&LibToken::RParen)?;
                            self.expect(&LibToken::Semi)?;
                        }
                        "index_2" => {
                            self.advance();
                            self.expect(&LibToken::LParen)?;
                            let vals = self.parse_string_list()?;
                            tmpl.index_2 = vals;
                            self.expect(&LibToken::RParen)?;
                            self.expect(&LibToken::Semi)?;
                        }
                        _ => {
                            self.advance();
                            self.skip_value_or_block();
                        }
                    }
                }
                _ => {
                    self.advance();
                }
            }
        }

        self.expect(&LibToken::RBrace)?;
        Ok(tmpl)
    }

    // ── Helper ──

    /// Skip a value after colon, or a block after identifier(…) { … }
    fn skip_value_or_block(&mut self) {
        if self.peek() == Some(&LibToken::Colon) {
            self.advance();
            // Skip until ;
            while !matches!(self.peek(), Some(LibToken::Semi) | None) {
                self.advance();
            }
            if self.peek() == Some(&LibToken::Semi) {
                self.advance();
            }
        } else if self.peek() == Some(&LibToken::LParen) {
            // Could be group: keyword(args) { ... }
            self.advance(); // skip (
            let mut depth = 1;
            while let Some(tok) = self.peek() {
                match tok {
                    LibToken::LParen => depth += 1,
                    LibToken::RParen => {
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
            if self.peek() == Some(&LibToken::LBrace) {
                self.skip_block();
            }
        }
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_simple() {
        let lib = r#"
        library(test) {
            cell(AND2) {
                area: 12;
                pin(A) { direction: input; }
                pin(Z) { 
                    direction: output;
                    timing() {
                        related_pin: "A";
                        cell_rise(delay_template_2x2) {
                            index_1("0.1, 0.5");
                            index_2("0.1, 0.5");
                            values("0.05, 0.10", "0.10, 0.20");
                        }
                    }
                }
            }
        }
        "#;
        let library = LibertyLibrary::parse(lib).unwrap();
        assert_eq!(library.name, "test");
        assert!(library.cells.contains_key("AND2"));
    }

    #[test]
    fn test_cell_pin_direction() {
        let lib = r#"
        library(test) {
            cell(BUF) {
                area: 5;
                pin(A) { direction: input; capacitance: 0.01; }
                pin(Y) { 
                    direction: output;
                    timing() {
                        related_pin: "A";
                        timing_sense: positive_unate;
                        cell_rise(delay_template_1x1) {
                            index_1("0.1");
                            values("0.05");
                        }
                        cell_fall(delay_template_1x1) {
                            index_1("0.1");
                            values("0.04");
                        }
                    }
                }
            }
        }
        "#;
        let library = LibertyLibrary::parse(lib).unwrap();
        let buf = library.cells.get("BUF").unwrap();
        assert!((buf.area.unwrap() - 5.0).abs() < 1e-9);

        let pin_a = buf.pins.get("A").unwrap();
        assert_eq!(pin_a.direction, LibertyPinDirection::Input);
        assert!((pin_a.capacitance.unwrap() - 0.01).abs() < 1e-9);

        let pin_y = buf.pins.get("Y").unwrap();
        assert_eq!(pin_y.direction, LibertyPinDirection::Output);
        assert_eq!(pin_y.timing_arcs.len(), 1);

        let arc = &pin_y.timing_arcs[0];
        assert_eq!(arc.related_pin, "A");
        assert!(arc.cell_rise.is_some());
        assert!(arc.cell_fall.is_some());

        let rise = library.get_rise_delay("BUF", "A", "Y");
        assert!(rise.is_some());
        assert!((rise.unwrap() - 0.05).abs() < 1e-9);

        let fall = library.get_fall_delay("BUF", "A", "Y");
        assert!(fall.is_some());
        assert!((fall.unwrap() - 0.04).abs() < 1e-9);
    }

    #[test]
    fn test_multi_input_gate() {
        let lib = r#"
        library(test) {
            cell(AND2) {
                pin(A) { direction: input; }
                pin(B) { direction: input; }
                pin(Z) {
                    direction: output;
                    timing() { related_pin: "A"; cell_rise() { index_1("0.1"); values("0.08"); } }
                    timing() { related_pin: "B"; cell_rise() { index_1("0.1"); values("0.12"); } }
                }
            }
            cell(OR2) {
                pin(A) { direction: input; }
                pin(B) { direction: input; }
                pin(Z) {
                    direction: output;
                    timing() { related_pin: "A"; cell_rise() { index_1("0.1"); values("0.09"); } }
                    timing() { related_pin: "B"; cell_rise() { index_1("0.1"); values("0.13"); } }
                }
            }
            cell(XOR2) {
                pin(A) { direction: input; }
                pin(B) { direction: input; }
                pin(Z) { direction: output; function: "A ^ B"; }
            }
        }
        "#;
        let library = LibertyLibrary::parse(lib).unwrap();
        assert!(library.cells.contains_key("AND2"));
        assert!(library.cells.contains_key("OR2"));
        assert!(library.cells.contains_key("XOR2"));

        let and2 = library.cells.get("AND2").unwrap();
        let pin_z = and2.pins.get("Z").unwrap();
        assert_eq!(pin_z.timing_arcs.len(), 2);

        let rise_a = library.get_rise_delay("AND2", "A", "Z");
        assert!((rise_a.unwrap() - 0.08).abs() < 1e-9);

        let rise_b = library.get_rise_delay("AND2", "B", "Z");
        assert!((rise_b.unwrap() - 0.12).abs() < 1e-9);
    }

    #[test]
    fn test_parse_file() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("maria_lib_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let lib_path = dir.join("test.lib");
        let lib_content = r#"
        library(test) {
            cell(BUF) {
                pin(A) { direction: input; }
                pin(Y) { 
                    direction: output;
                    timing() {
                        related_pin: "A";
                        cell_rise() { index_1("0.1"); values("0.05"); }
                    }
                }
            }
        }
        "#;
        {
            let mut f = std::fs::File::create(&lib_path).unwrap();
            f.write_all(lib_content.as_bytes()).unwrap();
        }

        let library = LibertyLibrary::parse_file(lib_path.to_str().unwrap()).unwrap();
        assert_eq!(library.name, "test");
        assert!(library.cells.contains_key("BUF"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_delay_template_parsing() {
        let lib = r#"
        library(test) {
            lu_table_template(delay_template_2x2) {
                variable_1: input_transition_time;
                variable_2: total_output_net_capacitance;
                index_1("0.1, 0.5");
                index_2("0.1, 0.5");
            }
            cell(AND2) {
                pin(Z) {
                    direction: output;
                    timing() {
                        related_pin: "A";
                        cell_rise(delay_template_2x2) {
                            index_1("0.1, 0.5");
                            index_2("0.1, 0.5");
                            values("0.05, 0.10", "0.10, 0.20");
                        }
                    }
                }
            }
        }
        "#;
        let library = LibertyLibrary::parse(lib).unwrap();
        assert!(library.delay_templates.contains_key("delay_template_2x2"));
        let tmpl = library.delay_templates.get("delay_template_2x2").unwrap();
        assert_eq!(tmpl.variable_1, "input_transition_time");
        assert_eq!(tmpl.index_1.len(), 2);
        assert!((tmpl.index_1[0] - 0.1).abs() < 1e-9);

        let and2 = library.cells.get("AND2").unwrap();
        let pin_z = and2.pins.get("Z").unwrap();
        let table = pin_z.timing_arcs[0].cell_rise.as_ref().unwrap();
        assert_eq!(table.values.len(), 2);
        assert_eq!(table.values[0].len(), 2);
        assert!((table.values[0][0] - 0.05).abs() < 1e-9);
    }

    #[test]
    fn test_ff_cell() {
        let lib = r#"
        library(test) {
            cell(DFF) {
                pin(CLK) { direction: input; clock: true; }
                pin(D) { direction: input; }
                pin(Q) { direction: output; }
                pin(QN) { direction: output; }
                ff() {
                    clocked_on: "CLK";
                    next_state: "D";
                }
            }
        }
        "#;
        let library = LibertyLibrary::parse(lib).unwrap();
        let dff = library.cells.get("DFF").unwrap();
        assert!(dff.ff.is_some());
        let ff = dff.ff.as_ref().unwrap();
        assert_eq!(ff.clocked_on, Some("CLK".to_string()));
        assert_eq!(ff.next_state, Some("D".to_string()));

        let pin_clk = dff.pins.get("CLK").unwrap();
        assert_eq!(pin_clk.clock, Some(true));
    }

    #[test]
    fn test_negative_timing_sense() {
        let lib = r#"
        library(test) {
            cell(NAND2) {
                pin(A) { direction: input; }
                pin(Z) {
                    direction: output;
                    timing() {
                        related_pin: "A";
                        timing_sense: negative_unate;
                        cell_rise() { index_1("0.1"); values("0.07"); }
                    }
                }
            }
        }
        "#;
        let library = LibertyLibrary::parse(lib).unwrap();
        let nand2 = library.cells.get("NAND2").unwrap();
        let pin_z = nand2.pins.get("Z").unwrap();
        let arc = &pin_z.timing_arcs[0];
        assert_eq!(arc.timing_sense, LibertyTimingSense::NegativeUnate);
    }

    #[test]
    fn test_complex_library() {
        // Test a more comprehensive library with multiple cells
        let lib = r#"
        library(gsc_lib) {
            delay_template(delay_template_2x2) {
                variable_1: input_net_transition;
                variable_2: total_output_net_capacitance;
                index_1("0.1, 1.0");
                index_2("0.1, 1.0");
            }
            cell(AND2) { area: 10;
                pin(A) { direction: input; capacitance: 0.005; }
                pin(B) { direction: input; capacitance: 0.005; }
                pin(Z) { direction: output; max_capacitance: 0.5;
                    timing() { related_pin: "A"; timing_sense: positive_unate;
                        cell_rise(delay_template_2x2) {
                            index_1("0.1, 1.0"); index_2("0.1, 1.0");
                            values("0.05, 0.15", "0.15, 0.45");
                        }
                        cell_fall(delay_template_2x2) {
                            index_1("0.1, 1.0"); index_2("0.1, 1.0");
                            values("0.04, 0.12", "0.12, 0.36");
                        }
                    }
                    timing() { related_pin: "B"; timing_sense: positive_unate;
                        cell_rise(delay_template_2x2) {
                            index_1("0.1, 1.0"); index_2("0.1, 1.0");
                            values("0.06, 0.18", "0.18, 0.54");
                        }
                        cell_fall(delay_template_2x2) {
                            index_1("0.1, 1.0"); index_2("0.1, 1.0");
                            values("0.05, 0.15", "0.15, 0.45");
                        }
                    }
                }
            }
            cell(INV) { area: 3;
                pin(A) { direction: input; capacitance: 0.003; }
                pin(Z) { direction: output; max_capacitance: 0.3;
                    timing() { related_pin: "A"; timing_sense: negative_unate;
                        cell_rise(delay_template_2x2) {
                            index_1("0.1, 1.0"); index_2("0.1, 1.0");
                            values("0.03, 0.10", "0.10, 0.30");
                        }
                        cell_fall(delay_template_2x2) {
                            index_1("0.1, 1.0"); index_2("0.1, 1.0");
                            values("0.02, 0.08", "0.08, 0.24");
                        }
                    }
                }
            }
        }
        "#;
        let library = LibertyLibrary::parse(lib).unwrap();
        assert!(library.cells.contains_key("AND2"));
        assert!(library.cells.contains_key("INV"));
        assert!(library.delay_templates.contains_key("delay_template_2x2"));

        // Check INV timing
        let inv_rise = library.get_rise_delay("INV", "A", "Z");
        assert!((inv_rise.unwrap() - 0.03).abs() < 1e-9);

        // Check AND2 timing from pin A
        let and2_rise_a = library.get_rise_delay("AND2", "A", "Z");
        assert!((and2_rise_a.unwrap() - 0.05).abs() < 1e-9);

        // Check AND2 fall from pin B
        let and2_fall_b = library.get_fall_delay("AND2", "B", "Z");
        assert!((and2_fall_b.unwrap() - 0.05).abs() < 1e-9);
    }
}
