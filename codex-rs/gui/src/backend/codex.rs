use super::{
    Backend, BackendEvent, HistoryEvent, HistoryItem, MessageRole, PromptPayload,
    SessionDescriptor, SessionId, SessionRequest, SessionStream, StatusEvent,
};
use crossbeam_channel::{Sender, unbounded};
use eyre::{Result, eyre};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use time::OffsetDateTime;
use tokio::runtime::Runtime;
use uuid::Uuid;

use codex_core::CodexConversation;
use codex_core::ConversationManager;
use codex_core::NewConversation;
use codex_core::config::Config;
use codex_core::protocol::AgentMessageDeltaEvent;
use codex_core::protocol::AgentMessageEvent;
use codex_core::protocol::ErrorEvent;
use codex_core::protocol::Event;
use codex_core::protocol::EventMsg;
use codex_core::protocol::ExecCommandBeginEvent;
use codex_core::protocol::ExecCommandEndEvent;
use codex_core::protocol::ExecCommandOutputDeltaEvent;
use codex_core::protocol::ExecOutputStream;
use codex_core::protocol::InputItem;
use codex_core::protocol::Op;
use codex_core::protocol::SessionConfiguredEvent;
use codex_core::protocol::TaskCompleteEvent;
use codex_core::protocol::TaskStartedEvent;

pub struct CodexBackend {
    runtime: Arc<Runtime>,
    state: Arc<State>,
}

struct State {
    manager: Arc<ConversationManager>,
    config: Config,
    sessions: RwLock<HashMap<SessionId, SessionEntry>>,
}

struct SessionEntry {
    descriptor: SessionDescriptor,
    conversation: Arc<CodexConversation>,
    history: Vec<HistoryItem>,
    subscribers: Vec<Sender<BackendEvent>>,
    pending_assistants: HashMap<String, Uuid>,
    pending_exec: HashMap<String, Uuid>,
    status: Option<StatusEvent>,
}

impl CodexBackend {
    pub fn new(runtime: Arc<Runtime>, manager: Arc<ConversationManager>, config: Config) -> Self {
        Self {
            runtime,
            state: Arc::new(State {
                manager,
                config,
                sessions: RwLock::new(HashMap::new()),
            }),
        }
    }

    fn spawn_event_loop(&self, session_id: SessionId, conversation: Arc<CodexConversation>) {
        let state = self.state.clone();
        let runtime = self.runtime.clone();
        runtime.spawn(async move {
            // Forward events from Codex into GUI subscribers.
            while let Ok(event) = conversation.next_event().await {
                Self::dispatch_event(&state, &session_id, event);
            }
        });
    }

    fn dispatch_event(state: &State, session_id: &SessionId, event: Event) {
        match event.msg {
            EventMsg::AgentMessageDelta(AgentMessageDeltaEvent { delta }) => {
                Self::on_agent_delta(state, session_id, event.id, delta);
            }
            EventMsg::AgentMessage(AgentMessageEvent { message }) => {
                Self::on_agent_message(state, session_id, event.id, message);
            }
            EventMsg::TaskStarted(TaskStartedEvent { .. }) => {
                Self::set_status(state, session_id, StatusEvent::TaskStarted);
            }
            EventMsg::TaskComplete(TaskCompleteEvent { .. }) => {
                Self::set_status(state, session_id, StatusEvent::TaskComplete);
            }
            EventMsg::SessionConfigured(ev) => {
                Self::handle_session_configured(state, session_id, ev);
            }
            EventMsg::Error(ErrorEvent { message }) => {
                Self::broadcast(state, session_id, BackendEvent::Error(message));
            }
            EventMsg::ExecCommandBegin(ev) => {
                Self::on_exec_begin(state, session_id, ev);
            }
            EventMsg::ExecCommandOutputDelta(ev) => {
                Self::on_exec_output_delta(state, session_id, ev);
            }
            EventMsg::ExecCommandEnd(ev) => {
                Self::on_exec_end(state, session_id, ev);
            }
            // Ignore or TODO: map other events later.
            _ => {}
        }
    }

