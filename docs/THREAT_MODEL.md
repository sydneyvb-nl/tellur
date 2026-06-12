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
   (`POST .../repos/{repo}/attributions`, contributor+), opt-in per-repo
   **source connection** (`PUT/GET .../repos/{repo}/source` — admin; stores
   `https://` URL templates + an optional provider token, never source;
   validated server-side; `GET` and audit log only ever report
   `token_configured`, never the token). The file view's `link` template renders
   as an external link (non-https/`javascript:` rejected so it can't inject); the
   optional `raw` template lets the browser fetch raw bytes **directly from the
   provider** for **public** repos (`/app`'s CSP `connect-src` only permits a
   small all-list of raw hosts — raw.githubusercontent.com, gitlab.com,
   bitbucket.org), bounding that surface. For **private** repos the
   **blob proxy** (`GET .../repos/{repo}/blob?path=` — viewer+, rate-limited,
   tenant-scoped) fetches the bytes server-side using the stored token: the
   resolved URL is rebuilt from the admin-set template and re-checked against a
   fixed host **allowlist** (SSRF guard — userinfo and explicit ports rejected),
   restricted to `https`, size-capped (2 MB), and the token is sent only as the
   provider's auth header and never returned. The bytes are the org's own source
   served to org members, so they are returned faithfully (not redacted) — keep
   the configured token least-privilege (read-only, scoped to the connected
   repo). The
   **export portal**
   — org bundles are **durable jobs**: `POST .../export/events|audit` enqueues
   (admin) and returns a job id, polled at `GET .../jobs/{id}` or listed via
   `GET .../jobs` (admin, tenant-scoped — the worker-produced result carries org
   data). Per-repo `.../export/slsa|spdx` are available both synchronously (`GET`,
   admin or per-repo admin) and as durable jobs (`POST`, **org admin only** — the
   result is polled via the org-admin-scoped `GET .../jobs/{id}`), and
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
   **no client secret or token material**. A background **retention** loop
   minimises data-at-rest by pruning expired sessions, stale login transactions,
   and (when `TELLUR_RETENTION_DAYS > 0`) finished job results. The audit log can
   also be minimised (`TELLUR_AUDIT_RETENTION_DAYS`) via a **sealed checkpoint**:
   old entries are deleted but the pruned prefix's tip hash + length are kept and
   `verify_audit_chain` seeds from that checkpoint, so the remaining chain still
   verifies and truncation past the checkpoint stays detectable. The event
   provenance chain is never pruned. Operational
   endpoints (`/healthz`, `/readyz`, `/metrics`) are unauthenticated but expose
   only liveness and aggregate counters — no tenant data. The **team dashboard
   SPA** is served as static assets at `/app/*` (unauthenticated, but they carry
   no tenant data — the browser fetches all data from the authenticated `/v1`
   API with the first-party SSO session cookie, same-origin, so no CORS and no
   token in the URL). A strict CSP (`default-src 'self'`, self-hosted scripts/
   styles/fonts) applies to `/app`; `connect-src` additionally allows only a
   small all-list of source-host raw origins for the opt-in A12 gutter. **SSO endpoints**
   (`/auth/login`, `/auth/callback`, `/auth/logout`) are unauthenticated entry
   points for the browser OIDC flow (404 when SSO is not configured). **Device
   authorization** for the CLI's `tellur login` (RFC 8628; 404 when SSO is off):
   `POST /v1/device/authorize` issues a secret `device_code` (polled by the CLI)
   and a short `user_code` (typed by the human); `POST /v1/device/token` is the
   CLI's poll — pending/denied/expired return RFC-8628 error codes, and only an
   **approved** request mints a token. Approval happens at `GET /auth/device`,
   which **requires a signed-in session** (an unauthenticated visit is bounced
   through SSO via a validated same-origin return cookie), and `POST
   /auth/device/decision` records the decision — that POST carries the
   `SameSite=Lax` session cookie, so a cross-site forgery can't approve. The token
   is minted only at poll time from the **member's current state** (a member
   deactivated between approval and poll gets nothing), bound by an
   advisory-locked transaction so it is **delivered at most once** (the row is
   consumed), and `user_code` is escaped into the approval HTML. Anonymous
   `/v1/device/authorize` rows are TTL-pruned (15 min) and hard-capped, like
   `/auth/login`. **SCIM
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
   endpoints (loopback `http` is allowed only for local dev; a non-loopback `http`
   issuer requires the explicit, **insecure** opt-in `TELLUR_OIDC_ALLOW_INSECURE_HTTP=1`
   for a trusted private network / homelab — without it a non-secure issuer is
   logged at startup and rejected at login, never silently accepted). The client secret
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
6. **Hub → source provider** (Tier 1, optional) — when a repo's private-source
   proxy is configured the hub makes an **outbound HTTPS** GET to the provider's
   raw/contents host. The target host is constrained to a fixed **allowlist** and
   the URL re-validated on every request (SSRF guard: an admin typo or tampered
   template can't redirect the hub at an internal host; userinfo/ports rejected),
   the response is size-capped, and the stored provider token is sent only as
   that host's auth header. The token at rest is a secret — keep the DB on a
   private network / encrypted at rest, and scope the token read-only to the
   connected repo (least privilege bounds the blast radius if the hub or DB is
   compromised).

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
- **CLI hub credentials** (`tellur login`) are written to
  `~/.config/tellur/hosts.json` with `0600` perms (owner-only on Unix); a
  compromised local OS user can read them — the same trust level as `.tellur`
  data. `tellur push` reaches the hub over HTTPS (rustls); the high-water mark in
  `.tellur/push_state.json` is non-sensitive (event ids + a count).

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
