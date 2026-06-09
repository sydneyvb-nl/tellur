# Tellur team dashboard (SPA)

The web dashboard for the self-hosted Tellur hub (`tellur-server`). Built with
Vite + Svelte 5 + TypeScript and **served by the hub at `/app`** — the assets are
embedded into the binary (`rust-embed`, the `dashboard` Cargo feature). The SPA
calls the hub's `/v1/...` JSON API same-origin with the SSO session cookie.

See `docs/proposals/TEAM_DASHBOARD_UI.md` for the product/UX/architecture plan.

## License

Part of `tellur-server` — **FSL-1.1-ALv2** (not Apache-2.0). See
`crates/server/LICENSE`.

## Develop

```bash
pnpm install
pnpm dev        # Vite dev server; proxies /v1 + /auth to a hub on :4920
pnpm check      # svelte-check (typecheck)
pnpm test       # vitest unit tests
pnpm build      # → dist/ (embedded by the hub at compile time)
```

A plain `cargo build` of the hub works without building this first: `build.rs`
creates an empty `ui/dist` and the hub serves a small placeholder at `/app`. The
release Docker image and the `dashboard` CI job build the real SPA and embed it.

## Status

D0 (foundation): app shell, org-scoped routing, SSO auth redirect, design
tokens, and an **Overview** screen on `GET /v1/orgs/{org}/dashboard`. Later
phases (D1–D5) add repos/files/sessions/policies/audit/exports/people per the
plan.
