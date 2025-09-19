# Resilience Chaos Harness

Run the resilience chaos loop to exercise queue/cache recovery for at least ten minutes:

```bash
cd codex-rs
./scripts/chaos/resilience_loop.sh 600 15
```

- `600` — total duration in seconds (default 600).
- `15` — sleep between iterations (default 15).
- Set `CHAOS_LOG_DIR` to override the log directory (defaults to `target/chaos`).

The script invokes `cargo test -p codex-core --test resilience_chaos -- --nocapture` repeatedly, logs each pass/fail, and exits non-zero if any iteration fails.
