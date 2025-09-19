use crate::resilience::metrics::CacheHitMiss;
use crate::resilience::metrics::CacheStats;
use crate::telemetry::TelemetryHub;
use chacha20poly1305::XChaCha20Poly1305;
use chacha20poly1305::XNonce;
use chacha20poly1305::aead::Aead;
use chacha20poly1305::aead::KeyInit;
use rand::random;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use sled::IVec;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey(String);

impl CacheKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for CacheKey {
    fn from(value: &str) -> Self {
        CacheKey::new(value)
    }
}

impl From<String> for CacheKey {
    fn from(value: String) -> Self {
        CacheKey::new(value)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CachePolicy {
    pub ttl: Option<Duration>,
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self { ttl: None }
    }
}

#[derive(Debug, Clone)]
pub struct CacheConfig {
    pub path: Option<PathBuf>,
    pub tree_name: Option<String>,
    pub encryption_key: Option<[u8; 32]>,
    pub default_ttl: Option<Duration>,
    pub temporary: bool,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            path: None,
            tree_name: Some("stellar-cache".to_string()),
            encryption_key: None,
            default_ttl: Some(Duration::from_secs(900)),
            temporary: true,
        }
    }
}

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("serialization error: {0}")]
    Serialization(#[from] bincode::Error),
    #[error("sled error: {0}")]
    Store(#[from] sled::Error),
    #[error("encryption error")]
    Encryption,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheRecord {
    payload: Vec<u8>,
    created_at: i64,
    expires_at: Option<i64>,
    hit_count: u64,
    version: u32,
}

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn duration_to_ts(base: SystemTime, ttl: Duration) -> i64 {
    base.checked_add(ttl)
        .unwrap_or_else(|| UNIX_EPOCH)
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

enum EncryptionLayer {
    None,
    XChaCha20(XChaCha20Poly1305),
}

impl EncryptionLayer {
    fn new(key: Option<[u8; 32]>) -> Result<Self, CacheError> {
        if let Some(key_bytes) = key {
            Ok(Self::XChaCha20(XChaCha20Poly1305::new((&key_bytes).into())))
        } else {
            Ok(Self::None)
        }
    }

    fn seal(&self, mut data: Vec<u8>) -> Result<Vec<u8>, CacheError> {
        match self {
            EncryptionLayer::None => Ok(data),
            EncryptionLayer::XChaCha20(cipher) => {
                let nonce_bytes: [u8; 24] = random();
                let nonce = XNonce::from_slice(&nonce_bytes);
                let ciphertext = cipher
                    .encrypt(nonce, data.as_ref())
                    .map_err(|_| CacheError::Encryption)?;
                let mut out = Vec::with_capacity(24 + ciphertext.len());
                out.extend_from_slice(&nonce_bytes);
                out.extend_from_slice(&ciphertext);
                data.clear();
                Ok(out)
            }
        }
    }

    fn open(&self, data: &[u8]) -> Result<Vec<u8>, CacheError> {
        match self {
            EncryptionLayer::None => Ok(data.to_vec()),
            EncryptionLayer::XChaCha20(cipher) => {
                if data.len() < 24 {
                    return Err(CacheError::Encryption);
                }
                let (nonce_bytes, ciphertext) = data.split_at(24);
                let nonce = XNonce::from_slice(nonce_bytes);
                let plaintext = cipher
                    .decrypt(nonce, ciphertext)
                    .map_err(|_| CacheError::Encryption)?;
                Ok(plaintext)
            }
        }
    }
}

pub struct ResilienceCache {
    tree: sled::Tree,
    encryption: EncryptionLayer,
    default_ttl: Option<Duration>,
    hit_counter: AtomicU64,
    miss_counter: AtomicU64,
}

impl ResilienceCache {
    pub fn open(config: CacheConfig) -> Result<Self, CacheError> {
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
            db.open_tree("stellar-cache")?
        };
        let encryption = EncryptionLayer::new(config.encryption_key)?;
        Ok(Self {
            tree,
            encryption,
            default_ttl: config.default_ttl,
            hit_counter: AtomicU64::new(0),
            miss_counter: AtomicU64::new(0),
        })
    }

    pub fn put<T>(
        &self,
        key: impl Into<CacheKey>,
        value: &T,
        policy: CachePolicy,
    ) -> Result<(), CacheError>
    where
        T: Serialize,
    {
        let key = key.into();
        let now = SystemTime::now();
        let expires_at = policy
            .ttl
            .or(self.default_ttl)
            .map(|ttl| duration_to_ts(now, ttl));
        let payload = bincode::serialize(value)?;
        let record = CacheRecord {
            payload,
            created_at: now_ts(),
            expires_at,
            hit_count: 0,
            version: 1,
        };
        let encoded_record = bincode::serialize(&record)?;
        let sealed = self.encryption.seal(encoded_record)?;
        self.tree.insert(key.as_str().as_bytes(), sealed)?;
        Ok(())
    }

    pub fn get<T>(&self, key: &CacheKey) -> Result<Option<T>, CacheError>
    where
        T: DeserializeOwned,
    {
        if let Some(value) = self.tree.get(key.as_str())? {
            match self.decode_record(&value)? {
                Some(mut record) => {
                    if let Some(expiry) = record.expires_at {
                        if expiry <= now_ts() {
                            let _ = self.tree.remove(key.as_str());
                            self.miss_counter.fetch_add(1, Ordering::Relaxed);
                            return Ok(None);
                        }
                    }
                    record.hit_count += 1;
                    self.hit_counter.fetch_add(1, Ordering::Relaxed);
                    TelemetryHub::global().record_cache_hit();
                    let serialized = bincode::serialize(&record)?;
                    let sealed = self.encryption.seal(serialized)?;
                    self.tree.insert(key.as_str(), sealed)?;
                    let value: T = bincode::deserialize(&record.payload)?;
                    Ok(Some(value))
                }
                None => {
                    self.miss_counter.fetch_add(1, Ordering::Relaxed);
                    TelemetryHub::global().record_cache_miss();
                    Ok(None)
                }
            }
        } else {
            self.miss_counter.fetch_add(1, Ordering::Relaxed);
            TelemetryHub::global().record_cache_miss();
            Ok(None)
        }
    }

    pub fn remove(&self, key: &CacheKey) -> Result<(), CacheError> {
        let _ = self.tree.remove(key.as_str())?;
        Ok(())
    }

    pub fn clear(&self) -> Result<(), CacheError> {
        self.tree.clear()?;
        self.hit_counter.store(0, Ordering::Relaxed);
        self.miss_counter.store(0, Ordering::Relaxed);
        Ok(())
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hit_counter.load(Ordering::Relaxed),
            misses: self.miss_counter.load(Ordering::Relaxed),
            items: self.tree.len(),
        }
    }

    pub fn hit_miss(&self) -> CacheHitMiss {
        CacheHitMiss {
            hits: self.hit_counter.load(Ordering::Relaxed),
            misses: self.miss_counter.load(Ordering::Relaxed),
        }
    }

    pub fn snapshot(&self) -> Result<CacheSnapshot, CacheError> {
        let mut entries = Vec::new();
        for item in self.tree.iter() {
            let (key, value) = item?;
            if let Some(record) = self.decode_record(&value)? {
                let encoded = bincode::serialize(&record)?;
                entries.push(CacheSnapshotEntry {
                    key: String::from_utf8_lossy(&key).into_owned(),
                    record: encoded,
                });
            }
        }
        Ok(CacheSnapshot { entries })
    }

    pub fn hydrate(&self, snapshot: CacheSnapshot) -> Result<(), CacheError> {
        self.clear()?;
        for entry in snapshot.entries {
            let sealed = self.encryption.seal(entry.record)?;
            self.tree.insert(entry.key.as_bytes(), sealed)?;
        }
        Ok(())
    }

    pub fn prune_expired(&self) -> Result<u64, CacheError> {
        let mut removed = 0u64;
        let now = now_ts();
        for item in self.tree.iter() {
            let (key, value) = item?;
            if let Some(record) = self.decode_record(&value)? {
                if let Some(expiry) = record.expires_at {
                    if expiry <= now {
                        self.tree.remove(key)?;
                        removed += 1;
                    }
                }
            }
        }
        Ok(removed)
    }

    fn decode_record(&self, raw: &IVec) -> Result<Option<CacheRecord>, CacheError> {
        let decrypted = self.encryption.open(raw)?;
        let record: CacheRecord = bincode::deserialize(&decrypted)?;
        Ok(Some(record))
    }
}

#[derive(Debug, Clone)]
pub struct CacheSnapshotEntry {
    pub key: String,
    pub record: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct CacheSnapshot {
    pub entries: Vec<CacheSnapshotEntry>,
}

impl CacheSnapshot {
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl Default for CacheSnapshot {
    fn default() -> Self {
        Self::empty()
    }
}

pub fn default_cache_path(base: &Path) -> PathBuf {
    base.join("resilience-cache")
}
