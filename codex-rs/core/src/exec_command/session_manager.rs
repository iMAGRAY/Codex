use std::collections::HashMap;
use std::collections::VecDeque;
use std::fmt;
use std::io::ErrorKind;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration as StdDuration;
use std::time::SystemTime;

use chrono::DateTime;
use chrono::Utc;
use dirs::cache_dir;
use futures::FutureExt;
use futures::future::Shared;
use portable_pty::CommandBuilder;
use portable_pty::PtySize;
use portable_pty::native_pty_system;
use regex_lite::Regex;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use tokio::fs::File as TokioFile;
use tokio::fs::{self};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio::sync::Notify;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tokio::time::Instant;
use tokio::time::sleep;

use crate::exec_command::exec_command_params::CommandLine;
use crate::exec_command::exec_command_params::ExecCommandParams;
use crate::exec_command::exec_command_params::WriteStdinParams;
use crate::exec_command::exec_command_session::ExecCommandSession;
use crate::exec_command::session_id::SessionId;
use crate::truncate::truncate_middle;
use codex_protocol::protocol::EXEC_COMMAND_PAYLOAD_VERSION;
use codex_protocol::protocol::ExecCommandPayload;
use codex_protocol::protocol::ExecCommandStatus;
use codex_protocol::protocol::ExecPatternMatchPayload;

use super::control::ExecControlAction;
use super::control::ExecControlParams;
use super::control::ExecControlResponse;
use super::control::ExecControlStatus;
use super::control::ExecWatchAction;

const DEFAULT_IDLE_TIMEOUT_MS: u64 = 300_000;
const MIN_IDLE_TIMEOUT_MS: u64 = 1_000;
const MAX_IDLE_TIMEOUT_MS: u64 = 86_400_000; // 24h
const DEFAULT_HARD_TIMEOUT_MS: Option<u64> = Some(7_200_000); // 2h
const MIN_GRACE_MS: u64 = 500;
const MAX_GRACE_MS: u64 = 60_000;
const IDLE_WATCH_INTERVAL: Duration = Duration::from_secs(1);
const PRUNE_AFTER_MS: u64 = 3_900_000; // ~65 minutes retention
const OUTPUT_RETENTION_BYTES: usize = 1024 * 1024; // 1 MiB of recent output
const LINE_RETENTION_COUNT: usize = 10_000; // Keep last 10k lines for line-based queries
// Reduced to minimize token cost when polling sessions (each list_exec_sessions call includes recent output)
const DESCRIPTOR_RECENT_LINES: usize = 2;
const DESCRIPTOR_RECENT_BYTES: usize = 4 * 1024;
const MIN_YIELD_TIME_MS: u64 = 100;
const MAX_YIELD_TIME_MS: u64 = 60_000;
const MIN_OUTPUT_CAP_BYTES: usize = 256;
/// Minimum lines required before applying compression to balance token savings vs overhead
const COMPRESSION_MIN_LINES: usize = 20;
const SESSION_EVENT_TTL: StdDuration = StdDuration::from_secs(3_600);
const SESSION_EVENT_MAX: usize = 256;
const IDLE_WARNING_WINDOW_MS: u64 = 3_000;
const DEFAULT_AUTO_POLL_CAP_TOKENS: u64 = 160;
const SUBSEQUENT_AUTO_POLL_CAP_TOKENS: u64 = 80;
const STOP_PATTERN_TAIL_LABEL: &str = "[stop_pattern tail omitted]";

fn session_event_ttl_duration() -> Duration {
    Duration::from_secs(SESSION_EVENT_TTL.as_secs())
        + Duration::from_nanos(SESSION_EVENT_TTL.subsec_nanos() as u64)
}

