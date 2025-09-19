# RFC 0005: Stellar Delivery & Governance

**Status**: Draft  
**Author**: GPT-5-codex  
**Created**: 2025-09-18  
**Updated**: 2025-09-18  
**Reviewers**: Platform Engineering, SecOps, DX  
**Target Release**: M5 / Week 6–8

---

## 1. Goal & Non-Goals
- **Goal**: Ship Trusted Pipeline, Governance Portal, Policy Validator, and Marketplace guardrails satisfying REQ-OPS-01, REQ-INT-01, REQ-DX-01 (#24, #42, #55, #75, #79, #85, #99).
- **Success Metrics**: METRIC-EXT-ADOPT ≥ 60%; METRIC-AVAIL ≥ 99.5%; Review Effort ↓ 30% at M6.
- **Non-Goals**: Core kernel internals, security primitives already defined in RFC 0004, observability overlays (M4).

## 2. Scope & Personas
- **Platform Engineer**: Orchestrates signed releases, monitors governance dashboards.
- **SecOps**: Verifies policy sync, inspects audit trails.
- **Partner Developer**: Submits plugins; receives guardrail feedback.
- **Reviewers**: Consume fast-review package artifacts for PR evaluation.

## 3. Architecture Overview
```
┌────────────────┐     ┌────────────────────┐
│ Trusted        │     │ Policy Validator   │
│ Pipeline (#79) │────▶│ Service (#55)      │
└──────┬─────────┘     └──────┬─────────────┘
       │ signed bundle         │policy verdicts
┌──────▼─────────┐     ┌───────▼────────────┐
│ Governance     │◀────▶│ Marketplace Guard │
│ Portal (#85)   │     │ Rails (#75, #24)   │
└──────┬─────────┘     └───────┬────────────┘
       │ telemetry                     │
┌──────▼─────────┐            ┌───────▼────────────┐
│ CLI/TUI Bridge │            │ Zero-Trust Connector│
│ (#99, RFC 0002)│            │ (#92, #13)          │
└───────────────┘            └─────────────────────┘
```

## 4. Detailed Design
### 4.1 Trusted Pipeline
- Declarative pipeline spec with signed steps; integrates with Secure Signing service (RFC 0004).
- Supports canary→GA rollout with automated rollback triggers.
- Knowledge Pack auto-update orchestrated with version negotiation (REQ-INT-01).

### 4.2 Governance Portal
- Provides dashboards for policy drift, audit ledger sync, and release calendar.
- SSO-aware hints surface outstanding approvals; multi-tenant view for partner developers.

### 4.3 Policy Validator & Guardrails
- Static/dynamic policy analysis for submitted modules; includes zero-trust connector outcomes.
- Marketplace guardrails enforce compatibility, security posture, and resource limits.

### 4.4 Metrics Bridge & Drift Hook
- Optional local Prometheus endpoint aggregating delivery metrics.
- Drift hook allows allowlisted scripts for detection, capturing results in governance portal.

## 5. Constraints & Dependencies
- Pipeline must integrate with existing CI without regressions; maintain offline signing fallback.
- Requires instrumentation aligned with observability mesh (M4) for end-to-end tracing.
- Guardrail decisions must be explainable to reduce review effort.

## 6. Trace Anchors
| Requirement | Artifact | Validation |
| ----------- | -------- | ---------- |
| REQ-OPS-01 (#79, #23, #34, #58, #85) | Trusted pipeline, governance portal | Pipeline E2E tests, dashboard smoke tests |
| REQ-INT-01 (#42, #99, #13) | Version negotiation, CLI/TUI bridge | Integration tests, compatibility matrix |
| REQ-DX-01 (#75, #55, #24) | Marketplace guardrails, policy validator | Acceptance tests, partner developer dry runs |

## 7. Validation Plan
- Pipeline smoke + rollback simulation; signed artifact verification.
- Policy validator unit tests + scenario-based acceptance tests.
- Governance portal snapshot and accessibility tests.
- Metrics capture: adoption %, release success rate, approval turnaround time.

## 8. Open Questions & Risks
- Need decision on hosting model for governance portal (self-hosted vs SaaS) → ADR-DEL-001.
- Clarify integration boundaries with external marketplaces.
- Ensure reviewer fast-review package automation included in pipeline.
