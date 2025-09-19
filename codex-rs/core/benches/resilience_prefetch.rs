use codex_core::resilience::CacheKey;
use codex_core::resilience::cache::CacheConfig;
use codex_core::resilience::cache::CachePolicy;
use codex_core::resilience::cache::ResilienceCache;
use codex_core::resilience::prefetch::PredictivePrefetch;
use criterion::Criterion;
use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use serde::Serialize;
use std::time::Duration;
use tempfile::tempdir;

#[derive(Serialize, serde::Deserialize, Clone)]
struct BenchPayload {
    insight: String,
    persona: String,
}

fn cache_put_get(c: &mut Criterion) {
    c.bench_function("resilience_cache_put_get", |b| {
        b.iter(|| {
            let cache = ResilienceCache::open(CacheConfig::default()).expect("cache open");
            let key = CacheKey::new("bench.insight");
            let payload = BenchPayload {
                insight: "Improve deployment pipeline".to_string(),
                persona: "operator".to_string(),
            };
            cache
                .put(
                    key.clone(),
                    &payload,
                    CachePolicy {
                        ttl: Some(Duration::from_secs(900)),
                    },
                )
                .expect("cache put");
            let _value: Option<BenchPayload> = cache.get(&key).expect("cache get");
        })
    });
}

fn cache_snapshot_restore(c: &mut Criterion) {
    c.bench_function("resilience_cache_snapshot_hydrate", |b| {
        b.iter(|| {
            let dir = tempdir().expect("snapshot tempdir");
            let cache = ResilienceCache::open(CacheConfig {
                path: Some(dir.path().join("cache")),
                tree_name: None,
                encryption_key: None,
                default_ttl: Some(Duration::from_secs(900)),
                temporary: false,
            })
            .expect("cache open");
            let payload = BenchPayload {
                insight: "Stabilise queue under outage".to_string(),
                persona: "sre".to_string(),
            };
            cache
                .put(CacheKey::new("snapshot"), &payload, CachePolicy::default())
                .expect("seed cache");
            let snapshot = cache.snapshot().expect("snapshot");
            drop(cache);

            let restored = ResilienceCache::open(CacheConfig {
                path: Some(dir.path().join("cache")),
                tree_name: None,
                encryption_key: None,
                default_ttl: Some(Duration::from_secs(900)),
                temporary: false,
            })
            .expect("cache reopen");
            restored.hydrate(snapshot).expect("hydrate");
            let _value: Option<BenchPayload> =
                restored.get(&CacheKey::new("snapshot")).expect("get");
        })
    });
}

fn prefetch_record_top(c: &mut Criterion) {
    let prefetch = PredictivePrefetch::default();
    c.bench_function("prefetch_record_top", |b| {
        b.iter(|| {
            for key in ["insight.submit", "runbook.invoke", "cache.flush"] {
                prefetch.record(black_box(key));
            }
            black_box(prefetch.top(3));
        })
    });
}

criterion_group!(
    resilience,
    cache_put_get,
    cache_snapshot_restore,
    prefetch_record_top
);
criterion_main!(resilience);
