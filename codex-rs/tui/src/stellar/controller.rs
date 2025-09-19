use crate::stellar::keymap::KeymapEngine;
use codex_core::resilience::CacheConfig;
use codex_core::resilience::QueueConfig;
use codex_core::resilience::ResilienceCache;
use codex_core::resilience::ResilienceServices;
use codex_core::resilience::RetryQueue;
use codex_core::stellar::ActionApplied;
use codex_core::stellar::GuardError;
use codex_core::stellar::KernelEvent;
use codex_core::stellar::KernelSnapshot;
use codex_core::stellar::LayoutMode;
use codex_core::stellar::StellarAction;
use codex_core::stellar::StellarCliEvent;
use codex_core::stellar::StellarKernel;
use codex_core::stellar::StellarPersona;
use crossterm::event::KeyEvent;

#[derive(Debug)]
pub struct StellarController {
    kernel: StellarKernel,
    keymap: KeymapEngine,
    resilience: ResilienceServices,
}

#[derive(Debug, Clone)]
pub enum ControllerOutcome {
    Consumed {
        #[allow(dead_code)]
        action: StellarAction,
        applied: ActionApplied,
    },
    Rejected {
        #[allow(dead_code)]
        action: StellarAction,
        error: GuardError,
    },
    Unhandled,
}

impl StellarController {
    pub fn new(persona: StellarPersona) -> Self {
        let resilience =
            ResilienceServices::default().unwrap_or_else(|_| fallback_resilience_services());
        let kernel = StellarKernel::with_resilience(persona, resilience.clone());
        Self {
            kernel,
            keymap: KeymapEngine::new(),
            resilience,
        }
    }

    pub fn persona(&self) -> StellarPersona {
        self.kernel.persona()
    }

    pub fn is_active(&self) -> bool {
        self.kernel.is_canvas_visible()
    }

    pub fn assistive_mode(&self) -> bool {
        self.kernel.assistive_mode()
    }

    pub fn handle_key_event(&mut self, event: KeyEvent) -> ControllerOutcome {
        let persona = self.persona();
        let assistive = self.assistive_mode();
        let Some(action) = self.keymap.resolve(persona, assistive, event) else {
            return ControllerOutcome::Unhandled;
        };
        match self.kernel.handle_action(action.clone()) {
            Ok(applied) => ControllerOutcome::Consumed { action, applied },
            Err(error) => ControllerOutcome::Rejected { action, error },
        }
    }

    #[allow(dead_code)]
    pub fn apply_cli_event(
        &mut self,
        cli_event: StellarCliEvent,
    ) -> Result<ActionApplied, GuardError> {
        if cli_event.persona != self.persona() {
            self.kernel =
                StellarKernel::with_resilience(cli_event.persona, self.resilience.clone());
        }
        self.kernel.handle_action(cli_event.action)
    }

    pub fn snapshot(&self) -> KernelSnapshot {
        self.kernel.snapshot()
    }

    pub fn take_events(&mut self) -> Vec<KernelEvent> {
        self.kernel.take_events()
    }

    pub fn preferred_height(&self) -> u16 {
        match self.kernel.layout_mode() {
            LayoutMode::Wide => 18,
            LayoutMode::Compact => 22,
        }
    }

    pub fn sync_layout(&mut self, width: u16) -> LayoutMode {
        let mode = if width >= 120 {
            LayoutMode::Wide
        } else {
            LayoutMode::Compact
        };
        self.kernel.set_layout_mode(mode);
        mode
    }

    #[allow(dead_code)]
    pub fn set_field_text(&mut self, value: impl Into<String>) {
        self.kernel.set_field_text(value);
    }

    #[allow(dead_code)]
    pub fn clear_status(&mut self) {
        self.kernel.clear_status_messages();
    }
}

fn fallback_resilience_services() -> ResilienceServices {
    let cache = ResilienceCache::open(CacheConfig::default()).expect("fallback cache init failed");
    let queue = RetryQueue::open(QueueConfig::default()).expect("fallback queue init failed");
    ResilienceServices::new(cache, queue)
}
