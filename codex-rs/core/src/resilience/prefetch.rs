use crate::resilience::metrics::PrefetchStats;
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Debug, Default)]
pub struct PredictivePrefetch {
    usage: RwLock<HashMap<String, u64>>,
    stats: RwLock<PrefetchStats>,
}

impl PredictivePrefetch {
    pub fn record(&self, key: impl Into<String>) {
        let key = key.into();
        {
            let mut usage = self.usage.write().expect("prefetch usage lock");
            *usage.entry(key.clone()).or_insert(0) += 1;
        }
        let mut stats = self.stats.write().expect("prefetch stats lock");
        stats.scheduled += 1;
        stats.completed += 1;
    }

    pub fn top(&self, limit: usize) -> Vec<(String, u64)> {
        let usage = self.usage.read().expect("prefetch usage lock");
        let mut pairs: Vec<_> = usage.iter().map(|(k, v)| (k.clone(), *v)).collect();
        pairs.sort_by(|a, b| b.1.cmp(&a.1));
        pairs.truncate(limit);
        pairs
    }

    pub fn stats(&self) -> PrefetchStats {
        *self.stats.read().expect("prefetch stats lock")
    }
}
