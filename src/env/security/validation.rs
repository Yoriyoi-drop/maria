//! Validasi input — cegah path/identifier berbahaya masuk pipeline.

use std::path::Path;

/// Validasi path tidak kosong dan tidak berupa direktori.
pub fn validate_path(path: &Path) -> Result<(), String> {
    let s = path.to_string_lossy();
    if s.trim().is_empty() {
        return Err("path kosong".into());
    }
    Ok(())
}

/// Validasi identifier SystemVerilog: `[a-zA-Z_][a-zA-Z0-9_$]*`.
pub fn validate_identifier(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return Err(format!("identifier tidak valid: '{}'", name)),
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$') {
        return Err(format!("identifier tidak valid: '{}'", name));
    }
    Ok(())
}

/// Cek nama file aman (tanpa path traversal / karakter berbahaya).
pub fn safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_path() {
        assert!(validate_path(Path::new("a.sv")).is_ok());
        assert!(validate_path(Path::new("")).is_err());
    }

    #[test]
    fn test_validate_identifier() {
        assert!(validate_identifier("top_ff").is_ok());
        assert!(validate_identifier("_x$1").is_ok());
        assert!(validate_identifier("1abc").is_err());
        assert!(validate_identifier("a-b").is_err());
        assert!(validate_identifier("").is_err());
    }

    #[test]
    fn test_safe_file_name() {
        assert!(safe_file_name("top.vcd"));
        assert!(!safe_file_name(""));
        assert!(!safe_file_name(".."));
        assert!(!safe_file_name("a/b"));
        assert!(!safe_file_name("a\\b"));
    }
}
