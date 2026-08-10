use maria_simulator::simulator::types::XPropagationMode;
use maria_simulator::simulator::value::set_xprop_mode;

/// XPropMode — mode X-propagation terenkripsi untuk verification context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XPropMode {
    Optimistic,
    Pessimistic,
    XAnywhere,
}

impl XPropMode {
    pub fn from_str(s: &str) -> Option<Self> {
        match XPropagationMode::from_str(s)? {
            XPropagationMode::Optimistic => Some(XPropMode::Optimistic),
            XPropagationMode::Pessimistic => Some(XPropMode::Pessimistic),
            XPropagationMode::XAnywhere => Some(XPropMode::XAnywhere),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            XPropMode::Optimistic => "optimistic",
            XPropMode::Pessimistic => "pessimistic",
            XPropMode::XAnywhere => "x-anywhere",
        }
    }

    /// Konversi ke mode engine.
    pub fn to_engine(&self) -> XPropagationMode {
        match self {
            XPropMode::Optimistic => XPropagationMode::Optimistic,
            XPropMode::Pessimistic => XPropagationMode::Pessimistic,
            XPropMode::XAnywhere => XPropagationMode::XAnywhere,
        }
    }
}

/// Setel mode X-propagation global (dipakai engine).
pub fn set_xprop(mode: XPropMode) {
    set_xprop_mode(mode.to_engine());
}

/// Mode X-propagation aktif saat ini.
pub fn current_xprop() -> XPropMode {
    let current = maria_simulator::simulator::value::get_xprop_mode();
    match current {
        XPropagationMode::Optimistic => XPropMode::Optimistic,
        XPropagationMode::Pessimistic => XPropMode::Pessimistic,
        XPropagationMode::XAnywhere => XPropMode::XAnywhere,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xprop_roundtrip() {
        assert_eq!(XPropMode::from_str("pessimistic"), Some(XPropMode::Pessimistic));
        assert_eq!(XPropMode::from_str("nope"), None);
        assert_eq!(XPropMode::Pessimistic.as_str(), "pessimistic");
    }

    #[test]
    fn test_set_xprop_global() {
        set_xprop(XPropMode::Optimistic);
        assert_eq!(current_xprop(), XPropMode::Optimistic);
    }
}