    fn handle_session_configured(
        state: &State,
        session_id: &SessionId,
        _event: SessionConfiguredEvent,
    ) {
        Self::set_status(
            state,
            session_id,
            StatusEvent::Info("Сессия готова".to_owned()),
        );
    }

    fn on_agent_delta(state: &State, session_id: &SessionId, submission_id: String, delta: String) {
        let mut events = Vec::new();
        {
            let mut sessions = state.sessions.write();
            if let Some(entry) = sessions.get_mut(session_id) {
                let now = OffsetDateTime::now_utc();
                let message_id = entry
                    .pending_assistants
                    .entry(submission_id.clone())
                    .or_insert_with(|| {
                        let item = HistoryItem {
                            id: Uuid::new_v4(),
                            role: MessageRole::Assistant,
                            content: String::new(),
                            created_at: now,
                        };
                        entry.history.push(item.clone());
                        events.push(BackendEvent::History(HistoryEvent::Append(item.clone())));
                        item.id
                    })
                    .clone();

                if let Some(item) = entry.history.iter_mut().find(|it| it.id == message_id) {
                    item.content.push_str(&delta);
                }

                events.push(BackendEvent::History(HistoryEvent::Delta {
                    id: message_id,
                    chunk: delta,
                }));
            }
        }
        for event in events {
            Self::broadcast(state, session_id, event);
        }
    }

    fn on_agent_message(
        state: &State,
        session_id: &SessionId,
        submission_id: String,
        message: String,
    ) {
        let mut events = Vec::new();
        {
            let mut sessions = state.sessions.write();
            if let Some(entry) = sessions.get_mut(session_id) {
                let message_id = entry
                    .pending_assistants
                    .entry(submission_id.clone())
                    .or_insert_with(|| {
                        let item = HistoryItem {
                            id: Uuid::new_v4(),
                            role: MessageRole::Assistant,
                            content: String::new(),
                            created_at: OffsetDateTime::now_utc(),
                        };
                        entry.history.push(item.clone());
                        events.push(BackendEvent::History(HistoryEvent::Append(item.clone())));
                        item.id
                    })
                    .clone();

                if let Some(item) = entry.history.iter_mut().find(|it| it.id == message_id) {
                    item.content = message.clone();
                }

                entry.pending_assistants.remove(&submission_id);
                events.push(BackendEvent::History(HistoryEvent::Update {
                    id: message_id,
                    content: message,
                }));
            }
        }
        for event in events {
            Self::broadcast(state, session_id, event);
        }
    }

    fn on_exec_begin(state: &State, session_id: &SessionId, event: ExecCommandBeginEvent) {
        let mut events = Vec::new();
        let mut status_event = None;
        {
            let mut sessions = state.sessions.write();
            if let Some(entry) = sessions.get_mut(session_id) {
                let now = OffsetDateTime::now_utc();
                let command_line = if event.command.is_empty() {
                    String::from("<пустая команда>")
                } else {
                    event.command.join(" ")
                };
                let mut content = String::from("$ ");
                content.push_str(&command_line);
                content.push('\n');
                let item = HistoryItem {
                    id: Uuid::new_v4(),
                    role: MessageRole::System,
                    content,
                    created_at: now,
                };
                entry.history.push(item.clone());
                entry.pending_exec.insert(event.call_id.clone(), item.id);
                let status = StatusEvent::Info(format!("Выполняется команда: {}", command_line));
                entry.status = Some(status.clone());
                status_event = Some(status);
                events.push(BackendEvent::History(HistoryEvent::Append(item)));
            }
        }
        if let Some(status) = status_event {
            events.push(BackendEvent::Status(status));
        }
        for event in events {
            Self::broadcast(state, session_id, event);
        }
    }

