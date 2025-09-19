//! Stellar kernel state machine with resilience integrations.
//!
//! Trace: REQ-REL-01 (resilience cache & queue), REQ-DATA-01 (conflict resolver),
//! REQ-PERF-01 (confidence scoring telemetry).

use crate::resilience::CacheConfig;
use crate::resilience::CacheKey;
use crate::resilience::CachePolicy;
use crate::resilience::ConfidenceInput;
use crate::resilience::ConflictDecision;
use crate::resilience::ConflictEntry;
use crate::resilience::QueueConfig;
use crate::resilience::ResilienceCache;
use crate::resilience::ResilienceServices;
use crate::resilience::ResolutionState;
use crate::resilience::RetryQueue;
use crate::resilience::SourceValue;
use crate::resilience::confidence::ConfidenceCalculator;
use crate::stellar::action::StellarAction;
use crate::stellar::action::StellarActionId;
use crate::stellar::event::KernelEvent;
use crate::stellar::guard::GuardContext;
use crate::stellar::guard::GuardError;
use crate::stellar::guard::InputGuard;
use crate::stellar::persona::StellarPersona;
use crate::stellar::snapshot::ConfidenceSnapshot;
use crate::stellar::snapshot::GoldenPathHint;
use crate::stellar::snapshot::KernelSnapshot;
use crate::stellar::snapshot::LayoutMode;
use crate::stellar::snapshot::PaneFocus;
use crate::stellar::snapshot::RiskAlert;
use crate::stellar::snapshot::RunbookShortcut;
use chrono::Utc;
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

const STATUS_CAPACITY: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionApplied {
    NoChange,
    StateChanged,
}

#[derive(Debug, Clone)]
pub struct StellarKernel {
    persona: StellarPersona,
    assistive_mode: bool,
    layout_mode: LayoutMode,
    canvas_visible: bool,
    focus: PaneFocus,
    telemetry_visible: bool,
    field_text: String,
    field_locked: bool,
    suggestions: Vec<String>,
    command_log: Vec<String>,
    runbook_shortcuts: Vec<RunbookShortcut>,
    golden_path: Vec<GoldenPathHint>,
    confidence: ConfidenceSnapshot,
    guard: InputGuard,
    undo_stack: Vec<String>,
    redo_stack: Vec<String>,
    status_messages: Vec<String>,
    events: Vec<KernelEvent>,
    resilience: ResilienceServices,
    confidence_calc: ConfidenceCalculator,
    user_override_bias: f32,
    risk_alerts: Vec<RiskAlert>,
}

impl StellarKernel {
    pub fn new(persona: StellarPersona) -> Self {
        let resilience = ResilienceServices::default()
            .unwrap_or_else(|_| fallback_resilience_services())
            .clone();
        Self::with_resilience(persona, resilience)
    }

