use portable_pty::CommandBuilder;
use portable_pty::PtySize;
use portable_pty::native_pty_system;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicI32;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tempfile::tempfile;
use tokio::fs::File as TokioFile;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio::sync::Notify;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::error;
use tracing::warn;

use crate::exec_command::ExecCommandSession;
use crate::truncate::truncate_middle;
use codex_protocol::protocol::UnifiedExecSessionState;
use codex_protocol::protocol::UnifiedExecSessionStatus;

mod errors;

pub use errors::UnifiedExecError;

const DEFAULT_TIMEOUT_MS: u64 = 1_000;
const MAX_TIMEOUT_MS: u64 = 60_000;
const UNIFIED_EXEC_OUTPUT_MAX_BYTES: usize = 128 * 1024; // 128 KiB
const UNIFIED_EXEC_WINDOW_DEFAULT_BYTES: usize = UNIFIED_EXEC_OUTPUT_MAX_BYTES;
const UNIFIED_EXEC_WINDOW_MAX_BYTES: usize = 2 * 1024 * 1024; // 2 MiB
const UNIFIED_EXEC_PREVIEW_MAX_BYTES: usize = 4 * 1024;

#[derive(Debug)]
pub(crate) struct UnifiedExecRequest<'a> {
    pub session_id: Option<i32>,
    pub input_chunks: &'a [String],
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UnifiedExecResult {
    pub session_id: Option<i32>,
    pub output: String,
}

#[derive(Debug, Clone)]
pub struct UnifiedExecSessionSnapshot {
    pub session_id: i32,
    pub command: Vec<String>,
    pub started_at: SystemTime,
    pub last_output_at: Option<SystemTime>,
    pub has_exited: bool,
    pub output_preview: String,
    pub output_truncated: bool,
}

#[derive(Debug, Clone)]
pub enum UnifiedExecOutputWindow {
    Tail { max_bytes: usize },
    Range { start: u64, max_bytes: usize },
}

impl UnifiedExecOutputWindow {
    fn clamp_bytes(&self) -> usize {
        let requested = match *self {
            UnifiedExecOutputWindow::Tail { max_bytes }
            | UnifiedExecOutputWindow::Range { max_bytes, .. } => max_bytes,
        };
        requested.clamp(1, UNIFIED_EXEC_WINDOW_MAX_BYTES)
    }

    pub fn tail_default() -> Self {
        Self::Tail {
            max_bytes: UNIFIED_EXEC_WINDOW_DEFAULT_BYTES,
        }
    }
}

#[derive(Debug, Clone)]
pub struct UnifiedExecSessionOutput {
    pub session_id: i32,
    pub command: Vec<String>,
    pub started_at: SystemTime,
    pub last_output_at: Option<SystemTime>,
    pub status: UnifiedExecSessionStatus,
    pub content: String,
    pub truncated: bool,
    pub truncated_suffix: bool,
    pub expandable_prefix: bool,
    pub expandable_suffix: bool,
    pub range_start: u64,
    pub range_end: u64,
    pub total_bytes: u64,
    pub window_bytes: usize,
}

impl UnifiedExecSessionSnapshot {
    fn status(&self) -> UnifiedExecSessionStatus {
        if self.has_exited {
            UnifiedExecSessionStatus::Exited
        } else {
            UnifiedExecSessionStatus::Running
        }
    }
}

impl From<UnifiedExecSessionSnapshot> for UnifiedExecSessionState {
    fn from(snapshot: UnifiedExecSessionSnapshot) -> Self {
        let status = snapshot.status();
        let UnifiedExecSessionSnapshot {
            session_id,
            command,
            started_at,
            last_output_at,
            has_exited: _,
            output_preview,
            output_truncated,
        } = snapshot;
        Self {
            session_id,
            command,
            status,
            started_at_ms: system_time_to_millis(started_at),
            last_output_at_ms: last_output_at.map(system_time_to_millis),
            output_preview,
            output_truncated,
        }
    }
}

fn system_time_to_millis(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis() as u64
}

#[derive(Debug, Default)]
pub(crate) struct UnifiedExecSessionManager {
    next_session_id: AtomicI32,
    sessions: Mutex<HashMap<i32, Arc<ManagedUnifiedExecSession>>>,
}

