use serde::Deserialize;
use serde::Serialize;
use serde::de::Deserializer;
use serde::de::{self};
use shlex::Shlex;
use shlex::try_join;
use std::path::Path;

use crate::exec_command::session_id::SessionId;

#[derive(Debug, Clone)]
pub(crate) struct CommandLine {
    joined: String,
    preview: String,
}

impl CommandLine {
    fn from_tokens(tokens: Vec<String>) -> Result<Self, String> {
        let joined = try_join(tokens.iter().map(String::as_str))
            .map_err(|err| format!("failed to join command parts: {err}"))?;
        let preview = compute_preview(&tokens);
        Ok(Self {
            preview: if preview.is_empty() {
                joined.clone()
            } else {
                preview
            },
            joined,
        })
    }

    fn from_shell(command: String) -> Self {
        let tokens: Vec<String> = Shlex::new(&command).collect();
        let preview = compute_preview(&tokens);
        Self {
            preview: if preview.is_empty() {
                command.clone()
            } else {
                preview
            },
            joined: command,
        }
    }

    fn maybe_from_bracketed(command: &str) -> Option<Self> {
        parse_bracketed_tokens(command).and_then(|tokens| Self::from_tokens(tokens).ok())
    }

    pub(crate) fn shell_command(&self) -> &str {
        &self.joined
    }

    pub(crate) fn preview(&self) -> &str {
        &self.preview
    }

    #[cfg(test)]
    pub(crate) fn test_shell(command: &str) -> Self {
        Self::from_shell(command.to_string())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecCommandParams {
    #[serde(deserialize_with = "deserialize_cmd")]
    pub(crate) cmd: CommandLine,

    #[serde(default = "default_yield_time")]
    pub(crate) yield_time_ms: u64,

    #[serde(default = "max_output_tokens")]
    pub(crate) max_output_tokens: u64,

    #[serde(default = "default_shell")]
    pub(crate) shell: String,

    #[serde(default = "default_login")]
    pub(crate) login: bool,

    #[serde(default)]
    pub(crate) idle_timeout_ms: Option<u64>,

    #[serde(default)]
    pub(crate) hard_timeout_ms: Option<u64>,

    #[serde(default = "default_grace_period_ms")]
    pub(crate) grace_period_ms: u64,

    #[serde(default = "default_log_threshold_bytes")]
    pub(crate) log_threshold_bytes: u64,
}

fn default_yield_time() -> u64 {
    1_000
}

fn max_output_tokens() -> u64 {
    // Default to 2000 tokens (~8KB) to prevent token waste on verbose commands.
    // Full logs always preserved in session storage. Agent can increase if needed.
    2_000
}

fn default_login() -> bool {
    true
}

fn default_shell() -> String {
    "/bin/bash".to_string()
}

fn default_grace_period_ms() -> u64 {
    5_000
}

fn default_log_threshold_bytes() -> u64 {
    4 * 1024
}

fn deserialize_cmd<'de, D>(deserializer: D) -> Result<CommandLine, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum CmdSpec {
        Str(String),
        Seq(Vec<String>),
    }

    match CmdSpec::deserialize(deserializer)? {
        CmdSpec::Str(raw) => {
            if let Some(parsed) = CommandLine::maybe_from_bracketed(&raw) {
                Ok(parsed)
            } else {
                Ok(CommandLine::from_shell(raw))
            }
        }
        CmdSpec::Seq(parts) => CommandLine::from_tokens(parts).map_err(de::Error::custom),
    }
}

fn parse_bracketed_tokens(source: &str) -> Option<Vec<String>> {
    let trimmed = source.trim();
    if trimmed.len() < 2 || !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return None;
    }
    let mut tokens = Vec::new();
    let mut chars = trimmed[1..trimmed.len() - 1].chars().peekable();
    loop {
        while matches!(chars.peek(), Some(c) if c.is_whitespace() || *c == ',') {
            chars.next();
        }
        let Some(ch) = chars.next() else {
            break;
        };
        if ch != '\'' && ch != '"' {
            return None;
        }
        let quote = ch;
        let mut current = String::new();
        let mut escaped = false;
        let mut closed = false;
        for next in chars.by_ref() {
            if escaped {
                current.push(next);
                escaped = false;
                continue;
            }
            match next {
                '\\' => {
                    escaped = true;
                }
                c if c == quote => {
                    tokens.push(current);
                    closed = true;
                    break;
                }
                c => current.push(c),
            }
        }
        if !closed || escaped {
            return None;
        }
    }
    Some(tokens)
}

fn compute_preview(tokens: &[String]) -> String {
    if tokens.is_empty() {
        return String::new();
    }
    let slice = if tokens.len() >= 2 && is_shell_wrapper(&tokens[0], &tokens[1]) {
        if tokens.len() > 2 {
            &tokens[2..]
        } else {
            &tokens[0..0]
        }
    } else {
        tokens
    };
    if slice.is_empty() {
        return String::new();
    }
    let joined = slice.join(" ");
    let condensed = condense_command(&joined);
    if condensed.is_empty() {
        joined
    } else {
        condensed
    }
}

fn is_shell_wrapper(first: &str, flag: &str) -> bool {
    matches!(flag, "-c" | "-lc")
        && Path::new(first)
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| matches!(name, "bash" | "sh"))
            .unwrap_or(false)
}

fn condense_command(command: &str) -> String {
    let trimmed = strip_matching_quotes(command.trim());
    let tail = strip_after_last_separator(trimmed);
    strip_matching_quotes(tail).trim().to_string()
}