    pub fn with_resilience(persona: StellarPersona, resilience: ResilienceServices) -> Self {
        let assistive_mode = matches!(persona, StellarPersona::AssistiveBridge);
        let mut kernel = Self {
            persona,
            assistive_mode,
            layout_mode: LayoutMode::Wide,
            canvas_visible: false,
            focus: PaneFocus::InsightCanvas,
            telemetry_visible: false,
            field_text: String::new(),
            field_locked: false,
            suggestions: vec![
                "Review last deployment metrics".to_string(),
                "Check error budget consumption".to_string(),
                "Summarize incidents from past 24h".to_string(),
            ],
            command_log: Vec::new(),
            runbook_shortcuts: vec![
                RunbookShortcut {
                    id: "RB-01".to_string(),
                    title: "Mitigate latency spike".to_string(),
                    description: "Warm cache, enable throttling, notify SRE".to_string(),
                },
                RunbookShortcut {
                    id: "RB-07".to_string(),
                    title: "Rollback last deployment".to_string(),
                    description: "Assess impact, trigger staged rollback".to_string(),
                },
            ],
            golden_path: vec![
                GoldenPathHint {
                    label: "Send Insight".to_string(),
                    description: "Submit current insight for review".to_string(),
                    action: StellarActionId::SubmitInsight,
                    shortcut: Some("Ctrl+Enter".to_string()),
                },
                GoldenPathHint {
                    label: "Show Metrics".to_string(),
                    description: "Toggle telemetry overlay".to_string(),
                    action: StellarActionId::ToggleTelemetryOverlay,
                    shortcut: Some("Ctrl+O".to_string()),
                },
            ],
            confidence: ConfidenceSnapshot {
                score: 0.82,
                trend: "Stable".to_string(),
                reasons: vec![
                    "Recent telemetry aligns with predicted range".to_string(),
                    "No blocking incidents in past 4h".to_string(),
                ],
                visible: false,
            },
            guard: InputGuard::default(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            status_messages: Vec::new(),
            events: Vec::new(),
            resilience: resilience.clone(),
            confidence_calc: resilience.confidence.clone(),
            user_override_bias: 0.5,
            risk_alerts: Vec::new(),
        };
        kernel.recompute_confidence();
        kernel
    }

    pub fn persona(&self) -> StellarPersona {
        self.persona
    }

    pub fn assistive_mode(&self) -> bool {
        self.assistive_mode
    }

    pub fn layout_mode(&self) -> LayoutMode {
        self.layout_mode
    }

    pub fn set_layout_mode(&mut self, mode: LayoutMode) {
        self.layout_mode = mode;
    }

    pub fn is_canvas_visible(&self) -> bool {
        self.canvas_visible
    }

    pub fn focus(&self) -> PaneFocus {
        self.focus
    }

    pub fn field_text(&self) -> &str {
        &self.field_text
    }

    pub fn set_field_text(&mut self, value: impl Into<String>) {
        self.field_text = value.into();
    }

    pub fn push_status(&mut self, message: impl Into<String>) {
        let msg = message.into();
        if msg.is_empty() {
            return;
        }
        self.status_messages.push(msg);
        if self.status_messages.len() > STATUS_CAPACITY {
            let excess = self.status_messages.len() - STATUS_CAPACITY;
            self.status_messages.drain(0..excess);
        }
    }

    pub fn take_events(&mut self) -> Vec<KernelEvent> {
        std::mem::take(&mut self.events)
    }

    fn guard_context(&self) -> GuardContext {
        GuardContext {
            persona: self.persona,
            assistive_mode: self.assistive_mode,
            field_locked: self.field_locked,
            field_empty: self.field_text.trim().is_empty(),
            undo_available: !self.undo_stack.is_empty(),
            redo_available: !self.redo_stack.is_empty(),
        }
    }

    fn focus_order(&self) -> Vec<PaneFocus> {
        let mut order = vec![PaneFocus::InsightCanvas];
        if self.telemetry_visible {
            order.push(PaneFocus::Telemetry);
        }
        order.push(PaneFocus::CommandLog);
        order.push(PaneFocus::Runbook);
        order.push(PaneFocus::FooterSubmit);
        order.push(PaneFocus::FooterOverlay);
        order
    }

    fn advance_focus(&mut self, forward: bool) {
        let order = self.focus_order();
        if order.is_empty() {
            return;
        }
        if let Some(idx) = order.iter().position(|pane| *pane == self.focus) {
            let next_idx = if forward {
                (idx + 1) % order.len()
            } else {
                (idx + order.len() - 1) % order.len()
            };
            self.focus = order[next_idx];
        } else {
            self.focus = order[0];
        }
    }

    fn toggle_canvas(&mut self) -> ActionApplied {
        self.canvas_visible = !self.canvas_visible;
        if !self.canvas_visible {
            self.focus = PaneFocus::InsightCanvas;
        }
        self.push_status(if self.canvas_visible {
            "Insight Canvas opened"
        } else {
            "Insight Canvas hidden"
        });
        ActionApplied::StateChanged
    }

    fn toggle_telemetry(&mut self) -> ActionApplied {
        self.telemetry_visible = !self.telemetry_visible;
        if self.telemetry_visible {
            self.focus = PaneFocus::Telemetry;
            self.push_status("Telemetry overlay enabled");
        } else {
            self.focus = PaneFocus::InsightCanvas;
            self.push_status("Telemetry overlay disabled");
        }
        self.recompute_confidence();
        ActionApplied::StateChanged
    }

    fn submit_insight(&mut self, text: Option<String>) -> ActionApplied {
        let submission = text.map(|t| t.trim().to_string()).unwrap_or_else(|| {
            if self.field_text.trim().is_empty() {
                String::new()
            } else {
                self.field_text.trim().to_string()
            }
        });
        if submission.is_empty() {
            return ActionApplied::NoChange;
        }
        self.command_log.push(format!("[insight] {submission}"));
        self.undo_stack.push(submission.clone());
        self.redo_stack.clear();
        self.events.push(KernelEvent::Submission {
            text: submission.clone(),
        });
        self.push_status("Insight submitted");
        self.field_text.clear();
        self.persist_insight(&submission);
        self.recompute_confidence();
        ActionApplied::StateChanged
    }

    fn undo(&mut self) -> ActionApplied {
        let Some(previous) = self.undo_stack.pop() else {
            return ActionApplied::NoChange;
        };
        if let Some(pos) = self
            .command_log
            .iter()
            .rposition(|entry| entry.contains(&previous))
        {
            self.command_log.remove(pos);
        }
        self.field_text = previous.clone();
        self.redo_stack.push(previous);
        self.push_status("Last Insight action undone");
        self.recompute_confidence();
        ActionApplied::StateChanged
    }

    fn redo(&mut self) -> ActionApplied {
        let Some(restored) = self.redo_stack.pop() else {
            return ActionApplied::NoChange;
        };
        self.command_log.push(format!("[insight] {restored}"));
        self.undo_stack.push(restored);
        self.push_status("Redo applied");
        self.recompute_confidence();
        ActionApplied::StateChanged
    }

    fn toggle_field_lock(&mut self) -> ActionApplied {
        self.field_locked = !self.field_locked;
        if self.field_locked {
            self.push_status("Insight field locked");
            self.register_lock_conflict();
        } else {
            self.push_status("Insight field unlocked");
        }
        self.recompute_confidence();
        ActionApplied::StateChanged
    }

    fn toggle_confidence(&mut self) -> ActionApplied {
        self.confidence.visible = !self.confidence.visible;
        if self.confidence.visible {
            self.push_status("Confidence panel shown");
        } else {
            self.push_status("Confidence panel hidden");
        }
        ActionApplied::StateChanged
    }

    fn invoke_runbook(&mut self, runbook_id: Option<String>) -> ActionApplied {
        let id = runbook_id.unwrap_or_else(|| "RB-01".to_string());
        self.push_status(format!("Runbook {id} launched"));
        self.events.push(KernelEvent::Info {
            message: format!("runbook:{id}"),
        });
        ActionApplied::StateChanged
    }

    fn toggle_accessibility(&mut self) -> ActionApplied {
        self.assistive_mode = !self.assistive_mode;
        self.push_status(if self.assistive_mode {
            "Assistive mode enabled"
        } else {
            "Assistive mode disabled"
        });
        self.recompute_confidence();
        ActionApplied::StateChanged
    }

    fn persist_insight(&mut self, submission: &str) {
        let record = CachedInsight {
            text: submission.to_string(),
            persona: self.persona.to_string(),
            submitted_at: Utc::now().timestamp(),
        };
        let cache_key = CacheKey::new(format!("insight:{}:{}", self.persona, record.submitted_at));
        if let Err(err) = self
            .resilience
            .cache
            .put(cache_key, &record, CachePolicy::default())
        {
            self.push_status(format!("Cache write error: {err}"));
        } else {
            self.events.push(KernelEvent::CacheStored {
                key: format!("insight:{}", record.submitted_at),
            });
        }
        let payload = json!({
            "text": submission,
            "persona": record.persona,
            "timestamp": record.submitted_at,
        });
        if let Err(err) = self.resilience.queue.enqueue("insight.submit", payload, 5) {
            self.push_status(format!("Queue enqueue error: {err}"));
        }
        self.resilience.prefetch.record("insight.submit");
        let top = self.resilience.prefetch.top(3);
        if !top.is_empty() {
            self.suggestions = top
                .into_iter()
                .map(|(key, count)| format!("Prefetch candidate {key} (score {count})"))
                .collect();
        }
    }

    fn recompute_confidence(&mut self) {
        let stats = self.resilience.cache.stats();
        let hit_ratio = stats.hit_ratio().clamp(0.0, 1.0);
        let pending_conflicts = self.pending_conflicts();
        let source_trust = if pending_conflicts == 0 { 0.9 } else { 0.45 };
        let schema_valid = if self.field_locked { 0.6 } else { 0.95 };
        let prefetch_stats = self.resilience.prefetch.stats();
        let telemetry_alignment = if prefetch_stats.scheduled > 0 {
            (prefetch_stats.completed as f32 / prefetch_stats.scheduled as f32).clamp(0.0, 1.0)
        } else if self.telemetry_visible {
            0.9
        } else {
            0.7
        };
        let input = ConfidenceInput {
            freshness: (hit_ratio + 0.2).min(1.0),
            source_trust,
            schema_valid,
            telemetry_alignment,
            user_overrides: self.user_override_bias.clamp(0.0, 1.0),
        };
        let score = self.confidence_calc.score(input);
        self.confidence.score = score.value;
        self.confidence.trend = if score.value >= 0.75 {
            "High".to_string()
        } else if score.value >= 0.4 {
            "Moderate".to_string()
        } else {
            "Low".to_string()
        };
        self.confidence.reasons = score
            .breakdown
            .into_iter()
            .map(|b| format!("{}: {:.2}", b.factor, b.contribution))
            .collect();
        self.refresh_risk_alerts();
    }

    fn refresh_risk_alerts(&mut self) {
        let mut alerts = Vec::new();
        let pending_conflicts = self.resilience.conflicts.list_pending(5);
        if !pending_conflicts.is_empty() {
            alerts.push(RiskAlert::warning(format!(
                "{} pending conflict{}",
                pending_conflicts.len(),
                if pending_conflicts.len() > 1 { "s" } else { "" }
            )));
        }
        let queued = self.resilience.queue.len();
        if queued > 0 {
            alerts.push(RiskAlert::warning(format!(
                "{} queued offline operation{}",
                queued,
                if queued > 1 { "s" } else { "" }
            )));
        }
        let hit_miss = self.resilience.cache.hit_miss();
        let total = hit_miss.total();
        if total >= 5 {
            let ratio = hit_miss.hits as f32 / total as f32;
            if ratio < 0.6 {
                alerts.push(RiskAlert::critical(format!(
                    "Cache hit ratio {:.0}% below target",
                    ratio * 100.0
                )));
            } else if ratio < 0.8 {
                alerts.push(RiskAlert::warning(format!(
                    "Cache hit ratio {:.0}% requires attention",
                    ratio * 100.0
                )));
            }
        }
        if alerts.is_empty() {
            alerts.push(RiskAlert::info("Resilience systems nominal"));
        }
        self.risk_alerts = alerts;
    }

    fn pending_conflicts(&self) -> usize {
        self.resilience.conflicts.list_pending(10).len()
    }

    fn open_conflict_overlay(&mut self) -> ActionApplied {
        let pending = self.resilience.conflicts.list_pending(3);
        if pending.is_empty() {
            self.push_status("No conflicts found");
        } else {
            self.push_status(format!("Conflict list opened ({})", pending.len()));
            for conflict in pending.iter() {
                self.push_status(format!("{} - {}", conflict.id, conflict.key));
            }
        }
        self.refresh_risk_alerts();
        ActionApplied::StateChanged
    }

    fn resolve_conflict(
        &mut self,
        conflict_id: Option<Uuid>,
        decision: ConflictDecision,
    ) -> ActionApplied {
        let target_id = conflict_id.or_else(|| {
            self.resilience
                .conflicts
                .list_pending(1)
                .first()
                .map(|entry| entry.id)
        });
        let Some(id) = target_id else {
            self.push_status("No conflicts available for resolution");
            return ActionApplied::NoChange;
        };
        match self
            .resilience
            .conflicts
            .apply_decision(id, decision.clone(), self.confidence.score)
        {
            Ok(entry) => {
                match decision {
                    ConflictDecision::Accept => {
                        self.user_override_bias = (self.user_override_bias + 0.1).min(1.0);
                    }
                    ConflictDecision::Reject => {
                        self.user_override_bias = (self.user_override_bias - 0.1).max(0.0);
                    }
                    ConflictDecision::Auto => {}
                }
                self.events.push(KernelEvent::ConflictResolution {
                    conflict_id: entry.id,
                    state: entry.resolution.clone(),
                });
                self.push_status(format!("Conflict {} -> {:?}", entry.id, entry.resolution));
                self.recompute_confidence();
                ActionApplied::StateChanged
            }
            Err(err) => {
                self.push_status(format!("Conflict resolution error: {err}"));
                ActionApplied::NoChange
            }
        }
    }

    fn register_lock_conflict(&mut self) {
        let timestamp = Utc::now().timestamp();
        let entry = ConflictEntry {
            id: Uuid::new_v4(),
            key: "insight.field".to_string(),
            reason_codes: vec!["field_locked".to_string()],
            resolution: ResolutionState::Pending,
            confidence: self.confidence.score,
            sources: vec![
                SourceValue {
                    source: "cache".to_string(),
                    value: json!({"field_locked": true}),
                    trust_score: 0.7,
                    timestamp,
                },
                SourceValue {
                    source: "remote".to_string(),
                    value: json!({"field_locked": false}),
                    trust_score: 0.5,
                    timestamp,
                },
            ],
            last_updated: timestamp,
        };
        self.resilience.conflicts.insert(entry);
        self.push_status("Conflict added: insight.field");
    }

    fn open_palette(&mut self) -> ActionApplied {
        self.push_status("Command palette opened");
        ActionApplied::StateChanged
    }

    pub fn handle_action(&mut self, action: StellarAction) -> Result<ActionApplied, GuardError> {
        let mut guard_ctx = self.guard_context();
        if let StellarAction::SubmitInsight {
            text: Some(payload),
        } = &action
        {
            if !payload.trim().is_empty() {
                guard_ctx.field_empty = false;
            }
        }
        self.guard.validate(&action, guard_ctx)?;
        let applied = match action {
            StellarAction::NavigateNextPane => {
                self.advance_focus(true);
                ActionApplied::StateChanged
            }
            StellarAction::NavigatePrevPane => {
                self.advance_focus(false);
                ActionApplied::StateChanged
            }
            StellarAction::OpenCommandPalette => self.open_palette(),
            StellarAction::ToggleCanvas => self.toggle_canvas(),
            StellarAction::ToggleTelemetryOverlay => self.toggle_telemetry(),
            StellarAction::RunbookInvoke { runbook_id } => self.invoke_runbook(runbook_id),
            StellarAction::InputUndo => self.undo(),
            StellarAction::InputRedo => self.redo(),
            StellarAction::FieldLockToggle => self.toggle_field_lock(),
            StellarAction::ToggleConfidencePanel => self.toggle_confidence(),
            StellarAction::SubmitInsight { text } => self.submit_insight(text),
            StellarAction::AccessibilityToggle => self.toggle_accessibility(),
            StellarAction::OpenConflictOverlay => self.open_conflict_overlay(),
            StellarAction::ResolveConflict {
                conflict_id,
                decision,
            } => self.resolve_conflict(conflict_id, decision),
        };
        Ok(applied)
    }

    pub fn snapshot(&self) -> KernelSnapshot {
        KernelSnapshot {
            persona: self.persona,
            assistive_mode: self.assistive_mode,
            layout_mode: self.layout_mode,
            canvas_visible: self.canvas_visible,
            telemetry_visible: self.telemetry_visible,
            focus: self.focus,
            field_text: self.field_text.clone(),
            field_locked: self.field_locked,
            suggestions: self.suggestions.clone(),
            command_log: self.command_log.clone(),
            runbook_shortcuts: self.runbook_shortcuts.clone(),
            golden_path: self.golden_path.clone(),
            confidence: Some(self.confidence.clone()),
            status_messages: self.status_messages.clone(),
            risk_alerts: self.risk_alerts.clone(),
        }
    }

    pub fn clear_status_messages(&mut self) {
        self.status_messages.clear();
    }
}

#[derive(Serialize)]
struct CachedInsight {
    text: String,
    persona: String,
    submitted_at: i64,
}

fn fallback_resilience_services() -> ResilienceServices {
    let cache = ResilienceCache::open(CacheConfig::default()).expect("fallback cache init failed");
    let queue = RetryQueue::open(QueueConfig::default()).expect("fallback retry queue init failed");
    ResilienceServices::new(cache, queue)
}
