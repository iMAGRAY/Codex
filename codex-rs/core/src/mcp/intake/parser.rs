//! Source parser for MCP intake workflows.
//!
//! Trace: REQ-DATA-01 (deterministic source analysis) and REQ-REL-01 (stable fingerprinting).

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use sha1::Digest;
use sha1::Sha1;

/// Input source type provided to the intake engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKind {
    Directory,
    Archive,
    Manifest,
}

/// Source metadata and fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFingerprint {
    pub digest: String,
    pub last_modified: Option<SystemTime>,
}

/// Normalised source object ready for downstream analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSource {
    path: PathBuf,
    kind: SourceKind,
    fingerprint: SourceFingerprint,
}

impl ParsedSource {
    pub fn new(path: PathBuf, kind: SourceKind, fingerprint: SourceFingerprint) -> Self {
        Self {
            path,
            kind,
            fingerprint,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn kind(&self) -> SourceKind {
        self.kind
    }

    pub fn fingerprint(&self) -> &SourceFingerprint {
        &self.fingerprint
    }
}

/// Parser configuration.
#[derive(Debug, Clone)]
pub struct SourceParser {
    cwd: Option<PathBuf>,
}

impl SourceParser {
    pub fn new(cwd: Option<PathBuf>) -> Self {
        Self { cwd }
    }

    pub fn cwd(&self) -> Option<&Path> {
        self.cwd.as_deref()
    }

    pub fn parse(&self, raw_input: &str) -> Result<ParsedSource> {
        let resolved = self.resolve_input(raw_input.trim())?;
        let metadata = fs::metadata(&resolved)
            .with_context(|| format!("Failed to read source: {}", resolved.display()))?;
        let kind = self.detect_kind(&resolved, &metadata)?;
        let fingerprint = self.fingerprint(&resolved, &metadata)?;

        Ok(ParsedSource::new(resolved, kind, fingerprint))
    }

    fn resolve_input(&self, raw: &str) -> Result<PathBuf> {
        if raw.is_empty() {
            return Err(anyhow!("Source not provided"));
        }

        if raw.starts_with(':') {
            return self.resolve_macro(raw);
        }

        let path = match raw {
            p if p.starts_with('~') => {
                let home =
                    dirs::home_dir().ok_or_else(|| anyhow!("Home directory is unavailable"))?;
                home.join(p.trim_start_matches('~'))
            }
            _ => PathBuf::from(raw),
        };

        if path.is_relative() {
            if let Some(base) = &self.cwd {
                Ok(base.join(path))
            } else {
                Ok(std::env::current_dir()?.join(path))
            }
        } else {
            Ok(path)
        }
    }

    fn resolve_macro(&self, macro_input: &str) -> Result<PathBuf> {
        match macro_input {
            ":cwd" => self
                .cwd
                .clone()
                .or_else(|| std::env::current_dir().ok())
                .ok_or_else(|| anyhow!("Current directory is unavailable")),
            ":home" => dirs::home_dir().ok_or_else(|| anyhow!("Home directory is unavailable")),
            other => Err(anyhow!("Unknown source macro: {other}")),
        }
    }

    fn detect_kind(&self, path: &Path, metadata: &fs::Metadata) -> Result<SourceKind> {
        if metadata.is_dir() {
            return Ok(SourceKind::Directory);
        }

        let filename = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();

        if filename == "mcp.json"
            || filename.ends_with(".manifest.json")
            || filename.ends_with(".toml")
        {
            return Ok(SourceKind::Manifest);
        }

        if filename.ends_with(".zip")
            || filename.ends_with(".tar")
            || filename.ends_with(".tar.gz")
            || filename.ends_with(".tgz")
        {
            return Ok(SourceKind::Archive);
        }

        Err(anyhow!(
            "Source {} is not recognised: supported directories, archives (.zip/.tar/.tar.gz) or manifest",
            path.display()
        ))
    }

    fn fingerprint(&self, path: &Path, metadata: &fs::Metadata) -> Result<SourceFingerprint> {
        let canonical = path
            .canonicalize()
            .with_context(|| format!("Failed to canonicalise path {}", path.display()))?;
        let mut hasher = Sha1::new();
        hasher.update(canonical.as_os_str().to_string_lossy().as_bytes());
        if let Ok(modified) = metadata.modified() {
            let nanos = modified
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default();
            hasher.update(nanos.to_le_bytes());
        }
        let digest = format!("{:x}", hasher.finalize());
        let last_modified = metadata.modified().ok();
        Ok(SourceFingerprint {
            digest,
            last_modified,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn parses_directory_source() {
        let dir = temp_dir();
        let parser = SourceParser::new(None);
        let parsed = parser
            .parse(dir.path().to_str().unwrap())
            .expect("parsed directory");
        assert_eq!(parsed.kind(), SourceKind::Directory);
        assert!(parsed.fingerprint().digest.len() > 0);
    }

    #[test]
    fn resolves_macros() {
        let cwd = temp_dir();
        let parser = SourceParser::new(Some(cwd.path().to_path_buf()));
        let parsed = parser.parse(":cwd").expect("macro resolved");
        assert_eq!(parsed.path(), cwd.path());
    }

    #[test]
    fn detects_manifest() {
        let dir = temp_dir();
        let manifest_path = dir.path().join("mcp.json");
        std::fs::write(&manifest_path, "{}").expect("write manifest");
        let parser = SourceParser::new(None);
        let parsed = parser
            .parse(manifest_path.to_str().unwrap())
            .expect("manifest parsed");
        assert_eq!(parsed.kind(), SourceKind::Manifest);
    }

    #[test]
    fn detects_archive() {
        let dir = temp_dir();
        let archive_path = dir.path().join("server.tar.gz");
        std::fs::File::create(&archive_path).expect("touch archive");
        let parser = SourceParser::new(None);
        let parsed = parser
            .parse(archive_path.to_str().unwrap())
            .expect("archive parsed");
        assert_eq!(parsed.kind(), SourceKind::Archive);
    }

    #[test]
    fn rejects_unknown_source() {
        let dir = temp_dir();
        let file = dir.path().join("README");
        std::fs::write(&file, "docs").expect("write");
        let parser = SourceParser::new(None);
        let err = parser.parse(file.to_str().unwrap()).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("not recognised"));
    }
}
