mod codex;

use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use time::OffsetDateTime;
use uuid::Uuid;

use eyre::{Result, eyre};

pub use codex::CodexBackend;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionId(Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Debug)]
pub struct SessionDescriptor {
    pub id: SessionId,
    pub title: String,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug)]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

#[derive(Clone, Debug)]
pub struct HistoryItem {
    pub id: Uuid,
    pub role: MessageRole,
    pub content: String,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug)]
pub struct PromptPayload {
    pub text: String,
}

#[derive(Clone, Debug)]
pub enum HistoryEvent {
    Snapshot(Vec<HistoryItem>),
    Append(HistoryItem),
    Update { id: Uuid, content: String },
    Delta { id: Uuid, chunk: String },
}

#[derive(Clone, Debug)]
pub enum StatusEvent {
    Info(String),
    TaskStarted,
    TaskComplete,
}

#[derive(Clone, Debug)]
pub enum BackendEvent {
    History(HistoryEvent),
    Status(StatusEvent),
    Error(String),
}

#[derive(Clone, Debug)]
pub struct SessionRequest {
    pub title: Option<String>,
    pub seed_messages: Vec<SeedMessage>,
}

impl Default for SessionRequest {
    fn default() -> Self {
        Self {
            title: Some("Новая сессия".to_owned()),
            seed_messages: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SeedMessage {
    pub role: MessageRole,
    pub content: String,
}

impl SeedMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
        }
    }
}

pub trait Backend: Send + Sync {
    fn list_sessions(&self) -> Result<Vec<SessionDescriptor>>;
    fn spawn_session(&self, request: SessionRequest) -> Result<SessionDescriptor>;
    fn subscribe(&self, session_id: &SessionId) -> Result<SessionStream>;
    fn list_history(&self, session_id: &SessionId) -> Result<Vec<HistoryItem>>;
    fn send_prompt(&self, session_id: &SessionId, prompt: PromptPayload) -> Result<()>;
}

#[derive(Clone)]
pub struct AppServiceHandle {
    backend: Arc<dyn Backend>,
}

impl AppServiceHandle {
    pub fn new(backend: Arc<dyn Backend>) -> Self {
        Self { backend }
    }

    pub fn mock() -> Self {
        Self::new(Arc::new(MockBackend::new()))
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionDescriptor>> {
        self.backend.list_sessions()
    }

    pub fn spawn_session(&self, request: SessionRequest) -> Result<SessionDescriptor> {
        self.backend.spawn_session(request)
    }

    pub fn subscribe(&self, session_id: &SessionId) -> Result<SessionStream> {
        self.backend.subscribe(session_id)
    }

    pub fn list_history(&self, session_id: &SessionId) -> Result<Vec<HistoryItem>> {
        self.backend.list_history(session_id)
    }

    pub fn send_prompt(&self, session_id: &SessionId, prompt: PromptPayload) -> Result<()> {
        self.backend.send_prompt(session_id, prompt)
    }
}

#[derive(Clone, Debug)]
pub struct SessionStream {
    inner: Receiver<BackendEvent>,
}

impl SessionStream {
    pub fn try_next(&self) -> Option<BackendEvent> {
        self.inner.try_recv().ok()
    }

    pub fn blocking_next(&self) -> Option<BackendEvent> {
        self.inner.recv().ok()
    }
}

#[derive(Default)]
struct AppState {
    sessions: RwLock<HashMap<SessionId, SessionRecord>>,
}

struct SessionRecord {
    descriptor: SessionDescriptor,
    history: Vec<HistoryItem>,
    subscribers: Vec<Sender<BackendEvent>>,
}

struct MockBackend {
    state: Arc<AppState>,
}

impl MockBackend {
    fn new() -> Self {
        let backend = Self {
            state: Arc::new(AppState::default()),
        };
        backend.bootstrap_demo_session();
        backend
    }

    fn bootstrap_demo_session(&self) {
        let mut request = SessionRequest::default();
        request.title = Some("Добро пожаловать".to_owned());
        request.seed_messages = vec![
            SeedMessage::system("Codex Desktop GUI — ранний предпросмотр"),
            SeedMessage::assistant(
                "Привет! Я помогу тебе программировать в новом десктопном интерфейсе. ".to_owned()
                    + "Готов обсудить задачи, посмотреть git-диффы и запустить команды.",
            ),
        ];
        let _ = self.spawn_session(request);
    }

