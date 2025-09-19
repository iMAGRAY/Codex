//! Builds a concise preview of MCP intake sources.
//!
//! Trace: REQ-DATA-01 (transparent intake recommendations).

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;

use super::parser::ParsedSource;
use super::state::SourcePreview;

const INTERESTING_FILES: &[&str] = &[
    "README",
    "README.md",
    "README.txt",
    "package.json",
    "pyproject.toml",
    "requirements.txt",
    "mcp.json",
    "manifest.json",
    "Dockerfile",
];

/// Maximum number of files displayed in the preview.
const PREVIEW_LIMIT: usize = 6;

/// Prepare a concise list of key files and warnings for the source.
pub fn build_preview(source: &ParsedSource) -> Result<SourcePreview> {
    let mut key_files = Vec::new();
    let mut warnings = Vec::new();

    match source.kind() {
        super::parser::SourceKind::Directory => {
            key_files.extend(scan_directory(source.path())?);
        }
        super::parser::SourceKind::Archive => {
            warnings.push("Archive will be extracted into a temporary directory.".to_string());
            key_files.push(
                source
                    .path()
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
            );
        }
        super::parser::SourceKind::Manifest => {
            key_files.push(
                source
                    .path()
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
            );
        }
    }

    Ok(SourcePreview::new(key_files, warnings))
}

fn scan_directory(root: &Path) -> Result<Vec<String>> {
    let mut results = Vec::new();
    let entries = fs::read_dir(root)
        .with_context(|| format!("Failed to read directory {}", root.display()))?;
    for entry in entries.flatten() {
        if results.len() >= PREVIEW_LIMIT {
            break;
        }
        let path = entry.path();
        if path.is_file() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if INTERESTING_FILES
                .iter()
                .any(|candidate| name.eq_ignore_ascii_case(candidate))
            {
                results.push(relative_name(&path, root));
            }
        }
    }
    if results.is_empty() {
        results.extend(
            entries_by_priority(root)?
                .into_iter()
                .map(|p| relative_name(&p, root)),
        );
    }
    if results.len() > PREVIEW_LIMIT {
        results.truncate(PREVIEW_LIMIT);
    }
    Ok(results)
}

fn entries_by_priority(root: &Path) -> Result<Vec<PathBuf>> {
    let mut collected: Vec<(u8, PathBuf)> = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            let priority = if path
                .extension()
                .and_then(|e| e.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("md"))
                == Some(true)
            {
                1
            } else {
                2
            };
            collected.push((priority, path));
        }
    }
    collected.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(collected.into_iter().map(|(_, p)| p).collect())
}

fn relative_name(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn builds_preview_for_directory() {
        let dir = tempdir().expect("tmp");
        let readme = dir.path().join("README.md");
        fs::write(&readme, "docs").unwrap();
        let parsed = ParsedSource::new(
            dir.path().to_path_buf(),
            super::super::parser::SourceKind::Directory,
            super::super::parser::SourceFingerprint {
                digest: "abc".into(),
                last_modified: None,
            },
        );
        let preview = build_preview(&parsed).expect("preview");
        assert!(preview.files().iter().any(|f| f.contains("README")));
        assert!(preview.warnings().is_empty());
    }

    #[test]
    fn builds_preview_for_archive() {
        let dir = tempdir().expect("tmp");
        let archive = dir.path().join("example.tar.gz");
        fs::File::create(&archive).unwrap();
        let parsed = ParsedSource::new(
            archive.clone(),
            super::super::parser::SourceKind::Archive,
            super::super::parser::SourceFingerprint {
                digest: "abc".into(),
                last_modified: None,
            },
        );
        let preview = build_preview(&parsed).expect("preview");
        assert!(preview.warnings().iter().any(|w| w.contains("Archive")));
        assert_eq!(preview.files()[0], "example.tar.gz");
    }
}
