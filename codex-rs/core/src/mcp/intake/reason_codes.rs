//! Explainable reason code registry for MCP intake insights.
//!
//! Trace: REQ-DATA-01 (reason codes for explainability).

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::SystemTime;

use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReasonCodeId(String);

impl ReasonCodeId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ReasonCodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for ReasonCodeId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for ReasonCodeId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasonCode {
    pub id: ReasonCodeId,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub severity: Option<String>,
}

#[derive(Debug, Default)]
pub struct ReasonCatalog {
    codes: HashMap<ReasonCodeId, ReasonCode>,
    version: Option<String>,
    last_loaded: Option<SystemTime>,
}

#[derive(Debug, Deserialize)]
struct ReasonCatalogFile {
    version: Option<String>,
    codes: Vec<ReasonCode>,
}

impl ReasonCatalog {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn load_embedded() -> Result<Self> {
        static DATA: &str = include_str!("reason_codes.json");
        Self::load_from_reader(DATA.as_bytes())
    }

    pub fn load_from_path(path: &Path) -> Result<Self> {
        let file = File::open(path)
            .with_context(|| format!("Failed to open reason codes: {}", path.display()))?;
        Self::load_from_reader(file)
    }

    pub fn load_from_reader<R: Read>(mut reader: R) -> Result<Self> {
        let mut buf = String::new();
        reader
            .read_to_string(&mut buf)
            .context("Failed to read reason codes")?;
        if buf.trim().is_empty() {
            return Err(anyhow!("Reason codes file is empty"));
        }
        let parsed: ReasonCatalogFile = serde_json::from_str(&buf)
            .context("Failed to parse reason codes (expected JSON with {\"codes\": [...]})")?;
        let mut map = HashMap::with_capacity(parsed.codes.len());
        for code in parsed.codes {
            map.insert(code.id.clone(), code);
        }
        Ok(Self {
            codes: map,
            version: parsed.version,
            last_loaded: Some(SystemTime::now()),
        })
    }

    pub fn get(&self, id: &ReasonCodeId) -> Option<&ReasonCode> {
        self.codes.get(id)
    }

    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    pub fn last_loaded(&self) -> Option<SystemTime> {
        self.last_loaded
    }

    pub fn codes(&self) -> impl Iterator<Item = &ReasonCode> {
        self.codes.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_reason_catalog() {
        let json = r#"{
            "version": "1.0.0",
            "codes": [
                {"id": "test.reason", "title": "Test", "description": "Demo"}
            ]
        }"#;
        let catalog = ReasonCatalog::load_from_reader(json.as_bytes()).expect("catalog");
        let code = catalog
            .get(&ReasonCodeId::from("test.reason"))
            .expect("code");
        assert_eq!(code.title, "Test");
        assert_eq!(catalog.version(), Some("1.0.0"));
    }

    #[test]
    fn rejects_empty_catalog() {
        let err = ReasonCatalog::load_from_reader("".as_bytes()).unwrap_err();
        assert!(format!("{err}").contains("empty"));
    }
}