    fn on_exec_output_delta(
        state: &State,
        session_id: &SessionId,
        event: ExecCommandOutputDeltaEvent,
    ) {
        let mut events = Vec::new();
        let chunk_text = match event.stream {
            ExecOutputStream::Stdout => String::from_utf8_lossy(&event.chunk).into_owned(),
            ExecOutputStream::Stderr => {
                let payload = String::from_utf8_lossy(&event.chunk);
                format!("[stderr] {}", payload)
            }
        };

        {
            let mut sessions = state.sessions.write();
            if let Some(entry) = sessions.get_mut(session_id) {
                let message_id = entry
                    .pending_exec
                    .entry(event.call_id.clone())
                    .or_insert_with(|| {
                        let item = HistoryItem {
                            id: Uuid::new_v4(),
                            role: MessageRole::System,
                            content: String::new(),
                            created_at: OffsetDateTime::now_utc(),
                        };
                        entry.history.push(item.clone());
                        events.push(BackendEvent::History(HistoryEvent::Append(item.clone())));
                        item.id
                    })
                    .clone();

                if let Some(item) = entry.history.iter_mut().find(|it| it.id == message_id) {
                    item.content.push_str(&chunk_text);
                }

                events.push(BackendEvent::History(HistoryEvent::Delta {
                    id: message_id,
                    chunk: chunk_text,
                }));
            }
        }

        for event in events {
            Self::broadcast(state, session_id, event);
        }
    }

    fn on_exec_end(state: &State, session_id: &SessionId, event: ExecCommandEndEvent) {
        let mut events = Vec::new();
        let mut status_event = None;
        {
            let mut sessions = state.sessions.write();
            if let Some(entry) = sessions.get_mut(session_id) {
                let message_id = entry
                    .pending_exec
                    .remove(&event.call_id)
                    .unwrap_or_else(|| Uuid::new_v4());

                let summary = format!(
                    "\n[exit code: {} | длительность: {:?}]",
                    event.exit_code, event.duration
                );

                let formatted_output = if !event.formatted_output.is_empty() {
                    event.formatted_output.clone()
                } else if !event.aggregated_output.is_empty() {
                    event.aggregated_output.clone()
                } else if !event.stdout.is_empty() {
                    event.stdout.clone()
                } else {
                    event.stderr.clone()
                };

                if let Some(item) = entry.history.iter_mut().find(|it| it.id == message_id) {
                    if !formatted_output.is_empty() {
                        if !item.content.ends_with('\n') {
                            item.content.push('\n');
                        }
                        item.content.push_str(&formatted_output);
                    }
                    item.content.push_str(&summary);
                }

                events.push(BackendEvent::History(HistoryEvent::Update {
                    id: message_id,
                    content: entry
                        .history
                        .iter()
                        .find(|it| it.id == message_id)
                        .map(|it| it.content.clone())
                        .unwrap_or_default(),
                }));

                let status = if event.exit_code == 0 {
                    StatusEvent::Info("Команда успешно завершена".to_owned())
                } else {
                    StatusEvent::Info(format!("Команда завершилась с кодом {}", event.exit_code))
                };
                entry.status = Some(status.clone());
                status_event = Some(status);
            }
        }

        if let Some(status) = status_event {
            events.push(BackendEvent::Status(status));
        }

        for event in events {
            Self::broadcast(state, session_id, event);
        }
    }

    fn set_status(state: &State, session_id: &SessionId, status: StatusEvent) {
        let mut should_emit = false;
        {
            let mut sessions = state.sessions.write();
            if let Some(entry) = sessions.get_mut(session_id) {
                entry.status = Some(status.clone());
                should_emit = true;
            }
        }
        if should_emit {
            Self::broadcast(state, session_id, BackendEvent::Status(status));
        }
    }

    fn broadcast(state: &State, session_id: &SessionId, event: BackendEvent) {
        let mut sessions = state.sessions.write();
        if let Some(entry) = sessions.get_mut(session_id) {
            entry
                .subscribers
                .retain(|sender| sender.send(event.clone()).is_ok());
        }
    }
}

impl Backend for CodexBackend {
    fn list_sessions(&self) -> Result<Vec<SessionDescriptor>> {
        let sessions = self.state.sessions.read();
        Ok(sessions
            .values()
            .map(|entry| entry.descriptor.clone())
            .collect())
    }

