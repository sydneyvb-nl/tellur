# Security Policy

Tellur is security tooling, so we hold ourselves to the standards we help teams
meet. This policy covers the whole repository (Apache-2.0 core + the
FSL-licensed `crates/server` hub).

## Reporting a vulnerability

**Please do not open public issues for security vulnerabilities.**

Report privately via GitHub Security Advisories
("Report a vulnerability" on the repository's **Security** tab), or by email to
**security@tellur.dev**.

Include, where possible: affected component/version, impact, and reproduction
steps. We follow **coordinated disclosure** and will credit reporters who wish
to be named.

### Our response targets

| Stage | Target |
| --- | --- |
| Acknowledge receipt | within 72 hours |
| Initial assessment / severity | within 7 days |
| Fix or mitigation plan | severity-dependent, communicated in the assessment |

For the commercial/server distribution, we additionally align with the **EU
Cyber Resilience Act** vulnerability-reporting duties that apply from
11 September 2026 (early warning within 24 hours, full notification within
72 hours of becoming aware of an actively exploited vulnerability).

## Supported versions / security updates

Pre-1.0: security fixes target the **latest released version** on a best-effort
basis. When the commercial hub ships, we will declare a concrete support period
(initially ~24 months per major version, evolving toward the CRA reference
expectation) and provide security updates throughout it.

## How we build securely

- Secure-by-default: the server refuses to expose a non-loopback address without
  an explicit opt-in, and requires auth/TLS for any data endpoints (added in
  later phases).
- Tamper-evident provenance: event logs are SHA-256 hash-chained; the server
  re-verifies chains on ingest.
- Privacy-first: prompts are hashed/redacted before leaving the client; the hub
  stores no raw prompts by default.
- Supply chain: `cargo-deny` enforces licenses, advisories, and allowed sources
  in CI; third-party Actions are pinned to reviewed commit SHAs. Release jobs
  receive read-only repository access except for the single publishing job,
  which receives `contents: write`. Current release assets ship SHA-256
  sidecars; signing/SBOM work remains roadmap rather than a shipped claim.
- Standards: we build to OWASP ASVS 5.0 (target L2), the OWASP API Security
  Top 10, and NIST SSDF practices. See
  [`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md) and
  [`docs/proposals/TEAM_SERVER_IMPLEMENTATION.md`](docs/proposals/TEAM_SERVER_IMPLEMENTATION.md).
