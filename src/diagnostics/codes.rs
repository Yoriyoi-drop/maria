//! Error codes — centralized error code definitions.

pub use super::diagnostic::DiagCode;

/// All parse error codes (E1xxx)
pub const PARSE_ERRORS: &[(DiagCode, &str)] = &[
    (DiagCode::UnexpectedToken, "unexpected token"),
    (DiagCode::ExpectedToken, "expected token"),
    (DiagCode::ExpectedSemi, "expected ';'"),
    (DiagCode::UnclosedBlock, "unclosed block"),
];

/// All semantic error codes (E2xxx)
pub const SEMANTIC_ERRORS: &[(DiagCode, &str)] = &[
    (DiagCode::UndefinedSignal, "undefined signal"),
    (DiagCode::TypeMismatch, "type mismatch"),
    (DiagCode::WidthMismatch, "width mismatch"),
];

/// All elaboration error codes (E3xxx)
pub const ELAB_ERRORS: &[(DiagCode, &str)] = &[
    (DiagCode::ModuleNotFound, "module not found"),
    (DiagCode::CircularDependency, "circular dependency"),
    (DiagCode::ParamMismatch, "parameter mismatch"),
];

/// All runtime error codes (E9xxx)
pub const RUNTIME_ERRORS: &[(DiagCode, &str)] = &[
    (DiagCode::SimulationError, "simulation error"),
    (DiagCode::OutOfBounds, "out of bounds"),
];

/// Get all error codes.
pub fn all_codes() -> Vec<(DiagCode, &'static str)> {
    PARSE_ERRORS
        .iter()
        .chain(SEMANTIC_ERRORS.iter())
        .chain(ELAB_ERRORS.iter())
        .chain(RUNTIME_ERRORS.iter())
        .cloned()
        .collect()
}

/// Lookup error code by number string.
pub fn lookup_code(code_str: &str) -> Option<DiagCode> {
    all_codes()
        .iter()
        .find(|(c, _)| c.as_str() == code_str)
        .map(|(c, _)| *c)
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_codes_count() {
        let codes = all_codes();
        assert!(codes.len() >= 10); // At least 10 error codes
    }

    #[test]
    fn test_lookup_code() {
        assert_eq!(lookup_code("E1001"), Some(DiagCode::UnexpectedToken));
        assert_eq!(lookup_code("E3001"), Some(DiagCode::ModuleNotFound));
        assert_eq!(lookup_code("E9999"), None);
    }
}
