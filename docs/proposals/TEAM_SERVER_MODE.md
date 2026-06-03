# Proposal: Team / Server Mode

**Status:** Proposal (not implemented) · **Roadmap:** PROJECT_STATUS.md #8 ·
**Last updated:** 2026-06-03

> This is a design proposal, not shipped behavior. It defines the direction and a
> phased plan for letting multiple people and repositories share AI code
> provenance without giving up Tellur's local-first, Git-native guarantees.
>
> **PRD reference (reconciled 2026-06-03).** The original PRD (working name
> *TraceLens*) specifies team/server mode in **§6 product surface 11
> ("Team/server mode")**, **§16.2 storage Layer 5 ("Optional remote sync — team
> server, object storage or artifact store")**, **§4.1 ("Optional team
> aggregation can exist later, but the local workflow must be complete")**, and
> the sequential build plan **§32 Step 20 ("Build team mode")**. Note: PRD §24 is
> *Architecture Guardian*, not team mode — earlier PROJECT_STATUS entries that
> cited "§24" for team mode were mislabeled. This proposal is consistent with the
> PRD; see [§PRD alignment](#prd-alignment) below.

## 1. Goal

Let teams see *how AI participated* across many contributors and repositories —
shared dashboards, org-wide policy, and compliance evidence — **without** forcing
a cloud account, a proprietary store, or sending raw prompts off the developer's
machine. Solo usage must keep working with no server, forever.

## 2. What we build on (existing primitives)

Team mode is mostly *aggregation* of things Tellur already produces locally:

| Primitive | Location | Role in team mode |
| --- | --- | --- |
| Append-only JSONL + SHA-256 hash chain per repo | `crates/core/src/storage/event_log.rs` | Tamper-evident evidence that stays verifiable across machines |
| Git notes `refs/notes/ai` (export/import/fetch/push/install-config) | `crates/core/src/notes.rs` | A free, distributed team transport — sync over the existing Git remote |
| Daemon: loopback-only, token-auth, **recomputes the hash chain** | `crates/core/src/daemon/mod.rs` | Foundation for the hub; server cannot forge provenance |
| Export profiles (`CorporateRedacted`, `AuditPrivate`, SLSA, SPDX) | `crates/core/src/storage/export.rs`, `crates/core/src/export/` | Decides *what* is shared without leaking prompts |
| Policy engine (YAML) | `crates/core/src/policy/` | Org-wide rules are a natural extension |
| `repo_id`, `workspace_id`, `actor.email_hash` | `crates/core/src/schema/types.rs` | Privacy-preserving identity + aggregation keys |
| Static dashboard backed by daemon endpoints | `web/index.html` | Reusable team dashboard UI |

**Key insight:** Tellur does not *need* a central server to deliver team value —
Git notes are already a distributed sync channel. A server is an *optional*
aggregation/governance layer, never a requirement. That distinction is what makes
the approach resonate across every segment.

## 3. Target segments & jobs-to-be-done

| Segment | Wants | Fears | Server appetite |
| --- | --- | --- | --- |
| Independent / OSS dev | Free, zero-config, stay in Git; show AI involvement in PRs | Lock-in, mandatory accounts, cloud cost | None — Git only |
| SMB / scale-up team (3–30) | Shared team view, easy self-host or cheap managed, light policy | Ops burden, complex infra, costly enterprise tiers | Light self-host or managed |
| Corporate / enterprise | On-prem/VPC, SSO/RBAC, audit trail, SLSA/SPDX export; **prompts must not leave** | Data exfiltration, supply-chain risk, vendor dependence | Self-host required, sometimes air-gapped |

Shared across all three: **data ownership**, **no forced cloud**, **provable
integrity**. The proposal is built on that common ground.

## 4. Design principles

1. **Local-first stays the default.** Solo works with no server, forever.
2. **Git is the default transport.** Team sync works out-of-the-box via
   `refs/notes/ai` on the existing remote — zero new infra for most teams.
3. **The server is optional and self-hostable.** One binary (`tellur serve`); no
   mandatory managed cloud. Managed is a convenience added later, never lock-in.
4. **Privacy by default.** Prompts are hashed/redacted before leaving the
   machine; the server stores no raw prompt text by default
   (`CorporateRedacted` behavior as the server default).
5. **Tamper-evidence across the boundary.** The server recomputes and verifies
   the hash chain (as the daemon already does); clients cannot forge provenance.
6. **No exclusive formats.** Everything stays exportable (JSONL, SLSA, SPDX, Git
   notes). Leaving Tellur means keeping your data.

## 5. Architecture — a spectrum, not a switch

Three tiers sharing the same data model and CLI; a team grows through them with
no migration.

```
 Solo dev         SMB team               Corporate
 ────────         ────────               ──────────
 local    ──Git notes──►  remote  ──►  Tellur Hub (self-host or managed)
 .tellur/                 (Tier 0)      indexes + verifies + dashboard
                                        + policy/RBAC/SSO/compliance (Tier 1/2)
```

### Tier 0 — Git-native team sync (no server) — **MVP first**

Teams push/pull `refs/notes/ai` to their existing remote.
`tellur notes install-config` already wires auto fetch/rewrite. A new
`tellur team report` aggregates every contributor's notes/bundles across a
branch or PR range into one team AI-involvement view (builds on the existing
PR report + notes).
→ Covers indie + much of SMB, free.

### Tier 1 — Tellur Hub (self-hosted, one binary) — **next**

`tellur serve` is the existing daemon promoted from loopback to network with real
multi-user auth. Repos push their redacted event bundles (or notes) to the hub;
the hub indexes them, **re-verifies each repo's hash chain**, and serves a team
dashboard + org-wide policy + compliance export. Storage: SQLite by default,
Postgres optional for scale.

This is where the PRD §32 Step 20 building blocks land:

- **Central policy distribution** — the hub serves the canonical
  `.tellur/policies/*.yml`; `tellur policy pull` syncs org policy into each repo
  so rules are defined once and enforced everywhere (CI + editor).
- **Metadata aggregation** — multi-repo, multi-contributor rollups for the
  dashboard and durability metrics (PRD §21).
- **Audit export portal** — generate org-level provenance/SLSA/SPDX bundles
  across repos and releases from one place (PRD §20.2, §22.1).

→ Covers SMB teams and corporate on-prem/VPC.

### Tier 2 — Managed / Enterprise (later)

Hosted Hub + SSO (OIDC/SAML), SCIM provisioning, RBAC, retention policies, audit
logging, multi-repo org views. Same binary, different deployment.
→ Corporate that does not want ops; the likely commercial layer (pricing TBD by
Sydney).

## 6. Data, identity & sync model

- **Sync unit:** redacted event bundles per session/commit, or Git notes per
  commit. The hub re-indexes and **re-verifies the hash chain per repo** — no
  blind trust.
- **Identity:** `actor.email_hash` links contributions to people without leaking
  emails; the hub maps hashes → members via an org-managed member list.
- **Privacy boundary:** the client redacts before upload; hub default = no raw
  prompts (hashes + metadata only). An org may explicitly opt into more detail
  on-prem.
- **Conflict-free:** append-only + per-repo chains make aggregation additive (no
  shared-state merge conflicts).

## 7. Auth & permissions per segment

- **Tier 0:** inherits Git remote permissions (whoever may push may push notes).
  No extra config.
- **Tier 1:** per-user tokens (extending today's single daemon token), roles
  *viewer / contributor / admin*.
- **Tier 2:** SSO (OIDC/SAML), SCIM, fine-grained RBAC, audit log.

## 8. Phased plan

### Phase A — Tier 0 (MVP)

1. **`tellur team report`** — aggregate notes/bundles from multiple contributors
   over a PR/branch range into one AI-involvement report (reuses pr-report +
   notes). Pure CLI, no server.
2. **Aggregation read model** — a function that ingests N contributors'
   notes/bundles, verifies each chain, and produces per-author / per-tool /
   per-model rollups.
3. **Docs + example CI** — show a GitHub Action posting the team report on PRs.

*Exit criteria:* a team using a shared Git remote gets a combined AI-involvement
view with zero server, and each contribution stays hash-verifiable.

### Phase B — Tier 1 (self-hosted hub)

1. **`tellur serve`** — promote the daemon to a network listener with a config
   for bind address, TLS termination guidance, and multi-token auth.
2. **Ingest endpoint** — accept redacted bundles/notes, re-verify hash chains,
   index into shared storage (SQLite → optional Postgres behind a trait).
3. **Team dashboard** — reuse `web/` against multi-repo hub endpoints; add
   per-repo and per-author views.
4. **Org policy** — evaluate the existing policy engine across aggregated data.

*Exit criteria:* a team self-hosts one binary, points repos at it, and sees a
shared dashboard + org policy without prompts ever reaching the server.

### Phase C — Tier 2 (managed/enterprise)

SSO/SCIM/RBAC, retention, audit logging, hosted option. Gated on validation of
Phases A–B and Sydney's pricing/hosting decisions.

## 9. Security & privacy posture

- Default-redacted uploads; server stores no raw prompts unless explicitly
  enabled on-prem.
- Hub re-verifies hash chains; a tampered or forged bundle is rejected.
- Self-host/air-gapped supported; managed is opt-in.
- Compliance export (SLSA v1.0, SPDX 2.3) already exists and flows through the
  hub for org-level attestation.

## 10. PRD alignment

Mapping of PRD §32 Step 20 ("Build team mode") deliverables to this proposal:

| PRD Step 20 deliverable | Where in this proposal |
| --- | --- |
| Optional self-hosted server | Tier 1 `tellur serve` (Phase B) |
| Central policy distribution | Tier 1 — `tellur policy pull` from the hub |
| Metadata aggregation | Tier 0 `tellur team report` (local) + Tier 1 hub rollups |
| SSO-ready architecture | Tier 2 (OIDC/SAML/SCIM); auth model designed for it from Tier 1 |
| Audit export portal | Tier 1 audit export across repos/releases (PRD §20.2, §22.1) |
| No mandatory cloud | Core principle #3; Tier 0 needs no server at all |

Consistency with other PRD requirements: local-first default (§4.1), privacy &
redaction first (§4.7, §14), tamper-evident logs (§14.5), export profiles
(§20.2), corporate policies (§13.5), and the enterprise wedge (§28.3) are all
honored. The **Git-notes-as-team-transport (Tier 0)** path is an enhancement
beyond the PRD's Layer 5 ("team server, object storage or artifact store"): it
adds a zero-infra option that fits the PRD's local-first, no-mandatory-cloud
spirit and reuses the already-built `refs/notes/ai` support.

### Licensing note (from PRD header)

The PRD targets **Apache-2.0 for the core**, with an **AGPL-3.0-compatible
license only for an optional hosted/server distribution** if a community server
edition is created. Practical implication: keep the Tier 1/2 server in a clearly
separable component (e.g. its own crate / `crates/server` or build feature) so a
different license can apply to the server without touching the Apache-2.0 core.
Decide the exact split with Sydney before shipping Tier 1.

## 11. Open questions (for Sydney)

1. ~~PRD reconciliation~~ — done 2026-06-03 (see §PRD alignment).
2. **Managed cloud: now or later?** Recommendation: self-host first, managed
   after validation.
3. **Pricing / licensing model** (open-core? paid Hub tier? Apache core +
   AGPL server per PRD header?) — determines where the Tier 1/2 line and the
   license boundary sit.
4. **Decided MVP path:** Tier 0 first, then Tier 1 (per 2026-06-03 review).

## 12. Non-goals (for now)

- Real-time collaborative editing or chat.
- Replacing Git as the source of truth for code.
- A mandatory hosted service.