#[derive(Debug, Clone)]
pub struct ExecSessionDescriptor {
    pub session_id: SessionId,
    pub command_preview: String,
    pub state: SessionLifecycle,
    pub uptime: Duration,
    pub idle_remaining: Option<Duration>,
    pub total_output_bytes: u64,
    pub log_path: Option<PathBuf>,
    pub recent_output: Vec<String>,
    pub note: Option<String>,
    pub lossy: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecSessionEventKind {
    Started,
    Updated,
    Terminated,
}

#[derive(Debug, Clone)]
pub struct ExecSessionEvent {
    pub kind: ExecSessionEventKind,
    pub descriptor: ExecSessionDescriptor,
}

#[derive(Debug, Clone)]
pub struct SessionManager {
    inner: Arc<SessionRegistry>,
}

impl SessionManager {
    fn new(inner: Arc<SessionRegistry>) -> Self {
        Self { inner }
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        ExecFlowRegistry::new().session_manager()
    }
}

#[derive(Debug, Clone)]
pub struct ExecFlowRegistry {
    inner: Arc<SessionRegistry>,
}

impl ExecFlowRegistry {
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(64);
        Self {
            inner: Arc::new(SessionRegistry {
                next_session_id: AtomicU32::new(0),
                sessions: Mutex::new(HashMap::new()),
                events,
                archived_events: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn session_manager(&self) -> SessionManager {
        SessionManager::new(Arc::clone(&self.inner))
    }

    pub fn subscribe_updates(&self) -> broadcast::Receiver<ExecSessionEvent> {
        self.inner.subscribe_events()
    }

    pub fn emit_update(&self, event: ExecSessionEvent) {
        self.inner.emit_event(event);
    }

    pub async fn list_descriptors(&self) -> Vec<ExecSessionDescriptor> {
        self.session_manager().list_session_descriptors().await
    }

    pub async fn get_descriptor(&self, session_id: SessionId) -> Option<ExecSessionDescriptor> {
        self.session_manager()
            .get_session_descriptor(session_id)
            .await
    }

    pub async fn exec_control(&self, params: ExecControlParams) -> ExecControlResponse {
        self.session_manager()
            .handle_exec_control_request(params)
            .await
    }
}

impl Default for ExecFlowRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    pub fn subscribe_events(&self) -> broadcast::Receiver<ExecSessionEvent> {
        self.inner.subscribe_events()
    }

    pub async fn list_session_descriptors(&self) -> Vec<ExecSessionDescriptor> {
        self.inner.prune_finished().await;
        let sessions = {
            let sessions_guard = self.inner.sessions.lock().await;
            sessions_guard.values().cloned().collect::<Vec<_>>()
        };

        let mut descriptors = Vec::with_capacity(sessions.len());
        for session in sessions {
            descriptors.push(
                session
                    .descriptor(DESCRIPTOR_RECENT_LINES, DESCRIPTOR_RECENT_BYTES)
                    .await,
            );
        }
        descriptors.sort_by(|a, b| a.session_id.0.cmp(&b.session_id.0));
        descriptors
    }

    pub async fn get_session_descriptor(
        &self,
        session_id: SessionId,
    ) -> Option<ExecSessionDescriptor> {
        self.inner.prune_finished().await;
        let session = {
            let sessions = self.inner.sessions.lock().await;
            sessions.get(&session_id).cloned()
        }?;
        Some(
            session
                .descriptor(DESCRIPTOR_RECENT_LINES, DESCRIPTOR_RECENT_BYTES)
                .await,
        )
    }

    pub async fn handle_exec_command_request(
        &self,
        params: ExecCommandParams,
    ) -> Result<ExecCommandOutput, String> {
        self.inner.prune_finished().await;

        let session_id = SessionId(self.inner.next_session_id.fetch_add(1, Ordering::SeqCst));

        let command_line = params.cmd.clone();
        let (session, output_rx, exit_rx) =
            create_exec_command_session(&command_line, params.shell.clone(), params.login)
                .await
                .map_err(|err| {
                    format!(
                        "failed to create exec command session for session id {}: {err}",
                        session_id.0
                    )
                })?;
        let exit_rx = exit_rx.shared();

        let idle_timeout = params
            .idle_timeout_ms
            .map(|ms| clamp(ms, MIN_IDLE_TIMEOUT_MS, MAX_IDLE_TIMEOUT_MS))
            .unwrap_or(DEFAULT_IDLE_TIMEOUT_MS);
        let hard_timeout = params
            .hard_timeout_ms
            .or(DEFAULT_HARD_TIMEOUT_MS)
            .map(Duration::from_millis);
        let grace_period =
            Duration::from_millis(clamp(params.grace_period_ms, MIN_GRACE_MS, MAX_GRACE_MS));
        let log_threshold = params.log_threshold_bytes.clamp(1_024, 4 * 1024 * 1024) as usize;

        let managed_session = ManagedSession::new(
            session_id,
            command_line,
            session,
            Duration::from_millis(idle_timeout),
            hard_timeout,
            grace_period,
            log_threshold,
        );

        let managed_session = Arc::new(managed_session);
        self.inner
            .sessions
            .lock()
            .await
            .insert(session_id, Arc::clone(&managed_session));
        managed_session
            .start_supervision(Arc::clone(&self.inner), output_rx, exit_rx.clone())
            .await;

        emit_session_event(&self.inner, &managed_session, ExecSessionEventKind::Started).await;

        // Collect output
        let cap_bytes = normalize_cap_bytes(params.max_output_tokens);
        let start_time = Instant::now();
        let deadline =
            start_time + Duration::from_millis(normalize_yield_time_ms(params.yield_time_ms));

        let collected = managed_session.collect_output(deadline, cap_bytes).await;

        let output = String::from_utf8_lossy(&collected.data).to_string();

        // Capture metadata before truncation
        let lines_count = output.lines().count();
        let chunk_bytes = output.len() as u64;
        let pattern_matched = false; // No pattern matching on initial exec_command
        let incremental = false; // First read is never incremental

        let (output, original_token_count) = truncate_middle(&output, cap_bytes);
        let wall_time = Instant::now().duration_since(start_time);

        let snapshot = managed_session.log_snapshot().await;
        let termination_note = managed_session.termination_note().await;

        let exit_status = if let Some(code) = managed_session.completed_exit_code().await {
            ExitStatus::Exited(code)
        } else if let Some(result) = exit_rx.clone().now_or_never() {
            match result {
                Ok(code) => ExitStatus::Exited(code),
                Err(_) => ExitStatus::Exited(-1),
            }
        } else {
            ExitStatus::Ongoing(session_id)
        };

        // Get total lines, lossy UTF-8 status, and partial tail length
        let (total_lines, lossy_utf8, partial_len_bytes, partial_snapshot) = {
            let line_buffer = managed_session.line_buffer.lock().await;
            (
                line_buffer.total_lines(),
                line_buffer.has_lossy_utf8(),
                line_buffer
                    .partial_tail()
                    .as_ref()
                    .map(|partial| partial.len() as u64)
                    .unwrap_or(0),
                line_buffer.partial_tail(),
            )
        };

        // Initialize the agent cursor to the current partial length but mark it as pending,
        // so the first incremental poll returns the visible partial once without duplicating
        // when it later completes into a full line.
        managed_session
            .agent_partial_len
            .store(partial_len_bytes, Ordering::SeqCst);
        managed_session
            .agent_partial_pending
            .store(partial_len_bytes > 0, Ordering::SeqCst);
        if let Some(snap) = partial_snapshot {
            if !snap.is_empty() {
                if let Ok(mut guard) = managed_session.agent_partial_snapshot.try_lock() {
                    *guard = Some(snap);
                }
            }
        }

        let lossy = collected.lost
            || managed_session.output_overflowed.load(Ordering::SeqCst)
            || lossy_utf8;

        Ok(ExecCommandOutput {
            wall_time,
            exit_status,
            original_token_count,
            output,
            log_path: snapshot.log_path,
            log_sha256: snapshot.log_sha256,
            total_output_bytes: snapshot.total_bytes,
            note: termination_note,
            lossy,
            lines_count,
            chunk_bytes,
            pattern_matched,
            pattern_metadata: None,
            tail_label: None,
            incremental,
            from_line: 0,
            to_line: total_lines,
            total_lines,
            compressed: false,
            original_line_count: lines_count,
            guidance: None,
            actions_summary: Vec::new(),
        })
    }

    pub async fn handle_write_stdin_request(
        &self,
        params: WriteStdinParams,
    ) -> Result<ExecCommandOutput, String> {
        self.inner.prune_finished().await;
        let session = {
            let sessions = self.inner.sessions.lock().await;
            sessions.get(&params.session_id).cloned()
        };

        let Some(session) = session else {
            return Err(format!("unknown session id {}", params.session_id.0));
        };

        let WriteStdinParams {
            session_id,
            chars,
            yield_time_ms,
            max_output_tokens,
            tail_lines,
            since_byte,
            reset_cursor,
            stop_pattern,
            stop_pattern_cut,
            stop_pattern_label_tail,
            raw,
            compact: _,
            all,
            from_line,
            to_line,
            smart_compress,
        } = params;

        // Warn about deprecated parameters (legacy byte-cursor mode)
        if since_byte.is_some() || reset_cursor || raw {
            tracing::warn!(
                session_id = session_id.0,
                "deprecated params ignored (since_byte/reset_cursor/raw). Use line-based modes: all/from_line/tail_lines"
            );
        }

        if !chars.is_empty() {
            let bytes = chars.into_bytes();
            if session.write_to_stdin(bytes).await.is_err() {
                return Err("failed to write to stdin".to_string());
            }
            session.record_activity().await;
        }

        let start_time = Instant::now();
        let deadline = start_time + Duration::from_millis(normalize_yield_time_ms(yield_time_ms));

        // Wait for output or deadline
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {}
            _ = session.output_notify.notified() => {}
        }

        let line_buffer = session.line_buffer.lock().await;
        let total_lines = line_buffer.total_lines();
        let partial_tail = line_buffer.partial_tail();
        let partial_len_bytes = partial_tail.as_ref().map(|s| s.len() as u64).unwrap_or(0);

        // Determine line range based on mode (validate mutually exclusive)
        let mode_count = [all, from_line.is_some(), tail_lines.is_some()]
            .iter()
            .filter(|&&x| x)
            .count();
        if mode_count > 1 {
            return Err(
                "mutually exclusive params: only one of all/from_line/tail_lines allowed"
                    .to_string(),
            );
        }

        let (from, to, mode_name) = if all {
            // Mode 1: All output from beginning
            (0, total_lines, "all")
        } else if let Some(f) = from_line {
            // Mode 2: Specific range
            let end = to_line.unwrap_or(total_lines);
            if f > end {
                return Err(format!("invalid range: from_line ({f}) > to_line ({end})"));
            }
            (f, end, "range")
        } else if let Some(n) = tail_lines {
            // Mode 3: Last N lines
            let n_clamped = n.min(total_lines.try_into().unwrap_or(usize::MAX));
            let n_u64 = n_clamped as u64;
            let start = total_lines.saturating_sub(n_u64);
            (start, total_lines, "tail")
        } else {
            // Mode 4: AUTO - incremental from last read
            let last_read = session.agent_read_line.load(Ordering::SeqCst);
            (last_read, total_lines, "auto")
        };

        // Get lines from buffer and prepare watcher segments
        let mut lines = line_buffer.get_lines(from, to);
        let mut pattern_segments: Vec<PatternSegment> = lines
            .iter()
            .enumerate()
            .map(|(idx, text)| PatternSegment {
                line_no: Some(from + idx as u64),
                text: text.clone(),
                is_partial: false,
            })
            .collect();
        let mut pristine_pattern_segments: Option<Vec<PatternSegment>> = None;

        let prev_partial_len = session.agent_partial_len.load(Ordering::SeqCst);

        // Include partial tail when caller requested the latest output
        let mut update_partial_cursor = None;
        if let Some(partial) = partial_tail.as_ref() {
            let include_partial = match mode_name {
                "auto" => true,
                "all" => true,
                "tail" => true,
                "range" => to >= total_lines,
                _ => false,
            };

            if include_partial && !partial.is_empty() {
                let partial_pending = session.agent_partial_pending.load(Ordering::SeqCst);
                if mode_name == "auto" {
                    if partial_pending {
                        // Prefer the start snapshot once if available to avoid race with later appends
                        let snapshot_opt = session
                            .agent_partial_snapshot
                            .try_lock()
                            .ok()
                            .and_then(|mut g| g.take());
                        if let Some(snap) = snapshot_opt {
                            lines.push(snap);
                        } else {
                            lines.push(partial.clone());
                        }
                        pattern_segments.push(PatternSegment {
                            line_no: Some(total_lines),
                            text: partial.clone(),
                            is_partial: true,
                        });
                        update_partial_cursor = Some(partial_len_bytes);
                        session.agent_partial_pending.store(false, Ordering::SeqCst);
                    } else if partial_len_bytes > prev_partial_len {
                        let start = prev_partial_len as usize;
                        let appended = partial[start..].to_string();
                        if !appended.is_empty() {
                            lines.push(appended);
                            pattern_segments.push(PatternSegment {
                                line_no: Some(total_lines),
                                text: partial.clone(),
                                is_partial: true,
                            });
                            update_partial_cursor = Some(partial_len_bytes);
                            session.agent_partial_pending.store(false, Ordering::SeqCst);
                        }
                    } else if lines.is_empty() && stop_pattern.is_some() {
                        // No new delta but caller expects stop_pattern visibility; include partial once.
                        lines.push(partial.clone());
                        pattern_segments.push(PatternSegment {
                            line_no: Some(total_lines),
                            text: partial.clone(),
                            is_partial: true,
                        });
                        update_partial_cursor = Some(partial_len_bytes);
                        session.agent_partial_pending.store(false, Ordering::SeqCst);
                    }
                } else {
                    lines.push(partial.clone());
                    pattern_segments.push(PatternSegment {
                        line_no: Some(total_lines),
                        text: partial.clone(),
                        is_partial: true,
                    });
                    session.agent_partial_pending.store(false, Ordering::SeqCst);
                }
            } else if partial_len_bytes > 0 {
                session.agent_partial_pending.store(true, Ordering::SeqCst);
            }
        }

        if pristine_pattern_segments.is_none() {
            pristine_pattern_segments = Some(pattern_segments.clone());
        }

        if mode_name == "auto"
            && partial_tail.is_none()
            && prev_partial_len > 0
            && !lines.is_empty()
        {
            drop_utf8_prefix(&mut lines[0], prev_partial_len as usize);
            if let Some(segment) = pattern_segments.first_mut() {
                drop_utf8_prefix(&mut segment.text, prev_partial_len as usize);
            }
            if lines.first().is_some_and(std::string::String::is_empty) {
                lines.remove(0);
                if !pattern_segments.is_empty() {
                    pattern_segments.remove(0);
                }
            }
        }

        drop(line_buffer); // Release lock

        if partial_tail.is_none() {
            session.agent_partial_pending.store(false, Ordering::SeqCst);
        }

        if mode_name == "auto" {
            if let Some(new_len) = update_partial_cursor {
                session.agent_partial_len.store(new_len, Ordering::SeqCst);
            } else if partial_tail.is_none() {
                session.agent_partial_len.store(0, Ordering::SeqCst);
            }
        }

        let pattern_segments_for_match = pristine_pattern_segments
            .as_ref()
            .unwrap_or(&pattern_segments);

        // Check stop_pattern and auto-terminate if matched
        let mut pattern_matched = false;
        let mut pattern_match_meta: Option<SessionPatternMatch> = None;
        let mut stop_pattern_index: Option<usize> = None;
        if let Some(pattern_str) = stop_pattern.as_ref() {
            match Regex::new(pattern_str) {
                Ok(re) => {
                    for (idx, segment) in pattern_segments_for_match.iter().enumerate() {
                        if re.is_match(&segment.text) {
                            pattern_matched = true;
                            stop_pattern_index = Some(idx);
                            pattern_match_meta = Some(SessionPatternMatch {
                                pattern: pattern_str.clone(),
                                matched_line_no: segment.line_no,
                                matched_text: Some(segment.text.clone()),
                            });
                            // Send Ctrl-C
                            let _ = session.write_to_stdin(vec![0x03]).await;
                            break;
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        session_id = session_id.0,
                        "invalid stop_pattern regex: {err}"
                    );
                }
            }
        }

        let stop_pattern_event_id =
            if let (true, Some(meta)) = (pattern_matched, pattern_match_meta.clone()) {
                Some(
                    session
                        .record_event(
                            SessionEventKind::StopPatternMatched,
                            SessionEventSource::StopPattern,
                            meta.matched_text.as_ref().map(|text| {
                                format!("matched `{}`", clamp_metadata_text(text, 120))
                            }),
                            Some(SessionEventAction::Log),
                            Some(meta.clone()),
                            None,
                        )
                        .await,
                )
            } else {
                None
            };

        if stop_pattern_event_id.is_some()
            && let Some(meta) = pattern_match_meta.clone()
        {
            session
                .record_event(
                    SessionEventKind::CtrlCSent,
                    SessionEventSource::StopPattern,
                    Some("stop_pattern ctrl-c".to_string()),
                    Some(SessionEventAction::SendCtrlC),
                    Some(meta),
                    stop_pattern_event_id,
                )
                .await;
        }

        if pattern_matched {
            session.stop_pattern_triggered.store(true, Ordering::SeqCst);
        }

        if pattern_matched && let Some(meta) = pattern_match_meta.as_ref() {
            let highlight = if let Some(text) = meta.matched_text.as_ref() {
                format!("▶ stop_pattern matched: {}", clamp_metadata_text(text, 160))
            } else {
                format!(
                    "▶ stop_pattern matched (pattern `{}`)",
                    clamp_metadata_text(&meta.pattern, 120)
                )
            };
            if !lines.iter().any(|line| line == &highlight) {
                lines.insert(0, highlight.clone());
                let highlight_segment = PatternSegment {
                    line_no: meta.matched_line_no,
                    text: highlight,
                    is_partial: false,
                };
                pattern_segments.insert(0, highlight_segment);
            }
        }

        let mut watcher_segments = pattern_segments.clone();
        let visibility_segments =
            pristine_pattern_segments.unwrap_or_else(|| watcher_segments.clone());
        let mut tail_label_applied = false;
        let mut tail_label_hint: Option<String> = None;
        let mut actions_summary: Vec<String> = Vec::new();
        if let Some(idx) = stop_pattern_index {
            if stop_pattern_cut {
                if idx + 1 < lines.len() {
                    lines.truncate(idx + 1);
                    session
                        .record_event(
                            SessionEventKind::OutputTrimmed,
                            SessionEventSource::StopPattern,
                            Some(format!("trimmed_after_stop_pattern idx={idx}")),
                            Some(SessionEventAction::TrimOutput),
                            pattern_match_meta.clone(),
                            stop_pattern_event_id,
                        )
                        .await;
                }
                if stop_pattern_label_tail {
                    lines.push(STOP_PATTERN_TAIL_LABEL.to_string());
                    tail_label_applied = true;
                    tail_label_hint = Some("tail_omitted_after_stop_pattern".to_string());
                    session
                        .record_event(
                            SessionEventKind::PatternTailLabeled,
                            SessionEventSource::StopPattern,
                            Some("tail_label_appended".to_string()),
                            Some(SessionEventAction::TrimOutput),
                            pattern_match_meta.clone(),
                            stop_pattern_event_id,
                        )
                        .await;
                }
            } else if stop_pattern_label_tail {
                tail_label_applied = true;
                tail_label_hint = Some("tail_detected_post_stop_pattern".to_string());
                session
                    .record_event(
                        SessionEventKind::PatternTailLabeled,
                        SessionEventSource::StopPattern,
                        Some("tail_detected".to_string()),
                        Some(SessionEventAction::Log),
                        pattern_match_meta.clone(),
                        stop_pattern_event_id,
                    )
                    .await;
            }

            // Ensure matched line (and optional immediate context) is visible when not already present.
            if let Some(segment) = visibility_segments.get(idx) {
                let match_text = &segment.text;
                if let Some(pos) = lines.iter().position(|line| line == match_text) {
                    if let Some(target) = watcher_segments.get_mut(pos) {
                        target.text = match_text.clone();
                    }
                } else if let Some(pos) = lines
                    .iter()
                    .position(|line| !line.is_empty() && match_text.ends_with(line))
                {
                    lines[pos] = match_text.clone();
                    if let Some(target) = watcher_segments.get_mut(pos) {
                        target.text = match_text.clone();
                    }
                } else {
                    let mut insertion_lines = Vec::new();
                    let mut insertion_segments = Vec::new();
                    if idx > 0
                        && let Some(prev) = visibility_segments.get(idx.saturating_sub(1))
                        && !lines.iter().any(|line| line == &prev.text)
                    {
                        insertion_lines.push(prev.text.clone());
                        insertion_segments.push(prev.clone());
                    }
                    insertion_lines.push(match_text.clone());
                    insertion_segments.push(segment.clone());
                    // prepend context+match to preserve ordering
                    let mut new_lines = insertion_lines;
                    new_lines.extend(lines);
                    lines = new_lines;
                    let mut new_watcher_segments = insertion_segments;
                    new_watcher_segments.extend(watcher_segments);
                    watcher_segments = new_watcher_segments;
                }
            }
        }

        if pattern_matched {
            if let Some(pos) = lines
                .iter()
                .position(|line| line.starts_with("▶ stop_pattern matched"))
                && pos != 0
            {
                let highlight_line = lines.remove(pos);
                lines.insert(0, highlight_line);
            }
            if let Some(pos) = pattern_segments
                .iter()
                .position(|segment| segment.text.starts_with("▶ stop_pattern matched"))
                && pos != 0
            {
                let highlight_segment = pattern_segments.remove(pos);
                pattern_segments.insert(0, highlight_segment);
            }
            if let Some(pos) = watcher_segments
                .iter()
                .position(|segment| segment.text.starts_with("▶ stop_pattern matched"))
                && pos != 0
            {
                let highlight_segment = watcher_segments.remove(pos);
                watcher_segments.insert(0, highlight_segment);
            }
        }

        // Apply smart compression
        let compress_enabled = smart_compress && !pattern_matched;
        let compression_result = compress_output(lines, compress_enabled);
        lines = compression_result.lines;
        let was_compressed = compression_result.was_compressed;
        let original_line_count = compression_result.original_count;
        let mut guidance = compression_result.guidance;

        if pattern_matched {
            let label = if stop_pattern_cut {
                "stop_pattern ctrl-c (cut)".to_string()
            } else {
                "stop_pattern ctrl-c".to_string()
            };
            actions_summary.push(label);
        }

        if tail_label_applied && !stop_pattern_cut {
            guidance = Some(match guidance {
                Some(existing) => format!("{existing} | stop_pattern tail detected"),
                None => "stop_pattern tail detected".to_string(),
            });
        }

        if tail_label_applied {
            actions_summary.push("stop_pattern tail omitted".to_string());
        }

        if let Some(watch_hint) = session
            .apply_watchers(&watcher_segments, from, total_lines, &mut actions_summary)
            .await
        {
            guidance = Some(match guidance {
                Some(existing) => format!("{existing} | {watch_hint}"),
                None => watch_hint,
            });
        }

        if let Some(remaining) = session.idle_remaining().await {
            let remaining_ms = remaining.as_millis() as u64;
            if remaining_ms <= IDLE_WARNING_WINDOW_MS
                && session
                    .idle_warning_sent
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
            {
                session
                    .record_event(
                        SessionEventKind::IdleWarning,
                        SessionEventSource::IdleWatchdog,
                        Some(format!("idle_remaining_ms={remaining_ms}")),
                        Some(SessionEventAction::Log),
                        None,
                        None,
                    )
                    .await;
                guidance = Some(match guidance {
                    Some(existing) => format!("{existing} | idle timeout soon"),
                    None => "idle timeout soon".to_string(),
                });
            }
        }

        // Update line cursor (only in AUTO mode)
        if mode_name == "auto" && to > from {
            session.agent_read_line.store(to, Ordering::SeqCst);
        }

        // Build output string
        let output = lines.join("\n");
        let lines_count = lines.len();
        let chunk_bytes = output.len() as u64;

        if !output.is_empty() {
            session.idle_warning_sent.store(false, Ordering::SeqCst);
        }

        let default_cap = session.agent_default_max_tokens.load(Ordering::SeqCst);
        let effective_tokens = if mode_name == "auto" {
            if !session.agent_cap_reduced.swap(true, Ordering::SeqCst) {
                session
                    .agent_default_max_tokens
                    .store(max_output_tokens, Ordering::SeqCst);
                max_output_tokens
            } else if max_output_tokens == default_cap {
                SUBSEQUENT_AUTO_POLL_CAP_TOKENS.min(max_output_tokens)
            } else {
                max_output_tokens
            }
        } else {
            max_output_tokens
        };

        let cap_bytes = normalize_cap_bytes(effective_tokens);
        let (output, original_token_count) = truncate_middle(&output, cap_bytes);
        let wall_time = Instant::now().duration_since(start_time);
        let snapshot = session.log_snapshot().await;
        let termination_note = session.termination_note().await;

        emit_session_event(&self.inner, &session, ExecSessionEventKind::Updated).await;

        let exit_status = if let Some(code) = session.completed_exit_code().await {
            ExitStatus::Exited(code)
        } else if !session.is_terminated() && session.session.has_exited() {
            session
                .mark_terminated(TerminationReason::Completed { exit_code: 0 })
                .await;
            ExitStatus::Exited(0)
        } else {
            ExitStatus::Ongoing(session_id)
        };

        // Check for lossy UTF-8 conversions
        let lossy_utf8 = {
            let line_buffer = session.line_buffer.lock().await;
            line_buffer.has_lossy_utf8()
        };

        let lossy = session.output_overflowed.load(Ordering::SeqCst) || lossy_utf8;
        let incremental = mode_name == "auto";

        let mut note = termination_note;
        if pattern_matched {
            let message = if stop_pattern_cut {
                "stop_pattern ctrl-c (cut)"
            } else {
                "stop_pattern ctrl-c"
            };
            note = Some(match note.take() {
                Some(existing) => format!("{existing}; {message}"),
                None => message.to_string(),
            });
        } else if matches!(exit_status, ExitStatus::Exited(_))
            && session.stop_pattern_triggered.load(Ordering::SeqCst)
            && note.is_none()
        {
            note = Some("stop_pattern ctrl-c".to_string());
        }

        Ok(ExecCommandOutput {
            wall_time,
            exit_status,
            original_token_count,
            output,
            log_path: snapshot.log_path,
            log_sha256: snapshot.log_sha256,
            total_output_bytes: snapshot.total_bytes,
            note,
            lossy,
            lines_count,
            chunk_bytes,
            pattern_matched,
            pattern_metadata: pattern_match_meta,
            tail_label: tail_label_hint,
            incremental,
            from_line: from,
            to_line: to,
            total_lines,
            compressed: was_compressed,
            original_line_count,
            guidance,
            actions_summary,
        })
    }

    pub async fn handle_exec_control_request(
        &self,
        params: ExecControlParams,
    ) -> ExecControlResponse {
        self.inner.prune_finished().await;

        let session = {
            let sessions = self.inner.sessions.lock().await;
            sessions.get(&params.session_id).cloned()
        };

        let Some(session) = session else {
            return ExecControlResponse {
                session_id: params.session_id,
                status: ExecControlStatus::NoSuchSession,
                note: None,
            };
        };

        if session.is_terminated() {
            return ExecControlResponse {
                session_id: params.session_id,
                status: ExecControlStatus::AlreadyTerminated,
                note: session.termination_note().await,
            };
        }

        let status = match params.action {
            ExecControlAction::Keepalive { extend_timeout_ms } => {
                session.keepalive(extend_timeout_ms).await;
                session.idle_warning_sent.store(false, Ordering::SeqCst);
                let reason = extend_timeout_ms.map(|ms| format!("extend_timeout_ms={ms}"));
                session
                    .record_event(
                        SessionEventKind::Keepalive,
                        SessionEventSource::ControlAction,
                        reason.clone(),
                        Some(SessionEventAction::Log),
                        None,
                        None,
                    )
                    .await;
                emit_session_event(&self.inner, &session, ExecSessionEventKind::Updated).await;
                let note = reason
                    .map(|r| format!("keepalive acknowledged ({r})"))
                    .unwrap_or_else(|| "keepalive acknowledged".to_string());
                return ExecControlResponse {
                    session_id: params.session_id,
                    status: ExecControlStatus::ack(),
                    note: Some(note),
                };
            }
            ExecControlAction::SendCtrlC => {
                if session.send_ctrl_c().await {
                    session
                        .record_event(
                            SessionEventKind::CtrlCSent,
                            SessionEventSource::ControlAction,
                            Some("manual ctrl-c".to_string()),
                            Some(SessionEventAction::SendCtrlC),
                            None,
                            None,
                        )
                        .await;
                    emit_session_event(&self.inner, &session, ExecSessionEventKind::Updated).await;
                    ExecControlStatus::ack()
                } else {
                    ExecControlStatus::reject("failed to send ctrl-c")
                }
            }
            ExecControlAction::Terminate => {
                let terminate_event = session
                    .record_event(
                        SessionEventKind::TerminateRequested,
                        SessionEventSource::ControlAction,
                        Some("manual terminate".to_string()),
                        Some(SessionEventAction::Log),
                        None,
                        None,
                    )
                    .await;
                session.mark_grace(TerminationReason::UserRequested).await;
                emit_session_event(&self.inner, &session, ExecSessionEventKind::Updated).await;
                if session.send_ctrl_c().await {
                    session
                        .record_event(
                            SessionEventKind::CtrlCSent,
                            SessionEventSource::ControlAction,
                            Some("terminate".to_string()),
                            Some(SessionEventAction::SendCtrlC),
                            None,
                            Some(terminate_event),
                        )
                        .await;
                    ExecControlStatus::ack()
                } else {
                    ExecControlStatus::reject("failed to signal process")
                }
            }
            ExecControlAction::ForceKill => {
                match session.force_kill(TerminationReason::ForceKilled).await {
                    Ok(()) => {
                        let force_event = session
                            .record_event(
                                SessionEventKind::ForceKill,
                                SessionEventSource::ControlAction,
                                Some("manual force_kill".to_string()),
                                Some(SessionEventAction::ForceKill),
                                None,
                                None,
                            )
                            .await;
                        session
                            .record_escalation_summary(
                                "force_kill escalation".to_string(),
                                Some(force_event),
                            )
                            .await;
                        emit_session_event(&self.inner, &session, ExecSessionEventKind::Terminated)
                            .await;
                        ExecControlStatus::ack()
                    }
                    Err(err) => ExecControlStatus::reject(err),
                }
            }
            ExecControlAction::SetIdleTimeout { timeout_ms } => {
                session.set_idle_timeout(timeout_ms).await;
                session
                    .record_event(
                        SessionEventKind::IdleTimeoutUpdated,
                        SessionEventSource::ControlAction,
                        Some(format!("timeout_ms={timeout_ms}")),
                        Some(SessionEventAction::Log),
                        None,
                        None,
                    )
                    .await;
                emit_session_event(&self.inner, &session, ExecSessionEventKind::Updated).await;
                ExecControlStatus::ack()
            }
            ExecControlAction::Watch {
                pattern,
                action,
                persist,
                cooldown_ms,
                auto_send_ctrl_c,
            } => {
                let auto_flag =
                    auto_send_ctrl_c.unwrap_or(persist && matches!(action, ExecWatchAction::Log));
                match session
                    .add_watch(
                        pattern.clone(),
                        action.clone(),
                        persist,
                        cooldown_ms,
                        auto_flag,
                    )
                    .await
                {
                    Ok(note) => {
                        emit_session_event(&self.inner, &session, ExecSessionEventKind::Updated)
                            .await;
                        return ExecControlResponse {
                            session_id: params.session_id,
                            status: ExecControlStatus::ack(),
                            note: Some(note),
                        };
                    }
                    Err(err) => ExecControlStatus::reject(err),
                }
            }
            ExecControlAction::Unwatch { pattern } => {
                if session.remove_watch(&pattern).await {
                    ExecControlStatus::ack()
                } else {
                    ExecControlStatus::reject(format!(
                        "no watch matching `{pattern}` for session #{:02}",
                        params.session_id.0
                    ))
                }
            }
        };

        ExecControlResponse {
            session_id: params.session_id,
            status,
            note: session.termination_note().await,
        }
    }

    pub async fn list_sessions(&self) -> Vec<ExecSessionSummary> {
        self.inner.prune_finished().await;
        let sessions = self.inner.sessions.lock().await;
        let mut summaries: Vec<_> = sessions.values().map(|session| session.summary()).collect();
        summaries.sort_by(|a, b| a.session_id.0.cmp(&b.session_id.0));
        summaries
    }

    pub async fn list_sessions_filtered(
        &self,
        state: Option<SessionLifecycle>,
        limit: Option<usize>,
        since_ms: Option<u64>,
    ) -> Vec<ExecSessionSummary> {
        self.inner.prune_finished().await;
        let cutoff = since_ms.and_then(|ms| Instant::now().checked_sub(Duration::from_millis(ms)));
        let state_filter = state;
        let sessions = self.inner.sessions.lock().await;
        let mut summaries: Vec<_> = sessions
            .values()
            .filter_map(|session| {
                if let Some(target) = state_filter
                    && session.state() != target
                {
                    return None;
                }
                if let Some(cutoff) = cutoff
                    && session.created_at < cutoff
                {
                    return None;
                }
                Some(session.summary())
            })
            .collect();
        if let Some(limit) = limit {
            summaries.sort_by(|a, b| b.session_id.0.cmp(&a.session_id.0));
            summaries.truncate(limit);
            summaries.sort_by(|a, b| a.session_id.0.cmp(&b.session_id.0));
            summaries
        } else {
            summaries.sort_by(|a, b| a.session_id.0.cmp(&b.session_id.0));
            summaries
        }
    }

    pub async fn session_events(
        &self,
        session_id: SessionId,
        since_id: Option<u64>,
        limit: Option<usize>,
    ) -> Result<Vec<SessionEventEntry>, String> {
        self.inner.prune_finished().await;
        let session = {
            let sessions = self.inner.sessions.lock().await;
            sessions.get(&session_id).cloned()
        };

        if let Some(session) = session {
            return Ok(session.session_events(since_id, limit).await);
        }

        let archived = {
            let mut archive = self.inner.archived_events.lock().await;
            archive.retain(|_, entry| entry.expires_at > Instant::now());
            archive
                .get(&session_id)
                .map(|entry| entry.entries(since_id, limit))
        };

        if let Some(entries) = archived {
            return Ok(entries);
        }

        Err(format!("unknown session id {}", session_id.0))
    }
}

#[derive(Debug)]
pub struct ExecCommandOutput {
    wall_time: Duration,
    exit_status: ExitStatus,
    original_token_count: Option<u64>,
    output: String,
    log_path: Option<PathBuf>,
    log_sha256: Option<String>,
    total_output_bytes: u64,
    note: Option<String>,
    lossy: bool,
    /// Number of lines in this response (compact metadata)
    lines_count: usize,
    /// Bytes returned in this response (not total, just this chunk)
    chunk_bytes: u64,
    /// Whether stop_pattern was matched (auto-terminated)
    pattern_matched: bool,
    pattern_metadata: Option<SessionPatternMatch>,
    tail_label: Option<String>,
    /// Whether this is incremental read (cursor advanced)
    incremental: bool,
    /// Start line number (0-indexed)
    from_line: u64,
    /// End line number (exclusive)
    to_line: u64,
    /// Total lines written to session (including evicted)
    total_lines: u64,
    /// Whether output was compressed (e.g., sequential numbers detected)
    compressed: bool,
    /// Original line count before compression
    original_line_count: usize,
    /// Optional contextual guidance for the caller
    guidance: Option<String>,
    /// Optional list of quick action summaries (e.g. ctrl-c, force-kill)
    actions_summary: Vec<String>,
}

impl ExecCommandOutput {
    /// Compact output format: plain text without JSON wrapper.
    /// Saves ~100-150 tokens per response. Only includes critical metadata.
    pub(crate) fn to_compact_output(&self) -> String {
        let mut result = String::new();

        // Output first (main content)
        if !self.output.is_empty() {
            result.push_str(&self.output);
            if !self.output.ends_with('\n') {
                result.push('\n');
            }
        }

        // Critical metadata on separate line only when needed
        let mut metadata = Vec::new();

        match self.exit_status {
            ExitStatus::Exited(code) => {
                metadata.push(format!("exit_code={code}"));
            }
            ExitStatus::Ongoing(session_id) => {
                // Only show metadata if something important happened
                if self.pattern_matched {
                    metadata.push(format!("session=#{:02}", session_id.0));
                    metadata.push("pattern_matched=true".to_string());
                    metadata.push("ctrl_c_sent".to_string());
                } else if self.output.is_empty() {
                    // No new output - inform user
                    metadata.push(format!("session=#{:02}", session_id.0));
                    metadata.push("no_new_output".to_string());
                }
            }
        }

        if let Some(pattern) = &self.pattern_metadata {
            metadata.push(format!("pattern={}", pattern.pattern));
            if let Some(line_no) = pattern.matched_line_no {
                metadata.push(format!("pattern_line={line_no}"));
            }
            if let Some(text) = pattern.matched_text.as_ref() {
                metadata.push(format!("pattern_text={}", clamp_metadata_text(text, 160)));
            }
        }

        if let Some(label) = &self.tail_label {
            metadata.push(format!("tail={label}"));
        }

        if !self.actions_summary.is_empty() {
            metadata.push(format!("actions={}", self.actions_summary.join(",")));
        }

        // Show compression info if applied
        if self.compressed {
            metadata.push(format!(
                "compressed: {} → {} lines",
                self.original_line_count, self.lines_count
            ));
        }

        if let Some(hint) = &self.guidance {
            metadata.push(format!("hint={hint}"));
        }

        // Show range info for non-empty output
        if !self.output.is_empty() {
            metadata.push(format!(
                "range: {}-{} of {}",
                self.from_line, self.to_line, self.total_lines
            ));
        }

        if let Some(note) = &self.note {
            metadata.push(format!("note={note}"));
        }

        if !metadata.is_empty() {
            result.push_str("---\n");
            result.push_str(&metadata.join(" | "));
            result.push('\n');
        }

        result
    }

    pub(crate) fn to_text_output(&self) -> String {
        let (status, session_id, exit_code, management_hint) = match self.exit_status {
            ExitStatus::Exited(code) => (ExecCommandStatus::Completed, None, Some(code), None),
            ExitStatus::Ongoing(session_id) => (
                ExecCommandStatus::Running,
                Some(session_id.0),
                None,
                Some(format!(
                    "Ctrl+E session #{:02} • write_stdin({0}, \"\", stop_pattern=\"TEXT\") stops on match • tail_lines=50 trims noise",
                    session_id.0
                )),
            ),
        };

        let payload = ExecCommandPayload {
            version: EXEC_COMMAND_PAYLOAD_VERSION,
            status,
            session_id,
            exit_code,
            wall_time_ms: duration_to_u64_ms(self.wall_time),
            output: self.output.clone(),
            truncated: self.lossy || self.original_token_count.is_some(),
            total_output_bytes: self.total_output_bytes,
            original_token_count: self.original_token_count,
            log_path: self.log_path.as_ref().map(|p| p.display().to_string()),
            log_sha256: self.log_sha256.clone(),
            management_hint,
            note: self.note.clone(),
            guidance: self.guidance.clone(),
            lines_count: self.lines_count,
            chunk_bytes: self.chunk_bytes,
            pattern_matched: self.pattern_matched,
            pattern_metadata: self
                .pattern_metadata
                .as_ref()
                .map(|meta| ExecPatternMatchPayload {
                    pattern: meta.pattern.clone(),
                    matched_line_no: meta.matched_line_no,
                    matched_text: meta.matched_text.clone(),
                }),
            tail_label: self.tail_label.clone(),
            actions_summary: (!self.actions_summary.is_empty())
                .then(|| self.actions_summary.clone()),
            incremental: self.incremental,
            from_line: self.from_line,
            to_line: self.to_line,
            total_lines: self.total_lines,
            compressed: self.compressed,
            original_line_count: self.original_line_count,
        };

        #[allow(clippy::expect_used)]
        serde_json::to_string(&payload).expect("serialize ExecCommandPayload")
    }
}

#[derive(Debug)]
pub enum ExitStatus {
    Exited(i32),
    Ongoing(SessionId),
}

#[derive(Debug)]
struct SessionRegistry {
    next_session_id: AtomicU32,
    sessions: Mutex<HashMap<SessionId, Arc<ManagedSession>>>,
    events: broadcast::Sender<ExecSessionEvent>,
    archived_events: Mutex<HashMap<SessionId, ArchivedSessionEvents>>,
}

impl SessionRegistry {
    async fn prune_finished(&self) {
        let mut sessions = self.sessions.lock().await;
        let now = Instant::now();
        let mut removed = Vec::new();
        sessions.retain(|session_id, session| {
            if session.prunable(now) {
                removed.push((*session_id, Arc::clone(session)));
                false
            } else {
                true
            }
        });
        drop(sessions);

        let mut removed_logs = Vec::new();
        for (session_id, session) in removed {
            let events = {
                let guard = session.events.lock().await;
                guard.clone()
            };
            if !events.is_empty() {
                removed_logs.push((session_id, events));
            }
        }

        let ttl = session_event_ttl_duration();
        let mut archive = self.archived_events.lock().await;
        archive.retain(|_, entry| entry.expires_at > Instant::now());
        for (session_id, events) in removed_logs {
            archive.insert(
                session_id,
                ArchivedSessionEvents {
                    events,
                    expires_at: Instant::now() + ttl,
                },
            );
        }
    }

    fn subscribe_events(&self) -> broadcast::Receiver<ExecSessionEvent> {
        self.events.subscribe()
    }

    fn emit_event(&self, event: ExecSessionEvent) {
        let _ = self.events.send(event);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionLifecycle {
    Running,
    Grace,
    Terminated,
}

impl fmt::Display for SessionLifecycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionLifecycle::Running => write!(f, "running"),
            SessionLifecycle::Grace => write!(f, "grace"),
            SessionLifecycle::Terminated => write!(f, "terminated"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecSessionSummary {
    pub session_id: SessionId,
    pub command_preview: String,
    pub state: SessionLifecycle,
    pub uptime_ms: u128,
    pub idle_remaining_ms: Option<u128>,
    pub total_output_bytes: u64,
    pub log_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventKind {
    StopPatternMatched,
    WatcherMatched,
    CtrlCSent,
    ForceKill,
    IdleWarning,
    IdleTimeout,
    Keepalive,
    IdleTimeoutUpdated,
    TerminateRequested,
    OutputTrimmed,
    PatternTailLabeled,
    EscalationSummary,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventSource {
    StopPattern,
    Watcher,
    IdleWatchdog,
    ControlAction,
    System,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventAction {
    Log,
    SendCtrlC,
    ForceKill,
    TrimOutput,
    Summary,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionPatternMatch {
    pub pattern: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_line_no: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_text: Option<String>,
}

#[derive(Debug, Clone)]
struct SessionEventRecord {
    id: u64,
    at: SystemTime,
    kind: SessionEventKind,
    source: SessionEventSource,
    reason: Option<String>,
    action: Option<SessionEventAction>,
    pattern: Option<SessionPatternMatch>,
    escalate_from: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionEventEntry {
    pub id: u64,
    pub timestamp: String,
    pub event: SessionEventKind,
    pub source: SessionEventSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<SessionEventAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<SessionPatternMatch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub escalate_from: Option<u64>,
}

impl SessionEventRecord {
    fn is_expired(&self, now: SystemTime) -> bool {
        now.duration_since(self.at)
            .map(|dur| dur > SESSION_EVENT_TTL)
            .unwrap_or(false)
    }

    fn to_entry(&self) -> SessionEventEntry {
        SessionEventEntry {
            id: self.id,
            timestamp: format_timestamp(self.at),
            event: self.kind,
            source: self.source,
            reason: self.reason.clone(),
            action: self.action,
            pattern: self.pattern.clone(),
            escalate_from: self.escalate_from,
        }
    }
}

#[derive(Debug, Clone)]
struct ArchivedSessionEvents {
    events: VecDeque<SessionEventRecord>,
    expires_at: Instant,
}

impl ArchivedSessionEvents {
    fn entries(&self, since_id: Option<u64>, limit: Option<usize>) -> Vec<SessionEventEntry> {
        self.events
            .iter()
            .filter(|entry| since_id.is_none_or(|id| entry.id > id))
            .rev()
            .take(limit.unwrap_or(SESSION_EVENT_MAX))
            .map(SessionEventRecord::to_entry)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }
}

fn format_timestamp(ts: SystemTime) -> String {
    let datetime: DateTime<Utc> = ts.into();
    datetime.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn clamp_metadata_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        let clipped: String = text.chars().take(max_chars).collect();
        format!("{clipped}…")
    }
}

#[derive(Debug, Clone)]
struct PatternSegment {
    line_no: Option<u64>,
    text: String,
    is_partial: bool,
}

/// Result of smart compression operation.
#[derive(Debug)]
struct CompressionResult {
    /// Compressed output lines (or original if no compression applied)
    lines: Vec<String>,
    /// Whether compression was applied
    was_compressed: bool,
    /// Original line count before compression
    original_count: usize,
    /// Optional operator guidance (e.g. recommended stop patterns)
    guidance: Option<String>,
}

#[derive(Debug)]
struct ExecPatternWatch {
    regex: Regex,
    pattern: String,
    action: ExecWatchAction,
    persist: bool,
    cooldown: Option<Duration>,
    last_fired: Option<Instant>,
    auto_send_ctrl_c: bool,
}

/// Detects if lines contain sequential numbers (1, 2, 3, ...).
/// Uses sampling for efficiency: checks first 20, middle 10, and last 20 lines.
/// Requires at least 10 lines for reliable detection.
fn is_sequential_numbers(lines: &[String]) -> bool {
    if lines.len() < 10 {
        return false;
    }

    // For small sets, check all lines
    if lines.len() <= 50 {
        let mut prev: Option<i64> = None;
        for line in lines {
            let trimmed = line.trim();
            if let Ok(num) = trimmed.parse::<i64>() {
                if let Some(p) = prev
                    && num != p + 1
                {
                    return false;
                }
                prev = Some(num);
            } else {
                return false;
            }
        }
        return true;
    }

    // For large sets (>50 lines), use sampling to avoid O(n) overhead
    let sample_count = 50.min(lines.len());
    let head_count = 20.min(lines.len());
    let tail_count = 20.min(lines.len());
    let mid_count = sample_count.saturating_sub(head_count + tail_count);
    let mid_start = (lines.len() - tail_count) / 2;

    let mut sample_indices = Vec::with_capacity(sample_count);
    // First 20
    sample_indices.extend(0..head_count);
    // Middle 10
    sample_indices.extend(mid_start..mid_start + mid_count);
    // Last 20
    sample_indices.extend(lines.len() - tail_count..lines.len());

    let mut prev: Option<(i64, usize)> = None;
    for &idx in &sample_indices {
        let trimmed = lines[idx].trim();
        if let Ok(num) = trimmed.parse::<i64>() {
            if let Some((prev_num, prev_idx)) = prev {
                // Check if numbers increment by expected delta
                let expected = num - prev_num;
                let actual = (idx - prev_idx) as i64;
                if expected != actual {
                    return false;
                }
            }
            prev = Some((num, idx));
        } else {
            return false;
        }
    }
    true
}

/// Compresses sequential numbers into 3-line summary.
/// Format: first line, summary line, last line.
/// Preserves original formatting (no trim) to maintain user intent.
fn compress_sequential(lines: Vec<String>) -> CompressionResult {
    if lines.is_empty() {
        return CompressionResult {
            lines: Vec::new(),
            was_compressed: false,
            original_count: 0,
            guidance: None,
        };
    }

    let first = &lines[0];
    let last = &lines[lines.len() - 1];
    let compressed = vec![
        lines[0].clone(),
        format!(
            "[... {} lines: {} to {} (incrementing numbers) ...]",
            lines.len().saturating_sub(2),
            first.trim(),
            last.trim()
        ),
        lines[lines.len() - 1].clone(),
    ];
    let example_target = last.trim();
    let guidance = format!(
        "Sequential numeric output detected; use stop_pattern=\"^{example_target}$\" (adjust target) or tail_lines=20 for a rolling tail."
    );

    CompressionResult {
        lines: compressed,
        was_compressed: true,
        original_count: lines.len(),
        guidance: Some(guidance),
    }
}

/// Applies smart compression to output lines.
///
/// Strategies:
/// 1. Sequential numbers (1,2,3...) → 3-line summary
/// 2. Very large outputs (>1000 lines) → first 5 + last 5 with summary
///
/// Compression triggers at COMPRESSION_MIN_LINES+ to balance token savings vs overhead.
fn compress_output(lines: Vec<String>, enable: bool) -> CompressionResult {
    let original_count = lines.len();

    if !enable || lines.len() < COMPRESSION_MIN_LINES {
        return CompressionResult {
            lines,
            was_compressed: false,
            original_count,
            guidance: None,
        };
    }

    // Strategy 1: Sequential numbers
    if is_sequential_numbers(&lines) {
        return compress_sequential(lines);
    }

    // Strategy 2: Sampling for very large outputs (fallback)
    if lines.len() > 1000 {
        let mut sampled = Vec::new();
        sampled.extend_from_slice(&lines[..5]); // First 5
        sampled.push(format!("[... {} lines omitted ...]", lines.len() - 10));
        sampled.extend_from_slice(&lines[lines.len() - 5..]); // Last 5

        return CompressionResult {
            lines: sampled,
            was_compressed: true,
            original_count,
            guidance: None,
        };
    }

    // No compression applied
    CompressionResult {
        lines,
        was_compressed: false,
        original_count,
        guidance: None,
    }
}

#[derive(Debug)]
struct ManagedSession {
    session_id: SessionId,
    command: CommandLine,
    session: ExecCommandSession,
    writer_tx: mpsc::Sender<Vec<u8>>,
    created_at: Instant,
    last_activity: Mutex<Instant>,
    idle_timeout: Mutex<Duration>,
    hard_deadline: Mutex<Option<Instant>>,
    grace_period: Duration,
    state: AtomicU8,
    termination: Mutex<Option<TerminationRecord>>,
    log: Mutex<LogDescriptor>,
    output_bytes: AtomicU64,
    output_buffer: Mutex<OutputBuffer>,
    line_buffer: Mutex<LineBuffer>,
    output_notify: Notify,
    last_delivered_seq: Mutex<u64>,
    output_overflowed: AtomicBool,
    watchers: Mutex<Vec<JoinHandle<()>>>,
    /// Line-based cursor for efficient agent queries (tracks last read line)
    agent_read_line: AtomicU64,
    /// Tracks length of last delivered partial line (bytes) for incremental polling
    agent_partial_len: AtomicU64,
    agent_partial_pending: AtomicBool,
    /// Snapshot of the partial line at exec start to ensure the first poll
    /// returns the initial visible fragment deterministically.
    agent_partial_snapshot: Mutex<Option<String>>,
    agent_default_max_tokens: AtomicU64,
    agent_cap_reduced: AtomicBool,
    idle_warning_sent: AtomicBool,
    stop_pattern_triggered: AtomicBool,
    /// Runtime pattern watchers configured via exec_control
    pattern_watchers: Mutex<Vec<ExecPatternWatch>>,
    events: Mutex<VecDeque<SessionEventRecord>>,
    event_seq: AtomicU64,
}

impl ManagedSession {
    async fn add_watch(
        &self,
        pattern: String,
        action: ExecWatchAction,
        persist: bool,
        cooldown_ms: Option<u64>,
        auto_send_ctrl_c: bool,
    ) -> Result<String, String> {
        let regex = Regex::new(&pattern).map_err(|err| format!("invalid watch pattern: {err}"))?;
        let cooldown_ms = cooldown_ms.or_else(|| persist.then_some(1_000));

        let mut watchers = self.pattern_watchers.lock().await;
        watchers.push(ExecPatternWatch {
            regex,
            pattern: pattern.clone(),
            action,
            persist,
            cooldown: cooldown_ms.map(|ms| Duration::from_millis(ms.clamp(100, 60_000))),
            last_fired: None,
            auto_send_ctrl_c,
        });

        Ok(format!(
            "watch registered for session #{:02} pattern `{pattern}`",
            self.session_id.0
        ))
    }

    async fn record_event(
        &self,
        kind: SessionEventKind,
        source: SessionEventSource,
        reason: Option<String>,
        action: Option<SessionEventAction>,
        pattern: Option<SessionPatternMatch>,
        escalate_from: Option<u64>,
    ) -> u64 {
        let id = self
            .event_seq
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        let record = SessionEventRecord {
            id,
            at: SystemTime::now(),
            kind,
            source,
            reason,
            action,
            pattern,
            escalate_from,
        };
        let mut guard = self.events.lock().await;
        guard.push_back(record);
        let now = SystemTime::now();
        while guard
            .front()
            .is_some_and(|entry| entry.is_expired(now) || guard.len() > SESSION_EVENT_MAX)
        {
            guard.pop_front();
        }
        id
    }

    async fn session_events(
        &self,
        since_id: Option<u64>,
        limit: Option<usize>,
    ) -> Vec<SessionEventEntry> {
        let guard = self.events.lock().await;
        guard
            .iter()
            .filter(|entry| since_id.is_none_or(|id| entry.id > id))
            .rev()
            .take(limit.unwrap_or(SESSION_EVENT_MAX))
            .map(SessionEventRecord::to_entry)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    async fn record_escalation_summary(&self, reason: String, escalate_from: Option<u64>) {
        let _ = self
            .record_event(
                SessionEventKind::EscalationSummary,
                SessionEventSource::System,
                Some(reason),
                Some(SessionEventAction::Summary),
                None,
                escalate_from,
            )
            .await;
    }

    async fn remove_watch(&self, pattern: &str) -> bool {
        let mut watchers = self.pattern_watchers.lock().await;
        let before = watchers.len();
        watchers.retain(|watch| watch.pattern != pattern);
        before != watchers.len()
    }

    async fn idle_remaining(&self) -> Option<Duration> {
        let timeout = *self.idle_timeout.lock().await;
        let last = *self.last_activity.lock().await;
        Some(timeout.saturating_sub(Instant::now().saturating_duration_since(last)))
    }

    async fn apply_watchers(
        &self,
        segments: &[PatternSegment],
        base_line: u64,
        total_lines: u64,
        actions_summary: &mut Vec<String>,
    ) -> Option<String> {
        if segments.is_empty() {
            return None;
        }

        let mut watchers_guard = self.pattern_watchers.lock().await;
        let now = Instant::now();
        let mut matched = Vec::new();
        let mut to_remove = Vec::new();

        for (idx, watch) in watchers_guard.iter_mut().enumerate() {
            if let Some((line_idx, segment)) = segments
                .iter()
                .enumerate()
                .find(|(_, segment)| watch.regex.is_match(&segment.text))
            {
                if let Some(cooldown) = watch.cooldown
                    && let Some(last) = watch.last_fired
                    && now.saturating_duration_since(last) < cooldown
                {
                    continue;
                }
                watch.last_fired = Some(now);
                let line_no = segment.line_no.or_else(|| {
                    if segment.is_partial {
                        Some(total_lines)
                    } else {
                        base_line.checked_add(line_idx as u64)
                    }
                });
                matched.push((
                    idx,
                    watch.action.clone(),
                    watch.pattern.clone(),
                    line_no,
                    segment.text.clone(),
                    watch.persist,
                    watch.auto_send_ctrl_c,
                ));
            }
        }

        if matched.is_empty() {
            return None;
        }

        for (idx, _, _, _, _, persist, _) in &matched {
            if !*persist {
                to_remove.push(*idx);
            }
        }

        for idx in to_remove.into_iter().rev() {
            watchers_guard.remove(idx);
        }
        drop(watchers_guard);

        let mut notes = Vec::new();
        for (_, action, pattern, line_no, line_text, _, auto_send_ctrl_c) in matched {
            let match_info = SessionPatternMatch {
                pattern: pattern.clone(),
                matched_line_no: line_no,
                matched_text: Some(line_text.clone()),
            };
            let reason = Some(format!("watch matched `{pattern}`"));
            let base_event = self
                .record_event(
                    SessionEventKind::WatcherMatched,
                    SessionEventSource::Watcher,
                    reason,
                    Some(SessionEventAction::Log),
                    Some(match_info.clone()),
                    None,
                )
                .await;

            match action {
                ExecWatchAction::Log => {
                    notes.push(format!("watch matched `{pattern}`"));
                    actions_summary.push(format!("watch `{pattern}` matched"));
                    if auto_send_ctrl_c {
                        let _ = self.send_ctrl_c().await;
                        notes.push(format!("watch `{pattern}` auto sent Ctrl-C"));
                        actions_summary.push(format!("watch `{pattern}` auto ctrl-c"));
                        self.record_event(
                            SessionEventKind::CtrlCSent,
                            SessionEventSource::Watcher,
                            Some("auto_send_ctrl_c".to_string()),
                            Some(SessionEventAction::SendCtrlC),
                            Some(match_info.clone()),
                            Some(base_event),
                        )
                        .await;
                    }
                }
                ExecWatchAction::SendCtrlC => {
                    let _ = self.send_ctrl_c().await;
                    notes.push(format!("watch `{pattern}` sent Ctrl-C"));
                    actions_summary.push(format!("watch `{pattern}` ctrl-c"));
                    self.record_event(
                        SessionEventKind::CtrlCSent,
                        SessionEventSource::Watcher,
                        Some(format!("pattern `{pattern}`")),
                        Some(SessionEventAction::SendCtrlC),
                        Some(match_info.clone()),
                        Some(base_event),
                    )
                    .await;
                }
                ExecWatchAction::ForceKill => {
                    match self.force_kill(TerminationReason::ForceKilled).await {
                        Ok(()) => {
                            notes.push(format!("watch `{pattern}` force-killed session"));
                            actions_summary.push(format!("watch `{pattern}` force_kill"));
                            let force_event = self
                                .record_event(
                                    SessionEventKind::ForceKill,
                                    SessionEventSource::Watcher,
                                    Some(format!("pattern `{pattern}`")),
                                    Some(SessionEventAction::ForceKill),
                                    Some(match_info),
                                    Some(base_event),
                                )
                                .await;
                            self.record_escalation_summary(
                                format!("watch `{pattern}` escalation"),
                                Some(force_event),
                            )
                            .await;
                        }
                        Err(err) => {
                            notes.push(format!("watch `{pattern}` force-kill failed: {err}"));
                            self.record_event(
                                SessionEventKind::WatcherMatched,
                                SessionEventSource::Watcher,
                                Some(format!("force_kill_failed: pattern `{pattern}` err={err}")),
                                None,
                                Some(match_info),
                                Some(base_event),
                            )
                            .await;
                            actions_summary.push(format!("watch `{pattern}` force_kill failed"));
                        }
                    }
                }
            }
        }

        if notes.is_empty() {
            None
        } else {
            Some(notes.join(" | "))
        }
    }

    fn new(
        session_id: SessionId,
        command: CommandLine,
        session: ExecCommandSession,
        idle_timeout: Duration,
        hard_deadline: Option<Duration>,
        grace_period: Duration,
        log_threshold: usize,
    ) -> Self {
        let now = Instant::now();
        let writer_tx = session.writer_sender();
        let hard_deadline_instant = hard_deadline.map(|d| now + d);
        Self {
            session_id,
            command,
            session,
            writer_tx,
            created_at: now,
            last_activity: Mutex::new(now),
            idle_timeout: Mutex::new(idle_timeout),
            hard_deadline: Mutex::new(hard_deadline_instant),
            grace_period,
            state: AtomicU8::new(SessionState::RUNNING),
            termination: Mutex::new(None),
            log: Mutex::new(LogDescriptor::new(log_threshold, session_id)),
            output_bytes: AtomicU64::new(0),
            output_buffer: Mutex::new(OutputBuffer::new(OUTPUT_RETENTION_BYTES)),
            line_buffer: Mutex::new(LineBuffer::new(LINE_RETENTION_COUNT)),
            output_notify: Notify::new(),
            last_delivered_seq: Mutex::new(0),
            output_overflowed: AtomicBool::new(false),
            watchers: Mutex::new(Vec::new()),
            agent_read_line: AtomicU64::new(0),
            agent_partial_len: AtomicU64::new(0),
            agent_partial_pending: AtomicBool::new(false),
            agent_partial_snapshot: Mutex::new(None),
            agent_default_max_tokens: AtomicU64::new(DEFAULT_AUTO_POLL_CAP_TOKENS),
            agent_cap_reduced: AtomicBool::new(false),
            idle_warning_sent: AtomicBool::new(false),
            stop_pattern_triggered: AtomicBool::new(false),
            pattern_watchers: Mutex::new(Vec::new()),
            events: Mutex::new(VecDeque::new()),
            event_seq: AtomicU64::new(0),
        }
    }

    async fn descriptor(&self, recent_lines: usize, recent_bytes: usize) -> ExecSessionDescriptor {
        let command_preview = preview_command(self.command.preview(), 80);
        let state = self.state();
        let uptime = Instant::now().saturating_duration_since(self.created_at);
        let idle_remaining = self.idle_timeout.try_lock().ok().and_then(|timeout| {
            self.last_activity
                .try_lock()
                .ok()
                .map(|last| timeout.saturating_sub(Instant::now().saturating_duration_since(*last)))
        });
        let total_output_bytes = self.output_bytes.load(Ordering::SeqCst);
        let log_path = self
            .log
            .try_lock()
            .ok()
            .and_then(|log| log.snapshot().log_path);
        let recent_output = self.recent_output_lines(recent_lines, recent_bytes).await;
        let note = self.termination_note().await;
        let lossy = self.output_overflowed.load(Ordering::SeqCst);

        ExecSessionDescriptor {
            session_id: self.session_id,
            command_preview,
            state,
            uptime,
            idle_remaining,
            total_output_bytes,
            log_path,
            recent_output,
            note,
            lossy,
        }
    }

    async fn recent_output_lines(&self, max_lines: usize, max_bytes: usize) -> Vec<String> {
        if max_lines == 0 || max_bytes == 0 {
            return Vec::new();
        }
        let buffer = self.output_buffer.lock().await;
        buffer.recent_lines(max_lines, max_bytes)
    }

    async fn start_supervision(
        self: &Arc<Self>,
        registry: Arc<SessionRegistry>,
        mut output_rx: tokio::sync::broadcast::Receiver<Vec<u8>>,
        exit_rx: Shared<oneshot::Receiver<i32>>,
    ) {
        let mut handles = self.watchers.lock().await;

        // Output sentinel — keeps sessions active and buffers stdout/stderr.
        let sentinel_session = Arc::clone(self);
        let sentinel_registry = Arc::clone(&registry);
        handles.push(tokio::spawn(async move {
            loop {
                if sentinel_session.is_terminated() {
                    break;
                }
                match output_rx.recv().await {
                    Ok(chunk) => {
                        sentinel_session.record_activity().await;
                        if let Err(err) = sentinel_session.append_log(&chunk).await {
                            tracing::error!("failed to append log: {err}");
                        }
                        sentinel_session.increment_output_bytes(chunk.len() as u64);
                        sentinel_session.push_output_chunk(chunk).await;
                        emit_session_event(
                            &sentinel_registry,
                            &sentinel_session,
                            ExecSessionEventKind::Updated,
                        )
                        .await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        sentinel_session
                            .output_overflowed
                            .store(true, Ordering::SeqCst);
                        emit_session_event(
                            &sentinel_registry,
                            &sentinel_session,
                            ExecSessionEventKind::Updated,
                        )
                        .await;
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
            sentinel_session.output_notify.notify_waiters();
        }));

        // Idle watchdog.
        let idle_session = Arc::clone(self);
        let idle_registry = Arc::clone(&registry);
        handles.push(tokio::spawn(async move {
            loop {
                if idle_session.is_terminated() {
                    break;
                }
                let idle_timeout = idle_session.current_idle_timeout().await;
                let since_activity = idle_session.since_last_activity().await;
                if since_activity >= idle_timeout {
                    let timeout_ms = idle_timeout.as_millis() as u64;
                    let timeout_event = idle_session
                        .record_event(
                            SessionEventKind::IdleTimeout,
                            SessionEventSource::IdleWatchdog,
                            Some(format!("timeout_ms={timeout_ms}")),
                            Some(SessionEventAction::Log),
                            None,
                            None,
                        )
                        .await;
                    idle_session
                        .mark_grace(TerminationReason::IdleTimeout { idle_timeout })
                        .await;
                    emit_session_event(
                        &idle_registry,
                        &idle_session,
                        ExecSessionEventKind::Updated,
                    )
                    .await;
                    if idle_session.send_ctrl_c().await {
                        idle_session
                            .record_event(
                                SessionEventKind::CtrlCSent,
                                SessionEventSource::IdleWatchdog,
                                Some("idle_timeout_ctrl_c".to_string()),
                                Some(SessionEventAction::SendCtrlC),
                                None,
                                Some(timeout_event),
                            )
                            .await;
                    }
                    sleep(idle_session.grace_period).await;
                    if !idle_session.is_terminated() {
                        match idle_session
                            .force_kill(TerminationReason::IdleTimeout { idle_timeout })
                            .await
                        {
                            Ok(()) => {
                                let force_event = idle_session
                                    .record_event(
                                        SessionEventKind::ForceKill,
                                        SessionEventSource::IdleWatchdog,
                                        Some("force_kill_idle_timeout".to_string()),
                                        Some(SessionEventAction::ForceKill),
                                        None,
                                        Some(timeout_event),
                                    )
                                    .await;
                                idle_session
                                    .record_escalation_summary(
                                        "idle_timeout escalation".to_string(),
                                        Some(force_event),
                                    )
                                    .await;
                                emit_session_event(
                                    &idle_registry,
                                    &idle_session,
                                    ExecSessionEventKind::Terminated,
                                )
                                .await;
                            }
                            Err(err) => {
                                tracing::warn!(
                                    session = idle_session.session_id.0,
                                    "failed to force kill idle session: {err}"
                                );
                                emit_session_event(
                                    &idle_registry,
                                    &idle_session,
                                    ExecSessionEventKind::Updated,
                                )
                                .await;
                            }
                        }
                    }
                    idle_registry.prune_finished().await;
                    break;
                }
                sleep(IDLE_WATCH_INTERVAL).await;
            }
        }));

        // Hard deadline watchdog.
        if let Some(deadline) = *self.hard_deadline.lock().await {
            let hard_session = Arc::clone(self);
            let hard_registry = Arc::clone(&registry);
            handles.push(tokio::spawn(async move {
                let now = Instant::now();
                if deadline > now {
                    sleep(deadline - now).await;
                }
                if hard_session.is_terminated() {
                    return;
                }
                hard_session
                    .mark_grace(TerminationReason::HardTimeout)
                    .await;
                emit_session_event(&hard_registry, &hard_session, ExecSessionEventKind::Updated)
                    .await;
                let _ = hard_session.send_ctrl_c().await;
                sleep(hard_session.grace_period).await;
                if !hard_session.is_terminated() {
                    match hard_session
                        .force_kill(TerminationReason::HardTimeout)
                        .await
                    {
                        Ok(()) => {
                            emit_session_event(
                                &hard_registry,
                                &hard_session,
                                ExecSessionEventKind::Terminated,
                            )
                            .await;
                        }
                        Err(err) => {
                            tracing::warn!(
                                session = hard_session.session_id.0,
                                "failed to force kill hard-timeout session: {err}"
                            );
                            emit_session_event(
                                &hard_registry,
                                &hard_session,
                                ExecSessionEventKind::Updated,
                            )
                            .await;
                        }
                    }
                }
                hard_registry.prune_finished().await;
            }));
        }

        // Exit watcher.
        let exit_session = Arc::clone(self);
        let exit_registry = Arc::clone(&registry);
        handles.push(tokio::spawn(async move {
            match exit_rx.await {
                Ok(code) => {
                    if !exit_session.is_terminated() {
                        exit_session
                            .mark_terminated(TerminationReason::Completed { exit_code: code })
                            .await;
                        emit_session_event(
                            &exit_registry,
                            &exit_session,
                            ExecSessionEventKind::Terminated,
                        )
                        .await;
                    }
                }
                Err(_) => {
                    if !exit_session.is_terminated() {
                        exit_session
                            .mark_terminated(TerminationReason::ForceKilled)
                            .await;
                        emit_session_event(
                            &exit_registry,
                            &exit_session,
                            ExecSessionEventKind::Terminated,
                        )
                        .await;
                    }
                }
            }
            exit_session.output_notify.notify_waiters();
            exit_registry.prune_finished().await;
        }));
    }

    async fn record_activity(&self) {
        let mut guard = self.last_activity.lock().await;
        *guard = Instant::now();
    }

    async fn since_last_activity(&self) -> Duration {
        let guard = self.last_activity.lock().await;
        Instant::now().saturating_duration_since(*guard)
    }

    async fn current_idle_timeout(&self) -> Duration {
        *self.idle_timeout.lock().await
    }

    async fn log_snapshot(&self) -> LogSnapshot {
        self.log.lock().await.snapshot()
    }

    async fn append_log(&self, chunk: &[u8]) -> std::io::Result<()> {
        self.log.lock().await.append(chunk).await
    }

    fn increment_output_bytes(&self, delta: u64) {
        self.output_bytes.fetch_add(delta, Ordering::SeqCst);
    }

    async fn completed_exit_code(&self) -> Option<i32> {
        let guard = self.termination.lock().await;
        guard.as_ref().and_then(|record| match record.reason {
            TerminationReason::Completed { exit_code } => Some(exit_code),
            _ => None,
        })
    }

    async fn push_output_chunk(&self, chunk: Vec<u8>) {
        let overflowed = {
            let mut buffer = self.output_buffer.lock().await;
            buffer.push(chunk.clone())
        };

        // Also push to line buffer for efficient line-based queries
        {
            let mut line_buffer = self.line_buffer.lock().await;
            line_buffer.push_bytes(&chunk);
        }

        if overflowed {
            self.output_overflowed.store(true, Ordering::SeqCst);
        }
        self.output_notify.notify_waiters();
    }

    async fn collect_output(&self, deadline: Instant, cap_bytes: usize) -> CollectedOutput {
        let mut aggregated = Vec::new();
        let mut lossy = false;

        loop {
            let (chunk, chunk_loss) = {
                let mut last_seq = self.last_delivered_seq.lock().await;
                let buffer = self.output_buffer.lock().await;
                let (data, loss, new_seq) = buffer.collect_since(*last_seq);
                if new_seq != *last_seq {
                    *last_seq = new_seq;
                }
                (data, loss)
            };

            if !chunk.is_empty() {
                aggregated.extend_from_slice(&chunk);
                if chunk_loss {
                    lossy = true;
                }
                if aggregated.len() > cap_bytes {
                    lossy = true;
                    break;
                }
                continue;
            }

            if chunk_loss {
                lossy = true;
            }

            if Instant::now() >= deadline {
                break;
            }

            if self.is_terminated() {
                break;
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            tokio::select! {
                _ = self.output_notify.notified() => {},
                _ = sleep(remaining) => break,
            }
        }

        if aggregated.len() > cap_bytes {
            aggregated.truncate(cap_bytes);
        }

        CollectedOutput {
            data: aggregated,
            lost: lossy,
        }
    }

    async fn mark_terminated(&self, reason: TerminationReason) {
        self.state.store(SessionState::TERMINATED, Ordering::SeqCst);
        let mut guard = self.termination.lock().await;
        *guard = Some(TerminationRecord {
            reason,
            at: Instant::now(),
        });
    }

    async fn mark_grace(&self, reason: TerminationReason) {
        let _ = self.state.compare_exchange(
            SessionState::RUNNING,
            SessionState::GRACE,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
        let mut guard = self.termination.lock().await;
        *guard = Some(TerminationRecord {
            reason,
            at: Instant::now(),
        });
    }

    async fn write_to_stdin(&self, bytes: Vec<u8>) -> Result<(), ()> {
        self.writer_tx.send(bytes).await.map_err(|_| ())
    }

    async fn keepalive(&self, extend_timeout_ms: Option<u64>) {
        self.record_activity().await;
        if let Some(ms) = extend_timeout_ms {
            let ms = clamp(ms, MIN_IDLE_TIMEOUT_MS, MAX_IDLE_TIMEOUT_MS);
            let mut guard = self.idle_timeout.lock().await;
            *guard = Duration::from_millis(ms);
        }
    }

    async fn set_idle_timeout(&self, timeout_ms: u64) {
        let ms = clamp(timeout_ms, MIN_IDLE_TIMEOUT_MS, MAX_IDLE_TIMEOUT_MS);
        let mut guard = self.idle_timeout.lock().await;
        *guard = Duration::from_millis(ms);
    }

    async fn send_ctrl_c(&self) -> bool {
        self.writer_tx.send(vec![0x03]).await.is_ok()
    }

    async fn force_kill(&self, reason: TerminationReason) -> Result<(), String> {
        if self.is_terminated() {
            return Ok(());
        }
        self.session.force_kill()?;
        self.mark_terminated(reason).await;
        self.output_notify.notify_waiters();
        Ok(())
    }

    fn is_terminated(&self) -> bool {
        self.state.load(Ordering::SeqCst) == SessionState::TERMINATED
    }

    fn summary(&self) -> ExecSessionSummary {
        let state = self.state();
        let uptime = Instant::now().saturating_duration_since(self.created_at);
        let idle_remaining = self.idle_timeout.try_lock().ok().and_then(|timeout| {
            self.last_activity
                .try_lock()
                .ok()
                .map(|last| timeout.saturating_sub(Instant::now().saturating_duration_since(*last)))
        });
        ExecSessionSummary {
            session_id: self.session_id,
            command_preview: preview_command(self.command.preview(), 80),
            state,
            uptime_ms: uptime.as_millis(),
            idle_remaining_ms: idle_remaining.map(|d| d.as_millis()),
            total_output_bytes: self.output_bytes.load(Ordering::SeqCst),
            log_path: self
                .log
                .try_lock()
                .ok()
                .and_then(|log| log.snapshot().log_path),
        }
    }

    fn state(&self) -> SessionLifecycle {
        match self.state.load(Ordering::SeqCst) {
            SessionState::RUNNING => SessionLifecycle::Running,
            SessionState::GRACE => SessionLifecycle::Grace,
            _ => SessionLifecycle::Terminated,
        }
    }

    async fn termination_note(&self) -> Option<String> {
        let record = self.termination.lock().await;
        record.as_ref().map(|r| r.reason.to_string())
    }

    fn prunable(&self, now: Instant) -> bool {
        if let Ok(record) = self.termination.try_lock()
            && let Some(record) = &*record
        {
            return now
                .checked_duration_since(record.at)
                .map(|dur| dur.as_millis() as u64 > PRUNE_AFTER_MS)
                .unwrap_or(false);
        }
        false
    }
}

#[derive(Debug, Clone)]
struct OutputChunk {
    seq: u64,
    data: Vec<u8>,
}

/// Safely truncates a string at a valid UTF-8 character boundary.
/// Finds the largest valid position <= max_bytes and truncates there.
fn truncate_utf8_safe(s: &mut String, max_bytes: usize, marker: &str) {
    if s.len() <= max_bytes {
        return;
    }

    let mut truncate_pos = max_bytes;
    while truncate_pos > 0 && !s.is_char_boundary(truncate_pos) {
        truncate_pos -= 1;
    }

    s.truncate(truncate_pos);
    s.push_str(marker);
}

fn drop_utf8_prefix(s: &mut String, max_bytes: usize) {
    if max_bytes == 0 || s.is_empty() {
        return;
    }

    let mut prefix = max_bytes.min(s.len());
    while prefix > 0 && !s.is_char_boundary(prefix) {
        prefix -= 1;
    }

    if prefix > 0 {
        s.drain(..prefix);
    }
}

/// Line-based output buffer for efficient agent access.
/// Stores output as individual lines instead of byte chunks.
#[derive(Debug)]
struct LineBuffer {
    lines: VecDeque<String>,
    max_lines: usize,
    total_lines_written: u64,
    /// Partial line buffer for chunks that don't end with newline
    partial_line: String,
    /// Maximum line length to prevent memory exhaustion (256 KB)
    max_line_bytes: usize,
    /// Tracks if any lossy UTF-8 conversion occurred (invalid bytes replaced with �)
    lossy_utf8: bool,
}

impl LineBuffer {
    fn new(max_lines: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            max_lines,
            total_lines_written: 0,
            partial_line: String::new(),
            max_line_bytes: 256 * 1024, // 256 KB per line (10k × 256KB = 2.5GB max theoretical)
            lossy_utf8: false,
        }
    }

    /// Returns true if any lossy UTF-8 conversion occurred (invalid bytes → �)
    fn has_lossy_utf8(&self) -> bool {
        self.lossy_utf8
    }

    /// Add new lines from raw byte output.
    /// Handles partial lines across chunk boundaries.
    /// Returns true if ring buffer overflowed (lines evicted).
    fn push_bytes(&mut self, data: &[u8]) -> bool {
        let text = String::from_utf8_lossy(data);
        let text_str = text.as_ref();

        // Track lossy UTF-8 conversion (invalid bytes → U+FFFD)
        if text_str.contains('\u{FFFD}') {
            self.lossy_utf8 = true;
        }

        // Early exit for empty input
        if text_str.is_empty() {
            return false;
        }

        // Check if input ends with newline
        let ends_with_newline = text_str.ends_with('\n') || text_str.ends_with("\r\n");

        // Split into lines
        let mut lines: Vec<&str> = text_str.lines().collect();
        let mut overflow = false;

        // If doesn't end with newline, last line is partial
        let (complete_lines, partial) = if ends_with_newline {
            (lines.as_slice(), None)
        } else {
            let last = lines.pop();
            (lines.as_slice(), last)
        };

        // Process complete lines (first one merges with existing partial_line)
        for (i, line) in complete_lines.iter().enumerate() {
            let should_add_line = !line.is_empty() || i > 0 || !self.partial_line.is_empty();

            if i == 0 && !self.partial_line.is_empty() {
                // Merge with existing partial line
                self.partial_line.push_str(line);

                // Truncate if too long (safe UTF-8 boundary handling)
                truncate_utf8_safe(
                    &mut self.partial_line,
                    self.max_line_bytes,
                    "...[truncated]",
                );

                self.lines.push_back(std::mem::take(&mut self.partial_line));
                self.total_lines_written += 1;

                if self.lines.len() > self.max_lines {
                    self.lines.pop_front();
                    overflow = true;
                }
            } else if should_add_line {
                // Skip lone empty lines when partial_line is also empty
                // (e.g., "\n" on empty partial_line should not create empty line)
                let mut line_str = line.to_string();

                // Truncate if too long (safe UTF-8 boundary handling)
                truncate_utf8_safe(&mut line_str, self.max_line_bytes, "...[truncated]");

                self.lines.push_back(line_str);
                self.total_lines_written += 1;

                if self.lines.len() > self.max_lines {
                    self.lines.pop_front();
                    overflow = true;
                }
            }
        }

        // Handle partial line at end
        if let Some(partial_str) = partial {
            self.partial_line.push_str(partial_str);

            // Check if partial_line exceeds limit after appending
            if self.partial_line.len() > self.max_line_bytes {
                // Partial line too long, flush it as truncated (safe UTF-8 boundary handling)
                truncate_utf8_safe(
                    &mut self.partial_line,
                    self.max_line_bytes,
                    "...[truncated]",
                );
                self.lines.push_back(std::mem::take(&mut self.partial_line));
                self.total_lines_written += 1;

                if self.lines.len() > self.max_lines {
                    self.lines.pop_front();
                    overflow = true;
                }
            }
        }

        overflow
    }

    /// Get total number of lines written (including evicted)
    fn total_lines(&self) -> u64 {
        self.total_lines_written
    }

    /// Get lines in range [from, to). Returns available lines within buffer.
    fn get_lines(&self, from_line: u64, to_line: u64) -> Vec<String> {
        let total = self.total_lines_written;
        let buffer_start = total.saturating_sub(self.lines.len() as u64);

        // Adjust range to buffer boundaries
        let start_idx = if from_line < buffer_start {
            0
        } else {
            (from_line - buffer_start) as usize
        };

        let end_idx = if to_line < buffer_start {
            0
        } else if to_line >= total {
            self.lines.len()
        } else {
            (to_line - buffer_start) as usize
        };

        if start_idx >= self.lines.len() || end_idx <= start_idx {
            return Vec::new();
        }

        self.lines
            .iter()
            .skip(start_idx)
            .take(end_idx - start_idx)
            .cloned()
            .collect()
    }

    fn partial_tail(&self) -> Option<String> {
        if self.partial_line.is_empty() {
            None
        } else {
            Some(self.partial_line.clone())
        }
    }
}

#[derive(Debug)]
struct OutputBuffer {
    chunks: VecDeque<OutputChunk>,
    retention_bytes: usize,
    current_bytes: usize,
    next_seq: u64,
}

#[derive(Debug, Default)]
struct CollectedOutput {
    data: Vec<u8>,
    lost: bool,
}

impl OutputBuffer {
    fn new(retention_bytes: usize) -> Self {
        Self {
            chunks: VecDeque::new(),
            retention_bytes,
            current_bytes: 0,
            next_seq: 1,
        }
    }

    fn push(&mut self, chunk: Vec<u8>) -> bool {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        self.current_bytes = self.current_bytes.saturating_add(chunk.len());
        self.chunks.push_back(OutputChunk { seq, data: chunk });

        let mut overflow = false;
        while self.current_bytes > self.retention_bytes {
            if let Some(front) = self.chunks.pop_front() {
                self.current_bytes = self.current_bytes.saturating_sub(front.data.len());
                overflow = true;
            } else {
                break;
            }
        }
        overflow
    }

    fn collect_since(&self, last_seq: u64) -> (Vec<u8>, bool, u64) {
        if self.chunks.is_empty() {
            return (Vec::new(), false, last_seq);
        }

        let mut data = Vec::new();
        let mut lossy = false;
        let mut new_seq = last_seq;
        let mut expected_next = last_seq.saturating_add(1);

        let first_seq = self
            .chunks
            .front()
            .map(|chunk| chunk.seq)
            .unwrap_or(expected_next);
        if first_seq > expected_next {
            lossy = true;
            expected_next = first_seq;
        }

        for chunk in self.chunks.iter() {
            if chunk.seq <= last_seq {
                continue;
            }
            if chunk.seq != expected_next {
                lossy = true;
            }
            data.extend_from_slice(&chunk.data);
            new_seq = chunk.seq;
            expected_next = chunk.seq.saturating_add(1);
        }

        if new_seq == last_seq {
            (Vec::new(), lossy, last_seq)
        } else {
            (data, lossy, new_seq)
        }
    }

    fn tail_bytes(&self, max_bytes: usize) -> Vec<u8> {
        if max_bytes == 0 || self.chunks.is_empty() {
            return Vec::new();
        }

        let mut remaining = max_bytes;
        let mut segments: Vec<Vec<u8>> = Vec::new();

        for chunk in self.chunks.iter().rev() {
            if remaining == 0 {
                break;
            }
            if chunk.data.len() > remaining {
                let start = chunk.data.len() - remaining;
                segments.push(chunk.data[start..].to_vec());
                break;
            } else {
                segments.push(chunk.data.clone());
                remaining = remaining.saturating_sub(chunk.data.len());
            }
        }

        segments.reverse();
        let mut buf = Vec::with_capacity(max_bytes.min(self.current_bytes));
        for seg in segments {
            buf.extend_from_slice(&seg);
        }
        buf
    }

    fn recent_lines(&self, max_lines: usize, max_bytes: usize) -> Vec<String> {
        if max_lines == 0 || max_bytes == 0 {
            return Vec::new();
        }

        let bytes = self.tail_bytes(max_bytes);
        if bytes.is_empty() {
            return Vec::new();
        }

        let text = String::from_utf8_lossy(&bytes);
        let mut lines: Vec<String> = text
            .split('\n')
            .map(|line| line.trim_end_matches('\r').to_string())
            .collect();

        if lines.len() > max_lines {
            lines = lines.split_off(lines.len() - max_lines);
        }

        lines
    }
}

#[derive(Debug, Clone)]
struct TerminationRecord {
    reason: TerminationReason,
    at: Instant,
}

#[derive(Debug, Clone)]
enum TerminationReason {
    Completed { exit_code: i32 },
    IdleTimeout { idle_timeout: Duration },
    HardTimeout,
    UserRequested,
    ForceKilled,
}

impl std::fmt::Display for TerminationReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TerminationReason::Completed { exit_code } => {
                write!(f, "completed (exit_code={exit_code})")
            }
            TerminationReason::IdleTimeout { idle_timeout } => {
                write!(f, "idle_timeout (timeout={}s)", idle_timeout.as_secs())
            }
            TerminationReason::HardTimeout => write!(f, "hard_timeout"),
            TerminationReason::UserRequested => write!(f, "user_requested"),
            TerminationReason::ForceKilled => write!(f, "force_killed"),
        }
    }
}

#[derive(Debug, Default, Clone)]
struct LogSnapshot {
    log_path: Option<PathBuf>,
    log_sha256: Option<String>,
    total_bytes: u64,
}

#[derive(Debug)]
struct LogDescriptor {
    threshold: usize,
    buffer: Vec<u8>,
    file: Option<LogFile>,
    hasher: Sha256,
    total_bytes: u64,
    session_label: String,
}

impl LogDescriptor {
    fn new(threshold: usize, session_id: SessionId) -> Self {
        Self {
            threshold,
            buffer: Vec::with_capacity(threshold),
            file: None,
            hasher: Sha256::new(),
            total_bytes: 0,
            session_label: format!("session-{:08}", session_id.0),
        }
    }

    async fn append(&mut self, chunk: &[u8]) -> std::io::Result<()> {
        self.total_bytes = self.total_bytes.saturating_add(chunk.len() as u64);
        self.hasher.update(chunk);
        if let Some(file) = &mut self.file {
            file.write(chunk).await?;
            return Ok(());
        }

        if self.buffer.len() + chunk.len() <= self.threshold {
            self.buffer.extend_from_slice(chunk);
            return Ok(());
        }

        let mut file = LogFile::create(&self.session_label).await?;
        file.write(&self.buffer).await?;
        file.write(chunk).await?;
        self.file = Some(file);
        self.buffer.clear();
        Ok(())
    }

    fn snapshot(&self) -> LogSnapshot {
        let hasher = self.hasher.clone();
        let digest = hasher.finalize();
        let mut hash_hex = String::with_capacity(digest.len() * 2);
        for byte in digest {
            use std::fmt::Write;
            let _ = write!(&mut hash_hex, "{byte:02x}");
        }
        LogSnapshot {
            log_path: self.file.as_ref().map(|file| file.path.clone()),
            log_sha256: if self.total_bytes > 0 {
                Some(hash_hex)
            } else {
                None
            },
            total_bytes: self.total_bytes,
        }
    }
}

#[derive(Debug)]
struct LogFile {
    path: PathBuf,
    file: TokioFile,
}

impl LogFile {
    async fn create(label: &str) -> std::io::Result<Self> {
        let base = log_base_dir().await?;
        let filename = format!("{label}-{}.ansi", Utc::now().format("%Y%m%dT%H%M%S%.3fZ"));
        let path = base.join(filename);
        let file = TokioFile::create(&path).await?;
        Ok(Self { path, file })
    }

    async fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.file.write_all(bytes).await
    }
}

async fn log_base_dir() -> std::io::Result<PathBuf> {
    let dir = cache_dir()
        .map(|p| p.join("codex").join("exec_logs"))
        .unwrap_or_else(|| std::env::temp_dir().join("codex-exec-logs"));
    fs::create_dir_all(&dir).await?;
    Ok(dir)
}

fn preview_command(cmd: &str, max: usize) -> String {
    if cmd.len() <= max {
        return cmd.to_string();
    }
    let keep = max / 2;
    format!("{}…{}", &cmd[..keep], &cmd[cmd.len() - keep..])
}

fn clamp(value: u64, min: u64, max: u64) -> u64 {
    value.min(max).max(min)
}

struct SessionState;
impl SessionState {
    const RUNNING: u8 = 0;
    const GRACE: u8 = 1;
    const TERMINATED: u8 = 2;
}

async fn emit_session_event(
    registry: &Arc<SessionRegistry>,
    session: &Arc<ManagedSession>,
    kind: ExecSessionEventKind,
) {
    let descriptor = session
        .descriptor(DESCRIPTOR_RECENT_LINES, DESCRIPTOR_RECENT_BYTES)
        .await;
    registry.emit_event(ExecSessionEvent { kind, descriptor });
}

fn duration_to_u64_ms(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn normalize_yield_time_ms(raw: u64) -> u64 {
    raw.clamp(MIN_YIELD_TIME_MS, MAX_YIELD_TIME_MS)
}

fn normalize_cap_bytes(max_output_tokens: u64) -> usize {
    let bytes = max_output_tokens
        .saturating_mul(4)
        .min(usize::MAX as u64)
        .max(MIN_OUTPUT_CAP_BYTES as u64);
    bytes as usize
}

async fn create_exec_command_session(
    command: &CommandLine,
    shell: String,
    login: bool,
) -> anyhow::Result<(
    ExecCommandSession,
    tokio::sync::broadcast::Receiver<Vec<u8>>,
    oneshot::Receiver<i32>,
)> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let mut command_builder = CommandBuilder::new(shell);
    let shell_mode_opt = if login { "-lc" } else { "-c" };
    command_builder.arg(shell_mode_opt);
    command_builder.arg(command.shell_command());

    let mut child = pair.slave.spawn_command(command_builder)?;
    let killer = child.clone_killer();

    let (writer_tx, mut writer_rx) = mpsc::channel::<Vec<u8>>(128);
    let (output_tx, _) = tokio::sync::broadcast::channel::<Vec<u8>>(256);

    let mut reader = pair.master.try_clone_reader()?;
    let output_tx_clone = output_tx.clone();
    let reader_handle = tokio::task::spawn_blocking(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let _ = output_tx_clone.send(buf[..n].to_vec());
                }
                Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(_) => break,
            }
        }
    });

    let writer = pair.master.take_writer()?;
    let writer = Arc::new(StdMutex::new(writer));
    let writer_handle = tokio::spawn({
        let writer = writer.clone();
        async move {
            while let Some(bytes) = writer_rx.recv().await {
                let writer = writer.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    if let Ok(mut guard) = writer.lock() {
                        use std::io::Write;
                        let _ = guard.write_all(&bytes);
                        let _ = guard.flush();
                    }
                })
                .await;
            }
        }
    });

    let (exit_tx, exit_rx) = oneshot::channel::<i32>();
    let exit_status = Arc::new(AtomicBool::new(false));
    let wait_exit_status = exit_status.clone();
    let wait_handle = tokio::task::spawn_blocking(move || {
        let code = match child.wait() {
            Ok(status) => status.exit_code() as i32,
            Err(_) => -1,
        };
        wait_exit_status.store(true, Ordering::SeqCst);
        let _ = exit_tx.send(code);
    });

    let (session, initial_output_rx) = ExecCommandSession::new(
        writer_tx,
        output_tx,
        killer,
        reader_handle,
        writer_handle,
        wait_handle,
        exit_status,
    );
    Ok((session, initial_output_rx, exit_rx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec_command::ExecControlAction;
    use crate::exec_command::ExecControlParams;
    use crate::exec_command::ExecControlStatus;
    use serde_json::Value;

    const TEST_SHELL: &str = "/bin/bash";

    fn base_params(cmd: &str) -> ExecCommandParams {
        ExecCommandParams {
            cmd: CommandLine::test_shell(cmd),
            yield_time_ms: 250,
            max_output_tokens: 1024,
            shell: TEST_SHELL.to_string(),
            login: false,
            idle_timeout_ms: Some(1_000),
            hard_timeout_ms: Some(2_000),
            grace_period_ms: 200,
            log_threshold_bytes: 1_024,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn idle_timeout_terminates_session() {
        let manager = SessionManager::default();
        let params = base_params("sleep 5");
        let output = match manager.handle_exec_command_request(params).await {
            Ok(output) => output,
            Err(err) => panic!("exec start failed: {err}"),
        };
        let session_id = match output.exit_status {
            ExitStatus::Ongoing(id) => id,
            ExitStatus::Exited(code) => panic!("session exited early {code}"),
        };

        tokio::time::sleep(Duration::from_millis(1_600)).await;

        let list = manager.list_sessions().await;
        let summary = match list.into_iter().find(|s| s.session_id == session_id) {
            Some(summary) => summary,
            None => panic!("session summary not found"),
        };
        assert!(matches!(
            summary.state,
            SessionLifecycle::Grace | SessionLifecycle::Terminated
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn keepalive_extends_session() {
        let manager = SessionManager::default();
        let params = ExecCommandParams {
            idle_timeout_ms: Some(1_000),
            hard_timeout_ms: Some(4_000),
            grace_period_ms: 200,
            ..base_params("sleep 6")
        };
        let output = match manager.handle_exec_command_request(params).await {
            Ok(output) => output,
            Err(err) => panic!("exec start failed: {err}"),
        };
        let session_id = match output.exit_status {
            ExitStatus::Ongoing(id) => id,
            ExitStatus::Exited(code) => panic!("session exited early {code}"),
        };

        tokio::time::sleep(Duration::from_millis(700)).await;
        let control = ExecControlParams {
            session_id,
            action: ExecControlAction::Keepalive {
                extend_timeout_ms: Some(2_000),
            },
        };
        let resp = manager.handle_exec_control_request(control).await;
        assert!(matches!(resp.status, ExecControlStatus::Ack));

        tokio::time::sleep(Duration::from_millis(1_500)).await;

        let list = manager.list_sessions().await;
        let summary = match list.into_iter().find(|s| s.session_id == session_id) {
            Some(summary) => summary,
            None => panic!("session summary not found"),
        };
        assert_eq!(summary.state, SessionLifecycle::Running);

        // Clean up to avoid lingering processes.
        let _ = manager
            .handle_exec_control_request(ExecControlParams {
                session_id,
                action: ExecControlAction::ForceKill,
            })
            .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn manual_force_kill_records_summary() {
        let manager = SessionManager::default();
        let output = manager
            .handle_exec_command_request(base_params("sleep 60"))
            .await
            .expect("exec start failed");
        let session_id = match output.exit_status {
            ExitStatus::Ongoing(id) => id,
            ExitStatus::Exited(code) => panic!("session exited unexpectedly {code}"),
        };

        let response = manager
            .handle_exec_control_request(ExecControlParams {
                session_id,
                action: ExecControlAction::ForceKill,
            })
            .await;
        assert!(matches!(response.status, ExecControlStatus::Ack));

        let events = manager
            .session_events(session_id, None, Some(8))
            .await
            .expect("fetch manual force kill events");
        assert!(
            events
                .iter()
                .any(|event| matches!(event.event, SessionEventKind::ForceKill))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event.event, SessionEventKind::EscalationSummary))
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn buffered_output_delivered_to_followup_requests() {
        let manager = SessionManager::default();
        let params = ExecCommandParams {
            yield_time_ms: 20,
            idle_timeout_ms: Some(2_000),
            hard_timeout_ms: Some(3_000),
            grace_period_ms: 200,
            ..base_params("sleep 0.2; printf 'sentinel\\n'; sleep 0.3")
        };

        let output = manager
            .handle_exec_command_request(params)
            .await
            .expect("exec start failed");
        let session_id = match output.exit_status {
            ExitStatus::Ongoing(id) => id,
            ExitStatus::Exited(code) => panic!("session exited prematurely {code}"),
        };

        tokio::time::sleep(Duration::from_millis(220)).await;

        let poll = manager
            .handle_write_stdin_request(WriteStdinParams {
                session_id,
                chars: String::new(),
                yield_time_ms: 200,
                max_output_tokens: 1_024,
                tail_lines: None,
                since_byte: None,
                reset_cursor: false,
                stop_pattern: None,
                stop_pattern_cut: false,
                stop_pattern_label_tail: false,
                raw: false,
                compact: false,
                all: false,
                from_line: None,
                to_line: None,
                smart_compress: true,
            })
            .await
            .expect("stdin poll failed");

        assert!(poll.output.contains("sentinel"), "expected buffered output");
        assert!(!poll.lossy, "unexpected lossy flag");

        // ensure cleanup
        let _ = manager
            .handle_exec_control_request(ExecControlParams {
                session_id,
                action: ExecControlAction::ForceKill,
            })
            .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stdout_activity_extends_idle_timeout() {
        let manager = SessionManager::default();
        let params = ExecCommandParams {
            idle_timeout_ms: Some(200),
            hard_timeout_ms: Some(3_000),
            grace_period_ms: 100,
            ..base_params("for i in $(seq 1 12); do echo ping$i; sleep 0.05; done; sleep 0.5")
        };

        let output = manager
            .handle_exec_command_request(params)
            .await
            .expect("exec start failed");
        let session_id = match output.exit_status {
            ExitStatus::Ongoing(id) => id,
            ExitStatus::Exited(code) => panic!("session exited prematurely {code}"),
        };

        tokio::time::sleep(Duration::from_millis(350)).await;

        let summaries = manager.list_sessions().await;
        let summary = summaries
            .into_iter()
            .find(|s| s.session_id == session_id)
            .expect("expected session summary");
        assert_eq!(summary.state, SessionLifecycle::Running);

        let _ = manager
            .handle_exec_control_request(ExecControlParams {
                session_id,
                action: ExecControlAction::ForceKill,
            })
            .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stop_pattern_interrupts_long_running_command() {
        let manager = SessionManager::default();
        let params = ExecCommandParams {
            yield_time_ms: 150,
            idle_timeout_ms: Some(5_000),
            hard_timeout_ms: Some(10_000),
            grace_period_ms: 200,
            ..base_params("while true; do echo loop; sleep 0.05; done")
        };

        let output = manager
            .handle_exec_command_request(params)
            .await
            .expect("exec start failed");
        let session_id = match output.exit_status {
            ExitStatus::Ongoing(id) => id,
            ExitStatus::Exited(code) => panic!("session exited prematurely {code}"),
        };

        tokio::time::sleep(Duration::from_millis(250)).await;

        let poll = manager
            .handle_write_stdin_request(WriteStdinParams {
                session_id,
                chars: String::new(),
                yield_time_ms: 250,
                max_output_tokens: 1_024,
                tail_lines: None,
                since_byte: None,
                reset_cursor: false,
                stop_pattern: Some("loop".to_string()),
                stop_pattern_cut: false,
                stop_pattern_label_tail: false,
                raw: false,
                compact: false,
                all: false,
                from_line: None,
                to_line: None,
                smart_compress: true,
            })
            .await
            .expect("write_stdin poll failed");

        assert!(poll.pattern_matched, "stop_pattern should trigger ctrl-c");
        let metadata = poll
            .pattern_metadata
            .as_ref()
            .expect("pattern metadata present");
        assert_eq!(metadata.pattern, "loop");
        assert!(
            metadata
                .matched_text
                .as_deref()
                .unwrap_or_default()
                .contains("loop")
        );
        assert!(
            poll.actions_summary
                .iter()
                .any(|entry| entry.contains("stop_pattern ctrl-c"))
        );
        match poll.exit_status {
            ExitStatus::Ongoing(id) => assert_eq!(id, session_id),
            ExitStatus::Exited(code) => panic!("session exited too early {code}"),
        }

        let events = manager
            .session_events(session_id, None, Some(16))
            .await
            .expect("fetch session events");
        assert!(
            events
                .iter()
                .any(|event| matches!(event.event, SessionEventKind::StopPatternMatched))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event.event, SessionEventKind::CtrlCSent))
        );

        // Wait for the watchdog to observe process exit.
        let mut attempts = 0;
        loop {
            tokio::time::sleep(Duration::from_millis(150)).await;
            attempts += 1;

            if attempts > 20 {
                panic!("session #{session_id:?} did not terminate after stop_pattern");
            }

            let summaries = manager.list_sessions().await;
            if let Some(summary) = summaries.into_iter().find(|s| s.session_id == session_id) {
                if matches!(
                    summary.state,
                    SessionLifecycle::Grace | SessionLifecycle::Terminated
                ) {
                    break;
                }
            } else {
                // Session removed from registry ⇒ terminated.
                break;
            }
        }

        // Final poll should report exit status without hanging or runaway output.
        let final_poll = manager
            .handle_write_stdin_request(WriteStdinParams {
                session_id,
                chars: String::new(),
                yield_time_ms: 200,
                max_output_tokens: 1_024,
                tail_lines: None,
                since_byte: None,
                reset_cursor: false,
                stop_pattern: None,
                stop_pattern_cut: false,
                stop_pattern_label_tail: false,
                raw: false,
                compact: false,
                all: true,
                from_line: None,
                to_line: None,
                smart_compress: true,
            })
            .await
            .expect("final poll failed");

        match final_poll.exit_status {
            ExitStatus::Exited(_) => {}
            ExitStatus::Ongoing(_) => panic!("session should be terminated after stop_pattern"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stop_pattern_cut_trims_output_and_labels_tail() {
        let manager = SessionManager::default();
        let params = ExecCommandParams {
            yield_time_ms: 150,
            idle_timeout_ms: Some(5_000),
            hard_timeout_ms: Some(10_000),
            grace_period_ms: 200,
            ..base_params(
                "python3 - <<'PY'\nimport sys,time\nfor line in ['pre', 'match', 'post']:\n    print(line)\n    sys.stdout.flush()\n    time.sleep(0.05)\nwhile True:\n    time.sleep(1)\nPY",
            )
        };

        let output = manager
            .handle_exec_command_request(params)
            .await
            .expect("exec start failed");
        let session_id = match output.exit_status {
            ExitStatus::Ongoing(id) => id,
            ExitStatus::Exited(code) => panic!("session exited prematurely {code}"),
        };

        tokio::time::sleep(Duration::from_millis(200)).await;

        let poll = manager
            .handle_write_stdin_request(WriteStdinParams {
                session_id,
                chars: String::new(),
                yield_time_ms: 200,
                max_output_tokens: 512,
                tail_lines: None,
                since_byte: None,
                reset_cursor: false,
                stop_pattern: Some("^match$".to_string()),
                stop_pattern_cut: true,
                stop_pattern_label_tail: true,
                raw: false,
                compact: false,
                all: false,
                from_line: None,
                to_line: None,
                smart_compress: true,
            })
            .await
            .expect("write_stdin with cut should succeed");

        assert!(poll.pattern_matched);
        assert!(poll.output.contains("pre"));
        assert!(poll.output.contains("match"));
        assert!(!poll.output.contains("post"));
        assert!(poll.output.contains(STOP_PATTERN_TAIL_LABEL));
        assert_eq!(
            poll.tail_label.as_deref(),
            Some("tail_omitted_after_stop_pattern")
        );
        assert!(
            poll.actions_summary
                .iter()
                .any(|entry| entry.contains("stop_pattern ctrl-c (cut)"))
        );
        assert!(
            poll.actions_summary
                .iter()
                .any(|entry| entry.contains("stop_pattern tail omitted"))
        );

        let events = manager
            .session_events(session_id, None, Some(16))
            .await
            .expect("fetch session events");
        let kinds: Vec<SessionEventKind> = events.iter().map(|e| e.event).collect();
        assert!(kinds.contains(&SessionEventKind::StopPatternMatched));
        assert!(kinds.contains(&SessionEventKind::OutputTrimmed));
        assert!(kinds.contains(&SessionEventKind::PatternTailLabeled));

        let _ = manager
            .handle_exec_control_request(ExecControlParams {
                session_id,
                action: ExecControlAction::ForceKill,
            })
            .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn keepalive_records_event_and_note() {
        let manager = SessionManager::default();
        let output = manager
            .handle_exec_command_request(base_params("sleep 2"))
            .await
            .expect("exec start");
        let session_id = match output.exit_status {
            ExitStatus::Ongoing(id) => id,
            ExitStatus::Exited(code) => panic!("session exited early {code}"),
        };

        let response = manager
            .handle_exec_control_request(ExecControlParams {
                session_id,
                action: ExecControlAction::Keepalive {
                    extend_timeout_ms: Some(5_000),
                },
            })
            .await;
        assert!(matches!(response.status, ExecControlStatus::Ack));
        assert!(
            response
                .note
                .as_deref()
                .unwrap_or_default()
                .contains("keepalive")
        );

        let events = manager
            .session_events(session_id, None, Some(8))
            .await
            .expect("fetch events");
        assert!(
            events
                .iter()
                .any(|event| matches!(event.event, SessionEventKind::Keepalive))
        );

        let _ = manager
            .handle_exec_control_request(ExecControlParams {
                session_id,
                action: ExecControlAction::ForceKill,
            })
            .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn list_sessions_filtered_respects_state_and_since() {
        let manager = SessionManager::default();
        let run_params = ExecCommandParams {
            yield_time_ms: 150,
            idle_timeout_ms: Some(5_000),
            hard_timeout_ms: Some(10_000),
            grace_period_ms: 200,
            ..base_params("sleep 60")
        };

        let output = manager
            .handle_exec_command_request(run_params)
            .await
            .expect("exec start failed");
        let running_id = match output.exit_status {
            ExitStatus::Ongoing(id) => id,
            ExitStatus::Exited(code) => panic!("unexpected exit {code}"),
        };

        // Spawn and terminate a second session.
        let finished = manager
            .handle_exec_command_request(base_params("sleep 60"))
            .await
            .expect("second exec start");
        let finished_id = match finished.exit_status {
            ExitStatus::Ongoing(id) => id,
            ExitStatus::Exited(code) => panic!("unexpected exit {code}"),
        };
        let _ = manager
            .handle_exec_control_request(ExecControlParams {
                session_id: finished_id,
                action: ExecControlAction::ForceKill,
            })
            .await;

        let running_only = manager
            .list_sessions_filtered(Some(SessionLifecycle::Running), None, None)
            .await;
        assert!(running_only.iter().any(|s| s.session_id == running_id));
        assert!(
            running_only
                .iter()
                .all(|s| s.state == SessionLifecycle::Running)
        );

        let recent_only = manager
            .list_sessions_filtered(None, None, Some(60_000))
            .await;
        assert!(recent_only.iter().any(|s| s.session_id == running_id));

        let _ = manager
            .handle_exec_control_request(ExecControlParams {
                session_id: running_id,
                action: ExecControlAction::ForceKill,
            })
            .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn exec_command_running_returns_json_payload() {
        let manager = SessionManager::default();
        let params = ExecCommandParams {
            cmd: CommandLine::test_shell("sleep 5"),
            yield_time_ms: 250,
            max_output_tokens: 1024,
            shell: TEST_SHELL.to_string(),
            login: false,
            idle_timeout_ms: Some(5_000),
            hard_timeout_ms: Some(10_000),
            grace_period_ms: 200,
            log_threshold_bytes: 1_024,
        };

        let output = manager
            .handle_exec_command_request(params)
            .await
            .expect("exec command should start");
        let payload: Value = serde_json::from_str(&output.to_text_output())
            .expect("exec_command payload should be JSON");
        assert_eq!(
            payload["version"].as_u64(),
            Some(EXEC_COMMAND_PAYLOAD_VERSION as u64)
        );
        assert_eq!(payload["status"], "started");
        let session_id = payload["session_id"].as_u64().expect("session id present");
        assert_eq!(payload["truncated"], false);
        assert!(
            payload["management_hint"]
                .as_str()
                .unwrap()
                .contains("Ctrl+E")
        );
        assert!(payload.get("note").is_none());
        assert!(payload.get("guidance").is_none());

        let response = manager
            .handle_exec_control_request(ExecControlParams {
                session_id: SessionId::new(session_id as u32),
                action: ExecControlAction::ForceKill,
            })
            .await;
        assert!(matches!(response.status, ExecControlStatus::Ack));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn exec_command_completed_returns_json_payload() {
        let manager = SessionManager::default();
        let output = manager
            .handle_exec_command_request(base_params("echo hello"))
            .await
            .expect("exec command should succeed");
        let payload: Value = serde_json::from_str(&output.to_text_output())
            .expect("exec_command payload should be JSON");
        assert_eq!(
            payload["version"].as_u64(),
            Some(EXEC_COMMAND_PAYLOAD_VERSION as u64)
        );
        assert_eq!(payload["status"], "completed");
        assert_eq!(payload["exit_code"].as_i64(), Some(0));
        assert!(payload.get("session_id").is_none());
        assert!(payload.get("management_hint").is_none());
        assert_eq!(payload["note"].as_str(), Some("completed (exit_code=0)"));
        assert!(payload.get("guidance").is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn partial_line_is_streamed_incrementally() {
        let manager = SessionManager::default();
        let params = ExecCommandParams {
            yield_time_ms: 80,
            idle_timeout_ms: Some(5_000),
            hard_timeout_ms: Some(10_000),
            grace_period_ms: 200,
            ..base_params(
                "printf 'prompt'; sleep 0.6; printf '>'; sleep 0.6; printf 'X'; sleep 0.2",
            )
        };

        let output = manager
            .handle_exec_command_request(params)
            .await
            .expect("exec start failed");
        let session_id = match output.exit_status {
            ExitStatus::Ongoing(id) => id,
            ExitStatus::Exited(code) => panic!("session exited too early {code}"),
        };

        tokio::time::sleep(Duration::from_millis(80)).await;

        let first_poll = {
            const RETRIES: usize = 5;
            let mut attempt = 0;
            loop {
                let response = manager
                    .handle_write_stdin_request(WriteStdinParams {
                        session_id,
                        chars: String::new(),
                        yield_time_ms: 120,
                        max_output_tokens: 1_024,
                        tail_lines: None,
                        since_byte: None,
                        reset_cursor: false,
                        stop_pattern: None,
                        stop_pattern_cut: false,
                        stop_pattern_label_tail: false,
                        raw: false,
                        compact: false,
                        all: false,
                        from_line: None,
                        to_line: None,
                        smart_compress: true,
                    })
                    .await
                    .expect("first poll failed");
                if !response.output.is_empty() {
                    break response;
                }
                attempt += 1;
                assert!(
                    attempt <= RETRIES,
                    "timed out awaiting initial partial output"
                );
                tokio::time::sleep(Duration::from_millis(60)).await;
            }
        };
        assert_eq!(first_poll.output, "prompt");

        tokio::time::sleep(Duration::from_millis(500)).await;

        let poll = manager
            .handle_write_stdin_request(WriteStdinParams {
                session_id,
                chars: String::new(),
                yield_time_ms: 120,
                max_output_tokens: 1_024,
                tail_lines: None,
                since_byte: None,
                reset_cursor: false,
                stop_pattern: None,
                stop_pattern_cut: false,
                stop_pattern_label_tail: false,
                raw: false,
                compact: false,
                all: false,
                from_line: None,
                to_line: None,
                smart_compress: true,
            })
            .await
            .expect("write_stdin poll failed");
        assert_eq!(poll.output, ">", "should stream next fragment only");

        tokio::time::sleep(Duration::from_millis(700)).await;

        let third = manager
            .handle_write_stdin_request(WriteStdinParams {
                session_id,
                chars: String::new(),
                yield_time_ms: 120,
                max_output_tokens: 1_024,
                tail_lines: None,
                since_byte: None,
                reset_cursor: false,
                stop_pattern: None,
                stop_pattern_cut: false,
                stop_pattern_label_tail: false,
                raw: false,
                compact: false,
                all: false,
                from_line: None,
                to_line: None,
                smart_compress: true,
            })
            .await
            .expect("third poll failed");
        assert_eq!(third.output, "X");

        let follow_up = manager
            .handle_write_stdin_request(WriteStdinParams {
                session_id,
                chars: String::new(),
                yield_time_ms: 80,
                max_output_tokens: 1_024,
                tail_lines: None,
                since_byte: None,
                reset_cursor: false,
                stop_pattern: None,
                stop_pattern_cut: false,
                stop_pattern_label_tail: false,
                raw: false,
                compact: false,
                all: false,
                from_line: None,
                to_line: None,
                smart_compress: true,
            })
            .await;
        match follow_up {
            Ok(resp) => assert!(
                resp.output.is_empty(),
                "no duplicate partial fragments expected"
            ),
            Err(err) => assert!(err.contains("unknown session")),
        }

        let _ = manager
            .handle_exec_control_request(ExecControlParams {
                session_id,
                action: ExecControlAction::ForceKill,
            })
            .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn auto_mode_does_not_repeat_completed_partial_line() {
        let manager = SessionManager::default();
        let params = ExecCommandParams {
            yield_time_ms: 80,
            idle_timeout_ms: Some(5_000),
            hard_timeout_ms: Some(10_000),
            grace_period_ms: 200,
            ..base_params(
                "python3 - <<'PY'\nimport sys, time\nsys.stdout.write('A' * 2048)\nsys.stdout.flush()\ntime.sleep(0.25)\nsys.stdout.write('B' * 16 + '\\n')\nsys.stdout.flush()\ntime.sleep(0.25)\nPY",
            )
        };

        let output = manager
            .handle_exec_command_request(params)
            .await
            .expect("exec start failed");
        assert!(output.output.chars().all(|c| c == 'A'));
        let session_id = match output.exit_status {
            ExitStatus::Ongoing(id) => id,
            ExitStatus::Exited(code) => panic!("session exited too early {code}"),
        };

        tokio::time::sleep(Duration::from_millis(220)).await;

        let poll = manager
            .handle_write_stdin_request(WriteStdinParams {
                session_id,
                chars: String::new(),
                yield_time_ms: 120,
                max_output_tokens: 1_024,
                tail_lines: None,
                since_byte: None,
                reset_cursor: false,
                stop_pattern: None,
                stop_pattern_cut: false,
                stop_pattern_label_tail: false,
                raw: false,
                compact: false,
                all: false,
                from_line: None,
                to_line: None,
                smart_compress: true,
            })
            .await
            .expect("poll after completion failed");
        assert_eq!(poll.output, "B".repeat(16));
        assert!(
            !poll.output.contains('A'),
            "completed line duplicated previous bytes: {}",
            poll.output
        );

        tokio::time::sleep(Duration::from_millis(200)).await;

        let follow_up = manager
            .handle_write_stdin_request(WriteStdinParams {
                session_id,
                chars: String::new(),
                yield_time_ms: 80,
                max_output_tokens: 1_024,
                tail_lines: None,
                since_byte: None,
                reset_cursor: false,
                stop_pattern: None,
                stop_pattern_cut: false,
                stop_pattern_label_tail: false,
                raw: false,
                compact: false,
                all: false,
                from_line: None,
                to_line: None,
                smart_compress: true,
            })
            .await;
        match follow_up {
            Ok(resp) => assert!(resp.output.is_empty()),
            Err(err) => assert!(err.contains("unknown session")),
        }

        let _ = manager
            .handle_exec_control_request(ExecControlParams {
                session_id,
                action: ExecControlAction::ForceKill,
            })
            .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stop_pattern_matches_across_partial_writes() {
        let manager = SessionManager::default();
        let params = ExecCommandParams {
            yield_time_ms: 80,
            idle_timeout_ms: Some(5_000),
            hard_timeout_ms: Some(10_000),
            grace_period_ms: 200,
            ..base_params(
                "python3 - <<'PY'\nimport sys, time\nsys.stdout.write('Enter ')\nsys.stdout.flush()\ntime.sleep(0.2)\nsys.stdout.write('password:')\nsys.stdout.flush()\ntime.sleep(0.2)\nPY",
            )
        };

        let output = manager
            .handle_exec_command_request(params)
            .await
            .expect("exec start failed");
        assert_eq!(output.output, "Enter ");
        let session_id = match output.exit_status {
            ExitStatus::Ongoing(id) => id,
            ExitStatus::Exited(code) => panic!("session exited unexpectedly {code}"),
        };

        tokio::time::sleep(Duration::from_millis(240)).await;

        let poll = manager
            .handle_write_stdin_request(WriteStdinParams {
                session_id,
                chars: String::new(),
                yield_time_ms: 160,
                max_output_tokens: 1_024,
                tail_lines: None,
                since_byte: None,
                reset_cursor: false,
                stop_pattern: Some("^Enter password:$".to_string()),
                stop_pattern_cut: false,
                stop_pattern_label_tail: false,
                raw: false,
                compact: false,
                all: false,
                from_line: None,
                to_line: None,
                smart_compress: true,
            })
            .await
            .expect("stop_pattern poll failed");
        assert!(
            poll.pattern_matched,
            "regex should match across partial writes"
        );

        let mut attempts = 0;
        loop {
            tokio::time::sleep(Duration::from_millis(120)).await;
            attempts += 1;
            if attempts > 20 {
                panic!("session #{session_id:?} did not terminate after stop_pattern");
            }

            let summaries = manager.list_sessions().await;
            if let Some(summary) = summaries.into_iter().find(|s| s.session_id == session_id) {
                if matches!(
                    summary.state,
                    SessionLifecycle::Grace | SessionLifecycle::Terminated
                ) {
                    break;
                }
            } else {
                break;
            }
        }

        let final_poll = manager
            .handle_write_stdin_request(WriteStdinParams {
                session_id,
                chars: String::new(),
                yield_time_ms: 120,
                max_output_tokens: 1_024,
                tail_lines: None,
                since_byte: None,
                reset_cursor: false,
                stop_pattern: None,
                stop_pattern_cut: false,
                stop_pattern_label_tail: false,
                raw: false,
                compact: false,
                all: true,
                from_line: None,
                to_line: None,
                smart_compress: true,
            })
            .await;
        match final_poll {
            Ok(resp) => assert!(matches!(resp.exit_status, ExitStatus::Exited(_))),
            Err(err) => assert!(err.contains("unknown session")),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn watch_pattern_logs_guidance() {
        let manager = SessionManager::default();
        let params = ExecCommandParams {
            yield_time_ms: 80,
            idle_timeout_ms: Some(5_000),
            hard_timeout_ms: Some(10_000),
            grace_period_ms: 200,
            ..base_params("for i in $(seq 1 60); do echo count$i; sleep 0.02; done")
        };

        let output = manager
            .handle_exec_command_request(params)
            .await
            .expect("exec start failed");
        let session_id = match output.exit_status {
            ExitStatus::Ongoing(id) => id,
            ExitStatus::Exited(code) => panic!("session exited too early {code}"),
        };

        let response = manager
            .handle_exec_control_request(ExecControlParams {
                session_id,
                action: ExecControlAction::Watch {
                    pattern: "count50".to_string(),
                    action: ExecWatchAction::Log,
                    persist: false,
                    cooldown_ms: None,
                    auto_send_ctrl_c: None,
                },
            })
            .await;
        assert!(matches!(response.status, ExecControlStatus::Ack));

        tokio::time::sleep(Duration::from_millis(1200)).await;

        let poll = manager
            .handle_write_stdin_request(WriteStdinParams {
                session_id,
                chars: String::new(),
                yield_time_ms: 120,
                max_output_tokens: 1_024,
                tail_lines: None,
                since_byte: None,
                reset_cursor: false,
                stop_pattern: None,
                stop_pattern_cut: false,
                stop_pattern_label_tail: false,
                raw: false,
                compact: false,
                all: false,
                from_line: None,
                to_line: None,
                smart_compress: true,
            })
            .await
            .expect("watch poll failed");
        let guidance = poll.guidance.clone().unwrap_or_default();
        assert!(guidance.contains("watch"));
        assert!(
            poll.actions_summary
                .iter()
                .any(|entry| entry.contains("watch `count50`"))
        );

        let _ = manager
            .handle_exec_control_request(ExecControlParams {
                session_id,
                action: ExecControlAction::ForceKill,
            })
            .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn watch_pattern_sends_ctrl_c() {
        let manager = SessionManager::default();
        let params = ExecCommandParams {
            yield_time_ms: 80,
            idle_timeout_ms: Some(5_000),
            hard_timeout_ms: Some(10_000),
            grace_period_ms: 200,
            ..base_params("for i in $(seq 1 100); do echo step$i; sleep 0.05; done")
        };

        let output = manager
            .handle_exec_command_request(params)
            .await
            .expect("exec start failed");
        let session_id = match output.exit_status {
            ExitStatus::Ongoing(id) => id,
            ExitStatus::Exited(code) => panic!("session exited too early {code}"),
        };

        let response = manager
            .handle_exec_control_request(ExecControlParams {
                session_id,
                action: ExecControlAction::Watch {
                    pattern: "step20".to_string(),
                    action: ExecWatchAction::SendCtrlC,
                    persist: false,
                    cooldown_ms: None,
                    auto_send_ctrl_c: None,
                },
            })
            .await;
        assert!(matches!(response.status, ExecControlStatus::Ack));

        tokio::time::sleep(Duration::from_millis(1200)).await;

        let poll = manager
            .handle_write_stdin_request(WriteStdinParams {
                session_id,
                chars: String::new(),
                yield_time_ms: 120,
                max_output_tokens: 1_024,
                tail_lines: None,
                since_byte: None,
                reset_cursor: false,
                stop_pattern: None,
                stop_pattern_cut: false,
                stop_pattern_label_tail: false,
                raw: false,
                compact: false,
                all: false,
                from_line: None,
                to_line: None,
                smart_compress: true,
            })
            .await
            .expect("watch ctrl-c poll failed");

        let guidance = poll.guidance.unwrap_or_default();
        assert!(guidance.contains("Ctrl-C"));
        assert!(
            poll.actions_summary
                .iter()
                .any(|entry| entry.contains("watch `step20` ctrl-c"))
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn archived_events_survive_prune() {
        let manager = SessionManager::default();
        let params = ExecCommandParams {
            yield_time_ms: 40,
            idle_timeout_ms: Some(5_000),
            hard_timeout_ms: Some(10_000),
            grace_period_ms: 200,
            ..base_params("sleep 60")
        };

        let output = manager
            .handle_exec_command_request(params)
            .await
            .expect("exec start failed");
        let session_id = match output.exit_status {
            ExitStatus::Ongoing(id) => id,
            ExitStatus::Exited(code) => panic!("session exited too early {code}"),
        };

        let response = manager
            .handle_exec_control_request(ExecControlParams {
                session_id,
                action: ExecControlAction::ForceKill,
            })
            .await;
        assert!(matches!(response.status, ExecControlStatus::Ack));

        let live_events = manager
            .session_events(session_id, None, Some(16))
            .await
            .expect("live events");
        assert!(
            live_events
                .iter()
                .any(|event| matches!(event.event, SessionEventKind::ForceKill))
        );

        {
            let sessions = manager.inner.sessions.lock().await;
            let session = sessions
                .get(&session_id)
                .cloned()
                .expect("session should exist before prune");
            drop(sessions);

            let mut termination = session.termination.lock().await;
            let adjusted_at = Instant::now()
                .checked_sub(Duration::from_millis(PRUNE_AFTER_MS + 1))
                .expect("adjust time");
            if let Some(record) = termination.as_mut() {
                record.at = adjusted_at;
            } else {
                *termination = Some(TerminationRecord {
                    reason: TerminationReason::ForceKilled,
                    at: adjusted_at,
                });
            }
        }

        manager.inner.prune_finished().await;

        {
            let sessions = manager.inner.sessions.lock().await;
            assert!(!sessions.contains_key(&session_id));
        }

        let archived_events = manager
            .session_events(session_id, None, Some(16))
            .await
            .expect("archived events");
        assert!(
            archived_events
                .iter()
                .any(|event| matches!(event.event, SessionEventKind::ForceKill))
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stop_pattern_returns_full_line_after_partial_prefix() {
        let manager = SessionManager::default();
        let params = ExecCommandParams {
            yield_time_ms: 60,
            idle_timeout_ms: Some(5_000),
            hard_timeout_ms: Some(10_000),
            grace_period_ms: 200,
            ..base_params(
                "python3 - <<'PY'\nimport sys, time\nsys.stdout.write('PROMPT')\nsys.stdout.flush()\ntime.sleep(0.15)\nsys.stdout.write('CONT\\n')\nsys.stdout.flush()\ntime.sleep(0.05)\nPY",
            )
        };

        let output = manager
            .handle_exec_command_request(params)
            .await
            .expect("exec start failed");
        let session_id = match output.exit_status {
            ExitStatus::Ongoing(id) => id,
            ExitStatus::Exited(code) => panic!("session exited unexpectedly {code}"),
        };

        tokio::time::sleep(Duration::from_millis(220)).await;

        let second = manager
            .handle_write_stdin_request(WriteStdinParams {
                session_id,
                chars: String::new(),
                yield_time_ms: 80,
                max_output_tokens: 1_024,
                tail_lines: None,
                since_byte: None,
                reset_cursor: false,
                stop_pattern: Some("PROMPTCONT".to_string()),
                stop_pattern_cut: false,
                stop_pattern_label_tail: false,
                raw: false,
                compact: false,
                all: false,
                from_line: None,
                to_line: None,
                smart_compress: true,
            })
            .await
            .expect("second poll");

        assert!(
            second.output.contains("PROMPTCONT"),
            "expected full matched line in output, got `{}`",
            second.output
        );
        let meta = second.pattern_metadata.expect("pattern metadata present");
        assert_eq!(meta.matched_text.as_deref(), Some("PROMPTCONT"));

        let _ = manager
            .handle_exec_control_request(ExecControlParams {
                session_id,
                action: ExecControlAction::ForceKill,
            })
            .await;
    }

    #[test]
    fn normalize_yield_time_clamps_bounds() {
        assert_eq!(normalize_yield_time_ms(0), MIN_YIELD_TIME_MS);
        assert_eq!(
            normalize_yield_time_ms(MAX_YIELD_TIME_MS + 10_000),
            MAX_YIELD_TIME_MS
        );
        assert_eq!(normalize_yield_time_ms(5_000), 5_000);
    }

    #[test]
    fn normalize_cap_bytes_enforces_minimum() {
        assert_eq!(normalize_cap_bytes(0), MIN_OUTPUT_CAP_BYTES);
        let huge = normalize_cap_bytes(u64::MAX);
        assert_eq!(huge, usize::MAX);
    }

    // LineBuffer unit tests
    #[test]
    fn line_buffer_handles_complete_lines() {
        let mut buffer = LineBuffer::new(100);
        buffer.push_bytes(b"line1\nline2\nline3\n");

        assert_eq!(buffer.total_lines(), 3);
        let lines = buffer.get_lines(0, 3);
        assert_eq!(lines, vec!["line1", "line2", "line3"]);
    }

    #[test]
    fn line_buffer_handles_partial_lines_across_chunks() {
        let mut buffer = LineBuffer::new(100);

        // First chunk: partial line
        buffer.push_bytes(b"hello wor");
        assert_eq!(buffer.total_lines(), 0); // No complete line yet
        assert_eq!(buffer.partial_line, "hello wor");

        // Second chunk: completes the line
        buffer.push_bytes(b"ld\n");
        assert_eq!(buffer.total_lines(), 1);
        let lines = buffer.get_lines(0, 1);
        assert_eq!(lines, vec!["hello world"]);
        assert_eq!(buffer.partial_line, "");
    }

    #[test]
    fn line_buffer_handles_multiple_partial_chunks() {
        let mut buffer = LineBuffer::new(100);

        buffer.push_bytes(b"1");
        buffer.push_bytes(b"2");
        buffer.push_bytes(b"3\n");
        assert_eq!(buffer.total_lines(), 1);

        buffer.push_bytes(b"4");
        buffer.push_bytes(b"5");
        assert_eq!(buffer.total_lines(), 1); // Still waiting for newline

        buffer.push_bytes(b"6\n");
        assert_eq!(buffer.total_lines(), 2);

        let lines = buffer.get_lines(0, 2);
        assert_eq!(lines, vec!["123", "456"]);
    }

    #[test]
    fn line_buffer_handles_empty_chunks() {
        let mut buffer = LineBuffer::new(100);
        buffer.push_bytes(b"");
        assert_eq!(buffer.total_lines(), 0);

        buffer.push_bytes(b"test\n");
        assert_eq!(buffer.total_lines(), 1);
    }

    #[test]
    fn line_buffer_truncates_long_lines() {
        let mut buffer = LineBuffer::new(100);
        let long_line = "x".repeat(300_000); // 300 KB line (exceeds 256 KB limit)

        buffer.push_bytes(long_line.as_bytes());
        assert_eq!(buffer.total_lines(), 1);

        buffer.push_bytes(b"\n");
        assert_eq!(buffer.total_lines(), 1);

        let lines = buffer.get_lines(0, 1);
        assert!(lines[0].len() <= buffer.max_line_bytes + 20); // +20 for "...[truncated]"
        assert!(lines[0].ends_with("...[truncated]"));
    }

    #[test]
    fn line_buffer_ring_buffer_eviction() {
        let mut buffer = LineBuffer::new(3); // Max 3 lines

        buffer.push_bytes(b"1\n2\n3\n4\n5\n");
        assert_eq!(buffer.total_lines(), 5);
        assert_eq!(buffer.lines.len(), 3); // Only last 3 retained

        let lines = buffer.get_lines(0, 5);
        assert_eq!(lines, vec!["3", "4", "5"]); // First 2 evicted
    }

    #[test]
    fn line_buffer_get_lines_range_query() {
        let mut buffer = LineBuffer::new(100);
        buffer.push_bytes(b"0\n1\n2\n3\n4\n5\n");

        let range = buffer.get_lines(2, 5);
        assert_eq!(range, vec!["2", "3", "4"]);
    }

    #[test]
    fn line_buffer_get_lines_out_of_bounds() {
        let mut buffer = LineBuffer::new(3);
        buffer.push_bytes(b"0\n1\n2\n3\n4\n5\n"); // Lines 3,4,5 retained

        // Request evicted lines
        let lines = buffer.get_lines(0, 2);
        assert_eq!(lines.len(), 0); // Lines 0-2 evicted

        // Partial overlap
        let lines = buffer.get_lines(2, 5);
        assert_eq!(lines, vec!["3", "4"]); // Only available lines
    }

    #[test]
    fn line_buffer_crlf_handling() {
        let mut buffer = LineBuffer::new(100);
        buffer.push_bytes(b"line1\r\nline2\r\n");

        assert_eq!(buffer.total_lines(), 2);
        let lines = buffer.get_lines(0, 2);
        assert_eq!(lines, vec!["line1", "line2"]);
    }

    #[test]
    fn line_buffer_no_trailing_newline() {
        let mut buffer = LineBuffer::new(100);
        buffer.push_bytes(b"line1\nline2");

        assert_eq!(buffer.total_lines(), 1); // Only line1 complete
        assert_eq!(buffer.partial_line, "line2");

        // Add more data to partial line
        buffer.push_bytes(b" continued\n");
        assert_eq!(buffer.total_lines(), 2);

        let lines = buffer.get_lines(0, 2);
        assert_eq!(lines, vec!["line1", "line2 continued"]);
    }

    #[test]
    fn line_buffer_partial_line_overflow_protection() {
        let mut buffer = LineBuffer::new(100);

        // Send partial line that exceeds max_line_bytes (256 KB)
        let huge_chunk = "x".repeat(300_000); // 300 KB
        buffer.push_bytes(huge_chunk.as_bytes());

        // Partial line should be flushed as truncated
        assert_eq!(buffer.total_lines(), 1);
        assert!(buffer.partial_line.is_empty());

        let lines = buffer.get_lines(0, 1);
        assert!(lines[0].ends_with("...[truncated]"));
        assert!(lines[0].len() <= buffer.max_line_bytes + 20);
    }

    #[test]
    fn line_buffer_partial_line_incremental_overflow() {
        let mut buffer = LineBuffer::new(100);

        // Send multiple chunks that together exceed max_line_bytes (256 KB)
        let chunk = "y".repeat(150_000); // 150 KB each
        buffer.push_bytes(chunk.as_bytes()); // partial = 150KB
        assert_eq!(buffer.total_lines(), 0); // Still partial

        buffer.push_bytes(chunk.as_bytes()); // partial would be 300KB (exceeds 256KB limit)

        // Should flush the overflowing partial line
        assert_eq!(buffer.total_lines(), 1);
        assert!(buffer.partial_line.is_empty());
    }

    #[test]
    fn line_buffer_utf8_multibyte_boundary_safety() {
        let mut buffer = LineBuffer::new(100);

        // Test with Chinese characters (3 bytes each in UTF-8)
        // 你 = [0xE4, 0xBD, 0xA0] (3 bytes)
        let chinese = "你好世界".repeat(90_000); // ~1.08 MB (exceeds 256 KB limit)
        buffer.push_bytes(chinese.as_bytes());

        // Should be truncated safely without panic
        assert_eq!(buffer.total_lines(), 1);
        let lines = buffer.get_lines(0, 1);
        assert!(lines[0].ends_with("...[truncated]"));

        // Verify no invalid UTF-8 (no panic on string operations)
        assert!(
            lines[0]
                .chars()
                .all(|c| c != '\u{FFFD}' || chinese.contains('\u{FFFD}'))
        );
    }

    #[test]
    fn line_buffer_utf8_emoji_boundary_safety() {
        let mut buffer = LineBuffer::new(100);

        // Test with emojis (4 bytes each in UTF-8)
        // 😀 = [0xF0, 0x9F, 0x98, 0x80] (4 bytes)
        let emoji_line = "😀".repeat(70_000); // ~280 KB (exceeds 256 KB limit)
        buffer.push_bytes(emoji_line.as_bytes());

        // Should truncate at valid boundary
        assert_eq!(buffer.total_lines(), 1);
        let lines = buffer.get_lines(0, 1);
        assert!(lines[0].ends_with("...[truncated]"));
        assert!(lines[0].len() <= buffer.max_line_bytes + 20);

        // No replacement characters unless original had them
        let has_replacement = lines[0].contains('\u{FFFD}');
        assert!(!has_replacement || emoji_line.contains('\u{FFFD}'));
    }

    #[test]
    fn line_buffer_lossy_utf8_tracking() {
        let mut buffer = LineBuffer::new(100);

        // Valid UTF-8
        buffer.push_bytes("hello world\n".as_bytes());
        assert!(!buffer.has_lossy_utf8());

        // Invalid UTF-8 byte sequence
        let invalid_utf8 = vec![0xFF, 0xFE, 0xFD]; // Invalid UTF-8
        buffer.push_bytes(&invalid_utf8);
        assert!(buffer.has_lossy_utf8()); // Should detect lossy conversion
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn write_stdin_validates_range() {
        let manager = SessionManager::default();
        let params = base_params("echo test");
        let output = manager
            .handle_exec_command_request(params)
            .await
            .expect("exec start failed");

        let session_id = match output.exit_status {
            ExitStatus::Ongoing(id) => id,
            ExitStatus::Exited(_) => return, // Completed immediately, skip test
        };

        // Wait for some output
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Test invalid range: from_line > to_line
        let result = manager
            .handle_write_stdin_request(WriteStdinParams {
                session_id,
                chars: String::new(),
                yield_time_ms: 100,
                max_output_tokens: 1_024,
                tail_lines: None,
                since_byte: None,
                reset_cursor: false,
                stop_pattern: None,
                stop_pattern_cut: false,
                stop_pattern_label_tail: false,
                raw: false,
                compact: false,
                all: false,
                from_line: Some(100),
                to_line: Some(50), // Invalid: 100 > 50
                smart_compress: true,
            })
            .await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err();
        assert!(err_msg.contains("invalid range"));
        assert!(err_msg.contains("from_line"));
        assert!(err_msg.contains("to_line"));

        // Cleanup
        let _ = manager
            .handle_exec_control_request(ExecControlParams {
                session_id,
                action: ExecControlAction::ForceKill,
            })
            .await;
    }

    #[test]
    fn truncate_utf8_safe_preserves_valid_boundaries() {
        // Test ASCII (1 byte per char)
        let mut s = "hello world".to_string();
        truncate_utf8_safe(&mut s, 5, "...");
        assert_eq!(s, "hello...");

        // Test 2-byte UTF-8 (Cyrillic)
        let mut s = "привет".to_string(); // 12 bytes (6 chars × 2 bytes)
        truncate_utf8_safe(&mut s, 7, "...");
        // Should truncate at 6 bytes (3 complete chars), not 7 (mid-char)
        assert_eq!(s, "при...");
        assert!(s.is_char_boundary(6)); // Verify boundary is valid

        // Test 3-byte UTF-8 (Chinese)
        let mut s = "你好世界".to_string(); // 12 bytes (4 chars × 3 bytes)
        truncate_utf8_safe(&mut s, 7, "...");
        // Should truncate at 6 bytes (2 complete chars), not 7 (mid-char)
        assert_eq!(s, "你好...");

        // Test 4-byte UTF-8 (Emoji)
        let mut s = "😀😀😀".to_string(); // 12 bytes (3 chars × 4 bytes)
        truncate_utf8_safe(&mut s, 5, "...");
        // Should truncate at 4 bytes (1 complete char), not 5 (mid-char)
        assert_eq!(s, "😀...");
    }
}
