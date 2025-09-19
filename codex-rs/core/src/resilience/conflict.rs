use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::RwLock;
use thiserror::Error;
use uuid::Uuid;

pub type ConflictId = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceValue {
    pub source: String,
    pub value: serde_json::Value,
    pub trust_score: f32,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ResolutionState {
    Pending,
    AutoResolved,
    UserAccepted,
    UserRejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConflictEntry {
    pub id: ConflictId,
    pub key: String,
    pub reason_codes: Vec<String>,
    pub resolution: ResolutionState,
    pub confidence: f32,
    pub sources: Vec<SourceValue>,
    pub last_updated: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConflictDecision {
    Accept,
    Reject,
    Auto,
}

#[derive(Debug, Error)]
pub enum ConflictError {
    #[error("conflict not found: {0}")]
    NotFound(ConflictId),
}

pub struct ConflictResolver {
    entries: RwLock<HashMap<ConflictId, ConflictEntry>>,
}

impl ConflictResolver {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    pub fn insert(&self, mut entry: ConflictEntry) {
        if entry.id.is_nil() {
            entry.id = Uuid::new_v4();
        }
        self.entries.write().unwrap().insert(entry.id, entry);
    }

    pub fn list_pending(&self, limit: usize) -> Vec<ConflictEntry> {
        let guard = self.entries.read().unwrap();
        guard
            .values()
            .filter(|entry| matches!(entry.resolution, ResolutionState::Pending))
            .take(limit)
            .cloned()
            .collect()
    }

    pub fn get(&self, id: ConflictId) -> Option<ConflictEntry> {
        self.entries.read().unwrap().get(&id).cloned()
    }

    pub fn apply_decision(
        &self,
        id: ConflictId,
        decision: ConflictDecision,
        confidence: f32,
    ) -> Result<ConflictEntry, ConflictError> {
        let mut guard = self.entries.write().unwrap();
        let entry = guard.get_mut(&id).ok_or(ConflictError::NotFound(id))?;
        entry.resolution = match decision {
            ConflictDecision::Accept => ResolutionState::UserAccepted,
            ConflictDecision::Reject => ResolutionState::UserRejected,
            ConflictDecision::Auto => ResolutionState::AutoResolved,
        };
        entry.confidence = confidence;
        entry.last_updated = Utc::now().timestamp();
        Ok(entry.clone())
    }

    pub fn remove(&self, id: ConflictId) -> Result<(), ConflictError> {
        let mut guard = self.entries.write().unwrap();
        guard
            .remove(&id)
            .map(|_| ())
            .ok_or(ConflictError::NotFound(id))
    }
}

impl Default for ConflictResolver {
    fn default() -> Self {
        Self::new()
    }
}
