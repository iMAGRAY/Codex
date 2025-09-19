pub mod cache;
pub mod confidence;
pub mod conflict;
pub mod metrics;
pub mod prefetch;
pub mod transport;

pub use cache::CacheConfig;
pub use cache::CacheError;
pub use cache::CacheKey;
pub use cache::CachePolicy;
pub use cache::CacheSnapshot;
pub use cache::ResilienceCache;
pub use confidence::ConfidenceBreakdown;
pub use confidence::ConfidenceCalculator;
pub use confidence::ConfidenceFactor;
pub use confidence::ConfidenceInput;
pub use confidence::ConfidenceScore;
pub use conflict::ConflictDecision;
pub use conflict::ConflictEntry;
pub use conflict::ConflictId;
pub use conflict::ConflictResolver;
pub use conflict::ResolutionState;
pub use conflict::SourceValue;
pub use metrics::CacheHitMiss;
pub use metrics::CacheStats;
pub use metrics::PrefetchStats;
pub use prefetch::PredictivePrefetch;
pub use transport::QueueConfig;
pub use transport::RetryItem;
pub use transport::RetryQueue;
pub use transport::TransportError;

use std::fmt;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ResilienceInitError {
    #[error("cache init: {0}")]
    Cache(#[from] cache::CacheError),
    #[error("transport init: {0}")]
    Transport(#[from] transport::TransportError),
}

#[derive(Clone)]
pub struct ResilienceServices {
    pub cache: Arc<ResilienceCache>,
    pub queue: Arc<RetryQueue>,
    pub conflicts: Arc<ConflictResolver>,
    pub confidence: ConfidenceCalculator,
    pub prefetch: Arc<PredictivePrefetch>,
}

impl ResilienceServices {
    pub fn new(cache: ResilienceCache, queue: RetryQueue) -> Self {
        Self {
            cache: Arc::new(cache),
            queue: Arc::new(queue),
            conflicts: Arc::new(ConflictResolver::new()),
            confidence: ConfidenceCalculator::default(),
            prefetch: Arc::new(PredictivePrefetch::default()),
        }
    }

    pub fn with_conflicts(
        cache: ResilienceCache,
        queue: RetryQueue,
        conflicts: ConflictResolver,
        confidence: ConfidenceCalculator,
        prefetch: PredictivePrefetch,
    ) -> Self {
        Self {
            cache: Arc::new(cache),
            queue: Arc::new(queue),
            conflicts: Arc::new(conflicts),
            confidence,
            prefetch: Arc::new(prefetch),
        }
    }

    pub fn default() -> Result<Self, ResilienceInitError> {
        let cache = ResilienceCache::open(CacheConfig::default())?;
        let queue = RetryQueue::open(QueueConfig::default())?;
        Ok(Self::new(cache, queue))
    }
}

impl fmt::Debug for ResilienceServices {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResilienceServices")
            .field("cache_items", &self.cache.stats().items)
            .field("queue_length", &self.queue.len())
            .field("conflicts", &self.conflicts.list_pending(10).len())
            .field("prefetch_top", &self.prefetch.top(3))
            .finish()
    }
}
