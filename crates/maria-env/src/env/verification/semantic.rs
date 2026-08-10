/// SemanticStatus — status readiness sebelum simulasi (mirip check readiness
/// di main.rs): parse, semantic, hierarchy, top resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticStatus {
    pub parse_ok: bool,
    pub semantic_ok: bool,
    pub hierarchy_ok: bool,
    pub top_resolved: bool,
    pub analysis_mode: bool,
}

impl SemanticStatus {
    pub fn ready(&self) -> bool {
        self.parse_ok
            && self.semantic_ok
            && self.hierarchy_ok
            && self.top_resolved
            && !self.analysis_mode
    }

    pub fn issues(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if !self.parse_ok {
            v.push("parse");
        }
        if !self.semantic_ok {
            v.push("semantic");
        }
        if !self.hierarchy_ok {
            v.push("hierarchy");
        }
        if !self.top_resolved {
            v.push("top resolution");
        }
        if self.analysis_mode {
            v.push("analysis mode");
        }
        v
    }
}

impl Default for SemanticStatus {
    fn default() -> Self {
        SemanticStatus {
            parse_ok: true,
            semantic_ok: true,
            hierarchy_ok: true,
            top_resolved: false,
            analysis_mode: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ready_when_all_ok() {
        let s = SemanticStatus {
            parse_ok: true,
            semantic_ok: true,
            hierarchy_ok: true,
            top_resolved: true,
            analysis_mode: false,
        };
        assert!(s.ready());
        assert!(s.issues().is_empty());
    }

    #[test]
    fn test_not_ready_when_top_missing() {
        let s = SemanticStatus::default();
        assert!(!s.ready());
        assert!(s.issues().contains(&"top resolution"));
    }
}
