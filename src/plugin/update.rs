#![allow(dead_code)]

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct PluginUpdatePackage {
    pub zip_url: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct PluginUpdateManifest {
    pub version: String,
    pub entry_version: u32,
    pub spawn_version: u32,
    #[serde(default)]
    pub x86_64: Option<PluginUpdatePackage>,
    #[serde(default)]
    pub aarch64: Option<PluginUpdatePackage>,
}

impl PluginUpdateManifest {
    pub fn from_json_str(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    pub fn package_for_current_arch(&self) -> Option<PluginUpdatePackage> {
        self.package_for_arch(std::env::consts::ARCH)
    }

    pub fn package_for_arch(&self, arch: &str) -> Option<PluginUpdatePackage> {
        match arch {
            "x86_64" => self.x86_64.clone(),
            "aarch64" => self.aarch64.clone(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_arch_packages_and_versions() {
        let src = r#"
            {
                "version": "0.2.0",
                "entry_version": 2,
                "spawn_version": 6,
                "x86_64": {
                    "zip_url": "https://example.org/owe/x86_64.zip",
                    "sha256": "x86_64-sha256"
                },
                "aarch64": {
                    "zip_url": "https://example.org/owe/aarch64.zip",
                    "sha256": "aarch64-sha256"
                }
            }
        "#;
        let m = PluginUpdateManifest::from_json_str(src).expect("parses");
        assert_eq!(m.entry_version, 2);
        assert_eq!(m.spawn_version, 6);
        let x86_64 = m.package_for_arch("x86_64").expect("x86_64 package");
        assert_eq!(x86_64.zip_url, "https://example.org/owe/x86_64.zip");
        assert_eq!(x86_64.sha256, "x86_64-sha256");
        let aarch64 = m.package_for_arch("aarch64").expect("aarch64 package");
        assert_eq!(aarch64.zip_url, "https://example.org/owe/aarch64.zip");
        assert_eq!(aarch64.sha256, "aarch64-sha256");
    }

    #[test]
    fn rejects_version_lists() {
        let src = r#"
            {
                "version": "0.2.0",
                "entry_version": [2],
                "spawn_version": [6],
                "x86_64": {
                    "zip_url": "https://example.org/owe/x86_64.zip",
                    "sha256": "x86_64-sha256"
                }
            }
        "#;
        assert!(PluginUpdateManifest::from_json_str(src).is_err());
    }
}
