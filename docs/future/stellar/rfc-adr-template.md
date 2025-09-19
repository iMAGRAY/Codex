# Stellar RFC & ADR Template

> Use this template for new Stellar TUI RFCs (docs/rfcs) and the paired ADR drafts (docs/future/stellar/adrs).

## RFC Metadata Block
```
# RFC 00XX: <Title>

**Status**: Draft | Review | Accepted | Superseded
**Author**: <Name>
**Created**: YYYY-MM-DD
**Updated**: YYYY-MM-DD
**Reviewers**: Core, SecOps, SRE, DX, Accessibility
**Target Release**: M<1-6> / Week <n>
```

## 1. Goal & Non-Goals
- **Goal**: tie to explicit requirements (e.g., REQ-UX-01 from `MaxThink-Stellar.md`).
- **Success Metrics**: map to METRIC-* ids defined in roadmap.
- **Non-Goals**: clarify exclusions to keep scope minimal.

## 2. Scope & Personas
- User personas impacted (Operator, SRE, SecOps, Platform Engineer, Partner Developer, Assistive Tech).
- Primary scenarios referencing blueprint idea numbers (#1–#99).

## 3. Architecture Overview
- High-level topology diagram or reference to existing blueprint section.
- Interaction flow with data/control planes.

## 4. Detailed Design
- Components, data structures, algorithms, lifecycle hooks.
- Accessibility, security, performance considerations inline.
- Call out feature flags and rollout phases.

## 5. Constraints & Dependencies
- Regulatory, compliance, interoperability constraints.
- Dependencies on other RFCs, ADRs, or third-party systems.

## 6. Trace Anchors
- Table linking requirements → design artifacts → validation assets (tests, metrics dashboards).

## 7. Validation Plan
- Unit tests, snapshot tests, security checks, observability instrumentation.
- Metrics baseline & success thresholds (APDEX, LATENCY, SEC-INC, etc.).

## 8. Open Questions & Risks
- Unresolved decisions flagged for follow-up in Validate phase.

## ADR Draft Template
```
# ADR-<ID>: <Decision Title>

## Status
Draft | Proposed | Accepted | Superseded

## Context
- Summarize problem statement and forces.
- Reference related REQ and RFC section.

## Decision
- Concise statement of the decision.

## Consequences
- Positive/negative outcomes, operational impact, testing implications.

## Alignment
- Requirements: <REQ IDs>
- Metrics: <METRIC IDs>
- Linked Artifacts: <tests, dashboards, runbooks>
```

## Usage Notes
- Store approved RFCs under `docs/rfcs/` with incremental numbering.
- Stage ADR drafts under `docs/future/stellar/adrs/` and promote to `docs/adrs/` once accepted.
- Update `docs/future/index.md` with new RFC/ADR links during Validate step.
