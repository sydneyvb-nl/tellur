# Tellur Agent Instructions

These instructions apply to the whole repository.

## Start Here (orientation for every run)

Tellur is an AI code provenance platform: Git tells you *what* changed, Tellur
tells you *how AI participated*. Local-first, no cloud dependency. A Rust core +
CLI (`crates/`), a VS Code extension and a JetBrains plugin (`editor/`), a static
dashboard (`web/`), and packaging (`dist/`).

Read these in order before acting; they are the source of truth:

1. **`PROJECT_STATUS.md`** — what is built, what is open, test counts, roadmap,
   blockers, and a dated changelog at the top. The single source of truth for
   status.
2. **This file's [Architecture Map](#architecture-map)** — where every layer
   lives, so you land changes in the right place.
3. **`README.md`** — user-facing behavior: commands, setup, adapters, limits.
4. **`docs/ADAPTERS.md`** — adapter mechanics, integration mechanisms (hooks vs
   extension vs MCP vs daemon webhook vs import), guarantees, and known limits.
5. **`CONTRIBUTING.md`** — dev workflow and repo structure.
6. **`docs/FINDINGS.md`** — historical review/remediation notes.
7. **`docs/proposals/`** — design proposals for not-yet-built features (e.g.
   `TEAM_SERVER_MODE.md`). Read before working on the matching roadmap item.

One-glance repository layout:

```
tellur/
├── crates/
│   ├── core/         # library: schema, storage, attribution, policy, redaction,
│   │                 #          export, daemon, mcp, notes, remap, report
│   ├── cli/          # the `tellur` binary (all commands + global setup)
│   ├── adapters/     # per-tool import parsers + hook/payload normalization
│   └── server/       # Tier 1 team hub (tellur-server) — FSL-1.1-ALv2, not Apache
│       └── ui/       #   embedded Svelte 5 dashboard SPA (built to ui/dist)
├── editor/
│   ├── tellur-vscode/    # VS Code / Cursor / Windsurf extension (TypeScript)
│   └── tellur-jetbrains/ # JetBrains IDE plugin (Kotlin/Gradle)
├── schemas/          # JSON Schema for session/event/attribution/pr-report/provenance
├── web/              # static session-replay dashboard (served by the daemon)
├── dist/             # npm wrapper + Homebrew formula
└── .github/workflows # CI + release automation
```

## Working Agreement (how changes land)

- **Branch off `main`; never commit to `main` directly.** One change = one
  feature branch = one PR.
- **The maintainer merges PRs — you do not.** Open the PR and stop; wait for the
  merge. After a PR is merged, **never push more commits onto that branch** — sync
  `main`, branch fresh, and (if it was a follow-up) cherry-pick. Before pushing a
  follow-up, re-check the PR is still open (`gh pr view <n> --json state`): the
  maintainer may have merged it between turns.
- **Conventional Commits** (`feat:`/`fix:`/`docs:`/`test:`/`chore:`, scoped where
  useful), and end every commit message with the trailer
  `Co-Authored-By: Claude <noreply@anthropic.com>`.
