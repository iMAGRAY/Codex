# ADR-DEL-001: Governance Portal Deployment Model

## Status
Draft

## Context
- Governance portal must serve platform engineers, SecOps, and partner developers with high availability (REQ-OPS-01, REQ-DX-01).
- Requires trusted pipeline integration and SSO-aware hints.

## Decision
Deploy governance portal as a self-hosted service bundled with Codex enterprise distribution, running on the same trust boundary as the Trusted Pipeline, with optional SaaS mirror for read-only stakeholders. Authentication handled via organization SSO with scoped tokens for partner developers.

## Consequences
- **Positive**: Maintains control over signing keys and audit data; easier compliance alignment.
- **Negative**: Increases operational burden for on-prem teams.
- **Operational**: Provide Terraform module + Helm chart for deployment; SaaS mirror must replicate dashboards without write access.

## Alignment
- Requirements: REQ-OPS-01, REQ-DX-01, REQ-INT-01.
- Metrics: METRIC-EXT-ADOPT, METRIC-AVAIL.
- Linked Artifacts: `docs/rfcs/0005-stellar-delivery.md`, deployment runbooks, pipeline integration tests.
