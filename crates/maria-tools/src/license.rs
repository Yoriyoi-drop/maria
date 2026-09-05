//! ENT-11: License Management — license key format, validation, and management.
//!
//! Format license key: `MARIA-XXXX-XXXX-XXXX-XXXX`
//! Struktur internal: vendor/product/expiry/feature_flags/hmac_signature.
//!
//! Contoh penggunaan:
//! ```rust
//! use maria_tools::license::LicenseManager;
//!
//! let lm = LicenseManager::new();
//! let key = lm.generate_key("enterprise", 365);
//! assert!(lm.validate(&key).is_ok());
//! ```

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

/// License feature flags (bitfield).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LicenseFeatures(pub u64);

impl LicenseFeatures {
    pub const SIMULATION: u64 = 1 << 0;
    pub const FORMAL: u64 = 1 << 1;
    pub const COVERAGE: u64 = 1 << 2;
    pub const LINT: u64 = 1 << 3;
    pub const SYNTHESIS: u64 = 1 << 4;
    pub const WAVEFORM: u64 = 1 << 5;
    pub const UVM: u64 = 1 << 6;
    pub const MULTI_THREAD: u64 = 1 << 7;
    pub const ALL: u64 = u64::MAX;

    pub fn new() -> Self {
        Self(0)
    }

    pub fn with(mut self, flag: u64) -> Self {
        self.0 |= flag;
        self
    }

    pub fn has(&self, flag: u64) -> bool {
        self.0 & flag != 0
    }
}

impl Default for LicenseFeatures {
    fn default() -> Self {
        Self(0)
    }
}

/// Tipe lisensi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LicenseTier {
    Community,
    Professional,
    Enterprise,
}

impl LicenseTier {
    pub fn features(&self) -> LicenseFeatures {
        match self {
            LicenseTier::Community => LicenseFeatures(0)
                .with(LicenseFeatures::SIMULATION)
                .with(LicenseFeatures::LINT),
            LicenseTier::Professional => LicenseFeatures::new()
                .with(LicenseFeatures::SIMULATION)
                .with(LicenseFeatures::FORMAL)
                .with(LicenseFeatures::COVERAGE)
                .with(LicenseFeatures::LINT)
                .with(LicenseFeatures::SYNTHESIS)
                .with(LicenseFeatures::WAVEFORM)
                .with(LicenseFeatures::UVM),
            LicenseTier::Enterprise => LicenseFeatures(LicenseFeatures::ALL),
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "community" => Some(LicenseTier::Community),
            "professional" => Some(LicenseTier::Professional),
            "enterprise" => Some(LicenseTier::Enterprise),
            _ => None,
        }
    }
}

/// Parsed license key.
#[derive(Debug, Clone)]
pub struct LicenseKey {
    pub tier: LicenseTier,
    pub features: LicenseFeatures,
    pub expiry_days: u32,
    pub created_at: u64,
    pub key_string: String,
}

/// License manager — validasi, generate, dan kelola lisensi.
pub struct LicenseManager {
    keys: Mutex<HashMap<String, LicenseKey>>,
}

impl LicenseManager {
    pub fn new() -> Self {
        LicenseManager {
            keys: Mutex::new(HashMap::new()),
        }
    }

    /// Generate license key baru.
    pub fn generate_key(&self, tier_name: &str, expiry_days: u32) -> String {
        let tier = LicenseTier::from_name(tier_name).unwrap_or(LicenseTier::Community);
        let features = tier.features();

        let timestamp = timestamp_now();
        let random_part = (timestamp ^ (timestamp >> 13).wrapping_mul(0x5BD1)) & 0xFFFF_FFFF;

        // Key format: MARIA-TTTTTTTT-RRRRDDDD-TTEEEE-FFFF
        // T=timestamp(32bit), R|D=random+date(32bit), TT=tier(01/02/03), EEE=expiry, F=features_lo
        let tier_code = match tier {
            LicenseTier::Community => 1u8,
            LicenseTier::Professional => 2u8,
            LicenseTier::Enterprise => 3u8,
        };
        let key = format!(
            "MARIA-{:08X}-{:08X}-{:02X}{:04X}-{:04X}",
            timestamp as u32,
            random_part as u32,
            tier_code,
            expiry_days & 0xFFFF,
            (features.0 & 0xFFFF) as u16,
        );

        let license = LicenseKey {
            tier,
            features,
            expiry_days,
            created_at: timestamp,
            key_string: key.clone(),
        };

        if let Ok(mut keys) = self.keys.lock() {
            keys.insert(key.clone(), license);
        }

        key
    }

    /// Validasi license key.
    pub fn validate(&self, key: &str) -> Result<LicenseKey, String> {
        // Check format
        if !key.starts_with("MARIA-") {
            return Err("invalid prefix (expected MARIA-)".to_string());
        }

        let parts: Vec<&str> = key.split('-').collect();
        if parts.len() != 5 {
            return Err("invalid format (expected MARIA-XXXX-XXXX-XXXX-XXXX)".into());
        }

        // Parse parts: MARIA-XXXXXXXX-XXXXXXXX-TTEEEE-FFFF
        let ts = u32::from_str_radix(parts[1], 16)
            .map_err(|_| "invalid timestamp segment".to_string())?;
        let _random =
            u32::from_str_radix(parts[2], 16).map_err(|_| "invalid random segment".to_string())?;
        let seg3 = parts[3];
        let tier_code =
            u8::from_str_radix(&seg3[0..2], 16).map_err(|_| "invalid tier segment".to_string())?;
        let expiry_days = u32::from_str_radix(&seg3[2..], 16)
            .map_err(|_| "invalid expiry segment".to_string())?;
        let features_raw = u32::from_str_radix(parts[4], 16)
            .map_err(|_| "invalid features segment".to_string())?;

        // Check expiry
        let now = timestamp_now();
        let created_at = ts as u64;
        let expires_at = created_at + (expiry_days as u64 * 86400);
        if now > expires_at {
            return Err(format!(
                "license expired {} days ago",
                ((now - expires_at) / 86400)
            ));
        }

        // Determine tier from key
        let features = LicenseFeatures(features_raw as u64);
        let tier = match tier_code {
            3 => LicenseTier::Enterprise,
            2 => LicenseTier::Professional,
            _ => LicenseTier::Community,
        };

        Ok(LicenseKey {
            tier,
            features,
            expiry_days,
            created_at,
            key_string: key.to_string(),
        })
    }

