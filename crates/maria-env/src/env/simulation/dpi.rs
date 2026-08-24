/// DpiInfo — informasi ketersediaan DPI-C pada build ini.
#[derive(Debug, Clone, Copy)]
pub struct DpiInfo {
    pub available: bool,
}

impl DpiInfo {
    pub fn detect() -> Self {
        DpiInfo {
            available: cfg!(feature = "dpi"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dpi_detect() {
        let d = DpiInfo::detect();
        assert_eq!(d.available, cfg!(feature = "dpi"));
    }
}
