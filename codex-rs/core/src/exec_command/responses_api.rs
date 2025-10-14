use std::collections::BTreeMap;

use crate::openai_tools::JsonSchema;
use crate::openai_tools::ResponsesApiTool;

pub const EXEC_COMMAND_TOOL_NAME: &str = "exec_command";
pub const WRITE_STDIN_TOOL_NAME: &str = "write_stdin";
pub const EXEC_CONTROL_TOOL_NAME: &str = "exec_control";
pub const LIST_EXEC_SESSIONS_TOOL_NAME: &str = "list_exec_sessions";
pub const GET_SESSION_EVENTS_TOOL_NAME: &str = "get_session_events";

pub fn create_exec_command_tool_for_responses_api() -> ResponsesApiTool {
    let mut properties = BTreeMap::<String, JsonSchema>::new();
    properties.insert(
        "cmd".to_string(),
        JsonSchema::String {
            description: Some("The shell command to execute.".to_string()),
        },
    );
    properties.insert(
        "yield_time_ms".to_string(),
        JsonSchema::Number {
            description: Some(
                "Collect output for this many milliseconds before yielding.\n- Default: 1000.\n- Command keeps running afterward; use write_stdin/exec_control to monitor."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "max_output_tokens".to_string(),
        JsonSchema::Number {
            description: Some(
                "Upper bound for inline payload size.\n- Default: 2000 tokens (≈8 KiB).\n- Lower values save tokens; full logs stay on disk."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "shell".to_string(),
        JsonSchema::String {
            description: Some("The shell to use. Defaults to \"/bin/bash\".".to_string()),
        },
    );
    properties.insert(
        "login".to_string(),
        JsonSchema::Boolean {
            description: Some(
                "Whether to run the command as a login shell. Defaults to true.".to_string(),
            ),
        },
    );
    properties.insert(
        "idle_timeout_ms".to_string(),
        JsonSchema::Number {
            description: Some(
                "Idle watchdog in milliseconds.\n- Min 1000, max 86400000 (24 h).\n- Resets on output, keepalive, or manual input."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "hard_timeout_ms".to_string(),
        JsonSchema::Number {
            description: Some(
                "Absolute deadline in milliseconds.\n- Default: 7_200_000 (2 h).\n- Set null to disable.".to_string(),
            ),
        },
    );
    properties.insert(
        "grace_period_ms".to_string(),
        JsonSchema::Number {
            description: Some(
                "Pause between Ctrl-C and SIGKILL escalations.\n- Default: 5000.".to_string(),
            ),
        },
    );
    properties.insert(
        "log_threshold_bytes".to_string(),
        JsonSchema::Number {
            description: Some(
                "Bytes of stdout/stderr kept inline before spilling to disk.\n- Default: 4096."
                    .to_string(),
            ),
        },
    );

    ResponsesApiTool {
        name: EXEC_COMMAND_TOOL_NAME.to_owned(),
        description: r#"Execute a shell command asynchronously.
- Yields after `yield_time_ms` with whatever stdout/stderr was produced.
- Session continues in background until idle/hard timeouts or manual stop.
- Use `write_stdin`/`exec_control`/`list_exec_sessions` to interact afterward."#
            .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["cmd".to_string()]),
            additional_properties: Some(false),
        },
    }
}

pub fn create_write_stdin_tool_for_responses_api() -> ResponsesApiTool {
    let mut properties = BTreeMap::<String, JsonSchema>::new();
    properties.insert(
        "session_id".to_string(),
        JsonSchema::Number {
            description: Some("The ID of the exec_command session.".to_string()),
        },
    );
    properties.insert(
        "chars".to_string(),
        JsonSchema::String {
            description: Some(
                "Write text to stdin before polling. Use \"\" to just poll.".to_string(),
            ),
        },
    );
    properties.insert(
        "yield_time_ms".to_string(),
        JsonSchema::Number {
            description: Some("Poll window in milliseconds.\n- Default: 500 (increase for long-running commands).".to_string()),
        },
    );
    properties.insert(
        "max_output_tokens".to_string(),
        JsonSchema::Number {
            description: Some("Cap inline response tokens.\n- Default: 160 (≈640 B). Auto-polls downshift after the first call.".to_string()),
        },
    );
    properties.insert(
        "tail_lines".to_string(),
        JsonSchema::Number {
            description: Some("Return only the last N lines (overrides auto mode).".to_string()),
        },
    );
    properties.insert(
        "stop_pattern".to_string(),
        JsonSchema::String {
            description: Some("Regex pattern to auto-terminate on match. Sends Ctrl-C when pattern found. Example: stop_pattern=\"tests passed\" auto-stops test suite.".to_string()),
        },
    );
    properties.insert(
        "stop_pattern_cut".to_string(),
        JsonSchema::Boolean {
            description: Some(
                "When true, trims output after the first stop_pattern match.".to_string(),
            ),
        },
    );
    properties.insert(
        "stop_pattern_label_tail".to_string(),
        JsonSchema::Boolean {
            description: Some(
                "When true, annotates the response when stop_pattern truncates or detects a tail. Combine with stop_pattern_cut for an explicit placeholder."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "raw".to_string(),
        JsonSchema::Boolean {
            description: Some("Deprecated. Ignored.".to_string()),
        },
    );
    properties.insert(
        "since_byte".to_string(),
        JsonSchema::Number {
            description: Some("Deprecated. Use line-based modes instead.".to_string()),
        },
    );
    properties.insert(
        "reset_cursor".to_string(),
        JsonSchema::Boolean {
            description: Some("Deprecated. Use all=true instead.".to_string()),
        },
    );
    properties.insert(
        "compact".to_string(),
        JsonSchema::Boolean {
            description: Some(
                "Return plain-text output only (default true) to save tokens.".to_string(),
            ),
        },
    );
    properties.insert(
        "all".to_string(),
        JsonSchema::Boolean {
            description: Some("Return the full output from start (resets cursor).".to_string()),
        },
    );
    properties.insert(
        "from_line".to_string(),
        JsonSchema::Number {
            description: Some("Start line (0-indexed) for range queries.".to_string()),
        },
    );
    properties.insert(
        "to_line".to_string(),
        JsonSchema::Number {
            description: Some("Exclusive end line for range queries.".to_string()),
        },
    );
    properties.insert(
        "smart_compress".to_string(),
        JsonSchema::Boolean {
            description: Some("Auto-compress repetitive output (default true).".to_string()),
        },
    );

    ResponsesApiTool {
        name: WRITE_STDIN_TOOL_NAME.to_owned(),
        description: r#"Send input to an exec session or poll its output.
- AUTO (default): incremental tail since last read.
- `tail_lines=N`: last N lines only.
- `from_line`/`to_line`: precise window.
- `all=true`: full transcript from start.
- `stop_pattern`: regex trigger that sends Ctrl-C when matched (see `stop_pattern_cut` / `stop_pattern_label_tail` for tail handling)."#
            .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["session_id".to_string(), "chars".to_string()]),
            additional_properties: Some(false),
        },
    }
}

pub fn create_exec_control_tool_for_responses_api() -> ResponsesApiTool {
    let mut action_properties = BTreeMap::<String, JsonSchema>::new();
    action_properties.insert(
        "type".to_string(),
        JsonSchema::String {
            description: Some(
                "Action to perform. Allowed values: keepalive, send_ctrl_c, terminate, force_kill, set_idle_timeout, watch, unwatch.".to_string(),
            ),
        },
    );
    action_properties.insert(
        "extend_timeout_ms".to_string(),
        JsonSchema::Number {
            description: Some(
                "Optional: when type=keepalive, reset idle timer to now and optionally extend the idle timeout.".to_string(),
            ),
        },
    );
    action_properties.insert(
        "timeout_ms".to_string(),
        JsonSchema::Number {
            description: Some(
                "Optional: when type=set_idle_timeout, new idle timeout in milliseconds."
                    .to_string(),
            ),
        },
    );
    action_properties.insert(
        "pattern".to_string(),
        JsonSchema::String {
            description: Some(
                "Required when type=watch/unwatch. Regex evaluated against new stdout lines."
                    .to_string(),
            ),
        },
    );
    action_properties.insert(
        "watch_action".to_string(),
        JsonSchema::String {
            description: Some(
                "Optional: when type=watch, action on match (log, send_ctrl_c, force_kill). Default log.".to_string(),
            ),
        },
    );
    action_properties.insert(
        "persist".to_string(),
        JsonSchema::Boolean {
            description: Some(
                "Optional: when type=watch, if true the watch remains active after first match (default false)."
                    .to_string(),
            ),
        },
    );
    action_properties.insert(
        "cooldown_ms".to_string(),
        JsonSchema::Number {
            description: Some(
                "Optional: when type=watch, minimum milliseconds between repeated matches (default 1000ms for persistent watchers)."
                    .to_string(),
            ),
        },
    );
    action_properties.insert(
        "auto_send_ctrl_c".to_string(),
        JsonSchema::Boolean {
            description: Some(
                "Optional: when type=watch and action=log, automatically send Ctrl-C on match. Defaults to true for persistent watchers.".to_string(),
            ),
        },
    );

    let mut properties = BTreeMap::<String, JsonSchema>::new();
    properties.insert(
        "session_id".to_string(),
        JsonSchema::Number {
            description: Some("The target exec session identifier.".to_string()),
        },
    );
    properties.insert(
        "action".to_string(),
        JsonSchema::Object {
            properties: action_properties,
            required: Some(vec!["type".to_string()]),
            additional_properties: Some(false),
        },
    );

    ResponsesApiTool {
        name: EXEC_CONTROL_TOOL_NAME.to_owned(),
        description:
            "Send control signals to a running exec session (keepalive, interrupt, terminate, watch patterns)."
                .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["session_id".to_string(), "action".to_string()]),
            additional_properties: Some(false),
        },
    }
}

pub fn create_list_exec_sessions_tool_for_responses_api() -> ResponsesApiTool {
    ResponsesApiTool {
        name: LIST_EXEC_SESSIONS_TOOL_NAME.to_owned(),
        description:
            "Summarize currently known exec sessions (running, graceful, or recently terminated). Returns brief snapshot with last ~2 lines of output per session.

IMPORTANT: Repeated polling is EXPENSIVE (~100-500 tokens per call). For monitoring streaming output, use write_stdin with empty string \"\" on the target session instead - it's cheaper and returns full recent output."
                .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties: BTreeMap::from([
                (
                    "state".to_string(),
                    JsonSchema::String {
                        description: Some("Optional filter: running | grace | terminated.".to_string()),
                    },
                ),
                (
                    "limit".to_string(),
                    JsonSchema::Number {
                        description: Some("Optional: cap number of summaries (default unlimited).".to_string()),
                    },
                ),
                (
                    "since_ms".to_string(),
                    JsonSchema::Number {
                        description: Some(
                            "Optional: include sessions created in the last N milliseconds.".to_string(),
                        ),
                    },
                ),
            ]),
            required: Some(Vec::new()),
            additional_properties: Some(false),
        },
    }
}

pub fn create_get_session_events_tool_for_responses_api() -> ResponsesApiTool {
    let mut properties = BTreeMap::<String, JsonSchema>::new();
    properties.insert(
        "session_id".to_string(),
        JsonSchema::Number {
            description: Some("Target exec session identifier.".to_string()),
        },
    );
    properties.insert(
        "since_id".to_string(),
        JsonSchema::Number {
            description: Some(
                "Optional: return only events with id greater than this value (for incremental polling).".to_string(),
            ),
        },
    );
    properties.insert(
        "limit".to_string(),
        JsonSchema::Number {
            description: Some("Optional: max events to return (default 256).".to_string()),
        },
    );

    ResponsesApiTool {
        name: GET_SESSION_EVENTS_TOOL_NAME.to_owned(),
        description:
            "Retrieve the structured event log for an exec session (stop_pattern matches, watcher actions, idle timeouts, etc.). Use since_id for incremental tails to stay token-efficient."
                .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["session_id".to_string()]),
            additional_properties: Some(false),
        },
    }
}
