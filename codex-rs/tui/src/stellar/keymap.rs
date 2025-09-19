use codex_core::stellar::StellarAction;
use codex_core::stellar::StellarPersona;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct KeyChord {
    code: KeyCode,
    modifiers: KeyModifiers,
}

impl KeyChord {
    fn new(mut code: KeyCode, modifiers: KeyModifiers) -> Self {
        if let KeyCode::Char(c) = code {
            code = KeyCode::Char(c.to_ascii_lowercase());
        }
        Self { code, modifiers }
    }
}

#[derive(Debug, Default)]
pub struct KeymapEngine {
    global: HashMap<KeyChord, StellarAction>,
    persona_overlays: HashMap<StellarPersona, HashMap<KeyChord, StellarAction>>,
    assistive_overrides: HashMap<KeyChord, StellarAction>,
}

impl KeymapEngine {
    pub fn new() -> Self {
        let mut engine = Self::default();
        engine.populate_global();
        engine.populate_persona_overlays();
        engine.populate_assistive();
        engine
    }

    fn populate_global(&mut self) {
        use StellarAction::*;
        self.global.insert(
            KeyChord::new(KeyCode::Tab, KeyModifiers::NONE),
            NavigateNextPane,
        );
        self.global.insert(
            KeyChord::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            NavigatePrevPane,
        );
        self.global.insert(
            KeyChord::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
            OpenCommandPalette,
        );
        self.global.insert(
            KeyChord::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            StellarAction::OpenConflictOverlay,
        );
        self.global.insert(
            KeyChord::new(KeyCode::Char('i'), KeyModifiers::CONTROL),
            ToggleCanvas,
        );
        self.global.insert(
            KeyChord::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
            ToggleTelemetryOverlay,
        );
        self.global.insert(
            KeyChord::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
            RunbookInvoke { runbook_id: None },
        );
        self.global.insert(
            KeyChord::new(KeyCode::Char('z'), KeyModifiers::CONTROL),
            InputUndo,
        );
        self.global.insert(
            KeyChord::new(
                KeyCode::Char('z'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            InputRedo,
        );
        self.global.insert(
            KeyChord::new(KeyCode::Char('y'), KeyModifiers::CONTROL),
            InputRedo,
        );
        self.global.insert(
            KeyChord::new(KeyCode::Char('l'), KeyModifiers::CONTROL),
            FieldLockToggle,
        );
        self.global.insert(
            KeyChord::new(KeyCode::Char('i'), KeyModifiers::NONE),
            ToggleConfidencePanel,
        );
        self.global.insert(
            KeyChord::new(KeyCode::Enter, KeyModifiers::CONTROL),
            SubmitInsight { text: None },
        );
        self.global.insert(
            KeyChord::new(
                KeyCode::Char('a'),
                KeyModifiers::CONTROL | KeyModifiers::ALT,
            ),
            AccessibilityToggle,
        );
    }

    fn populate_persona_overlays(&mut self) {
        use StellarAction::*;
        let mut sre = HashMap::new();
        sre.insert(
            KeyChord::new(
                KeyCode::Char('o'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            ToggleTelemetryOverlay,
        );
        self.persona_overlays.insert(StellarPersona::Sre, sre);

        let mut secops = HashMap::new();
        secops.insert(
            KeyChord::new(
                KeyCode::Char('k'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            OpenCommandPalette,
        );
        self.persona_overlays.insert(StellarPersona::SecOps, secops);

        let mut platform = HashMap::new();
        platform.insert(
            KeyChord::new(KeyCode::Enter, KeyModifiers::ALT),
            SubmitInsight { text: None },
        );
        self.persona_overlays
            .insert(StellarPersona::PlatformEngineer, platform);
    }

    fn populate_assistive(&mut self) {
        use StellarAction::*;
        self.assistive_overrides.insert(
            KeyChord::new(KeyCode::Right, KeyModifiers::CONTROL),
            NavigateNextPane,
        );
        self.assistive_overrides.insert(
            KeyChord::new(KeyCode::Left, KeyModifiers::CONTROL),
            NavigatePrevPane,
        );
        self.assistive_overrides.insert(
            KeyChord::new(KeyCode::Char('i'), KeyModifiers::ALT),
            ToggleCanvas,
        );
        self.assistive_overrides.insert(
            KeyChord::new(KeyCode::F(1), KeyModifiers::NONE),
            OpenCommandPalette,
        );
        self.assistive_overrides.insert(
            KeyChord::new(KeyCode::F(6), KeyModifiers::NONE),
            ToggleTelemetryOverlay,
        );
        self.assistive_overrides.insert(
            KeyChord::new(KeyCode::F(7), KeyModifiers::NONE),
            StellarAction::OpenConflictOverlay,
        );
        self.assistive_overrides.insert(
            KeyChord::new(KeyCode::F(9), KeyModifiers::NONE),
            RunbookInvoke { runbook_id: None },
        );
        self.assistive_overrides.insert(
            KeyChord::new(KeyCode::Backspace, KeyModifiers::ALT),
            InputUndo,
        );
        self.assistive_overrides.insert(
            KeyChord::new(KeyCode::Enter, KeyModifiers::SHIFT),
            ToggleConfidencePanel,
        );
        self.assistive_overrides.insert(
            KeyChord::new(KeyCode::F(10), KeyModifiers::NONE),
            AccessibilityToggle,
        );
    }

    pub fn resolve(
        &self,
        persona: StellarPersona,
        assistive_mode: bool,
        event: KeyEvent,
    ) -> Option<StellarAction> {
        if !matches!(event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }
        let chord = KeyChord::new(event.code, event.modifiers);
        if assistive_mode {
            if let Some(action) = self.assistive_overrides.get(&chord) {
                return Some(with_payload(action.clone(), event));
            }
        }
        if let Some(persona_map) = self.persona_overlays.get(&persona) {
            if let Some(action) = persona_map.get(&chord) {
                return Some(with_payload(action.clone(), event));
            }
        }
        self.global
            .get(&chord)
            .cloned()
            .map(|action| with_payload(action, event))
    }
}

fn with_payload(action: StellarAction, event: KeyEvent) -> StellarAction {
    match action {
        StellarAction::RunbookInvoke { runbook_id } => {
            if runbook_id.is_some() {
                return StellarAction::RunbookInvoke { runbook_id };
            }
            // Allow launching runbooks via numeric function keys, e.g. F2 -> RB-02.
            if let KeyCode::F(n) = event.code {
                let id = format!("RB-{n:02}");
                return StellarAction::RunbookInvoke {
                    runbook_id: Some(id),
                };
            }
            StellarAction::RunbookInvoke { runbook_id: None }
        }
        StellarAction::SubmitInsight { text: payload } => {
            if payload.is_some() {
                return StellarAction::SubmitInsight { text: payload };
            }
            // Allow typing text via modifier chords (Ctrl+Enter carries no text).
            StellarAction::SubmitInsight { text: None }
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;
    use crossterm::event::KeyEvent;
    use crossterm::event::KeyEventKind;
    use crossterm::event::KeyModifiers;

    #[test]
    fn ctrl_enter_maps_to_submit() {
        let engine = KeymapEngine::new();
        let event = KeyEvent {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        };
        let action = engine
            .resolve(StellarPersona::Operator, false, event)
            .expect("should map to submit");
        match action {
            StellarAction::SubmitInsight { text } => assert!(text.is_none()),
            other => panic!("expected submit, got {other:?}"),
        }
    }

    #[test]
    fn ctrl_shift_c_opens_conflict_overlay() {
        let engine = KeymapEngine::new();
        let event = KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        };
        let action = engine
            .resolve(StellarPersona::Operator, false, event)
            .expect("should map to conflict overlay");
        assert!(matches!(action, StellarAction::OpenConflictOverlay));
    }
}