#[derive(Debug)]
struct ManagedUnifiedExecSession {
    command: Vec<String>,
    started_at: SystemTime,
    session: ExecCommandSession,
    output_buffer: OutputBuffer,
    /// Notifies waiters whenever new output has been appended to
    /// `output_buffer`, allowing clients to poll for fresh data.
    output_notify: Arc<Notify>,
    output_task: JoinHandle<()>,
    output_spool: Option<Arc<OutputSpool>>,
}

#[derive(Debug, Default)]
struct OutputBufferState {
    chunks: VecDeque<Vec<u8>>,
    total_bytes: usize,
    last_output_at: Option<SystemTime>,
    truncated_prefix: bool,
}

impl OutputBufferState {
    fn push_chunk(&mut self, chunk: Vec<u8>) {
        self.total_bytes = self.total_bytes.saturating_add(chunk.len());
        self.chunks.push_back(chunk);
        self.last_output_at = Some(SystemTime::now());

        let mut excess = self
            .total_bytes
            .saturating_sub(UNIFIED_EXEC_OUTPUT_MAX_BYTES);

        while excess > 0 {
            match self.chunks.front_mut() {
                Some(front) if excess >= front.len() => {
                    excess -= front.len();
                    self.total_bytes = self.total_bytes.saturating_sub(front.len());
                    self.chunks.pop_front();
                    self.truncated_prefix = true;
                }
                Some(front) => {
                    front.drain(..excess);
                    self.total_bytes = self.total_bytes.saturating_sub(excess);
                    self.truncated_prefix = true;
                    break;
                }
                None => break,
            }
        }
    }

    fn drain(&mut self) -> Vec<Vec<u8>> {
        let drained: Vec<Vec<u8>> = self.chunks.drain(..).collect();
        self.total_bytes = 0;
        drained
    }

    fn snapshot_bytes(&self) -> Vec<u8> {
        let mut aggregated = Vec::with_capacity(self.total_bytes);
        for chunk in &self.chunks {
            aggregated.extend_from_slice(chunk);
        }
        aggregated
    }

    fn was_truncated(&self) -> bool {
        self.truncated_prefix
    }

    fn last_output_at(&self) -> Option<SystemTime> {
        self.last_output_at
    }
}

type OutputBuffer = Arc<Mutex<OutputBufferState>>;
type OutputHandles = (OutputBuffer, Arc<Notify>);

fn resolve_window_bounds(
    total_bytes: u64,
    window: UnifiedExecOutputWindow,
) -> (u64, u64, bool, bool, usize) {
    if total_bytes == 0 {
        return (0, 0, false, false, 0);
    }

    let max_bytes = window.clamp_bytes().min(total_bytes as usize);

    match window {
        UnifiedExecOutputWindow::Tail { .. } => {
            let end = total_bytes;
            let start = end.saturating_sub(max_bytes as u64);
            let actual = (end - start) as usize;
            (start, end, start > 0, false, actual)
        }
        UnifiedExecOutputWindow::Range { start, .. } => {
            let clamped_start = start.min(total_bytes);
            let end = (clamped_start + max_bytes as u64).min(total_bytes);
            let actual = (end - clamped_start) as usize;
            (
                clamped_start,
                end,
                clamped_start > 0,
                end < total_bytes,
                actual,
            )
        }
    }
}

#[derive(Debug)]
struct OutputSpool {
    file: Arc<StdMutex<std::fs::File>>,
    total_bytes: AtomicU64,
    failed: AtomicBool,
}

impl OutputSpool {
    fn new() -> Result<Self, std::io::Error> {
        let file = tempfile()?;
        Ok(Self {
            file: Arc::new(StdMutex::new(file)),
            total_bytes: AtomicU64::new(0),
            failed: AtomicBool::new(false),
        })
    }

    fn append(&self, chunk: &[u8]) -> Result<(), std::io::Error> {
        if self.failed.load(Ordering::SeqCst) {
            return Ok(());
        }
        let mut guard = self
            .file
            .lock()
            .map_err(|_| std::io::Error::other("spool poisoned"))?;
        if let Err(err) = guard.write_all(chunk) {
            self.failed.store(true, Ordering::SeqCst);
            return Err(err);
        }
        if let Err(err) = guard.flush() {
            self.failed.store(true, Ordering::SeqCst);
            return Err(err);
        }
        self.total_bytes
            .fetch_add(chunk.len() as u64, Ordering::SeqCst);
        Ok(())
    }

    fn len(&self) -> u64 {
        self.total_bytes.load(Ordering::SeqCst)
    }

