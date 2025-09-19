# ADR-SEC-001: Immutable Audit Ledger Storage

## Status
Draft

## Context
- Security roadmap mandates tamper-evident audit logging (REQ-SEC-02) with export support for governance portal (#57).
- Ledger must operate offline and sync once connectivity returns, aligning with resilience requirements.

## Decision
Adopt a Merkleized append-only log stored locally using RocksDB with periodic signed snapshots pushed to governance services; snapshots chained with Ed25519 signatures managed by Secure Signing service.

## Consequences
- **Positive**: Efficient append performance, strong tamper detection, compatibility with signed snapshot exports.
- **Negative**: Adds dependency on RocksDB; requires careful tuning for SSD footprint.
- **Operational**: Snapshot rotation policy every 5 minutes or 1 MB, whichever comes first; failure triggers alert to SecOps.

## Alignment
- Requirements: REQ-SEC-02, REQ-OPS-01.
- Metrics: METRIC-SEC-INC, METRIC-AUDIT-OK.
- Linked Artifacts: `docs/rfcs/0004-stellar-security.md`, audit integration tests, governance export pipeline.
