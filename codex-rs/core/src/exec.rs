#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::time::Duration;
use std::time::Instant;

use async_channel::Sender;
use chrono::DateTime;
use chrono::Utc;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::io::BufReader;
use tokio::process::Child;

use crate::error::CodexErr;
use crate::error::Result;
use crate::error::SandboxErr;
use crate::landlock::spawn_command_under_linux_sandbox;
use crate::protocol::Event;
use crate::protocol::EventMsg;
use crate::protocol::ExecCommandOutputDeltaEvent;
use crate::protocol::ExecOutputStream;
use crate::protocol::SandboxPolicy;
use crate::seatbelt::spawn_command_under_seatbelt;
use crate::security::AuditEvent;
use crate::security::AuditEventKind;
use crate::security::ResourceLimits;
use crate::security::SecretBroker;
use crate::security::append_audit_event;
use crate::spawn::StdioPolicy;
use crate::spawn::spawn_child_async;
use crate::telemetry::TelemetryHub;

const DEFAULT_TIMEOUT_MS: u64 = 10_000;

// Hardcode these since it does not seem worth including the libc crate just
// for these.
const SIGKILL_CODE: i32 = 9;
const TIMEOUT_CODE: i32 = 64;
const EXIT_CODE_SIGNAL_BASE: i32 = 128; // conventional shell: 128 + signal
const EXEC_TIMEOUT_EXIT_CODE: i32 = 124; // conventional timeout exit code

// I/O buffer sizing
const READ_CHUNK_SIZE: usize = 8192; // bytes per read
const AGGREGATE_BUFFER_INITIAL_CAPACITY: usize = 8 * 1024; // 8 KiB

/// Limit the number of ExecCommandOutputDelta events emitted per exec call.
/// Aggregation still collects full output; only the live event stream is capped.
pub(crate) const MAX_EXEC_OUTPUT_DELTAS_PER_CALL: usize = 10_000;

#[derive(Debug, Clone)]
pub struct ExecParams {
    pub command: Vec<String>,
    pub cwd: PathBuf,
    pub timeout_ms: Option<u64>,
    pub env: HashMap<String, String>,
    pub with_escalated_permissions: Option<bool>,
    pub justification: Option<String>,
}

