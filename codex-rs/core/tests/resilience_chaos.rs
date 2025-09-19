use codex_core::resilience::CacheKey;
use codex_core::resilience::ResilienceServices;
use codex_core::resilience::cache::CacheConfig;
use codex_core::resilience::cache::CachePolicy;
use codex_core::resilience::cache::ResilienceCache;
use codex_core::resilience::transport::QueueConfig;
use codex_core::resilience::transport::RetryQueue;
use codex_core::stellar::StellarKernel;
use codex_core::stellar::StellarPersona;
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn retry_queue_recovers_after_simulated_network_drop() {
    let dir = tempdir().expect("queue tempdir");
    let queue = RetryQueue::open(QueueConfig {
        path: Some(dir.path().join("queue")),
        tree_name: None,
        temporary: false,
    })
    .expect("queue");

    // enqueue insight submissions while network is down
    for idx in 0..5 {
        queue
            .enqueue(
                "insight.submit",
                serde_json::Value::String(format!("payload-{idx}")),
                5,
            )
            .expect("enqueue");
    }
    assert_eq!(queue.len(), 5, "all items persisted during outage");

    // network restored; drain and re-dispatch items
    let drained = queue.drain_ready(10).expect("drain ready");
    assert_eq!(
        drained.len(),
        5,
        "all items drained once connection restored"
    );

    for item in drained {
        // ensure re-delivery drops items permanently after handler succeeds
        queue.remove(item.id).expect("remove");
    }

    assert_eq!(queue.len(), 0, "queue empty after successful replay");
}

#[test]
fn cache_snapshot_hydrate_survives_restart() {
    let dir = tempdir().expect("cache tempdir");
    let cache = ResilienceCache::open(CacheConfig {
        path: Some(dir.path().join("cache")),
        tree_name: None,
        encryption_key: None,
        default_ttl: Some(Duration::from_secs(1)),
        temporary: false,
    })
    .expect("cache");
    let key = CacheKey::new("chaos.insight");
    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct CacheRecord {
        payload: String,
    }

    cache
        .put(
            key.clone(),
            &CacheRecord {
                payload: "resilience".to_string(),
            },
            CachePolicy::default(),
        )
        .expect("cache put");

    let snapshot = cache.snapshot().expect("snapshot");
    assert!(!snapshot.entries.is_empty(), "snapshot captured entries");

    drop(cache);

    // simulate restart by reopening and hydrating from snapshot
    let replacement = ResilienceCache::open(CacheConfig {
        path: Some(dir.path().join("cache")),
        tree_name: None,
        encryption_key: None,
        default_ttl: Some(Duration::from_secs(5)),
        temporary: false,
    })
    .expect("cache reopen");
    replacement.hydrate(snapshot).expect("hydrate");

    let restored: Option<CacheRecord> = replacement.get(&key).expect("hydrate get");
    assert_eq!(
        restored,
        Some(CacheRecord {
            payload: "resilience".to_string(),
        })
    );
}

#[test]
fn kernel_risk_alerts_reflect_resilience_state() {
    let services = ResilienceServices::default().expect("resilience services");
    // force telemetric adjustments by submitting insight and leaving queue populated
    let mut kernel = StellarKernel::with_resilience(StellarPersona::Operator, services.clone());
    kernel.set_field_text("Investigate retry backlog");
    let _ = kernel.handle_action(codex_core::stellar::StellarAction::SubmitInsight { text: None });
    assert!(
        !kernel.snapshot().risk_alerts.is_empty(),
        "risk alerts rendered"
    );
}