    fn broadcast(&self, session_id: &SessionId, event: BackendEvent) {
        let mut guard = self.state.sessions.write();
        if let Some(session) = guard.get_mut(session_id) {
            session
                .subscribers
                .retain(|sender| sender.send(event.clone()).is_ok());
        }
    }
}

impl Backend for MockBackend {
    fn list_sessions(&self) -> Result<Vec<SessionDescriptor>> {
        Ok(self
            .state
            .sessions
            .read()
            .values()
            .map(|record| record.descriptor.clone())
            .collect())
    }

    fn spawn_session(&self, request: SessionRequest) -> Result<SessionDescriptor> {
        let id = SessionId::new();
        let now = OffsetDateTime::now_utc();
        let descriptor = SessionDescriptor {
            id: id.clone(),
            title: request.title.unwrap_or_else(|| "Новая сессия".to_owned()),
            created_at: now,
        };
        let mut history = Vec::new();
        for seed in request.seed_messages {
            let item = HistoryItem {
                id: Uuid::new_v4(),
                role: seed.role,
                content: seed.content,
                created_at: now,
            };
            history.push(item.clone());
        }
        self.state.sessions.write().insert(
            id.clone(),
            SessionRecord {
                descriptor: descriptor.clone(),
                history: history.clone(),
                subscribers: Vec::new(),
            },
        );
        if !history.is_empty() {
            self.broadcast(
                &id,
                BackendEvent::History(HistoryEvent::Snapshot(history.clone())),
            );
        }
        Ok(descriptor)
    }

    fn subscribe(&self, session_id: &SessionId) -> Result<SessionStream> {
        let (tx, rx) = unbounded();
        let mut guard = self.state.sessions.write();
        let Some(session) = guard.get_mut(session_id) else {
            return Err(eyre!("Session {} not found", session_id));
        };
        // Immediately send snapshot for new subscriber
        tx.send(BackendEvent::History(HistoryEvent::Snapshot(
            session.history.clone(),
        )))
        .ok();
        session.subscribers.push(tx);
        Ok(SessionStream { inner: rx })
    }

    fn list_history(&self, session_id: &SessionId) -> Result<Vec<HistoryItem>> {
        let guard = self.state.sessions.read();
        let Some(session) = guard.get(session_id) else {
            return Err(eyre!("Session {} not found", session_id));
        };
        Ok(session.history.clone())
    }

    fn send_prompt(&self, session_id: &SessionId, prompt: PromptPayload) -> Result<()> {
        let now = OffsetDateTime::now_utc();
        let mut guard = self.state.sessions.write();
        let Some(session) = guard.get_mut(session_id) else {
            return Err(eyre!("Session {} not found", session_id));
        };
        let user_item = HistoryItem {
            id: Uuid::new_v4(),
            role: MessageRole::User,
            content: prompt.text.clone(),
            created_at: now,
        };
        session.history.push(user_item.clone());
        drop(guard);
        self.broadcast(
            session_id,
            BackendEvent::History(HistoryEvent::Append(user_item.clone())),
        );

        let synthetic = synthesize_reply(&prompt.text);
        let assistant_item = HistoryItem {
            id: Uuid::new_v4(),
            role: MessageRole::Assistant,
            content: synthetic,
            created_at: OffsetDateTime::now_utc(),
        };
        self.state
            .sessions
            .write()
            .get_mut(session_id)
            .map(|session| session.history.push(assistant_item.clone()));
        self.broadcast(
            session_id,
            BackendEvent::History(HistoryEvent::Append(assistant_item)),
        );
        Ok(())
    }
}

fn synthesize_reply(prompt: &str) -> String {
    if prompt.trim().is_empty() {
        return "Похоже, сообщение пустое. Добавь контекст, и я подключусь.".to_owned();
    }

    let guidance = [
        "Я зафиксировал задачу в панели Insight. Готов добавить план действий.",
        "Давай разобьём работу на шаги и проверим git-диффы.",
        "Я подготовил быстрый предпросмотр. Проверь и скажи, что улучшить.",
    ];
    let hint = guidance[prompt.len() % guidance.len()];
    format!("Получил запрос: “{prompt}”. {hint}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_and_push_messages() {
        let handle = AppServiceHandle::mock();
        let sessions = handle.list_sessions();
        assert!(!sessions.is_empty());
        let descriptor = handle
            .spawn_session(SessionRequest::default())
            .expect("session created");
        handle
            .send_prompt(
                &descriptor.id,
                PromptPayload {
                    text: "ping".into(),
                },
            )
            .expect("send prompt");
        let history = handle
            .list_history(&descriptor.id)
            .expect("history available");
        assert!(history.iter().any(|h| matches!(h.role, MessageRole::User)));
        assert!(
            history
                .iter()
                .any(|h| matches!(h.role, MessageRole::Assistant))
        );
    }
}
