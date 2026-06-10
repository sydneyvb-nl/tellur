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
   **reads/report/dashboard** (`GET .../repos`, `.../events`, `.../report`,
   `.../dashboard` — viewer+, the dashboard adds a recent-activity feed), **central
   policy distribution** (`PUT/GET .../policies[/{name}]` — admin write of policy
   bodies, validated before storage), **attribution ingest**
   (`POST .../repos/{repo}/attributions`, contributor+), and the **export portal**
   — org bundles are **durable jobs**: `POST .../export/events|audit` enqueues
   (admin) and returns a job id, polled at `GET .../jobs/{id}` or listed via
   `GET .../jobs` (admin, tenant-scoped — the worker-produced result carries org
   data). Per-repo `.../export/slsa|spdx` are available both synchronously (`GET`)
   and as durable jobs (`POST`, admin / per-repo-admin), and
   `POST .../export/evidence` enqueues an org-wide evidence pack (every repo's
   SLSA provenance + latest compliance + audit-chain status; admin). Durable jobs
   carry a `params` column scoped under the job's own `org_id`, so a per-repo
   export job can only ever read that org's repo. Admins can
   also **read the audit log** (`GET .../audit` — paginated, filterable, tenant-
   scoped; audit detail can name members/actions, so it is admin-only and the
   first page returns `chain_intact` for the tamper-evident hash chain).
   **Policy compliance** (`POST .../policies/compliance` enqueues a job that
   evaluates the org policy over stored attribution; `GET .../policies/compliance`
   reads the latest snapshot — admin, tenant-scoped) and **People & Access**
   reads (`GET .../members`, `GET .../groups` — a session-auth mirror of
   `/scim/v2/Groups` so the browser never needs a SCIM token, `GET .../sso-status`)
   are admin-only; sso-status returns configuration/health and counts only —
   **no client secret or token material**. Operational
   endpoints (`/healthz`, `/readyz`, `/metrics`) are unauthenticated but expose
   only liveness and aggregate counters — no tenant data. The **team dashboard
   SPA** is served as static assets at `/app/*` (unauthenticated, but they carry
   no tenant data — the browser fetches all data from the authenticated `/v1`
   API with the first-party SSO session cookie, same-origin, so no CORS and no
   token in the URL). A strict same-origin CSP (`default-src 'self'`, no remote
   origins, self-hosted fonts) applies to `/app`. **SSO endpoints**
   (`/auth/login`, `/auth/callback`, `/auth/logout`) are unauthenticated entry
   points for the browser OIDC flow (404 when SSO is not configured). **SCIM
   provisioning** (`/scim/v2/Users`) authenticates with a dedicated, org-scoped
   bearer token (separate from member tokens, stored Argon2id-hashed); the org
   is derived from the token (never the URL), so an IdP can only provision into
   its own tenant. Deprovisioning (`DELETE` / `PATCH active=false`) sets
   `member.active = false`, which all auth paths (API token, session, SSO email)
   reject — so revocation is immediate across every credential type. **SCIM
   Groups** (`/scim/v2/Groups`) drive org roles: a group `displayName` of
   `tellur-admin|tellur-contributor|tellur-viewer` sets its members' role
   (recomputed on membership change); removal from the last mapping group (or
   its deletion) **revokes** the elevated role back to the `viewer` baseline, so
   group sync leaves no stale access. All IdP-driven SCIM mutations
   (user/group create/replace/patch/delete) are written to the tamper-evident
   audit log. The browser dashboard (`web/`) is intended to be served
   **same-origin** with the hub so its session cookie is first-party and no CORS
   relaxation is needed.
4. **Hub → IdP** (Tier 2, optional) — when SSO is configured the hub calls the
   OIDC issuer's discovery + token endpoints over **TLS** (OIDC Authorization
   Code + PKCE). The ID token is obtained on this direct TLS channel, so its
   integrity rests on TLS server validation (OIDC Core §3.1.3.7); the hub still
   validates `iss`/`aud`/`exp` and the per-login `nonce`. Because that integrity
   depends on TLS, the hub **rejects non-HTTPS** issuer/authorization/token
   endpoints (loopback `http` is allowed only for local dev). The client secret
   is a secret (env/secret store). No open self-registration: only
   pre-provisioned members (by verified email) may sign in, and the OIDC subject
   is bound on first login and never silently re-bound (a second IdP account on
   the same email is refused). The discovered metadata `issuer` must match the
   configured issuer, and subject bindings are keyed by `(issuer, subject)` (a
   `sub` is only unique per issuer). The callback is bound to the initiating
   browser via a short-lived `HttpOnly`/`Secure` login cookie matched against a
   server-stored secret (defeats login-CSRF / session fixation from a forwarded
   callback URL). Anonymous `/auth/login` rows are TTL-pruned and hard-capped.
5. **Hub → storage** — SQLite (embedded, same host) or Postgres (network, via
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
| **Spoofing** | Forged identity / stolen token | Per-user tokens hashed at rest (Argon2id); **OIDC SSO** (Authorization Code + PKCE, with CSRF `state` and replay-binding `nonce`) issues opaque, server-stored **session cookies** (`HttpOnly`/`Secure`/`SameSite=Lax`, expiring) — no token in the cookie, revocable by deleting the session; sign-in is restricted to pre-provisioned members (verified email), so a valid IdP account alone cannot self-register. Deny-by-default extractor accepts either a bearer token or a session cookie. |
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
