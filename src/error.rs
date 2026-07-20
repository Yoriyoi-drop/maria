use thiserror::Error;

#[derive(Error, Debug)]
pub enum SimError {
    #[error("{0}")]
    Parse(String),
    #[error("{0}")]
    Elaborate(String),
    #[error("{0}")]
    Runtime(String),
    #[error("{0}")]
    Preprocessor(String),
    #[error("{0}")]
    Waveform(String),
    #[error("{0}")]
    Debugger(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

impl SimError {
    pub fn new(line: Option<usize>, message: impl Into<String>) -> Self {
        let msg = message.into();
        match line {
            Some(line) => SimError::Parse(format!("line {}: {}", line, msg)),
            None => SimError::Runtime(msg),
        }
    }
    pub fn parse(msg: impl Into<String>) -> Self { SimError::Parse(msg.into()) }
    pub fn elaborate(msg: impl Into<String>) -> Self { SimError::Elaborate(msg.into()) }
    pub fn runtime(msg: impl Into<String>) -> Self { SimError::Runtime(msg.into()) }
    pub fn preprocessor(msg: impl Into<String>) -> Self { SimError::Preprocessor(msg.into()) }
    pub fn waveform(msg: impl Into<String>) -> Self { SimError::Waveform(msg.into()) }
    pub fn debugger(msg: impl Into<String>) -> Self { SimError::Debugger(msg.into()) }
}

impl From<String> for SimError {
    fn from(msg: String) -> Self {
        let is_parse = msg.starts_with("line ") && msg[5..].find(':').is_some();
        if is_parse { SimError::Parse(msg) } else { SimError::Runtime(msg) }
    }
}

impl From<&str> for SimError {
    fn from(msg: &str) -> Self { SimError::Runtime(msg.to_string()) }
}
