# Threat Model

**Last updated:** 2026-06-03 · **Scope:** Tellur core/CLI (local-first) and the
`tellur-server` hub (Tier 1, in progress). Updated whenever the API surface or
trust boundaries change (per `AGENTS.md` / NIST SSDF).

## Assets

- Provenance evidence (events, attribution) and its **integrity** (hash chain).
- Sensitive content that may pass through capture: prompts, command output,
  file paths, secrets.
- Author identity (stored as salted hashes — personal data under GDPR).
- Hub credentials: API tokens, session cookies, signing keys.
- Audit log integrity.

## Trust boundaries

1. **Local machine** — CLI, editor plugins, daemon (loopback). Trusted to the
   OS-user level.
2. **Git remote** — transports `refs/notes/ai` (Tier 0). Integrity from the
   hash chain, not from the transport.
3. **Network → hub** (Tier 1) — the main new boundary: untrusted clients over
   the network reach `tellur-server`. Authenticated, tenant-scoped surfaces:
   provenance **ingest** (`POST .../repos/{repo}/events`, redaction + storage),
   **reads/report** (`GET .../repos`, `.../events`, `.../report`), **central
   policy distribution** (`PUT/GET .../policies[/{name}]` — admin write of policy
   bodies, validated before storage), **attribution ingest**
   (`POST .../repos/{repo}/attributions`, contributor+), and the **export portal**
   (`GET .../export/events|audit` and per-repo `.../export/slsa|spdx` — admin,
   org/repo data disclosure). Operational
   endpoints (`/healthz`, `/readyz`, `/metrics`) are unauthenticated but expose
   only liveness and aggregate counters — no tenant data.
4. **Hub → storage** — SQLite (embedded, same host) or Postgres (network, via
   `TELLUR_DATABASE_URL`); tenant isolation enforced here. The Postgres client
   connects with **NoTls**, so the hub↔Postgres link is a trust boundary that
   must be kept on a private network or fronted by a TLS-terminating proxy; the
   connection string is a secret (provide it via env/secret store, never commit
   it). The same server-side hash-chain recomputation + head checkpoints apply
   regardless of backend, so a compromised DB cannot silently forge provenance
   without detection by `verify_*_chain`.

## STRIDE analysis (hub focus)

| Category | Threat | Mitigation |
| --- | --- | --- |
| **Spoofing** | Forged identity / stolen token | Per-user tokens hashed at rest (Argon2id), short-lived session cookies (HttpOnly/Secure/SameSite=strict); OIDC SSO in Tier 2; deny-by-default. |
| **Tampering** | Forged or altered provenance; modified data in transit | On ingest the server **recomputes the per-repo hash chain** (`hash_event`) — client-supplied hashes are ignored. Both the audit log and each repo's event chain persist a **head-hash + length checkpoint** so tail truncation / rollback to an earlier prefix is detected by `verify_*_chain`. TLS 1.3 in transit; append-only logs. |
| **Repudiation** | "I didn't do that" | Tamper-evident, hash-chained **audit log** of auth/data/policy/export events; ingests, **reads**, reports, and access denials are all recorded. Corrupt stored payloads surface as errors rather than silent nulls. |
| **Information disclosure** | Secrets/PII leak via ingested payloads, logs, cross-tenant reads, or bulk export | Inbound ingest payloads are **recursively secret-redacted** before storage; hub stores no raw prompts by default; **data-layer tenant scoping** (every query filtered by `org_id`) prevents BOLA; org-wide **export endpoints are admin-only, rate-limited, and audited**; no secrets/PII in logs; encryption at rest. |
| **Denial of service** | Resource exhaustion via large/abusive requests | Ingest has a 1 MiB body cap (router layer), a max-events-per-request cap, and a per-member rate limiter (`429`). Reads are paginated (clamped); the org report and exports are rate-limited + index-backed; a job-backed report/export path is planned for B5. |
| **Elevation of privilege** | Viewer acts as admin / cross-org access | RBAC enforced at the data layer on **object + tenant**, not just role (ingest needs contributor+; policy writes and exports need admin; all scoped to the caller's own org); BOLA regression tests (two orgs cannot touch each other's objects). **Fine-grained per-repo grants are additive only** (effective role = `max(org_role, repo_grant)`): a grant can elevate a member on a specific repo but never reduce them below their org role, and grants are tenant-scoped (the repo *and* the member must belong to the org, blocking cross-tenant grants). Grant management is org-admin only and audited (`repo_role.set`/`repo_role.remove`). Uploaded policy bodies are declarative YAML, validated before storage — no code execution. |

## Local-first surfaces (existing)

- **Daemon** is loopback-only, token-authenticated, with a Host-header check
  (anti DNS-rebinding) and server-side hash-chain recomputation.
- **Hook/webhook ingestion** never captures the whole working tree without a
  concrete file path; invalid payloads are ignored; redaction runs on inbound
  command/text fields.
- **Editor capture** records file changes; origin (AI vs human) is decided by the
  attribution layer, not asserted by the client.

## Key residual risks

- Import-only adapters prove what was in the imported source, not that Tellur
  observed it live (documented in `docs/ADAPTERS.md`).
- A compromised local OS user can read local `.tellur` data — out of scope for
  local-first; encryption-at-rest options are documented for sensitive setups.
- Prompt-injection of AI agents is recorded as evidence, not fully prevented
  (PRD §14.6).

## Review triggers

Re-run this analysis when: adding a network endpoint, changing auth/authz,
adding a storage backend, changing what data is stored, or adding a new capture
source.
