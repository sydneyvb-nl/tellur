# Proposal: Zero-touch provenance + GitHub App

**Status:** P1-P3 shipped; P4 open · **Last updated:** 2026-06-16
**Builds on:** `tellur login`/`push` (#33), source connection + blob proxy (#35),
`refs/notes/ai` notes commands, `tellur setup` agent hooks.

> Design proposal plus shipped implementation notes. It defines how Tellur becomes
> **zero-touch for developers** — after a one-time install, no one runs a
> `tellur` command again — and where an optional **GitHub App** fits. It does
> **not** change Tellur's local-first, Git-native guarantees.

## 1. Guiding principle

**After a one-time setup, a developer never touches the terminal for Tellur
again.** Capture, hub sync, and `refs/notes/ai` push all happen automatically in
the background. The terminal/`tellur` commands remain available for power users
and CI, but are not part of the normal loop.

Two user-stated requirements drive this:
1. `refs/notes/ai` should be **pushed automatically**.
2. `tellur push` (events → hub) should happen **automatically**.

## 2. Where the data actually lives (ground truth)

This shapes everything below — a GitHub App can only ever see what reaches git.

| Data | Location | Reaches GitHub? |
| --- | --- | --- |
| `.tellur/config.yml`, `.tellur/policies/` | git-tracked | ✅ as files (config, **not** provenance) |
| **Events / sessions** (`.tellur/traces/`) | **gitignored** | ❌ never — local → hub only |
| **Line-level attribution index** (`.tellur/index/`) | **gitignored** | ❌ never |
| Local secrets/state (`daemon.token`, `push_state.json`) | gitignored | ❌ |
| **Commit-level attribution** (`refs/notes/ai`) | git note | ✅ **only if pushed** (and clients don't fetch notes by default) |

**Consequence:** the rich provenance (live, line-level, per-session) is
deliberately kept out of git — it can contain prompts/diffs and is large. The
only provenance that travels with git is the **condensed commit-level note**. So
a GitHub App is **not** a way to "get all the repo data"; it can harvest the
git-notes slice and provide source access, but the rich stream must still be
captured at the source and pushed to the hub.

## 3. Two automatic data paths into the hub

```
                    ┌───────────────── rich path (depth) ─────────────────┐
  editor / agent ──▶│ tellur watch (live) → background tellur push → HUB  │
  (local capture)   └─────────────────────────────────────────────────────┘
                    ┌──────────────── git-native path (breadth) ──────────┐
  git commit ──────▶│ refs/notes/ai → auto-pushed to GitHub → App harvest │──▶ HUB
                    └─────────────────────────────────────────────────────┘
```

- **Rich path** — line-level, sessions, prompts-hashed; from machines running the
  agent. Forwarded by the background pusher (idempotent high-water mark, #33).
- **Git-native path** — commit-level; the note is written **locally from the
  attribution index**, then travels with the repo. Once pushed, any clone (and the
  harvester) can read it without a hub account, and the GitHub App mirrors notes
  GitHub → hub on push.

  **Caveat — it does not cover uninstrumented machines.** A commit made where
  Tellur isn't installed has no local index and no `post-commit` hook, so it
  produces **no `refs/notes/ai`** to harvest. The git-native path broadens
  *distribution* of provenance that was captured somewhere with Tellur; it does
  not *generate* provenance for commits made without it (CI, a contributor without
  Tellur). Closing that gap would need a separate note source — out of scope here.

They are complementary, not redundant: depth (rich, per-session) vs. portable
distribution (commit-level notes that ride along with git).

## 4. Part A — Zero-touch client (provider-agnostic, no GitHub App)

This delivers the "never touch the terminal" goal on its own and works for
GitHub/GitLab/Bitbucket alike. **Do this first.**

One-time: `tellur connect --hub <url>` (new umbrella command) performs:
1. `tellur login` — device flow → hub credentials (`~/.config/tellur/hosts.json`).
2. `tellur setup agents` — installs the editor/agent hooks (live capture).
3. **Installs git hooks** (chained, never clobbering existing hooks):
   - `post-commit` → write/refresh `refs/notes/ai` for the new commit.
   - `pre-push` → flush events to the hub (`tellur push`) **and** push the notes
     ref alongside the branch.
4. **Configures notes sync** — extends `tellur notes install-config` to also add a
   push refspec (`+refs/notes/ai:refs/notes/ai`) so a plain `git push` carries
   notes.
5. **Registers a background agent** — a per-user service (launchd on macOS,
   systemd `--user` on Linux, Scheduled Task on Windows) that keeps `tellur
   watch` running and runs a **debounced periodic `tellur push`**. This is what
   removes the terminal from the loop.

Design notes / risks:
- **Hook chaining**: detect and preserve pre-existing hooks (append, or use
  `core.hooksPath` with a dispatcher) — never overwrite a team's hooks.
- **Notes push is opt-in**: pushing notes publishes commit-level AI attribution to
  the remote (visible to repo readers). Default on for team installs, with a clear
  consent + an off switch; document the privacy implication.
- **Background agent failure must be silent + self-healing**: never block a
  commit/push if the hub is unreachable; the high-water mark means the next run
  catches up. `pre-push` flush has a short timeout and never fails the push.
- **Uninstall** (`tellur connect --remove`) must cleanly remove hooks, the service,
  and notes config.

## 5. Part B — GitHub App (GitHub-specific enhancements)

Layered on top of Part A. Each item is independently shippable.

### B1. Installation tokens for the blob proxy
Replace the manually-pasted PAT (#35) with **GitHub App installation tokens**:
the hub signs an App JWT and exchanges it for a short-lived, per-repo installation
token (Contents:read) to fetch private source in the proxy. Wins: short-lived +
auto-rotating, per-repo least privilege, revoked by uninstalling the App, no
human-managed secret in the DB. The PAT path stays as the provider-agnostic
fallback (GitLab/Bitbucket/self-managed).

### B2. Repo discovery + auto-provision
List the installation's repos → create/sync them on the hub by name, auto-fill
the source connection (templates) — no manual `--repo` or template entry.
Shipped via signed GitHub webhooks plus an explicit installation→org mapping.

### B3. Notes harvester (the git-native sync)
On the App's `push` **webhook**, the hub fetches the repo's updated `refs/notes/ai`
(via the installation token) and ingests the commit-level attribution. This makes
auto-pushed notes pay off automatically: a developer pushes code+notes to GitHub
as usual; the hub stays current without any per-developer hub push of notes.
Shipped as `POST /webhook/github`, HMAC-verified with
`TELLUR_GITHUB_WEBHOOK_SECRET`, idempotent per `(org, repo, commit)`.

### B4. PR checks
Post the Tellur PR risk report as a **Check Run** on the PR (native, replacing/
augmenting the example GitHub Action).

### Permissions (least privilege) + secrets
- Repository permissions: **Contents: read** (source + notes), **Metadata: read**,
  **Checks: write** (B4), **Pull requests: read** (B4).
- Webhook events: `push` (B3), `pull_request` (B4).
- New secrets: the **App private key** (signs JWTs) and a **webhook secret**
  (HMAC verification). Both env/secret-store, never committed.

## 6. Trust boundaries & security (threat-model deltas)

New or changed boundaries to add to `docs/THREAT_MODEL.md` when built:
- **GitHub → hub (inbound webhook)** — a new untrusted inbound surface. Verify the
  `X-Hub-Signature-256` HMAC against the webhook secret; ignore unsigned/replayed
  deliveries; the harvest only ever reads repos of the delivering installation
  (tenant-bound), so a forged delivery can't cross orgs.
- **Hub → GitHub (outbound, App-authed)** — installation-token fetches are bounded
  to the installation's repos and the existing SSRF host allowlist (#35) still
  applies; tokens are short-lived and never returned to clients.
- **Auto-notes-push privacy** — commit-level AI attribution becomes visible to
  anyone with repo read on GitHub. This is a deliberate, opt-in publication;
  surface it at install time.
- **Background agent** — holds the hub token (`0600`) and runs as the user; same
  trust level as `.tellur` local data. Must fail closed (never block git, never
  leak on errors).

## 7. What this explicitly does NOT do

- It does **not** make the GitHub App the provenance source. The rich, line-level
  data is generated at the editor/agent and is never in git; the App harvests only
  the commit-level notes slice and provides source access.
- It does **not** force the rich `traces/` into git (privacy + size). The
  git-native channel stays the condensed `refs/notes/ai`.
- It does **not** couple the core product to GitHub: Part A is provider-agnostic;
  Part B is an optional GitHub-only layer with a PAT fallback.

## 8. Phased plan

| Phase | Scope | Provider | Value |
| --- | --- | --- | --- |
| **P1** | `tellur connect`: git hooks + background pusher + auto notes push | any | ✅ Shipped |
| **P2** | GitHub App: installation tokens for the blob proxy | GitHub | ✅ Shipped |
| **P3** | GitHub App: repo discovery + notes-harvester webhook | GitHub | ✅ Shipped |
| **P4** | GitHub App: PR risk-report Check Run | GitHub | Native PR feedback |

Recommended order: **P1 first** (delivers the developer's "never touch the
terminal" requirement, provider-agnostic), then P2–P4 layer GitHub-specific
convenience and security on top.

## 9. Open questions

- Background agent: ship our own supervisor, or lean on the existing `tellur
  daemon`? (Likely reuse the daemon + a thin service wrapper.)
- Notes-push default: on or off for individual (non-team) installs?
- Harvester dedup: notes are re-pushable; ingest must be idempotent per
  `(repo, commit)` so re-delivered webhooks don't double-count.
- GitHub Enterprise Server support for the App (different base URLs).