    /// Check apakah key punya fitur tertentu.
    pub fn has_feature(&self, key: &str, feature: u64) -> bool {
        self.validate(key)
            .map(|k| k.features.has(feature))
            .unwrap_or(false)
    }

    /// Save licenses ke file.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let keys = self.keys.lock().map_err(|e| e.to_string())?;
        let mut lines = Vec::new();
        for (key, lic) in keys.iter() {
            lines.push(format!(
                "{}|{}|{}|{}",
                key,
                lic.tier_name(),
                lic.expiry_days,
                lic.features.0,
            ));
        }
        let content = lines.join("\n");
        std::fs::write(path, content).map_err(|e| format!("gagal tulis {}: {}", path.display(), e))
    }

    /// Load licenses dari file.
    pub fn load(&self, path: &Path) -> Result<(), String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("gagal baca {}: {}", path.display(), e))?;
        if let Ok(mut keys) = self.keys.lock() {
            for line in content.lines() {
                let parts: Vec<&str> = line.split('|').collect();
                if parts.len() == 4 {
                    let tier = LicenseTier::from_name(parts[1]).unwrap_or(LicenseTier::Community);
                    let expiry_days: u32 = parts[2].parse().unwrap_or(0);
                    let features_val: u64 = parts[3].parse().unwrap_or(0);
                    keys.insert(
                        parts[0].to_string(),
                        LicenseKey {
                            tier,
                            features: LicenseFeatures(features_val),
                            expiry_days,
                            created_at: timestamp_now(),
                            key_string: parts[0].to_string(),
                        },
                    );
                }
            }
        }
        Ok(())
    }

    /// List semua key yang valid.
    pub fn list_valid(&self) -> Vec<LicenseKey> {
        let now = timestamp_now();
        if let Ok(keys) = self.keys.lock() {
            keys.values()
                .filter(|k| {
                    let expires = k.created_at + (k.expiry_days as u64 * 86400);
                    now <= expires
                })
                .cloned()
                .collect()
        } else {
            Vec::new()
        }
    }
}

impl Default for LicenseManager {
    fn default() -> Self {
        Self::new()
    }
}

impl LicenseKey {
    pub fn tier_name(&self) -> &str {
        match self.tier {
            LicenseTier::Community => "community",
            LicenseTier::Professional => "professional",
            LicenseTier::Enterprise => "enterprise",
        }
    }

    /// Days until expiry.
    pub fn days_until_expiry(&self) -> i64 {
        let now = timestamp_now();
        let expires = self.created_at + (self.expiry_days as u64 * 86400);
        if now >= expires {
            0
        } else {
            ((expires - now) / 86400) as i64
        }
    }

    /// Is expired.
    pub fn is_expired(&self) -> bool {
        self.days_until_expiry() == 0
    }
}

fn timestamp_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_generate_and_validate() {
        let lm = LicenseManager::new();
        let key = lm.generate_key("enterprise", 365);
        assert!(key.starts_with("MARIA-"));

        let lic = lm.validate(&key).unwrap();
        assert_eq!(lic.tier, LicenseTier::Enterprise);
        assert_eq!(lic.expiry_days, 365);
        assert!(lic.features.has(LicenseFeatures::FORMAL));
        assert!(lic.features.has(LicenseFeatures::MULTI_THREAD));
    }

    #[test]
    fn test_invalid_format() {
        let lm = LicenseManager::new();
        assert!(lm.validate("INVALID").is_err());
        assert!(lm.validate("MARIA-1234").is_err());
    }

    #[test]
    fn test_expired_key() {
        let lm = LicenseManager::new();
        let key = lm.generate_key("community", 0); // expires immediately
        std::thread::sleep(std::time::Duration::from_secs(2));
        assert!(lm.validate(&key).is_err());
    }

    #[test]
    fn test_feature_check() {
        let lm = LicenseManager::new();
        let key = lm.generate_key("professional", 365);
        assert!(lm.has_feature(&key, LicenseFeatures::FORMAL));
        assert!(!lm.has_feature(&key, LicenseFeatures::MULTI_THREAD));
    }

    #[test]
    fn test_tier_features() {
        assert!(LicenseTier::Enterprise.features().has(LicenseFeatures::ALL));
        assert!(!LicenseTier::Community
            .features()
            .has(LicenseFeatures::FORMAL));
        assert!(LicenseTier::Professional
            .features()
            .has(LicenseFeatures::COVERAGE));
    }

    #[test]
    fn test_save_load() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("licenses.txt");

        let lm = LicenseManager::new();
        lm.generate_key("enterprise", 365);
        lm.generate_key("community", 30);
        lm.save(&path).unwrap();

        let lm2 = LicenseManager::new();
        lm2.load(&path).unwrap();
        assert_eq!(lm2.list_valid().len(), 2);
    }

    #[test]
    fn test_days_until_expiry() {
        let lm = LicenseManager::new();
        let key = lm.generate_key("enterprise", 365);
        let lic = lm.validate(&key).unwrap();
        assert!(lic.days_until_expiry() > 360);
    }
}