fn strip_after_last_separator(command: &str) -> &str {
    let mut candidate: Option<(usize, usize)> = None;
    for sep in ["&&", "||", ";"] {
        if let Some(pos) = command.rfind(sep) {
            let end = pos + sep.len();
            if candidate.is_none_or(|(existing, _)| end > existing) {
                candidate = Some((end, sep.len()));
            }
        }
    }
    if let Some((cut, _)) = candidate {
        let rest = command[cut..].trim();
        if !rest.is_empty() {
            return rest;
        }
    }
    command
}

fn strip_matching_quotes(input: &str) -> &str {
    let trimmed = input.trim();
    if trimmed.len() >= 2 {
        let first = trimmed.as_bytes()[0] as char;
        let last = trimmed.as_bytes()[trimmed.len() - 1] as char;
        if (first == '\'' && last == '\'') || (first == '"' && last == '"') {
            return trimmed[1..trimmed.len() - 1].trim();
        }
    }
    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn array_cmd_preserves_shell_and_produces_clean_preview() {
        let params: ExecCommandParams = serde_json::from_value(json!({
            "cmd": ["bash", "-lc", "sleep 5"],
        }))
        .expect("deserialize exec params");
        assert_eq!(params.cmd.shell_command(), "bash -lc 'sleep 5'");
        assert_eq!(params.cmd.preview(), "sleep 5");
    }

    #[test]
    fn bracketed_string_cmd_is_normalized() {
        let params: ExecCommandParams = serde_json::from_value(json!({
            "cmd": "['bash','-lc','sleep 5']",
        }))
        .expect("deserialize exec params");
        assert_eq!(params.cmd.shell_command(), "bash -lc 'sleep 5'");
        assert_eq!(params.cmd.preview(), "sleep 5");
    }

    #[test]
    fn shell_string_condenses_to_tail_command() {
        let params: ExecCommandParams = serde_json::from_value(json!({
            "cmd": "bash -lc \"cd /work && make build\"",
        }))
        .expect("deserialize exec params");
        assert_eq!(
            params.cmd.shell_command(),
            "bash -lc \"cd /work && make build\""
        );
        assert_eq!(params.cmd.preview(), "make build");
    }

    #[test]
    fn simple_command_array_produces_compact_preview() {
        let params: ExecCommandParams = serde_json::from_value(json!({
            "cmd": ["git", "status"],
        }))
        .expect("deserialize exec params");
        assert_eq!(params.cmd.shell_command(), "git status");
        assert_eq!(params.cmd.preview(), "git status");
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WriteStdinParams {
    pub(crate) session_id: SessionId,
    pub(crate) chars: String,

    #[serde(default = "write_stdin_default_yield_time_ms")]
    pub(crate) yield_time_ms: u64,

    #[serde(default = "write_stdin_default_max_output_tokens")]
    pub(crate) max_output_tokens: u64,

    /// Number of recent output lines to return (default: all within token limit).
    /// Use 1-5 for quick checks, 10-50 for monitoring, 100+ for full context.
    #[serde(default)]
    pub(crate) tail_lines: Option<usize>,

    /// Byte offset to start reading from (for incremental reads).
    /// If omitted, automatically uses cursor from last read (auto-incremental).
    /// Pass 0 to reset and read from beginning.
    #[serde(default)]
    pub(crate) since_byte: Option<u64>,

    /// Force return full output from beginning (resets cursor).
    #[serde(default)]
    pub(crate) reset_cursor: bool,

    /// Stop pattern (regex): automatically send Ctrl-C when output matches.
    /// Example: "^50$" to stop when line exactly equals "50".
    #[serde(default)]
    pub(crate) stop_pattern: Option<String>,

    /// Trim output after first stop_pattern match when true (default false).
    #[serde(default)]
    pub(crate) stop_pattern_cut: bool,

    /// Label the omitted tail after stop_pattern match when true (default false).
    #[serde(default)]
    pub(crate) stop_pattern_label_tail: bool,

    /// Raw mode: disable auto-incremental cursor, return all output (legacy behavior).
    #[serde(default)]
    pub(crate) raw: bool,

    /// Compact response: return plain text output without JSON wrapper (default true).
    /// Saves ~100-150 tokens per poll. Use false for full JSON with all metadata.
    #[serde(default = "write_stdin_default_compact")]
    pub(crate) compact: bool,

    // === LINE-BASED QUERY MODES (mutually exclusive with byte-based) ===
    /// Get all output from beginning (resets line cursor). Takes precedence over other line modes.
    #[serde(default)]
    pub(crate) all: bool,

    /// Start line number for range query (0-indexed). Use with to_line for precise ranges.
    #[serde(default)]
    pub(crate) from_line: Option<u64>,

    /// End line number for range query (exclusive). Use with from_line.
    #[serde(default)]
    pub(crate) to_line: Option<u64>,

    /// Enable smart compression for repetitive output (default true).
    /// Compresses sequential numbers, repeated lines, etc. Saves tokens on verbose output.
    #[serde(default = "write_stdin_default_smart_compress")]
    pub(crate) smart_compress: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SessionEventsParams {
    pub(crate) session_id: SessionId,
    #[serde(default)]
    pub(crate) since_id: Option<u64>,
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

impl SessionEventsParams {
    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub fn since_id(&self) -> Option<u64> {
        self.since_id
    }

    pub fn limit(&self) -> Option<usize> {
        self.limit
    }
}

fn write_stdin_default_yield_time_ms() -> u64 {
    // Increased to 500ms to reduce polling frequency and token waste
    500
}

fn write_stdin_default_max_output_tokens() -> u64 {
    // Default to 160 tokens (~640 bytes) for the first incremental read.
    // Subsequent auto polls dynamically downshift to keep output lean.
    160
}

fn write_stdin_default_compact() -> bool {
    true
}

fn write_stdin_default_smart_compress() -> bool {
    true
}