    fn spawn_session(&self, request: SessionRequest) -> Result<SessionDescriptor> {
        let title = request.title.unwrap_or_else(|| "Новая сессия".to_owned());
        let config = self.state.config.clone();
        let manager = self.state.manager.clone();
        let conversation = self
            .runtime
            .block_on(manager.new_conversation(config))
            .map_err(|err| eyre!("Не удалось создать сессию Codex: {err}"))?;

        self.initialize_session(conversation, title, request.seed_messages)
    }

    fn subscribe(&self, session_id: &SessionId) -> Result<SessionStream> {
        let (tx, rx) = unbounded();
        let mut sessions = self.state.sessions.write();
        let Some(entry) = sessions.get_mut(session_id) else {
            return Err(eyre!("Session {} not found", session_id));
        };
        tx.send(BackendEvent::History(HistoryEvent::Snapshot(
            entry.history.clone(),
        )))
        .ok();
        if let Some(status) = &entry.status {
            tx.send(BackendEvent::Status(status.clone())).ok();
        }
        entry.subscribers.push(tx);
        Ok(SessionStream { inner: rx })
    }

    fn list_history(&self, session_id: &SessionId) -> Result<Vec<HistoryItem>> {
        let sessions = self.state.sessions.read();
        let Some(entry) = sessions.get(session_id) else {
            return Err(eyre!("Session {} not found", session_id));
        };
        Ok(entry.history.clone())
    }

    fn send_prompt(&self, session_id: &SessionId, prompt: PromptPayload) -> Result<()> {
        let PromptPayload { text } = prompt;
        if text.trim().is_empty() {
            return Ok(());
        }

        let mut events = Vec::new();
        let conversation = {
            let mut sessions = self.state.sessions.write();
            let Some(entry) = sessions.get_mut(session_id) else {
                return Err(eyre!("Session {} not found", session_id));
            };
            let now = OffsetDateTime::now_utc();
            let user_item = HistoryItem {
                id: Uuid::new_v4(),
                role: MessageRole::User,
                content: text.clone(),
                created_at: now,
            };
            entry.history.push(user_item.clone());
            events.push(BackendEvent::History(HistoryEvent::Append(user_item)));
            entry.conversation.clone()
        };
        for event in events {
            CodexBackend::broadcast(&self.state, session_id, event);
        }
        let send_text = text.clone();
        let history_text = text;
        let runtime = self.runtime.clone();
        runtime.spawn(async move {
            if let Err(err) = conversation
                .submit(Op::UserInput {
                    items: vec![InputItem::Text { text: send_text }],
                })
                .await
            {
                tracing::error!("Не удалось отправить сообщение Codex: {err}");
                return;
            }
            if let Err(err) = conversation
                .submit(Op::AddToHistory { text: history_text })
                .await
            {
                tracing::warn!("Не удалось обновить историю Codex: {err}");
            }
        });

        Ok(())
    }
}

impl CodexBackend {
    fn initialize_session(
        &self,
        NewConversation {
            conversation_id: _,
            conversation,
            session_configured,
        }: NewConversation,
        title: String,
        seed_messages: Vec<super::SeedMessage>,
    ) -> Result<SessionDescriptor> {
        let session_id = SessionId::new();
        let now = OffsetDateTime::now_utc();
        let descriptor = SessionDescriptor {
            id: session_id.clone(),
            title,
            created_at: now,
        };

        let mut history = Vec::new();
        for seed in seed_messages {
            history.push(HistoryItem {
                id: Uuid::new_v4(),
                role: seed.role,
                content: seed.content,
                created_at: now,
            });
        }

        let entry = SessionEntry {
            descriptor: descriptor.clone(),
            conversation: conversation.clone(),
            history,
            subscribers: Vec::new(),
            pending_assistants: HashMap::new(),
            pending_exec: HashMap::new(),
            status: None,
        };

        self.state
            .sessions
            .write()
            .insert(session_id.clone(), entry);

        Self::handle_session_configured(&self.state, &session_id, session_configured);
        self.spawn_event_loop(session_id, conversation);

        Ok(descriptor)
    }
}
