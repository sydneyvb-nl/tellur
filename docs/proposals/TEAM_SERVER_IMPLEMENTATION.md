# Implementation plan: Tier 1 hub (`tellur serve`)

**Status:** Plan (not implemented) · **Roadmap:** PROJECT_STATUS.md #8 (Tier 1) ·
**Last updated:** 2026-06-03

This is the secure-by-design implementation plan for the self-hostable team hub
described in [`TEAM_SERVER_MODE.md`](TEAM_SERVER_MODE.md). It is grounded in
current (2025–2026) security and compliance standards and optimized for code
that is maintainable by humans and horizontally scalable. Not legal advice.

## 1. Standards we build to (research summary)

| Standard | Version / status | What it forces us to do |
| --- | --- | --- |
| OWASP ASVS | 5.0 (May 2025), 17 chapters, ~350 controls; AI/cloud/API-first | Our baseline checklist for authn, authz, session, validation, crypto, logging. Target **ASVS L2** for the hub. |
| OWASP API Security Top 10 | 2023 | BOLA (#1) → object-level authorization in the **data layer**, not handlers; plus broken-auth, BOPLA, unrestricted resource consumption. |
| OWASP Top 10 | 2021 | Web app baseline (injection, SSRF, broken access control, vulnerable components). |
| NIST SSDF | SP 800-218 (+ 800-218A for AI) | Secure SDLC: threat modeling, reviewed code, signed builds, vuln response process. |
| EU Cyber Resilience Act (CRA) | In force Dec 2024; vuln reporting **11 Sep 2026**; full **11 Dec 2027** | Secure-by-design, **SBOM**, documented vulnerability handling + coordinated disclosure, security updates over a support period, CE for commercial distribution. We position as "open-source steward" for the OSS core; the commercial hub must meet manufacturer duties. |
| SLSA | v1.0 (Build track) | Signed build provenance for our own releases; distribute provenance with artifacts. |
| OpenSSF Scorecard / Best Practices Badge | current | Repo-level hygiene gate in CI. |
| Sigstore / cosign | current | Keyless signing of release binaries + container images + SBOM. |
| OAuth 2.1 / OIDC | current | SSO (Tier 2) and token issuance patterns; PKCE, short-lived tokens. |
| GDPR | current | EU personal data (author identity): data minimization (hash emails), retention, export/delete, data residency (self-host = customer-controlled), DPA-ready. |
| SOC 2 / ISO 27001 | readiness, not cert (yet) | Audit logging, access control, change management, encryption — design now so certification later is incremental. |

Tellur's existing design already aligns with the *spirit* of these: local-first
(data minimization), tamper-evident hash chain (integrity/audit), redaction
(privacy), SLSA/SPDX export. The hub must not regress any of that.

## 2. Security architecture principles

1. **Secure by design & default (CRA/SSDF).** TLS-only, auth required on every
   mutating route, redaction-on by default, least privilege, deny-by-default
   authorization.
2. **Defense in depth.** Network (loopback/private by default, explicit bind),
   transport (TLS 1.3), app (authz at data layer), data (encryption at rest),
   audit (tamper-evident log).
3. **Don't trust the client — re-verify.** The hub recomputes/verifies every
   uploaded hash chain (as today's daemon already does). Provenance integrity is
   server-enforced.
4. **Privacy boundary stays at the client.** Clients redact before upload; hub
   default stores no raw prompts. Personal data minimized to salted hashes.
5. **Multi-tenant isolation from day one.** Every row carries a tenant/org id;
   all queries are scoped by it at the data layer (kills BOLA structurally).
6. **Auditable everything.** Append-only, hash-chained audit log of security and
   data events — reuse Tellur's own event-log primitive.

## 3. Component & code structure (maintainability)

New FSL-licensed crate, separable from the Apache core (see
[`LICENSING.md`](LICENSING.md)):

```
crates/server/                 # FSL-1.1-Apache-2.0, depends on tellur-core
  src/
    main.rs                    # binary entry: config load, wiring, graceful shutdown
    config.rs                  # 12-factor config (env + file), validated at boot
    app.rs                     # Router assembly, middleware stack, AppState
    error.rs                   # one typed error enum -> RFC 9457 problem+json
    auth/                      # tokens, sessions, OIDC (Tier 2), middleware
    authz/                     # roles, policy checks, tenant scoping helpers
    api/                       # thin HTTP handlers (one module per resource)
    domain/                    # business logic services (pure, testable)
    storage/                   # Store trait + sqlite/ + postgres/ backends
    audit/                     # tamper-evident audit log writer
    telemetry/                 # tracing, metrics, request IDs
  migrations/                  # SQL migrations (sqlx), reviewed like code
  tests/                       # integration tests (testcontainers for pg)
```

Maintainability rules:

- **Thin handlers, fat services.** `api/` does parsing + authz + calls a
  `domain/` service; business logic is in `domain/` and unit-tested without HTTP.
- **One storage trait, swappable backends.** `Store` trait in `storage/`;
  `SqliteStore` (default, zero-config self-host) and `PostgresStore` (scale)
  implement it. Handlers/services never see SQL directly.
- **One error type → RFC 9457 (`application/problem+json`).** No `unwrap()` on
  reachable paths (matches `AGENTS.md`); typed errors map to status codes
  centrally, never leaking internals.
- **OpenAPI-first.** Generate the spec from types (`utoipa`); it is the contract
  for the dashboard, CI, and external clients, and the basis for contract tests.
- **Config validated at boot, fail fast.** Refuse to start on insecure config
  (e.g. binding non-loopback without TLS/auth).

## 4. Data model & tenancy

- Core tables: `org`, `member`, `repo`, `provenance_bundle`/`event`,
  `attribution`, `policy`, `audit_log`, `api_token`.
- **Every tenant-scoped row has `org_id`.** A request-scoped `TenantContext`
  carries the caller's org; the `Store` API takes it and filters in SQL so a
  handler *cannot* forget the check (structural BOLA prevention per API Top 10).
- Author identity stored as **salted hash** (extends today's `email_hash`),
  never raw email — GDPR data minimization.
- Reuse `tellur-core` schema types for events/attribution so client and server
  never drift.

## 5. AuthN / AuthZ

| Concern | Tier 1 (self-host MVP) | Tier 2 (enterprise) |
| --- | --- | --- |
| Authentication | Per-user API tokens (hashed at rest, prefixed, revocable), short-lived session cookies for the dashboard (HttpOnly, Secure, SameSite=strict) | OIDC/OAuth 2.1 SSO (PKCE), SCIM provisioning |
| Authorization | RBAC: `viewer / contributor / admin`, enforced at the data layer with `org_id` scoping | Fine-grained RBAC/ABAC, per-repo roles |
| Secrets | Argon2id for any password/token-at-rest hashing; tokens are random 256-bit | KMS/HSM-backed signing keys |

Rules: deny by default; authorize on **object + tenant**, not just role; log
every failed authz attempt to the audit log (API Top 10 guidance).

## 6. Crypto, transport, secrets

- TLS 1.3 (terminate at the app or document a reverse-proxy pattern); HSTS.
- At rest: rely on disk/DB encryption; document SQLCipher/Postgres TDE options.
  Prompt data is not stored by default; if an org opts in, encrypt it with a
  per-org key.
- Secrets via env/secret manager only; never in the DB or logs. Redaction engine
  (already in core) runs on anything inbound that could carry secrets.

## 7. Audit & integrity (our differentiator)

- Append-only, **hash-chained audit log** for auth events, data access, policy
  changes, exports — built on the same `EventWriter` primitive the provenance log
  uses. This gives SOC 2 / ISO 27001-grade audit evidence with tamper-evidence
  most products lack.
- The hub re-verifies every uploaded provenance chain on ingest and rejects
  forgeries (today's daemon behavior, kept).

## 8. API surface (incremental)

Build on today's daemon endpoints, add tenancy + authz:

- `POST /v1/orgs/{org}/bundles` — ingest redacted bundles/notes (re-verify chain).
- `GET  /v1/orgs/{org}/repos`, `.../repos/{id}/sessions`, `.../attributions`.
- `POST /v1/orgs/{org}/reports/team` — server-side team report across repos.
- `GET  /v1/orgs/{org}/policy` / `PUT ...` — central policy distribution
  (`tellur policy pull` client command).
- `POST /v1/orgs/{org}/export` — org-level SLSA/SPDX/audit bundle.
- `GET  /healthz`, `/readyz`, `/metrics` (no auth, no tenant data).

All under `/v1`, versioned; OpenAPI-described; rate-limited; request-size capped
(unrestricted-resource-consumption control).

## 9. Scalability

- **Stateless app servers** → scale horizontally behind a load balancer; all
  state in Postgres + object storage for large bundles.
- SQLite for single-node self-host; Postgres for teams/enterprise — same `Store`
  trait, chosen by config.
- Heavy work (report aggregation, export) runs as **background jobs** with a
  queue table; API stays responsive.
- Pagination + cursor-based listing everywhere; indexes per `org_id` + hot keys.
- Caching of computed reports keyed by range + content hash.

## 10. Observability & operations

- Structured logging (`tracing`) with request IDs; **no secrets/PII in logs**.
- Prometheus `/metrics`; OpenTelemetry traces (optional, opt-in).
- Health/readiness endpoints; graceful shutdown; DB migration on deploy with
  rollback discipline.
- Documented backup/restore and retention jobs (GDPR + retention policy).

## 11. Supply-chain & release security (CRA / SLSA / SSDF)

- **SBOM** generated per release (CycloneDX/SPDX) and published — CRA requirement
  and Tellur already does SPDX export.
- **Signed builds**: cosign/Sigstore keyless signing of binaries + container
  images + SBOM; SLSA v1.0 build provenance from CI.
- **Pinned, scanned dependencies**: `cargo-deny` (licenses + advisories),
  `cargo-audit`, Dependabot; OpenSSF Scorecard in CI.
- **Documented vulnerability handling**: `SECURITY.md` with coordinated
  disclosure + the CRA 24h/72h reporting workflow (effective Sep 2026).
- Reproducible builds where feasible.

## 12. Compliance traceability (where each standard is met)

| Standard | Addressed by |
| --- | --- |
| ASVS 5.0 L2 | §5 authn/authz, §6 crypto, §7 audit, §3 input validation/error handling |
| API Top 10 (BOLA) | §4 tenant scoping at data layer, §5 object-level authz |
| CRA | §11 SBOM/signing/vuln handling, §2 secure-by-default, support-period updates |
| SLSA v1.0 | §11 signed build provenance |
| NIST SSDF | §13 SDLC gates, threat model, code review, signed releases |
| GDPR | §4 hashed identity/minimization, §6 encryption, §10 retention/export/delete |
| SOC 2 / ISO 27001 | §7 audit log, §5 access control, §10 ops, §13 change mgmt |

## 13. Secure SDLC gates (per phase, CI-enforced)

- Threat model (STRIDE) updated when the API surface changes.
- `cargo fmt`, `clippy -D warnings`, `cargo test`, `cargo-deny`, `cargo-audit`
  green required to merge (extends current `AGENTS.md` verification).
- Integration tests run against Postgres (testcontainers) and SQLite.
- Authz tests: two orgs/users must not read or mutate each other's objects
  (explicit BOLA regression tests, per API Top 10).
- DAST smoke (e.g. OWASP ZAP baseline) against a disposable instance.

## 14. Phased build plan (Tier 1)

Each phase ships independently, behind config, with its own tests + docs.

- **B0 — Scaffolding & threat model.** ✅ Done. `crates/server` (FSL), config,
  AppState, error type, `Store` trait + SQLite backend, `/healthz`+`/readyz`,
  tracing, CI gate (`cargo-deny`), `SECURITY.md`, STRIDE doc. No data endpoints.
- **B1 — Identity & tenancy.** ✅ Done (branch `feat/server-b1-identity-tenancy`).
  orgs, members, Argon2id API tokens, RBAC roles, deny-by-default `Principal`
  auth extractor, tenant-scoped `/v1/me` + `/v1/orgs/{org}/me`, hash-chained
  audit log, admin bootstrap CLI. BOLA regression tests included.
- **B2 — Ingest & verify.** ✅ Done (branch `feat/server-b2-ingest`).
  `POST /v1/orgs/{org}/repos/{repo}/events` (contributor+; cross-tenant → 403);
  hub recomputes the per-repo hash chain (`hash_event`); inbound payloads are
  secret-redacted; 1 MiB body cap + max-events cap + per-member rate limit (429).
- **B3 — Read & report.** ✅ Done (branch `feat/server-b3-read-report`).
  Tenant-scoped `GET .../repos`, `GET .../repos/{repo}/events` (cursor
  pagination), and `GET .../report` (org rollup across repos). Reusing the
  `web/` dashboard against these endpoints is deferred to a follow-up.
- **B4 — Central policy & export.** `tellur policy pull`; org-level
  SLSA/SPDX/audit export portal.
- **B5 — Scale & ops.** Postgres backend, background jobs, pagination, metrics,
  backup/retention. Packaging: single binary + signed multi-arch Docker image +
  Docker Compose example (Helm deferred).
- **B6 — Enterprise (Tier 2).** OIDC SSO, SCIM, fine-grained RBAC, signed
  release pipeline hardening, SOC 2 evidence collection.

## 15. Decisions (2026-06-03)

| Question | Decision |
| --- | --- |
| Managed hosting / GDPR residency | **Self-host first; managed later, EU-first** (add US regions afterwards). Self-host keeps residency under the customer's control. |
| Self-host packaging (B5) | **Single binary (always) + signed multi-arch Docker image + Docker Compose example** (app + Postgres). Helm chart deferred until enterprise demand. |
| CRA support period | **`SECURITY.md` best-effort policy now**; commit a concrete period (~24 months/major, evolving toward the CRA 5-year reference) when the commercial hub ships. |
| SOC 2 / ISO 27001 | **Readiness now, certify on demand.** Build controls + audit log now; certify (ISO 27001 for EU, or SOC 2) when a paying enterprise requires it, accelerated with a compliance platform. |

Remaining inputs needed only at their phase: exact managed region(s) at Tier 2;
final support-period number at commercial launch.
