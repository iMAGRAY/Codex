//! Policy evaluation guardrails for MCP intake sources.
//!
//! Trace: REQ-SEC-01 linkage for intake risk surfacing.

use std::path::Path;

use super::parser::ParsedSource;

/// Evaluate security and environment policies for a source.
pub fn evaluate_policy(parsed: &ParsedSource, project_root: Option<&Path>) -> Vec<String> {
    let mut warnings = Vec::new();

    if let Ok(canonical) = parsed.path().canonicalize() {
        if is_forbidden_prefix(&canonical) {
            warnings.push(format!(
                "Source {} resides in a potentially sensitive directory (for example, /etc or /usr).",
                canonical.display()
            ));
        }
        if let Some(root) = project_root {
            if !canonical.starts_with(root) {
                warnings.push(format!(
                    "Source {} is located outside the current project {}.",
                    canonical.display(),
                    root.display()
                ));
            }
        }
    }

    warnings
}

fn is_forbidden_prefix(path: &Path) -> bool {
    #[cfg(target_os = "windows")]
    const FORBIDDEN_PREFIXES: &[&str] = &["C:/Windows", "C:/Program Files", "C:/ProgramData"];
    #[cfg(not(target_os = "windows"))]
    const FORBIDDEN_PREFIXES: &[&str] = &["/etc", "/usr", "/var", "/bin"];

    FORBIDDEN_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(Path::new(prefix)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn warns_outside_project_root() {
        let dir = tempdir().expect("tmp");
        let parsed = ParsedSource::new(
            dir.path().to_path_buf(),
            super::super::parser::SourceKind::Directory,
            super::super::parser::SourceFingerprint {
                digest: "abc".into(),
                last_modified: None,
            },
        );
        let warnings = evaluate_policy(&parsed, Some(Path::new("/home/test/project")));
        #[cfg(not(target_os = "windows"))]
        assert!(!warnings.is_empty());
    }
}
