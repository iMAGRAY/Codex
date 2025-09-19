# Stellar Security & Sandbox Plan (M3) - Threat Model & Hardening

## Inputs
- `docs/rfcs/0004-stellar-security.md` — рамки RBAC, Secure Signing, Dynamic Secrets, Sandbox Runner (REQ-SEC-01/02/03; #9, #10, #11, #27, #70, #74, #88).
- `docs/future/MaxThink-Stellar.md` — метрики METRIC-SEC-INC, METRIC-AUDIT-OK, требования SRE/SecOps.
- `docs/future/stellar-tui-vision.md` — UX/Accessibility требования для consent flows и banners.
- `docs/future/stellar/backlog.md` — EPIC-SECURITY трейс к #9/#10/#11/#27/#57/#74/#88.
- `docs/future/stellar/definition-of-done.md` — секьюрити и compliance чек-листы.

## Checklist
- [ ] THREAT — Провести threat modeling по RBAC, секретам, supply chain (REQ-SEC-01/02/03; #9, #11, #27, #70).
- [ ] SANDBOX — Спроектировать Sandbox Runner + Manifest Write Shield (bubblewrap/nsjail) с guardrails (#10, #70).
- [ ] COMPLIANCE — Подготовить Compliance Pre-flight checklist и Consent Banner copy (#38, #57).

## Outputs
- Threat Model canvas (attack vectors, mitigations, owners, trace to REQ-SEC-01/02/03).
- Sandbox Runner architecture: privilege boundaries, manifest policy, integration points.
- Compliance pre-flight checklist + consent banner draft (A11y + legal review path).

---

## 1. Threat Modeling Summary (REQ-SEC-01/02/03)

| Vector | Impact | Mitigation | Trace |
| ------ | ------ | ---------- | ----- |
| Unauthorized command execution via CLI bridge (RBAC gap) | Privilege escalation, METRIC-SEC-INC breach | Persona-scoped RBAC policies enforced in `codex_cli::mcp_cmd` before dispatch; signed command envelope (`Secure Command Signing`) with timestamped nonce. | REQ-SEC-01 (#9, #74) |
| Secrets leakage during wizard analysis | Exposure of credentials, partner breach | Dynamic Secrets Injection with temp vault lease, scrubbed from logs; secure clipboard redaction; wizard masking for env vars. | REQ-SEC-03 (#27, #88, #92) |
| Supply chain tampering of manifests/plugins | Compromised sandbox runtime | Signed manifests with checksum validation, policy allowlist, immutable audit entry per load; runner verifies signature before hydrated manifest. | REQ-SEC-02 (#10, #57) |
| Sandbox escape via filesystem writes | Host compromise | Manifest Write Shield (bubblewrap/nsjail) restricting write targets; runtime seccomp profile with CPU/mem quotas; audit on violation. | REQ-SEC-01/02 (#9, #10) |
| Audit trail forgery | Loss of traceability, METRIC-AUDIT-OK drop | Immutable Audit Ledger (append-only, hash-linked), periodic export to secure storage; cross-check with governance portal. | REQ-SEC-02 (#57) |

### Action Items
1. Define RBAC policy schema & signing envelope format (owner: Security eng).
2. Implement dynamic secrets broker + scrubber hooks in wizard pipeline (owner: Platform).
3. Extend audit ledger to hash link entries + export schedule (owner: SecOps).

## 2. Sandbox Runner & Manifest Guardrails (#10, #70)

### Architecture
- **Runner**: Bubblewrap/nsjail profile with namespace isolation, CPU/RAM/time quotas, outbound network deny by default.
- **Manifest Write Shield**: Declarative allowlist of writable paths (tmpfs, project cache). All manifests validated before run; unauthorized writes trigger runner halt.
- **Integration**: Runner invoked via `codex_exec` provider; telemetry events feed METRIC-SEC-INC and METRIC-AUDIT-OK.

### Policy Hooks
- Manifest Validation Pipeline:
  1. Signature + checksum check (Secure Signing key rotation).
  2. Schema validation (include RBAC persona scope).
  3. Capability diff vs baseline → audit entry.
- Runtime Enforcement:
  - Seccomp profile loaded per manifest class.
  - Real-time resource monitor -> resilience queue for throttling.
  - Consent state forwarded to compliance log.

## 3. Compliance Pre-Flight & Consent Banner (#38, #57)

### Pre-Flight Checklist (to run before sandbox exec)
1. Verify RBAC persona and scope (no wildcard roles).
2. Confirm manifest signature + hash (log ledger ID).
3. Validate secrets source (vault lease active, expiry ≥ session window).
4. Confirm audit channel healthy (ledger append works, export destination reachable).
5. Accessibility review (voiceover compatible banner text).

### Consent Banner Copy (draft)
> "Codex will execute commands in a secured sandbox with limited access to your workspace. By continuing, you confirm that RBAC policies and manifest permissions for this session are approved. Recorded events will appear in the immutable audit ledger."

- Localized strings to follow TUI language pack.
- Logging: consent accepted/declined events recorded with timestamp + persona + manifest ID.

## 4. Validation Plan (Preview)
- Security unit tests for RBAC enforcement, secrets scrubber, audit ledger hashing.
- Sandbox integration tests (nsjail/bubblewrap) with manifest fixtures.
- Compliance pre-flight automation: CLI command `codex sandbox preflight --manifest <path>` returns checklist status.

---

*Prepared 2025-09-18 referencing `MaxThink-Stellar.md` (REQ-SEC-01/02/03) and `stellar-tui-vision.md` accessibility guardrails.*
