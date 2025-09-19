# ADR-RES-001: Local Resilience Cache Backend

## Status
Draft

## Context
- Resilience cache must offer offline support, crash recovery, and deterministic snapshots (REQ-REL-01, REQ-DATA-01).
- Need lightweight dependency with cross-platform support for CLI/TUI usage.

## Decision
Default to `sled` embedded store with encryption-at-rest wrapper, while providing pluggable backend interface allowing sqlite or remote KV implementations for specialized deployments.

## Consequences
- **Positive**: `sled` delivers fast reads/writes, minimal operational footprint.
- **Negative**: Requires compaction tuning; limited tooling compared to sqlite.
- **Operational**: Provide migration utilities and self-heal logic to rebuild cache on corruption.

## Alignment
- Requirements: REQ-REL-01, REQ-DATA-01.
- Metrics: METRIC-AVAIL, METRIC-LATENCY.
- Linked Artifacts: `docs/rfcs/0003-stellar-resilience.md`, cache integration tests, chaos scripts.
