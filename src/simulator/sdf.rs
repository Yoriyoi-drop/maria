use std::collections::HashMap;
use std::fs;

/// SDF (Standard Delay Format) parser for timing annotation.
/// Supports basic SDF constructs: DELCELL, DELNET, TIMINGCHECK.
#[derive(Debug, Clone)]
pub struct SdfData {
    pub cell_delays: HashMap<String, CellDelay>,
    pub net_delays: HashMap<String, NetDelay>,
    pub timing_checks: Vec<TimingCheck>,
}

#[derive(Debug, Clone)]
pub struct CellDelay {
    pub rise: Option<f64>,
    pub fall: Option<f64>,
    pub to_rise: HashMap<String, f64>,
    pub to_fall: HashMap<String, f64>,
}

#[derive(Debug, Clone)]
pub struct NetDelay {
    pub rise: Option<f64>,
    pub fall: Option<f64>,
}

#[derive(Debug, Clone)]
pub enum TimingCheck {
    Setup {
        signal: String,
        ref_signal: String,
        delay: f64,
    },
    Hold {
        signal: String,
        ref_signal: String,
        delay: f64,
    },
    Width {
        signal: String,
        delay: f64,
    },
    Period {
        signal: String,
        delay: f64,
    },
}

impl SdfData {
    pub fn new() -> Self {
        SdfData {
            cell_delays: HashMap::new(),
            net_delays: HashMap::new(),
            timing_checks: Vec::new(),
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
                        pos += 2;
                        while pos < tokens.len() && tokens[pos] != ")" {
                            pos += 1;
                        }
                        if pos < tokens.len() { pos += 1; }
                    }
                    "DELAYCELL" | "CELL" => {
                        let (name, delay, new_pos) = parse_cell(&tokens, pos)?;
                        sdf.cell_delays.insert(name, delay);
                        pos = new_pos;
                    }
                    "DELAYNET" | "NET" => {
                        let (name, delay, new_pos) = parse_net(&tokens, pos)?;
                        sdf.net_delays.insert(name, delay);
                        pos = new_pos;
                    }
                    "TIMINGCHECK" => {
                        let (checks, new_pos) = parse_timing_checks(&tokens, pos)?;
                        sdf.timing_checks.extend(checks);
                        pos = new_pos;
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

fn tokenize(content: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_comment = false;
    let mut chars = content.chars().peekable();

    while let Some(c) = chars.next() {
        if in_comment {
            if c == '\n' {
                in_comment = false;
            }
            continue;
        }
        match c {
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                in_comment = true;
            }
            '/' if chars.peek() == Some(&'/') => {
                in_comment = true;
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
                let mut s = String::new();
                for c2 in chars.by_ref() {
                    if c2 == '"' { break; }
                    s.push(c2);
                }
                tokens.push(format!("\"{}\"", s));
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

fn parse_cell(tokens: &[String], pos: usize) -> Result<(String, CellDelay, usize), String> {
    let mut p = pos + 2; // skip "(" and "DELAYCELL"/"CELL"
    let name = if p < tokens.len() && tokens[p] != "(" {
        let n = tokens[p].trim_matches('"').to_string();
        p += 1;
        n
    } else {
        "unknown".to_string()
    };

    let mut delay = CellDelay {
        rise: None,
        fall: None,
        to_rise: HashMap::new(),
        to_fall: HashMap::new(),
    };

    while p < tokens.len() && tokens[p] != ")" {
        if tokens[p] == "(" && p + 1 < tokens.len() {
            match tokens[p + 1].as_str() {
                "IOPATH" => {
                    p += 2;
                    let from = if p < tokens.len() && tokens[p] != "(" {
                        let s = tokens[p].trim_matches('"').to_string();
                        p += 1;
                        s
                    } else {
                        "*".to_string()
                    };
                    let to = if p < tokens.len() && tokens[p] != "(" {
                        let s = tokens[p].trim_matches('"').to_string();
                        p += 1;
                        s
                    } else {
                        "*".to_string()
                    };
                    let (rise, fall, new_p) = parse_rise_fall(tokens, p)?;
                    p = new_p;
                    if from == "*" && to == "*" {
                        delay.rise = Some(rise);
                        delay.fall = Some(fall);
                    } else {
                        delay.to_rise.insert(format!("{}->{}", from, to), rise);
                        delay.to_fall.insert(format!("{}->{}", from, to), fall);
                    }
                }
                _ => {
                    p += 1;
                }
            }
        } else {
            p += 1;
        }
    }
    if p < tokens.len() { p += 1; } // skip closing )

    Ok((name, delay, p))
}

fn parse_net(tokens: &[String], pos: usize) -> Result<(String, NetDelay, usize), String> {
    let mut p = pos + 2; // skip "(" and "DELAYNET"/"NET"
    let name = if p < tokens.len() && tokens[p] != "(" {
        let n = tokens[p].trim_matches('"').to_string();
        p += 1;
        n
    } else {
        "unknown".to_string()
    };

    let mut net_delay = NetDelay { rise: None, fall: None };

    while p < tokens.len() && tokens[p] != ")" {
        if tokens[p] == "(" && p + 1 < tokens.len() {
            match tokens[p + 1].as_str() {
                "ABSDELAY" | "DELAY" => {
                    p += 2;
                    let (rise, fall, new_p) = parse_rise_fall(tokens, p)?;
                    p = new_p;
                    net_delay.rise = Some(rise);
                    net_delay.fall = Some(fall);
                }
                _ => {
                    p += 1;
                }
            }
        } else {
            p += 1;
        }
    }
    if p < tokens.len() { p += 1; }

    Ok((name, net_delay, p))
}

fn parse_rise_fall(tokens: &[String], pos: usize) -> Result<(f64, f64, usize), String> {
    let mut p = pos;
    let mut rise = 0.0;
    let mut fall = 0.0;

    if p < tokens.len() && tokens[p] == "(" {
        p += 1;
        // Parse triple: (value r_f r_f)
        if p < tokens.len() {
            rise = tokens[p].parse::<f64>().unwrap_or(0.0);
            p += 1;
        }
        if p < tokens.len() {
            fall = tokens[p].parse::<f64>().unwrap_or(0.0);
            p += 1;
        }
        if p < tokens.len() { p += 1; } // skip )
    } else if p < tokens.len() {
        rise = tokens[p].parse::<f64>().unwrap_or(0.0);
        fall = rise;
        p += 1;
    }

    Ok((rise, fall, p))
}

fn parse_timing_checks(tokens: &[String], pos: usize) -> Result<(Vec<TimingCheck>, usize), String> {
    let mut p = pos + 1; // skip "(TIMINGCHECK"
    let mut checks = Vec::new();

    while p < tokens.len() && tokens[p] != ")" {
        match tokens[p].as_str() {
            "(SETUP" | "(SETUPHOLD" => {
                p += 1;
                let mut sig = String::new();
                let mut ref_sig = String::new();
                let mut delay = 0.0;

                while p < tokens.len() && tokens[p] != ")" {
                    if tokens[p] == "(" {
                        p += 1;
                        let label = if p < tokens.len() { tokens[p].clone() } else { break };
                        p += 1;
                        let value = if p < tokens.len() { tokens[p].clone() } else { break };
                        p += 1;
                        if p < tokens.len() { p += 1; } // skip )

                        match label.as_str() {
                            "IOTIMING" | "NETTIMING" => {}
                            _ => {}
                        }
                        let value = value.trim_matches('"');
                        if sig.is_empty() {
                            sig = value.to_string();
                        } else if ref_sig.is_empty() {
                            ref_sig = value.to_string();
                        } else if let Ok(d) = value.parse::<f64>() {
                            delay = d;
                        }
                    } else {
                        p += 1;
                    }
                }
                if p < tokens.len() { p += 1; } // skip )
                checks.push(TimingCheck::Setup {
                    signal: sig,
                    ref_signal: ref_sig,
                    delay,
                });
            }
            "(HOLD" => {
                p += 1;
                let mut sig = String::new();
                let mut ref_sig = String::new();
                let mut delay = 0.0;

                while p < tokens.len() && tokens[p] != ")" {
                    if tokens[p] == "(" {
                        p += 1;
                        let _label = if p < tokens.len() { tokens[p].clone() } else { break };
                        p += 1;
                        let value = if p < tokens.len() { tokens[p].clone() } else { break };
                        p += 1;
                        if p < tokens.len() { p += 1; }

                        let value = value.trim_matches('"');
                        if sig.is_empty() {
                            sig = value.to_string();
                        } else if ref_sig.is_empty() {
                            ref_sig = value.to_string();
                        } else if let Ok(d) = value.parse::<f64>() {
                            delay = d;
                        }
                    } else {
                        p += 1;
                    }
                }
                if p < tokens.len() { p += 1; }
                checks.push(TimingCheck::Hold {
                    signal: sig,
                    ref_signal: ref_sig,
                    delay,
                });
            }
            _ => {
                p += 1;
            }
        }
    }

    Ok((checks, p))
}
