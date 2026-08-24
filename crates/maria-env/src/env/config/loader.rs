//! Loader config: membaca file TOML / JSON menjadi `MariaConfig`.
//! (YAML belum didukung — belum ada dependency `serde_yaml`.)

use maria_core::config::MariaConfig;
use std::path::Path;

/// Format file config yang didukung.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFileFormat {
    Toml,
    Json,
}

impl ConfigFileFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConfigFileFormat::Toml => "toml",
            ConfigFileFormat::Json => "json",
        }
    }
}

/// Deteksi format dari ekstensi file (.toml / .json).
pub fn detect_format(path: &Path) -> Result<ConfigFileFormat, String> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("toml") => Ok(ConfigFileFormat::Toml),
        Some("json") => Ok(ConfigFileFormat::Json),
        Some(ext) => Err(format!(
            "config '{}': format '{}' tidak didukung (pakai .toml atau .json)",
            path.display(),
            ext
        )),
        None => Err(format!(
            "config '{}': tanpa ekstensi — pakai .toml atau .json",
            path.display()
        )),
    }
}

/// Muat config dari file (TOML atau JSON, sesuai ekstensi).
pub fn load_from_path(path: &Path) -> Result<MariaConfig, String> {
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("config '{}': {}", path.display(), e))?;
    load_from_str(&content, detect_format(path)?)
        .map_err(|e| format!("config '{}': {}", path.display(), e))
}

/// Muat config dari string (format eksplisit).
pub fn load_from_str(content: &str, format: ConfigFileFormat) -> Result<MariaConfig, String> {
    match format {
        ConfigFileFormat::Toml => {
            toml::from_str(content).map_err(|e| format!("invalid TOML: {}", e))
        }
        ConfigFileFormat::Json => {
            serde_json::from_str(content).map_err(|e| format!("invalid JSON: {}", e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_format() {
        assert_eq!(
            detect_format(Path::new("a.toml")).unwrap(),
            ConfigFileFormat::Toml
        );
        assert_eq!(
            detect_format(Path::new("a.json")).unwrap(),
            ConfigFileFormat::Json
        );
        assert!(detect_format(Path::new("a.yaml")).is_err());
        assert!(detect_format(Path::new("a")).is_err());
    }

    #[test]
    fn test_load_toml_str() {
        let cfg = load_from_str("[compiler]\njobs = 4\n", ConfigFileFormat::Toml).unwrap();
        assert_eq!(cfg.compiler.jobs, Some(4));
    }

    #[test]
    fn test_load_json_str() {
        let cfg = load_from_str(r#"{"compiler": {"jobs": 2}}"#, ConfigFileFormat::Json).unwrap();
        assert_eq!(cfg.compiler.jobs, Some(2));
    }

    #[test]
    fn test_load_invalid_toml() {
        assert!(load_from_str("jobs = [", ConfigFileFormat::Toml).is_err());
    }
}
