use crate::resilience::ConflictId;
use crate::resilience::ResolutionState;
use crate::stellar::action::StellarAction;
use crate::stellar::persona::StellarPersona;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum KernelEvent {
    Info {
        message: String,
    },
    Submission {
        text: String,
    },
    CacheStored {
        key: String,
    },
    ConflictResolution {
        conflict_id: ConflictId,
        state: ResolutionState,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StellarCliEvent {
    pub persona: StellarPersona,
    pub action: StellarAction,
}

impl StellarCliEvent {
    pub fn new(persona: StellarPersona, action: StellarAction) -> Self {
        Self { persona, action }
    }
}