    fn read_range(&self, start: u64, max_bytes: usize) -> Result<Vec<u8>, std::io::Error> {
        if self.failed.load(Ordering::SeqCst) {
            return Err(std::io::Error::other("spool unavailable"));
        }
        let file = {
            let guard = self
                .file
                .lock()
                .map_err(|_| std::io::Error::other("spool poisoned"))?;
            guard.try_clone().inspect_err(|_| {
                self.failed.store(true, Ordering::SeqCst);
            })?
        };
        let mut reader = std::io::BufReader::new(file);
        reader.seek(SeekFrom::Start(start)).inspect_err(|_| {
            self.failed.store(true, Ordering::SeqCst);
        })?;
        let mut take = reader.take(max_bytes as u64);
        let mut buf = Vec::with_capacity(max_bytes);
        take.read_to_end(&mut buf).inspect_err(|_| {
            self.failed.store(true, Ordering::SeqCst);
        })?;
        Ok(buf)
    }

    fn copy_to_path<P: AsRef<std::path::Path>>(
        &self,
        destination: P,
    ) -> Result<(), std::io::Error> {
        if self.failed.load(Ordering::SeqCst) {
            return Err(std::io::Error::other("spool unavailable"));
        }
        let file = {
            let guard = self
                .file
                .lock()
                .map_err(|_| std::io::Error::other("spool poisoned"))?;
            guard.try_clone().inspect_err(|_| {
                self.failed.store(true, Ordering::SeqCst);
            })?
        };
        let mut reader = std::io::BufReader::new(file);
        reader.seek(SeekFrom::Start(0)).inspect_err(|_| {
            self.failed.store(true, Ordering::SeqCst);
        })?;

        if let Some(parent) = destination.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut writer = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(destination)?;
        std::io::copy(&mut reader, &mut writer).inspect_err(|_| {
            self.failed.store(true, Ordering::SeqCst);
        })?;
        writer.flush().inspect_err(|_| {
            self.failed.store(true, Ordering::SeqCst);
        })?;
        Ok(())
    }

    fn is_available(&self) -> bool {
        !self.failed.load(Ordering::SeqCst)
    }
}

