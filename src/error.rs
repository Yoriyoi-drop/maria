use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct SimError {
    pub line: Option<usize>,
    pub message: String,
}

impl SimError {
    pub fn new(line: Option<usize>, message: impl Into<String>) -> Self {
        SimError { line, message: message.into() }
    }
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "line {}: {}", line, self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

impl From<String> for SimError {
    fn from(msg: String) -> Self {
        let line = if msg.starts_with("line ") {
            let rest = &msg[5..];
            if let Some(end) = rest.find(':') {
                rest[..end].parse::<usize>().ok()
            } else {
                None
            }
        } else {
            None
        };
        SimError { line, message: msg }
    }
}