- **Build AND test what you change** in its owning toolchain before claiming it
  works (see [Verification](#verification)); say so explicitly if a toolchain is
  unavailable.
- **Keep docs in sync in the same change** (see below) — especially
  `PROJECT_STATUS.md` test counts/open-work and this map.
- After review feedback (e.g. Codex), fix on the **same open PR's branch** (it's
  still open); if it already merged, open a fresh follow-up PR off `main`.

## Documentation Must Track Behavior

When changing any behavior, workflow, adapter, integration mechanism, setup
command, config shape, storage format, policy behavior, public CLI surface, or
editor/plugin/MCP/daemon implementation detail, update the relevant
documentation in the same change.

At minimum, check these files before finishing:

- `README.md` for user-facing behavior, commands, setup, and limits.
- `docs/ADAPTERS.md` for adapter mechanics, integration mechanisms, guarantees,
  and known limits.
- `PROJECT_STATUS.md` for implementation status, open work, test counts, and
  roadmap state.
- `CONTRIBUTING.md` when development workflow, verification, or repo structure
  changes.

If a code change intentionally does not require documentation updates, state why
in the final response or commit/PR summary.

Do not leave stale documentation that contradicts the implementation. In
particular, update docs when changing:

- lifecycle hooks or hook payload handling;
- editor integration behavior for VS Code, Cursor, Codex, Claude Code, or
  future agents;
- setup/uninstall/status commands and files they write;
- adapter import formats, redaction, prompt hashing, or provenance guarantees;
- daemon, MCP, Git notes, policy, export, or storage behavior;
- test counts or verification commands in `PROJECT_STATUS.md`.

## Architecture Map

Use this map before changing code so you land changes in the right layer.

| Area | Path | What lives there |
| --- | --- | --- |
| Core data model | `crates/core/src/schema/` | `types.rs` (Session, Event, Actor, AgentInfo, EventType + wire strings), `ids.rs` (hashing + prefixed ID generation). |
| JSON Schemas | `schemas/*.json` | Canonical session/event/attribution/pr-report/provenance schemas. Keep in sync with `schema/types.rs`. |
| Append-only provenance log | `crates/core/src/storage/event_log.rs` | `EventWriter`: JSONL event writing, imported-event preservation, SHA-256 hash-chain (re)sealing + verification. |
| SQLite query index | `crates/core/src/storage/index.rs` | `TraceIndex`: session/event/attribution indexing used by CLI, MCP, daemon, editor, dashboard. |
| Repo storage layout | `crates/core/src/storage/repo.rs` | `.tellur/` discovery/init (traces, index, policies, config, daemon token, `disable`). |
| Git/file capture | `crates/core/src/capture.rs`, `crates/core/src/storage/file_watcher.rs` | Working-tree diff capture, filtered path capture, attribution writes. |
| Attribution engine | `crates/core/src/attribution/` | Line-level patch → AI/human attribution that powers `explain`/`blame`/`pr-report`. |
| Policy / redaction | `crates/core/src/policy/`, `crates/core/src/redaction/` | YAML policy rules + sensitive paths; regex secret detection/redaction. |
| Export | `crates/core/src/storage/export.rs`, `crates/core/src/export/` | Provenance bundles + SLSA v1.0 / SPDX 2.3 export profiles. |
| Reports | `crates/core/src/report/` | PR risk report generation + markdown rendering. |
| Git notes | `crates/core/src/notes.rs` + `crates/cli/src/notes.rs` | `refs/notes/ai` Git AI-compatible authorship notes (`tellur notes …`). Normal export is commit-scoped: ranges must match the commit's blob and additions. `notes attest-ai` is an explicit missed-capture recovery and is labelled `claimed`; team reports expose evidence strength and exclude base-branch merge commits so first-parent merge diffs cannot inflate PR additions. |
| Git remapping | `crates/core/src/remap/` | SHA remap across rebase/amend via `git diff-tree`. |
| Glob matching | `crates/core/src/glob.rs` | Path glob matcher shared by policy/capture filters. |
| Local daemon | `crates/core/src/daemon/` | `mod.rs`: loopback-only, token-auth HTTP API (events, sessions, export, dashboard). `webhook.rs`: normalizes cloud-agent (Devin) native payloads on `POST /webhook/{source}` and recomputes the hash chain. |
| MCP server | `crates/core/src/mcp/` | Stdio JSON-RPC tools exposed to agents/editors. |
| Adapter implementations | `crates/adapters/src/` | Per-tool import parsers (`<tool>.rs`) + hook payload normalization; `import.rs` is the shared tolerant JSONL/array/envelope loop; `sanitize.rs` redaction/hashing helpers. Keep prompt hashing/redaction here. |
| Adapter registry | `crates/core/src/adapter/builtin.rs`, `crates/adapters/src/lib.rs` | Built-in adapter metadata (`supports_hooks`) and exports. |
| CLI commands/setup | `crates/cli/src/` | `main.rs` is a thin dispatcher; commands live in focused modules — `cli` (clap args), `repo`/`inspect`/`capture`/`maintain`/`policy`/`push`/`notes`/`connect`/`setup`/`serve`, plus `hooks` (the universal hook/webhook `ingest` entrypoint + source normalization) and `util`/`git` shared helpers. Import dispatch is in `maintain`. |
| CLI integration tests | `crates/cli/tests/cli_integration.rs` | End-to-end binary behavior and generated config fixtures. |
| VS Code/Cursor/Windsurf extension | `editor/tellur-vscode/src/` | Extension client, commands, tree providers, save/watch capture, model diagnostics. Same trusted-workspace extension serves VS Code, Cursor, and Windsurf (all VS Code-compatible), auto-detects the host, runs beside local/remote workspaces, and owns one watcher per multi-root folder. |
| JetBrains plugin | `editor/tellur-jetbrains/` | IntelliJ Platform plugin (Kotlin/Gradle): `VFS_CHANGES` listener → `hooks ingest --source jetbrains`, settings UI. Built with Gradle, **not** the Rust CI (see Verification). |
| Team/server hub (Tier 1) | `crates/server/` | `tellur-server` binary — **FSL-1.1-ALv2**, not Apache (own `LICENSE`, `publish = false`). Layered: `config`/`error`/`auth` (roles + Argon2id tokens)/`ratelimit` (fixed-window)/`storage` (Store trait with two backends — **SQLite** default zero-config (`storage/sqlite/`) and **Postgres** (`storage/postgres/`, r2d2 pool, NoTls) for horizontal scale, selected by `TELLUR_DATABASE_URL`: orgs/members/tokens/hash-chained audit + repos/per-repo event chain. Each backend is a directory whose `mod.rs` holds the pool/connection, shared query helpers, and a thin delegating `impl Store`; the per-domain SQL lives in submodules (`orgs`/`repos`/`events`/`policy`/`audit`/`auth_sessions`/`jobs`/`scim`/`schema`). `storage/chain.rs` is the shared tamper-evident hash-chain helper — head checkpoint + verify — used by both chains in SQLite; the Postgres backend inlines the same chain logic guarded by `pg_advisory_xact_lock` for append atomicity. Postgres tests `crates/server/tests/postgres.rs` run only when `TELLUR_TEST_DATABASE_URL` is set)/`api/` (per-domain handler modules — `common` holds the deny-by-default `Principal` extractor — accepts an API **bearer token** *or* an SSO **session cookie**; tenant-scoped handlers: ingest w/ redaction + read/report + policy + attribution + export endpoints incl. per-repo SLSA/SPDX via core generators; **fine-grained per-repo RBAC** — additive `repo_role` grants, effective role = `max(org_role, grant)`, managed via `PUT/DELETE /v1/orgs/{org}/repos/{repo}/roles/{member}` + `GET .../roles`, org-admin only)/`oidc` (OIDC SSO: Authorization Code + PKCE, mockable `OidcClient` trait + `HttpOidcClient` over ureq/rustls; `/auth/login|callback|logout`; ID-token claims validated, signature trust via the TLS token-endpoint channel per OIDC Core §3.1.3.7; opaque DB-backed sessions; `member_identity`/`oidc_login`/`session` tables; enabled by `TELLUR_OIDC_*`. Issuer/endpoints must be `https`; loopback `http` always allowed, non-loopback `http` only with the explicit insecure opt-in `TELLUR_OIDC_ALLOW_INSECURE_HTTP=1` — a non-secure issuer is warned at startup, not an opaque login 500. **Device login** (`tellur login`, RFC 8628): `/v1/device/authorize`+`/token` + session-gated `/auth/device` approval page mint a member token)/`scim` (**SCIM 2.0** `/scim/v2/Users` + `/scim/v2/Groups` — list/create/get/put/patch/delete; org-scoped `ScimAuth` bearer (`scim_token` table, Argon2id); maps userName→email, active→can-auth, roles→org role; deprovision deactivates `member.active=0` so all auth paths reject; **Groups drive roles** — `displayName` `tellur-admin|contributor|viewer` → org role, recomputed on membership change (`scim_group`/`scim_group_member`); mutations audited)/`jobs` (**durable job queue** — `job` table + background worker `spawn_worker`; `process_one` deterministic for tests; org exports are enqueued: `POST .../export/events|audit` → 202+job_id, `GET .../jobs/{id}` polls status+result, admin-only, tenant-scoped)/`metrics` (`/metrics`)/`dashboard` (**team dashboard SPA** — feature `dashboard` (default on), Svelte 5 + TS source in `crates/server/ui/`, built to `ui/dist`, embedded via `rust-embed`, served same-origin at `/app/*` with SPA fallback; first-party SSO session cookie; org-scoped routes `/app/orgs/:org/...`; FSL like the rest of the server. `build.rs` ensures `ui/dist` exists so plain `cargo build` works (empty → placeholder; the `dashboard` CI job + Docker build the real SPA))/`app` (router + body-limit). Hub data for the dashboard: `GET /v1/orgs/{org}/dashboard` (rollup + recent feed), `GET .../activity?range=&group_by=type|actor` (A1, daily time-series), `GET .../repos/{repo}` (A3, repo summary incl. AI share + review coverage); review-gap math lives in the pure `review` module (`review_stats`, decision §12.1). D2: `GET .../repos/{repo}/attributions[?path=]` (A4, metadata-only), `GET .../sessions[?repo=&actor=&range=]` + `GET .../sessions/{id}` (A6; `list_sessions`/`session_events`). SPA screens: Overview (+trend), Repositories, Repo detail (+attributed files + admin source-connection card), File provenance view (metadata-first gutter), Sessions list + **dynamic Session timeline** (`SessionDetail.svelte` + pure `lib/timeline.ts`: category/actor filters, search, color-coded nodes, prompt bubbles). Prompts show only when a repo opts into `redaction.store_prompt_excerpt` (CLI `UserPromptSubmit` hook stores a redacted, length-bounded `prompt_excerpt` in the event payload using the repo's own redaction rules). All viewer+, tenant-scoped. `web/index.html` is the older Apache local session-replay viewer with a hub mode (`?hub=&org=`). **A12 source connection** (`source.rs` + `api::{set,get}_repo_source`/`source_blob`): admins connect a repo to its provider (`PUT/GET .../repos/{repo}/source`, link+raw templates + optional token in `repo_source.source_token`, schema v19; token `#[serde(skip)]`, never returned/logged). Public repos are fetched browser-direct; **private** repos go through the **SSRF-guarded blob proxy** (`GET .../repos/{repo}/blob?path=`, viewer+) — `source` module re-validates the URL against a host allowlist, https-only, 2 MB cap, per-provider auth header. **B1 GitHub App tokens** (`github_app.rs`): when `TELLUR_GITHUB_APP_*` is configured, the proxy mints a short-lived per-repo `Contents:read` **installation token** (App JWT RS256 via `jsonwebtoken` → `/repos/{o}/{r}/installation` → `/app/installations/{id}/access_tokens`, cached per `(owner,repo)` until ~5 min before expiry) instead of the stored PAT for GitHub repos; the network boundary is the mockable `GithubAppApi` trait (`HttpGithubAppApi` real impl), and `api::resolve_source_token` picks App-token vs PAT-fallback. The PAT stays the fallback for GitLab/Bitbucket/self-managed (and GitHub when the App isn't installed). SPA: admin `SourceConnection.svelte` card + `buildTemplates` preset generator; FileView routes via the proxy when `source_proxy`. `main.rs` = `serve` + `admin` bootstrap CLI (`create-org`/`create-token`/`set-policy`/`add-member` [SSO]/`create-scim-token`/`grant-repo-role`/`revoke-repo-role`/`list-repo-roles`/`set-repo-source`). Heavy ops run via `spawn_blocking` or the job worker. Packaging: `dist/docker/`. B0–B6 done (Postgres, per-repo RBAC, OIDC SSO, SCIM users+groups, durable jobs, dashboard API). |
| GitHub App webhook (P3) | `crates/server/src/github_webhook.rs`, `crates/server/src/github_app.rs`, `crates/server/src/storage/{sqlite,postgres}/` | `POST /webhook/github` verifies `X-Hub-Signature-256` with `TELLUR_GITHUB_WEBHOOK_SECRET`, resolves `installation.id` through `github_installation`, syncs installation repos/source templates, fetches `refs/notes/ai` through installation tokens, and appends compact `github.note.harvest` events. `github_note_harvest` enforces idempotency per `(org, repo, commit)` (schema v20). Setup maps an installation with `tellur-server admin set-github-installation`. |
| Unified setup (`tellur setup`) | `crates/cli/src/onboarding.rs` + `setup.rs` + `connect.rs` + `service.rs` | Supported onboarding path: idempotently generates machine-wide agent hook/MCP/editor settings, initializes the current repo, optionally performs secure Team Hub device login, and installs **chained, non-clobbering git hooks** (`post-commit` → commit-scoped `notes export`; `pre-push` → idempotent `notes install-config` + `push` + `notes push "${1:-origin}"`, with a `TELLUR_CONNECT_PREPUSH` recursion guard, all `|| true` so hub/notes failures never block git). Hub setup installs a per-user background service by default; `setup update` refreshes absolute executable paths and an existing service after upgrades; `setup status` combines global and repo health. `tellur connect` and granular `setup <tool>` remain compatibility/recovery surfaces. The generated personal plugin is the **sole Codex hook owner**. VS Code settings are prepared but do not install the unpublished VSIX; JetBrains marketplace installation is also still open. **No `remote.push` refspec** — it would break default branch push. |
| Hub client (Apache CLI) | `crates/cli/src/hub.rs` + `tellur login`/`push`/`logout`/`policy pull` | `ureq` client (HTTPS via the `tls`/rustls feature). `tellur login` = RFC 8628 device grant against the hub (`/v1/device/authorize`+`/token`), stores the minted member token at `~/.config/tellur/hosts.json` (`0600`). `tellur push` forwards local events **and line-level attribution** to the ingest API (`/events` + `/attributions`) with a per-`(hub,org,repo)` high-water mark in `.tellur/push_state.json` (incremental + idempotent; `push_start_index` is the pure, unit-tested slice helper). Attribution is a current-state snapshot pushed every run (the hub upserts per file; **empty ranges = delete-tombstone** for files removed from disk) — this is what drives the AI-share/AI-lines metrics, so events alone show 0 AI. `policy pull` fetches a central policy into `.tellur/policies/`. Server side: `device_auth` table (schema v18, both backends) + `device_*` handlers + session-gated `/auth/device` approval page in `api/device.rs`. |
| Packaging (server) | `dist/docker/` | Multi-stage `Dockerfile` + `docker-compose.yml`; CI builds the image (`.github/workflows/ci.yml` docker job). See `docs/proposals/TEAM_SERVER_IMPLEMENTATION.md`. |
| Security policy / threat model | `SECURITY.md`, `docs/THREAT_MODEL.md` | Coordinated disclosure + CRA reporting; STRIDE analysis. Update the threat model when trust boundaries/endpoints change. |
| Supply-chain gate | `deny.toml`, `.github/workflows/ci.yml` | `cargo-deny` (licenses/advisories/sources) + `cargo-audit` + fmt/clippy/test in CI. |
| Web dashboard | `web/index.html` | Static dashboard client backed by daemon endpoints. |
| Packaging | `dist/`, `.github/workflows/` | npm wrapper, Homebrew formula, release and CI workflows. |
| User docs | `README.md`, `docs/ADAPTERS.md`, `docs/FINDINGS.md` | Public commands/mechanisms/limits; adapter mechanics; historical review notes. |
| Project source of truth | `PROJECT_STATUS.md` | Implementation status, open work, test counts, roadmap, blockers, changelog. |

Cross-layer rules:

- New adapter: add `crates/adapters/src/<adapter>.rs`, export it from
  `crates/adapters/src/lib.rs`, register metadata in
  `crates/core/src/adapter/builtin.rs`, add CLI import dispatch in
  `crates/cli/src/maintain.rs` (and setup dispatch in `crates/cli/src/setup.rs`),
  add tests, and update `README.md`,
  `docs/ADAPTERS.md`, and `PROJECT_STATUS.md`.
- New global setup surface: add install/status/uninstall paths together; use an
  absolute `tellur` executable path; refuse to overwrite malformed JSON; add an
  integration test for generated config.
- New hook source: route through `tellur hooks ingest --source <source>
  --auto-init` unless the surface has a stronger documented API; add the source
  to `normalize_hook_source` in `crates/cli/src/hooks.rs`; never capture the whole
  working tree from a tool hook without a concrete file path.
- New webhook/cloud-agent source: add the native-payload mapping to
  `crates/core/src/daemon/webhook.rs` (core cannot depend on the adapters crate);
  it is reached via the existing `POST /webhook/{source}` route. Keep prompt
  hashing + command redaction and let `EventWriter` recompute the hash chain.
- New editor/runtime behavior: update the relevant editor integration
  (`editor/tellur-vscode` and/or `editor/tellur-jetbrains`) and the setup docs,
  because users should configure it once globally.
- Pick the integration mechanism honestly per tool (lifecycle hook > editor
  extension/plugin > MCP tool access > daemon webhook > import). Document which
  one a tool uses in `docs/ADAPTERS.md` and do not model a tool as having
  Codex-style hooks when it does not.

## Verification

**Always build and test what you change** — in the toolchain that owns it. Do
not claim something works unless you compiled/ran it. Rust changes go through
`cargo`; the VS Code extension through `npm`; the JetBrains plugin through
Gradle. If a toolchain is genuinely unavailable in your environment, say so
explicitly rather than implying it was verified.

For Rust changes, run:

```bash
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test
cargo deny check   # supply-chain gate: licenses + advisories + sources
```

The **Postgres backend tests** (`crates/server/tests/postgres.rs`) run only when
`TELLUR_TEST_DATABASE_URL` is set — without it they no-op, so set it (local:
`postgres://postgres@127.0.0.1:5433/tellur_test`) when touching the storage layer.

For **dashboard SPA** changes (`crates/server/ui/`), run from that dir:

```bash
pnpm install
pnpm check        # svelte-check (type + template)
pnpm test         # vitest (pure helpers + i18n catalog parity)
pnpm build        # produces ui/dist, which the server embeds at build time
pnpm e2e          # Playwright (real bundle, mocked /v1) — when routing/screens change
```

The hub **embeds `ui/dist` at build time**, so after an SPA change rebuild the
server (or rely on the `dashboard` CI job + Docker build) for `/app` to update.

`cargo deny check` must stay green (CI enforces it). New crates must declare a
license (`license.workspace = true` for Apache core crates; the FSL server crate
uses `license-file` + `publish = false`).

**Toolchain is pinned** in `rust-toolchain.toml` so local and CI run the same
clippy — bump it deliberately and fix any new lints in that change.

**Platform-specific code:** local `cargo clippy` on macOS does **not** compile
`#[cfg(target_os = "...")]` blocks for other OSes, so a lint inside a
Linux/Windows-only block can pass locally yet fail CI (which runs on Ubuntu).
Keep `cfg` branches trivial, and treat the Ubuntu CI run as the cross-platform
authority before considering a change green.

For VS Code extension changes, also run from `editor/tellur-vscode`:

```bash
npm run compile
npm run test:unit
```

Run `npm run test:extension` when the change touches activation, commands,
settings, or VS Code runtime behavior.

### JetBrains plugin (`editor/tellur-jetbrains`)

This plugin is Kotlin built with **Gradle + the IntelliJ Platform SDK**, which
the Rust `cargo` toolchain and the Rust CI do **not** build. The Gradle wrapper
is committed (pinned to 8.9), so building requires only **JDK 17** and network
access (the IntelliJ SDK downloads on first run). Verify plugin changes here, not
with `cargo test`:

```bash
cd editor/tellur-jetbrains
JAVA_HOME=/path/to/jdk-17 ./gradlew buildPlugin   # compile + package the plugin zip
JAVA_HOME=/path/to/jdk-17 ./gradlew runIde        # sandbox IDE for manual capture testing
```

`./gradlew buildPlugin` is known-good on JDK 17 (the build's
`buildSearchableOptions` step boots a headless IDE with the plugin installed,
exercising `plugin.xml` and the listener/service classes). Plugin versions
(Kotlin 1.9.24 + IntelliJ Platform Gradle Plugin 2.0.1) target Gradle 8.9 — do
not bump the wrapper to Gradle 9.x without also bumping those plugins.

"Builds outside Rust CI" means only that the *verification path* differs: plugin
changes are not covered by `cargo test`/clippy and must be compiled with Gradle.
**Always build what you change.** If you genuinely cannot run Gradle in your
environment, say so explicitly instead of claiming the plugin was compiled.
