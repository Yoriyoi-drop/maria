//! ENT-37: IP Packaging - IP-XACT (IEEE 1685) XML component description.

use std::path::Path;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpxactComponent {
    pub name: String,
    pub version: String,
    pub vendor: String,
    pub library: String,
    pub description: Option<String>,
    pub ports: Vec<IpxactPort>,
    pub parameters: Vec<IpxactParameter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpxactPort {
    pub name: String,
    pub direction: String,
    pub width: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpxactParameter {
    pub name: String,
    pub value: String,
}

impl IpxactComponent {
    pub fn from_module(name: &str, ports: &[(String, String, Option<u32>)]) -> Self {
        IpxactComponent {
            name: name.to_string(),
            version: "1.0".into(),
            vendor: "maria".into(),
            library: "rtl".into(),
            description: None,
            ports: ports.iter().map(|(n, d, w)| IpxactPort {
                name: n.clone(),
                direction: d.clone(),
                width: *w,
            }).collect(),
            parameters: Vec::new(),
        }
    }

    pub fn to_xml(&self) -> String {
        let mut xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<component xmlns=\"http://www.accellera.org/XMLSchema/IPXACT/1685\">\n\
  <vendor>{}</vendor>\n  <library>{}</library>\n\
  <name>{}</name>\n  <version>{}</version>\n",
            esc(&self.vendor), esc(&self.library), esc(&self.name), esc(&self.version)
        );
        if let Some(ref d) = self.description {
            xml.push_str(&format!("  <description>{}</description>\n", esc(d)));
        }
        xml.push_str("  <model>\n    <ports>\n");
        for p in &self.ports {
            xml.push_str(&format!("      <port>\n        <name>{}</name>\n        <wire>\n          <direction>{}</direction>\n", esc(&p.name), esc(&p.direction)));
            if let Some(w) = p.width {
                xml.push_str(&format!("          <vector><left>{}</left><right>0</right></vector>\n", w - 1));
            }
            xml.push_str("        </wire>\n      </port>\n");
        }
        xml.push_str("    </ports>\n  </model>\n</component>");
        xml
    }

    pub fn save_xml(&self, path: &Path) -> Result<(), String> {
        std::fs::write(path, self.to_xml()).map_err(|e| format!("{}", e))
    }

    pub fn summary(&self) -> String {
        format!("{}/{} v{} ({} ports)", self.vendor, self.name, self.version, self.ports.len())
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_ipxact_xml() {
        let c = IpxactComponent::from_module("counter", &[
            ("clk".into(), "in".into(), None),
            ("cnt".into(), "out".into(), Some(8)),
        ]);
        let xml = c.to_xml();
        assert!(xml.contains("<name>counter</name>"));
        assert!(xml.contains("<direction>in</direction>"));
        assert!(xml.contains("<left>7</left>"));
    }

    #[test]
    fn test_ipxact_save() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.xml");
        let c = IpxactComponent::from_module("alu", &[]);
        c.save_xml(&path).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("component"));
    }

    #[test]
    fn test_ipxact_summary() {
        let c = IpxactComponent::from_module("alu", &[("a".into(), "in".into(), Some(8))]);
        assert!(c.summary().contains("alu"));
    }
}
