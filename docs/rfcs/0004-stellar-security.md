# RFC 0004: Stellar Security & Sandbox Hardening

**Status**: Draft  
**Author**: GPT-5-codex  
**Created**: 2025-09-18  
**Updated**: 2025-09-18  
**Reviewers**: SecOps, Security Architecture, Core  
**Target Release**: M3 / Week 4–6

---

## 1. Goal & Non-Goals
- **Goal**: Implement RBAC, Secure Command Signing, Dynamic Secrets, Sandbox Runner hardening, and immutable audit ledger (REQ-SEC-01/02/03, REQ-OPS-01; blueprint #9, #10, #11, #27, #57, #74, #88).
- **Success Metrics**: METRIC-SEC-INC = 0 critical findings; METRIC-AUDIT-OK ≥ 95%; no P0 sandbox escapes.
- **Non-Goals**: Observability overlays (RFC 0005) and delivery pipeline automation outside security gating.

## 2. Scope & Personas
- **SecOps**: Manage policies, rotate secrets, approve command signing keys.
- **SRE/Operator**: Consume security banners, receive inline guardrail feedback.
- **Platform Engineer**: Integrate security signals into pipeline gating.

## 3. Architecture Overview
```
┌────────────┐   policy query    ┌─────────────────┐
│ Command    │──────────────────▶│ RBAC Policy     │
│ Router     │                   │ Engine (#11)    │
└────┬───────┘                   └─────┬───────────┘
     │ allow/deny                        │
┌────▼────────┐   sign requests   ┌──────▼─────────┐
│ Sandbox     │◀──────────────────│ Secure Signing │
│ Runner      │                   │ Service (#9)   │
└────┬────────┘                   └──────┬─────────┘
     │ secrets lease                      │audit trail
┌────▼────────┐                       ┌────▼──────────┐
│ Dynamic     │                       │ Immutable     │
│ Secrets (#27)│                      │ Audit Ledger  │
└─────────────┘                       └──────────────┘
```

## 4. Detailed Design
### 4.1 RBAC Policy Engine
- Hierarchical roles aligned with alignment matrix; policies expressed via declarative YAML with static analysis.
- Enforces scoped capabilities, attribute-based conditions, and time-bound overrides.

### 4.2 Secure Command Signing
- Use Ed25519 signatures stored in HSM-backed key vault; pipeline enforces signature verification before execution.
- Session banners warn users about unsigned modules; Input Guard (RFC 0002) blocks execution.

### 4.3 Dynamic Secrets & Injection
- Lease-based secret distribution; integrate with resilience cache for offline usage with secure expiration.
- Clipboard redaction and secure logging ensure no secret persistence.

### 4.4 Sandbox Runner Hardening
- Hardened seccomp profiles, resource quotas, and network isolation toggles.
- Security telemetry forwarded to observability mesh for incident detection.

### 4.5 Immutable Audit Ledger
- Append-only log with hash chaining; exports to governance portal.
- Supports tamper detection, SSO-aware context, and redaction filters.

## 5. Constraints & Dependencies
- Must integrate with existing `CODEX_SANDBOX_ENV_VAR` controls without modification.
- Requires cryptographic libraries that meet enterprise compliance.
- Works with offline mode, deferring verification until connection restored but marking commands as pending.

## 6. Trace Anchors
| Requirement | Artifact | Validation |
| ----------- | -------- | ---------- |
| REQ-SEC-01 (#9, #11, #74) | RBAC engine, secure signing | Policy unit tests, signing integration tests |
| REQ-SEC-02 (#10, #57) | Audit ledger, export adapters | Ledger consistency tests, export verification |
| REQ-SEC-03 (#27, #88) | Dynamic secrets, sandbox hardening | Secrets lease testing, sandbox smoke tests |
| REQ-OPS-01 (#23) | Security signals to operations | Pipeline gating tests, alerting simulation |

## 7. Validation Plan
- Security unit/integration tests (`cargo test -p codex-security` TBD crate).
- Static analysis (cargo audit, custom lint) and dependency scanning.
- Threat modeling review with SecOps, update STRIDE findings.
- Chaos security drills (credential expiration, sandbox escape attempts).

## 8. Open Questions & Risks
- Need final decision on ledger storage backend → ADR-SEC-001.
- Determine policy authoring tooling (in-house vs open-source DSL).
- Evaluate compatibility with partner marketplace modules for signed distribution.