impl ManagedUnifiedExecSession {
    fn new(
        command: Vec<String>,
        session: ExecCommandSession,
        initial_output_rx: tokio::sync::broadcast::Receiver<Vec<u8>>,
    ) -> Self {
        let output_buffer = Arc::new(Mutex::new(OutputBufferState::default()));
        let output_notify = Arc::new(Notify::new());
        let output_spool = match OutputSpool::new() {
            Ok(spool) => Some(Arc::new(spool)),
            Err(err) => {
                warn!(error = ?err, "failed to initialize unified exec spool; falling back to in-memory buffer only");
                None
            }
        };
        let mut receiver = initial_output_rx;
        let buffer_clone = Arc::clone(&output_buffer);
        let notify_clone = Arc::clone(&output_notify);
        let spool_clone = output_spool.clone();
        let output_task = tokio::spawn(async move {
            loop {
                match receiver.recv().await {
                    Ok(chunk) => {
                        if let Some(spool) = spool_clone.as_ref()
                            && let Err(err) = spool.append(&chunk)
                        {
                            error!(error = ?err, "failed to persist unified exec output; continuing without spool");
                        }
                        let mut guard = buffer_clone.lock().await;
                        guard.push_chunk(chunk);
                        drop(guard);
                        notify_clone.notify_waiters();
                    }
                    // If we lag behind the broadcast buffer, skip missed
                    // messages but keep the task alive to continue streaming.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        continue;
                    }
                    // When the sender closes, exit the task.
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        Self {
            command,
            started_at: SystemTime::now(),
            session,
            output_buffer,
            output_notify,
            output_task,
            output_spool,
        }
    }

    fn writer_sender(&self) -> mpsc::Sender<Vec<u8>> {
        self.session.writer_sender()
    }

    fn output_handles(&self) -> OutputHandles {
        (
            Arc::clone(&self.output_buffer),
            Arc::clone(&self.output_notify),
        )
    }

    fn has_exited(&self) -> bool {
        self.session.has_exited()
    }

    async fn snapshot(&self, session_id: i32) -> UnifiedExecSessionSnapshot {
        let (aggregated, last_output_at, truncated_prefix) = {
            let guard = self.output_buffer.lock().await;
            (
                guard.snapshot_bytes(),
                guard.last_output_at(),
                guard.was_truncated(),
            )
        };
        let preview_raw = String::from_utf8_lossy(&aggregated);
        let (preview, maybe_tokens) = truncate_middle(&preview_raw, UNIFIED_EXEC_PREVIEW_MAX_BYTES);
        let output_truncated = truncated_prefix || maybe_tokens.is_some();

        UnifiedExecSessionSnapshot {
            session_id,
            command: self.command.clone(),
            started_at: self.started_at,
            last_output_at,
            has_exited: self.has_exited(),
            output_preview: preview,
            output_truncated,
        }
    }

    async fn output_window(
        &self,
        session_id: i32,
        window: UnifiedExecOutputWindow,
    ) -> Result<UnifiedExecSessionOutput, UnifiedExecError> {
        let (tail_bytes, last_output_at, ring_truncated) = {
            let guard = self.output_buffer.lock().await;
            (
                guard.snapshot_bytes(),
                guard.last_output_at(),
                guard.was_truncated(),
            )
        };

        let status = if self.has_exited() {
            UnifiedExecSessionStatus::Exited
        } else {
            UnifiedExecSessionStatus::Running
        };

        if let Some(spool) = &self.output_spool
            && spool.is_available()
        {
            let total_bytes = spool.len();
            let (range_start, range_end, truncated_prefix, truncated_suffix, window_bytes) =
                resolve_window_bounds(total_bytes, window);

            let bytes = if window_bytes == 0 {
                Ok(Vec::new())
            } else {
                let spool_clone = Arc::clone(spool);
                let read_len = window_bytes;
                tokio::task::spawn_blocking(move || spool_clone.read_range(range_start, read_len))
                    .await
                    .map_err(|err| {
                        UnifiedExecError::read_output(std::io::Error::other(err.to_string()))
                    })?
            };

            match bytes {
                Ok(bytes) => {
                    let content = String::from_utf8_lossy(&bytes).into_owned();
                    return Ok(UnifiedExecSessionOutput {
                        session_id,
                        command: self.command.clone(),
                        started_at: self.started_at,
                        last_output_at,
                        status,
                        content,
                        truncated: truncated_prefix,
                        truncated_suffix,
                        expandable_prefix: truncated_prefix,
                        expandable_suffix: truncated_suffix,
                        range_start,
                        range_end,
                        total_bytes,
                        window_bytes,
                    });
                }
                Err(err) => {
                    warn!(
                        error = ?err,
                        "failed to read unified exec spool; falling back to in-memory tail"
                    );
                }
            }
        }

        let total_bytes = tail_bytes.len() as u64;
        let content = String::from_utf8_lossy(&tail_bytes).into_owned();
        let window_bytes = tail_bytes.len();
        let range_end = total_bytes;
        let range_start = range_end.saturating_sub(window_bytes as u64);

        Ok(UnifiedExecSessionOutput {
            session_id,
            command: self.command.clone(),
            started_at: self.started_at,
            last_output_at,
            status,
            content,
            truncated: ring_truncated,
            truncated_suffix: false,
            expandable_prefix: false,
            expandable_suffix: false,
            range_start,
            range_end,
            total_bytes,
            window_bytes,
        })
    }

    fn kill(&self) {
        self.session.kill();
    }
}

impl Drop for ManagedUnifiedExecSession {
    fn drop(&mut self) {
        self.output_task.abort();
    }
}

impl UnifiedExecSessionManager {
    pub async fn snapshot(&self) -> Vec<UnifiedExecSessionSnapshot> {
        let sessions = {
            let guard = self.sessions.lock().await;
            guard
                .iter()
                .map(|(id, session)| (*id, Arc::clone(session)))
                .collect::<Vec<_>>()
        };

        let mut snapshots = Vec::with_capacity(sessions.len());
        for (id, session) in sessions {
            snapshots.push(session.snapshot(id).await);
        }
        snapshots.sort_by_key(|snapshot| snapshot.session_id);
        snapshots
    }

    pub async fn session_output_window(
        &self,
        session_id: i32,
        window: UnifiedExecOutputWindow,
    ) -> Option<UnifiedExecSessionOutput> {
        let session = {
            let guard = self.sessions.lock().await;
            guard.get(&session_id).cloned()
        }?;

        match session.output_window(session_id, window).await {
            Ok(output) => Some(output),
            Err(err) => {
                warn!(
                    error = ?err,
                    session_id,
                    "failed to load unified exec output window"
                );
                None
            }
        }
    }

    pub async fn export_session_log<P: AsRef<std::path::Path>>(
        &self,
        session_id: i32,
        destination: P,
    ) -> Result<(), UnifiedExecError> {
        let session = {
            let guard = self.sessions.lock().await;
            guard.get(&session_id).cloned()
        }
        .ok_or(UnifiedExecError::UnknownSessionId { session_id })?;

        let destination = destination.as_ref().to_path_buf();

        if let Some(spool) = session.output_spool.as_ref()
            && spool.is_available()
        {
            let spool_clone = Arc::clone(spool);
            let result = tokio::task::spawn_blocking(move || spool_clone.copy_to_path(destination))
                .await
                .map_err(|err| {
                    UnifiedExecError::export_log(std::io::Error::other(err.to_string()))
                })?;
            return result.map_err(UnifiedExecError::export_log);
        }

        let output = session
            .output_window(
                session_id,
                UnifiedExecOutputWindow::Range {
                    start: 0,
                    max_bytes: UNIFIED_EXEC_WINDOW_MAX_BYTES,
                },
            )
            .await?;

        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(UnifiedExecError::export_log)?;
        }

        let mut file = TokioFile::create(destination.clone())
            .await
            .map_err(UnifiedExecError::export_log)?;
        file.write_all(output.content.as_bytes())
            .await
            .map_err(UnifiedExecError::export_log)?;
        file.flush().await.map_err(UnifiedExecError::export_log)?;
        Ok(())
    }

    pub async fn kill_session(&self, session_id: i32) -> bool {
        let session = {
            let guard = self.sessions.lock().await;
            guard.get(&session_id).cloned()
        };
        if let Some(session) = session {
            session.kill();
            true
        } else {
            false
        }
    }

    pub async fn remove_session(&self, session_id: i32) -> bool {
        let session = self.sessions.lock().await.remove(&session_id);
        if let Some(session) = session {
            session.kill();
            true
        } else {
            false
        }
    }

    pub async fn handle_request(
        &self,
        request: UnifiedExecRequest<'_>,
    ) -> Result<UnifiedExecResult, UnifiedExecError> {
        let (timeout_ms, timeout_warning) = match request.timeout_ms {
            Some(requested) if requested > MAX_TIMEOUT_MS => (
                MAX_TIMEOUT_MS,
                Some(format!(
                    "Warning: requested timeout {requested}ms exceeds maximum of {MAX_TIMEOUT_MS}ms; clamping to {MAX_TIMEOUT_MS}ms.\n"
                )),
            ),
            Some(requested) => (requested, None),
            None => (DEFAULT_TIMEOUT_MS, None),
        };

        let mut new_session: Option<Arc<ManagedUnifiedExecSession>> = None;
        let session_id;
        let writer_tx;
        let output_buffer;
        let output_notify;

        if let Some(existing_id) = request.session_id {
            let session = {
                let mut sessions = self.sessions.lock().await;
                match sessions.get(&existing_id) {
                    Some(session) if !session.has_exited() => Arc::clone(session),
                    Some(_) => {
                        sessions.remove(&existing_id);
                        return Err(UnifiedExecError::UnknownSessionId {
                            session_id: existing_id,
                        });
                    }
                    None => {
                        return Err(UnifiedExecError::UnknownSessionId {
                            session_id: existing_id,
                        });
                    }
                }
            };
            let (buffer, notify) = session.output_handles();
            session_id = existing_id;
            writer_tx = session.writer_sender();
            output_buffer = buffer;
            output_notify = notify;
        } else {
            let command = request.input_chunks.to_vec();
            let new_id = self.next_session_id.fetch_add(1, Ordering::SeqCst);
            let (session, initial_output_rx) = create_unified_exec_session(&command).await?;
            let managed_session = Arc::new(ManagedUnifiedExecSession::new(
                command,
                session,
                initial_output_rx,
            ));
            let (buffer, notify) = managed_session.output_handles();
            writer_tx = managed_session.writer_sender();
            output_buffer = buffer;
            output_notify = notify;
            session_id = new_id;
            new_session = Some(managed_session);
        };

        if request.session_id.is_some() {
            let joined_input = request.input_chunks.join(" ");
            if !joined_input.is_empty() && writer_tx.send(joined_input.into_bytes()).await.is_err()
            {
                return Err(UnifiedExecError::WriteToStdin);
            }
        }

        let mut collected: Vec<u8> = Vec::with_capacity(4096);
        let start = Instant::now();
        let deadline = start + Duration::from_millis(timeout_ms);

        loop {
            let drained_chunks;
            let mut wait_for_output = None;
            {
                let mut guard = output_buffer.lock().await;
                drained_chunks = guard.drain();
                if drained_chunks.is_empty() {
                    wait_for_output = Some(output_notify.notified());
                }
            }

            if drained_chunks.is_empty() {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining == Duration::ZERO {
                    break;
                }

                let notified = wait_for_output.unwrap_or_else(|| output_notify.notified());
                tokio::pin!(notified);
                tokio::select! {
                    _ = &mut notified => {}
                    _ = tokio::time::sleep(remaining) => break,
                }
                continue;
            }

            for chunk in drained_chunks {
                collected.extend_from_slice(&chunk);
            }

            if Instant::now() >= deadline {
                break;
            }
        }

        let (output, _maybe_tokens) = truncate_middle(
            &String::from_utf8_lossy(&collected),
            UNIFIED_EXEC_OUTPUT_MAX_BYTES,
        );
        let output = if let Some(warning) = timeout_warning {
            format!("{warning}{output}")
        } else {
            output
        };

        let should_store_session = if let Some(session) = new_session.as_ref() {
            !session.has_exited()
        } else if request.session_id.is_some() {
            let mut sessions = self.sessions.lock().await;
            if let Some(existing) = sessions.get(&session_id) {
                if existing.has_exited() {
                    sessions.remove(&session_id);
                    false
                } else {
                    true
                }
            } else {
                false
            }
        } else {
            true
        };

        if should_store_session {
            if let Some(session) = new_session {
                self.sessions.lock().await.insert(session_id, session);
            }
            Ok(UnifiedExecResult {
                session_id: Some(session_id),
                output,
            })
        } else {
            Ok(UnifiedExecResult {
                session_id: None,
                output,
            })
        }
    }
}

async fn create_unified_exec_session(
    command: &[String],
) -> Result<
    (
        ExecCommandSession,
        tokio::sync::broadcast::Receiver<Vec<u8>>,
    ),
    UnifiedExecError,
> {
    if command.is_empty() {
        return Err(UnifiedExecError::MissingCommandLine);
    }

    let pty_system = native_pty_system();

    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(UnifiedExecError::create_session)?;

    // Safe thanks to the check at the top of the function.
    let mut command_builder = CommandBuilder::new(command[0].clone());
    for arg in &command[1..] {
        command_builder.arg(arg);
    }

    let mut child = pair
        .slave
        .spawn_command(command_builder)
        .map_err(UnifiedExecError::create_session)?;
    let killer = child.clone_killer();

    let (writer_tx, mut writer_rx) = mpsc::channel::<Vec<u8>>(128);
    let (output_tx, _) = tokio::sync::broadcast::channel::<Vec<u8>>(256);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(UnifiedExecError::create_session)?;
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

    let writer = pair
        .master
        .take_writer()
        .map_err(UnifiedExecError::create_session)?;
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

    let exit_status = Arc::new(AtomicBool::new(false));
    let wait_exit_status = Arc::clone(&exit_status);
    let wait_handle = tokio::task::spawn_blocking(move || {
        let _ = child.wait();
        wait_exit_status.store(true, Ordering::SeqCst);
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
    Ok((session, initial_output_rx))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use core_test_support::skip_if_sandbox;
    use tempfile::tempdir;

    #[test]
    fn push_chunk_trims_only_excess_bytes() {
        let mut buffer = OutputBufferState::default();
        buffer.push_chunk(vec![b'a'; UNIFIED_EXEC_OUTPUT_MAX_BYTES]);
        buffer.push_chunk(vec![b'b']);
        buffer.push_chunk(vec![b'c']);

        assert_eq!(buffer.total_bytes, UNIFIED_EXEC_OUTPUT_MAX_BYTES);
        assert_eq!(buffer.chunks.len(), 3);
        assert_eq!(
            buffer.chunks.front().unwrap().len(),
            UNIFIED_EXEC_OUTPUT_MAX_BYTES - 2
        );
        assert_eq!(buffer.chunks.pop_back().unwrap(), vec![b'c']);
        assert_eq!(buffer.chunks.pop_back().unwrap(), vec![b'b']);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unified_exec_persists_across_requests_jif() -> Result<(), UnifiedExecError> {
        skip_if_sandbox!(Ok(()));

        let manager = UnifiedExecSessionManager::default();

        let open_shell = manager
            .handle_request(UnifiedExecRequest {
                session_id: None,
                input_chunks: &["bash".to_string(), "-i".to_string()],
                timeout_ms: Some(2_500),
            })
            .await?;
        let session_id = open_shell.session_id.expect("expected session_id");

        manager
            .handle_request(UnifiedExecRequest {
                session_id: Some(session_id),
                input_chunks: &[
                    "export".to_string(),
                    "CODEX_INTERACTIVE_SHELL_VAR=codex\n".to_string(),
                ],
                timeout_ms: Some(2_500),
            })
            .await?;

        let out_2 = manager
            .handle_request(UnifiedExecRequest {
                session_id: Some(session_id),
                input_chunks: &["echo $CODEX_INTERACTIVE_SHELL_VAR\n".to_string()],
                timeout_ms: Some(2_500),
            })
            .await?;
        assert!(out_2.output.contains("codex"));

        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn multi_unified_exec_sessions() -> Result<(), UnifiedExecError> {
        skip_if_sandbox!(Ok(()));

        let manager = UnifiedExecSessionManager::default();

        let shell_a = manager
            .handle_request(UnifiedExecRequest {
                session_id: None,
                input_chunks: &["/bin/bash".to_string(), "-i".to_string()],
                timeout_ms: Some(2_500),
            })
            .await?;
        let session_a = shell_a.session_id.expect("expected session id");

        manager
            .handle_request(UnifiedExecRequest {
                session_id: Some(session_a),
                input_chunks: &["export CODEX_INTERACTIVE_SHELL_VAR=codex\n".to_string()],
                timeout_ms: Some(2_500),
            })
            .await?;

        let out_2 = manager
            .handle_request(UnifiedExecRequest {
                session_id: None,
                input_chunks: &[
                    "echo".to_string(),
                    "$CODEX_INTERACTIVE_SHELL_VAR\n".to_string(),
                ],
                timeout_ms: Some(2_500),
            })
            .await?;
        assert!(!out_2.output.contains("codex"));

        let out_3 = manager
            .handle_request(UnifiedExecRequest {
                session_id: Some(session_a),
                input_chunks: &["echo $CODEX_INTERACTIVE_SHELL_VAR\n".to_string()],
                timeout_ms: Some(2_500),
            })
            .await?;
        assert!(out_3.output.contains("codex"));

        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unified_exec_timeouts() -> Result<(), UnifiedExecError> {
        skip_if_sandbox!(Ok(()));

        let manager = UnifiedExecSessionManager::default();

        let open_shell = manager
            .handle_request(UnifiedExecRequest {
                session_id: None,
                input_chunks: &["bash".to_string(), "-i".to_string()],
                timeout_ms: Some(2_500),
            })
            .await?;
        let session_id = open_shell.session_id.expect("expected session id");

        manager
            .handle_request(UnifiedExecRequest {
                session_id: Some(session_id),
                input_chunks: &[
                    "export".to_string(),
                    "CODEX_INTERACTIVE_SHELL_VAR=codex\n".to_string(),
                ],
                timeout_ms: Some(2_500),
            })
            .await?;

        let out_2 = manager
            .handle_request(UnifiedExecRequest {
                session_id: Some(session_id),
                input_chunks: &["sleep 5 && echo $CODEX_INTERACTIVE_SHELL_VAR\n".to_string()],
                timeout_ms: Some(10),
            })
            .await?;
        assert!(!out_2.output.contains("codex"));

        tokio::time::sleep(Duration::from_secs(7)).await;

        let empty = Vec::new();
        let out_3 = manager
            .handle_request(UnifiedExecRequest {
                session_id: Some(session_id),
                input_chunks: &empty,
                timeout_ms: Some(100),
            })
            .await?;

        assert!(out_3.output.contains("codex"));

        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    #[ignore] // Ignored while we have a better way to test this.
    async fn requests_with_large_timeout_are_capped() -> Result<(), UnifiedExecError> {
        let manager = UnifiedExecSessionManager::default();

        let result = manager
            .handle_request(UnifiedExecRequest {
                session_id: None,
                input_chunks: &["echo".to_string(), "codex".to_string()],
                timeout_ms: Some(120_000),
            })
            .await?;

        assert!(result.output.starts_with(
            "Warning: requested timeout 120000ms exceeds maximum of 60000ms; clamping to 60000ms.\n"
        ));
        assert!(result.output.contains("codex"));

        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    #[ignore] // Ignored while we have a better way to test this.
    async fn completed_commands_do_not_persist_sessions() -> Result<(), UnifiedExecError> {
        let manager = UnifiedExecSessionManager::default();
        let result = manager
            .handle_request(UnifiedExecRequest {
                session_id: None,
                input_chunks: &["/bin/echo".to_string(), "codex".to_string()],
                timeout_ms: Some(2_500),
            })
            .await?;

        assert!(result.session_id.is_none());
        assert!(result.output.contains("codex"));

        assert!(manager.sessions.lock().await.is_empty());

        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reusing_completed_session_returns_unknown_session() -> Result<(), UnifiedExecError> {
        skip_if_sandbox!(Ok(()));

        let manager = UnifiedExecSessionManager::default();

        let open_shell = manager
            .handle_request(UnifiedExecRequest {
                session_id: None,
                input_chunks: &["/bin/bash".to_string(), "-i".to_string()],
                timeout_ms: Some(2_500),
            })
            .await?;
        let session_id = open_shell.session_id.expect("expected session id");

        manager
            .handle_request(UnifiedExecRequest {
                session_id: Some(session_id),
                input_chunks: &["exit\n".to_string()],
                timeout_ms: Some(2_500),
            })
            .await?;

        tokio::time::sleep(Duration::from_millis(200)).await;

        let err = manager
            .handle_request(UnifiedExecRequest {
                session_id: Some(session_id),
                input_chunks: &[],
                timeout_ms: Some(100),
            })
            .await
            .expect_err("expected unknown session error");

        match err {
            UnifiedExecError::UnknownSessionId { session_id: err_id } => {
                assert_eq!(err_id, session_id);
            }
            other => panic!("expected UnknownSessionId, got {other:?}"),
        }

        assert!(!manager.sessions.lock().await.contains_key(&session_id));

        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unified_exec_spool_persists_full_output() -> Result<(), UnifiedExecError> {
        skip_if_sandbox!(Ok(()));

        let manager = UnifiedExecSessionManager::default();

        let start = manager
            .handle_request(UnifiedExecRequest {
                session_id: None,
                input_chunks: &["/bin/sh".to_string(), "-i".to_string()],
                timeout_ms: Some(2_500),
            })
            .await?;
        let session_id = start.session_id.expect("expected session id");

        let count = 25_000;
        let script = format!("for i in $(seq 1 {count}); do printf \"L%05d\\n\" \"$i\"; done\n");

        manager
            .handle_request(UnifiedExecRequest {
                session_id: Some(session_id),
                input_chunks: &[script],
                timeout_ms: Some(5_000),
            })
            .await?;

        let tail = manager
            .session_output_window(
                session_id,
                UnifiedExecOutputWindow::Tail {
                    max_bytes: UNIFIED_EXEC_WINDOW_DEFAULT_BYTES,
                },
            )
            .await
            .expect("tail output available");
        assert!(tail.truncated, "expected tail to report truncation");
        assert!(tail.expandable_prefix, "expected tail to expand backwards");

        let prefix = manager
            .session_output_window(
                session_id,
                UnifiedExecOutputWindow::Range {
                    start: 0,
                    max_bytes: 24 * 1024,
                },
            )
            .await
            .expect("prefix output available");
        assert_eq!(prefix.range_start, 0, "expected window to start at head");
        assert!(
            prefix.content.contains("L00001"),
            "expected prefix window to include earliest lines"
        );
        assert!(prefix.truncated_suffix);
        assert!(prefix.expandable_suffix);

        let tmp_dir = tempdir().expect("tempdir");
        let export_path = tmp_dir.path().join("session.log");
        manager
            .export_session_log(session_id, export_path.clone())
            .await?;
        let exported = std::fs::read(&export_path).expect("read exported log");
        assert!(
            exported.len() as u64 >= tail.total_bytes,
            "expected exported log to contain at least tail bytes"
        );
        let exported_text = String::from_utf8_lossy(&exported);
        assert!(
            exported_text.contains("L00001"),
            "expected export to include earliest lines"
        );
        assert!(
            exported_text.contains(&format!("L{count:05}")),
            "expected export to include final lines"
        );

        manager.remove_session(session_id).await;

        Ok(())
    }
}
