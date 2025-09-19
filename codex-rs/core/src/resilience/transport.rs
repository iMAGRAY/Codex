use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use sled::IVec;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("sled error: {0}")]
    Store(#[from] sled::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetryItem {
    pub id: u64,
    pub command: String,
    pub payload: serde_json::Value,
    pub attempts: u32,
    pub last_attempt: Option<DateTime<Utc>>,
    pub max_attempts: u32,
}

impl RetryItem {
    pub fn can_retry(&self) -> bool {
        self.attempts < self.max_attempts
    }
}

#[derive(Debug, Clone)]
pub struct QueueConfig {
    pub path: Option<PathBuf>,
    pub tree_name: Option<String>,
    pub temporary: bool,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            path: None,
            tree_name: Some("stellar-retry-queue".to_string()),
            temporary: true,
        }
    }
}

pub struct RetryQueue {
    tree: sled::Tree,
    seq: AtomicU64,
}

impl RetryQueue {
    pub fn open(config: QueueConfig) -> Result<Self, TransportError> {
        let mut builder = sled::Config::new();
        if let Some(path) = config.path.as_ref() {
            builder = builder.path(path);
        }
        if config.temporary {
            builder = builder.temporary(true);
        }
        let db = builder.open()?;
        let tree = if let Some(name) = config.tree_name {
            db.open_tree(name)?
        } else {
            db.open_tree("stellar-retry-queue")?
        };
        let seq = tree
            .last()
            .map(|opt| opt.map(|(k, _)| decode_seq(&k)).unwrap_or(0))
            .unwrap_or(0);
        Ok(Self {
            tree,
            seq: AtomicU64::new(seq),
        })
    }

    pub fn enqueue(
        &self,
        command: impl Into<String>,
        payload: serde_json::Value,
        max_attempts: u32,
    ) -> Result<u64, TransportError> {
        let id = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let item = RetryItem {
            id,
            command: command.into(),
            payload,
            attempts: 0,
            last_attempt: None,
            max_attempts,
        };
        let encoded = serde_json::to_vec(&item)?;
        self.tree.insert(encode_seq(id), encoded)?;
        Ok(id)
    }

    pub fn peek(&self, limit: usize) -> Result<Vec<RetryItem>, TransportError> {
        let mut items = Vec::new();
        for result in self.tree.iter() {
            let (_, value) = result?;
            let item: RetryItem = serde_json::from_slice(&value)?;
            if item.can_retry() {
                items.push(item);
            }
            if items.len() == limit {
                break;
            }
        }
        Ok(items)
    }

    pub fn record_attempt(&self, id: u64) -> Result<(), TransportError> {
        if let Some(value) = self.tree.get(encode_seq(id))? {
            let mut item: RetryItem = serde_json::from_slice(&value)?;
            item.attempts += 1;
            item.last_attempt = Some(Utc::now());
            let encoded = serde_json::to_vec(&item)?;
            self.tree.insert(encode_seq(id), encoded)?;
        }
        Ok(())
    }

    pub fn remove(&self, id: u64) -> Result<(), TransportError> {
        let _ = self.tree.remove(encode_seq(id))?;
        Ok(())
    }

    pub fn drain_ready(&self, limit: usize) -> Result<Vec<RetryItem>, TransportError> {
        let mut drained = Vec::new();
        for result in self.tree.iter() {
            let (key, value) = result?;
            if drained.len() == limit {
                break;
            }
            let item: RetryItem = serde_json::from_slice(&value)?;
            if item.can_retry() {
                self.tree.remove(key)?;
                drained.push(item);
            }
        }
        Ok(drained)
    }

    pub fn len(&self) -> usize {
        self.tree.len()
    }
}

fn encode_seq(id: u64) -> [u8; 8] {
    id.to_be_bytes()
}

fn decode_seq(bytes: &IVec) -> u64 {
    let mut arr = [0u8; 8];
    arr.copy_from_slice(bytes);
    u64::from_be_bytes(arr)
}

pub fn default_queue_path(base: &Path) -> PathBuf {
    base.join("resilience-queue")
}