impl ExecParams {
    pub fn timeout_duration(&self) -> Duration {
        Duration::from_millis(self.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SandboxType {
    None,

    /// Only available on macOS.
    MacosSeatbelt,

    /// Only available on Linux.
    LinuxSeccomp,
}

#[derive(Clone)]
pub struct StdoutStream {
    pub sub_id: String,
    pub call_id: String,
    pub tx_event: Sender<Event>,
}

pub async fn process_exec_tool_call(
    params: ExecParams,
    sandbox_type: SandboxType,
    sandbox_policy: &SandboxPolicy,
    codex_linux_sandbox_exe: &Option<PathBuf>,
    stdout_stream: Option<StdoutStream>,
) -> Result<ExecToolCallOutput> {
    let audit_started_at = Utc::now();
    let start = Instant::now();

    let mut params = params;
    SecretBroker::global().ensure_env_secret(&mut params.env);
    let audit_snapshot = params.clone();

    let timeout_duration = params.timeout_duration();

    let raw_output_result: std::result::Result<RawExecToolCallOutput, CodexErr> = match sandbox_type
    {
        SandboxType::None => exec(params, sandbox_policy, stdout_stream.clone()).await,
        SandboxType::MacosSeatbelt => {
            let ExecParams {
                command, cwd, env, ..
            } = params;
            let child = spawn_command_under_seatbelt(
                command,
                sandbox_policy,
                cwd,
                StdioPolicy::RedirectForShellTool,
                env,
            )
            .await?;
            consume_truncated_output(child, timeout_duration, stdout_stream.clone()).await
        }
        SandboxType::LinuxSeccomp => {
            let ExecParams {
                command, cwd, env, ..
            } = params;

            let codex_linux_sandbox_exe = codex_linux_sandbox_exe
                .as_ref()
                .ok_or(CodexErr::LandlockSandboxExecutableNotProvided)?;
            let child = spawn_command_under_linux_sandbox(
                codex_linux_sandbox_exe,
                command,
                sandbox_policy,
                cwd,
                StdioPolicy::RedirectForShellTool,
                env,
            )
            .await?;

            consume_truncated_output(child, timeout_duration, stdout_stream).await
        }
    };
    let duration = start.elapsed();
    match raw_output_result {
        Ok(raw_output) => {
            #[allow(unused_mut)]
            let mut timed_out = raw_output.timed_out;

            #[allow(unused_mut)]
            let mut resource_notice: Option<String> = None;

            #[cfg(target_family = "unix")]
            {
                if let Some(signal) = raw_output.exit_status.signal() {
                    if signal == TIMEOUT_CODE {
                        timed_out = true;
                    } else if let Some(msg) = crate::security::resource_signal_message(signal) {
                        resource_notice = Some(format!("{msg} (signal {signal})"));
                    } else {
                        return Err(CodexErr::Sandbox(SandboxErr::Signal(signal)));
                    }
                }
            }

            let mut exit_code = raw_output.exit_status.code().unwrap_or_else(|| {
                #[cfg(target_family = "unix")]
                {
                    if let Some(signal) = raw_output.exit_status.signal() {
                        return EXIT_CODE_SIGNAL_BASE + signal;
                    }
                }
                -1
            });
            if timed_out {
                exit_code = EXEC_TIMEOUT_EXIT_CODE;
            }

            let broker = SecretBroker::global();
            let mut stdout = raw_output.stdout.from_utf8_lossy();
            broker.scrub_string(&mut stdout.text);
            let mut stderr = raw_output.stderr.from_utf8_lossy();
            broker.scrub_string(&mut stderr.text);
            let mut aggregated_output = raw_output.aggregated_output.from_utf8_lossy();
            broker.scrub_string(&mut aggregated_output.text);

            if let Some(notice) = resource_notice.as_ref() {
                if !stderr.text.is_empty() && !stderr.text.ends_with("\n") {
                    stderr.text.push_str("\n");
                }
                stderr
                    .text
                    .push_str(&format!("[resource-shield] {notice}\n"));
                if !aggregated_output.text.is_empty() && !aggregated_output.text.ends_with("\n") {
                    aggregated_output.text.push_str("\n");
                }
                aggregated_output
                    .text
                    .push_str(&format!("[resource-shield] {notice}\n"));
                tracing::warn!(notice, "sandbox resource shield triggered");
            }

            let exec_output = ExecToolCallOutput {
                exit_code,
                stdout,
                stderr,
                aggregated_output,
                duration,
                timed_out,
            };
            let status = if timed_out {
                ExecAuditStatus::Timeout
            } else if exit_code != 0 && is_likely_sandbox_denied(sandbox_type, exit_code) {
                ExecAuditStatus::SandboxDenied
            } else {
                ExecAuditStatus::Success
            };

            if status == ExecAuditStatus::Success {
                TelemetryHub::global().record_exec_latency(duration);
            }

            append_exec_audit_event(
                audit_started_at,
                &audit_snapshot,
                sandbox_type,
                sandbox_policy,
                duration,
                status,
                Some(&exec_output),
                None,
                resource_notice.clone(),
            )?;

            match status {
                ExecAuditStatus::Success => Ok(exec_output),
                ExecAuditStatus::Timeout => Err(CodexErr::Sandbox(SandboxErr::Timeout {
                    output: Box::new(exec_output),
                })),
                ExecAuditStatus::SandboxDenied => Err(CodexErr::Sandbox(SandboxErr::Denied {
                    output: Box::new(exec_output),
                })),
                ExecAuditStatus::Failure => unreachable!("failure status handled in error branch"),
            }
        }
        Err(err) => {
            tracing::error!("exec error: {err}");
            let err_string = err.to_string();
            if let Err(audit_err) = append_exec_audit_event(
                audit_started_at,
                &audit_snapshot,
                sandbox_type,
                sandbox_policy,
                duration,
                ExecAuditStatus::Failure,
                None,
                Some(err_string.as_str()),
                None,
            ) {
                tracing::error!(error = ?audit_err, "failed to append exec audit entry after error");
            }
            Err(err)
        }
    }
}

/// We don't have a fully deterministic way to tell if our command failed
/// because of the sandbox - a command in the user's zshrc file might hit an
/// error, but the command itself might fail or succeed for other reasons.
/// For now, we conservatively check for 'command not found' (exit code 127),
/// and can add additional cases as necessary.
fn is_likely_sandbox_denied(sandbox_type: SandboxType, exit_code: i32) -> bool {
    if sandbox_type == SandboxType::None {
        return false;
    }

    // Quick rejects: well-known non-sandbox shell exit codes
    // 127: command not found, 2: misuse of shell builtins
    if exit_code == 127 {
        return false;
    }

    // For all other cases, we assume the sandbox is the cause
    true
}

#[derive(Debug)]
pub struct StreamOutput<T> {
    pub text: T,
    pub truncated_after_lines: Option<u32>,
}
#[derive(Debug)]
struct RawExecToolCallOutput {
    pub exit_status: ExitStatus,
    pub stdout: StreamOutput<Vec<u8>>,
    pub stderr: StreamOutput<Vec<u8>>,
    pub aggregated_output: StreamOutput<Vec<u8>>,
    pub timed_out: bool,
}

impl StreamOutput<String> {
    pub fn new(text: String) -> Self {
        Self {
            text,
            truncated_after_lines: None,
        }
    }
}

impl StreamOutput<Vec<u8>> {
    pub fn from_utf8_lossy(&self) -> StreamOutput<String> {
        StreamOutput {
            text: String::from_utf8_lossy(&self.text).to_string(),
            truncated_after_lines: self.truncated_after_lines,
        }
    }
}

#[inline]
fn append_all(dst: &mut Vec<u8>, src: &[u8]) {
    dst.extend_from_slice(src);
}

#[derive(Debug)]
pub struct ExecToolCallOutput {
    pub exit_code: i32,
    pub stdout: StreamOutput<String>,
    pub stderr: StreamOutput<String>,
    pub aggregated_output: StreamOutput<String>,
    pub duration: Duration,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecAuditStatus {
    Success,
    Timeout,
    SandboxDenied,
    Failure,
}

impl ExecAuditStatus {
    fn action(self) -> &'static str {
        match self {
            ExecAuditStatus::Success => "exec_succeeded",
            ExecAuditStatus::Timeout => "exec_timeout",
            ExecAuditStatus::SandboxDenied => "exec_denied",
            ExecAuditStatus::Failure => "exec_failed",
        }
    }

    fn label(self) -> &'static str {
        match self {
            ExecAuditStatus::Success => "success",
            ExecAuditStatus::Timeout => "timeout",
            ExecAuditStatus::SandboxDenied => "sandbox_denied",
            ExecAuditStatus::Failure => "failure",
        }
    }
}

fn append_exec_audit_event(
    audit_started_at: DateTime<Utc>,
    snapshot: &ExecParams,
    sandbox_type: SandboxType,
    sandbox_policy: &SandboxPolicy,
    duration: Duration,
    status: ExecAuditStatus,
    output: Option<&ExecToolCallOutput>,
    error_message: Option<&str>,
    resource_notice: Option<String>,
) -> io::Result<()> {
    let broker = SecretBroker::global();
    let command_repr = snapshot.command.join(" ");
    let scrubbed_command = broker.scrub_text(&command_repr);

    let mut metadata = HashMap::new();
    metadata.insert("status".to_string(), status.label().to_string());
    metadata.insert("command".to_string(), scrubbed_command);
    metadata.insert("cwd".to_string(), snapshot.cwd.display().to_string());
    metadata.insert("sandbox_type".to_string(), format!("{sandbox_type:?}"));
    metadata.insert("duration_ms".to_string(), duration.as_millis().to_string());

    if let Some(timeout_ms) = snapshot.timeout_ms {
        metadata.insert("timeout_ms".to_string(), timeout_ms.to_string());
    }
    if let Some(flag) = snapshot.with_escalated_permissions {
        metadata.insert("escalated_permissions".to_string(), flag.to_string());
    }
    if let Some(justification) = snapshot.justification.as_ref() {
        metadata.insert(
            "justification".to_string(),
            broker.scrub_text(justification),
        );
    }
    if let Some(notice) = resource_notice.as_ref() {
        metadata.insert("resource_notice".to_string(), broker.scrub_text(notice));
    }
    if let Ok(policy) = serde_json::to_string(sandbox_policy) {
        metadata.insert("sandbox_policy".to_string(), policy);
    }

    if let Some(out) = output {
        metadata.insert("exit_code".to_string(), out.exit_code.to_string());
        metadata.insert("timed_out".to_string(), out.timed_out.to_string());
    }

    if let Some(message) = error_message {
        metadata.insert("error".to_string(), broker.scrub_text(message));
    }

    let mut event = AuditEvent::new(
        AuditEventKind::SandboxExec,
        "core:exec",
        status.action(),
        snapshot.cwd.display().to_string(),
    )
    .with_timestamp(audit_started_at);

    for (key, value) in metadata {
        event = event.with_metadata(key, value);
    }

    if let Err(err) = append_audit_event(event) {
        tracing::error!(
            error = ?err,
            "failed to append exec audit entry (REQ-SEC-02); continuing without audit"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::SecretScope;
    use crate::security::export_audit_records;
    use std::collections::HashMap;
    use tempfile::TempDir;

    #[test]
    fn append_exec_audit_event_masks_secrets() {
        let codex_home = TempDir::new().expect("tempdir");
        let codex_home_path = codex_home.path().to_path_buf();
        unsafe {
            std::env::set_var("CODEX_HOME", &codex_home_path);
        }
        std::mem::forget(codex_home);

        let secret = "super-secret".to_string();
        SecretBroker::global().register_secret(secret.clone(), SecretScope::Session);

        let snapshot = ExecParams {
            command: vec!["/bin/echo".to_string(), secret.clone()],
            cwd: codex_home_path.clone(),
            timeout_ms: Some(1000),
            env: HashMap::new(),
            with_escalated_permissions: Some(false),
            justification: Some("test".to_string()),
        };

        append_exec_audit_event(
            Utc::now(),
            &snapshot,
            SandboxType::None,
            &SandboxPolicy::new_read_only_policy(),
            Duration::from_millis(10),
            ExecAuditStatus::SandboxDenied,
            None,
            Some("permission denied"),
            None,
        )
        .expect("append audit");

        let records = export_audit_records().expect("export records");
        let last = records.last().expect("audit record");
        let command_meta = last
            .metadata
            .iter()
            .find(|entry| entry.key == "command")
            .expect("command metadata");
        assert!(!command_meta.value.contains(&secret));
        assert!(command_meta.value.contains("***"));
    }
}

async fn exec(
    params: ExecParams,
    sandbox_policy: &SandboxPolicy,
    stdout_stream: Option<StdoutStream>,
) -> Result<RawExecToolCallOutput> {
    let timeout = params.timeout_duration();
    let ExecParams {
        command, cwd, env, ..
    } = params;

    let (program, args) = command.split_first().ok_or_else(|| {
        CodexErr::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "command args are empty",
        ))
    })?;
    let arg0 = None;
    let child = spawn_child_async(
        PathBuf::from(program),
        args.into(),
        arg0,
        cwd,
        sandbox_policy,
        StdioPolicy::RedirectForShellTool,
        env,
        Some(ResourceLimits::standard()),
    )
    .await?;
    consume_truncated_output(child, timeout, stdout_stream).await
}

/// Consumes the output of a child process, truncating it so it is suitable for
/// use as the output of a `shell` tool call. Also enforces specified timeout.
async fn consume_truncated_output(
    mut child: Child,
    timeout: Duration,
    stdout_stream: Option<StdoutStream>,
) -> Result<RawExecToolCallOutput> {
    // Both stdout and stderr were configured with `Stdio::piped()`
    // above, therefore `take()` should normally return `Some`.  If it doesn't
    // we treat it as an exceptional I/O error

    let stdout_reader = child.stdout.take().ok_or_else(|| {
        CodexErr::Io(io::Error::other(
            "stdout pipe was unexpectedly not available",
        ))
    })?;
    let stderr_reader = child.stderr.take().ok_or_else(|| {
        CodexErr::Io(io::Error::other(
            "stderr pipe was unexpectedly not available",
        ))
    })?;

    let (agg_tx, agg_rx) = async_channel::unbounded::<Vec<u8>>();

    let stdout_handle = tokio::spawn(read_capped(
        BufReader::new(stdout_reader),
        stdout_stream.clone(),
        false,
        Some(agg_tx.clone()),
    ));
    let stderr_handle = tokio::spawn(read_capped(
        BufReader::new(stderr_reader),
        stdout_stream.clone(),
        true,
        Some(agg_tx.clone()),
    ));

    let (exit_status, timed_out) = tokio::select! {
        result = tokio::time::timeout(timeout, child.wait()) => {
            match result {
                Ok(status_result) => {
                    let exit_status = status_result?;
                    (exit_status, false)
                }
                Err(_) => {
                    // timeout
                    child.start_kill()?;
                    // Debatable whether `child.wait().await` should be called here.
                    (synthetic_exit_status(EXIT_CODE_SIGNAL_BASE + TIMEOUT_CODE), true)
                }
            }
        }
        _ = tokio::signal::ctrl_c() => {
            child.start_kill()?;
            (synthetic_exit_status(EXIT_CODE_SIGNAL_BASE + SIGKILL_CODE), false)
        }
    };

    let stdout = stdout_handle.await??;
    let stderr = stderr_handle.await??;

    drop(agg_tx);

    let mut combined_buf = Vec::with_capacity(AGGREGATE_BUFFER_INITIAL_CAPACITY);
    while let Ok(chunk) = agg_rx.recv().await {
        append_all(&mut combined_buf, &chunk);
    }
    let aggregated_output = StreamOutput {
        text: combined_buf,
        truncated_after_lines: None,
    };

    Ok(RawExecToolCallOutput {
        exit_status,
        stdout,
        stderr,
        aggregated_output,
        timed_out,
    })
}

async fn read_capped<R: AsyncRead + Unpin + Send + 'static>(
    mut reader: R,
    stream: Option<StdoutStream>,
    is_stderr: bool,
    aggregate_tx: Option<Sender<Vec<u8>>>,
) -> io::Result<StreamOutput<Vec<u8>>> {
    let mut buf = Vec::with_capacity(AGGREGATE_BUFFER_INITIAL_CAPACITY);
    let mut tmp = [0u8; READ_CHUNK_SIZE];
    let mut emitted_deltas: usize = 0;

    // No caps: append all bytes

    loop {
        let n = reader.read(&mut tmp).await?;
        if n == 0 {
            break;
        }

        if let Some(stream) = &stream
            && emitted_deltas < MAX_EXEC_OUTPUT_DELTAS_PER_CALL
        {
            let chunk = tmp[..n].to_vec();
            let msg = EventMsg::ExecCommandOutputDelta(ExecCommandOutputDeltaEvent {
                call_id: stream.call_id.clone(),
                stream: if is_stderr {
                    ExecOutputStream::Stderr
                } else {
                    ExecOutputStream::Stdout
                },
                chunk,
            });
            let event = Event {
                id: stream.sub_id.clone(),
                msg,
            };
            #[allow(clippy::let_unit_value)]
            let _ = stream.tx_event.send(event).await;
            emitted_deltas += 1;
        }

        if let Some(tx) = &aggregate_tx {
            let _ = tx.send(tmp[..n].to_vec()).await;
        }

        append_all(&mut buf, &tmp[..n]);
        // Continue reading to EOF to avoid back-pressure
    }

    Ok(StreamOutput {
        text: buf,
        truncated_after_lines: None,
    })
}

#[cfg(unix)]
fn synthetic_exit_status(code: i32) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(code)
}

#[cfg(windows)]
fn synthetic_exit_status(code: i32) -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    #[expect(clippy::unwrap_used)]
    std::process::ExitStatus::from_raw(code.try_into().unwrap())
}
