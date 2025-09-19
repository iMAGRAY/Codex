use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, Default)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub items: usize,
}

impl CacheStats {
    pub fn hit_ratio(&self) -> f32 {
        let total = self.hits + self.misses;
        if total == 0 {
            return 1.0;
        }
        self.hits as f32 / total as f32
    }
}

#[derive(Debug, Clone, Copy, Serialize, Default)]
pub struct CacheHitMiss {
    pub hits: u64,
    pub misses: u64,
}

impl CacheHitMiss {
    pub fn total(&self) -> u64 {
        self.hits + self.misses
    }
}

#[derive(Debug, Clone, Copy, Serialize, Default)]
pub struct PrefetchStats {
    pub scheduled: u64,
    pub completed: u64,
    pub skipped: u64,
    pub avg_latency_ms: f32,
}
