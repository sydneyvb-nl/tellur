# Licensing & repository structure

**Status:** Direction (proposal) — not all of this is implemented yet.
**Last updated:** 2026-06-03

This documents how Tellur is licensed and structured. It is intentionally
minimal. It is not legal advice and may change.

## License layers

| Component | Path | License |
| --- | --- | --- |
| Core library | `crates/core` | Apache-2.0 |
| CLI | `crates/cli` | Apache-2.0 |
| Adapters | `crates/adapters` | Apache-2.0 |
| Schemas | `schemas/` | Apache-2.0 |
| Editor integrations | `editor/` | Apache-2.0 |
| Team/server component (future) | `crates/server` | FSL-1.1-Apache-2.0 (planned) |

The CLI, core, adapters, schemas, and editor integrations are permissively
licensed (Apache-2.0) to maximize adoption and keep the provenance schema and
adapter ecosystem open.

The optional team/server component (see
[`TEAM_SERVER_MODE.md`](TEAM_SERVER_MODE.md), not yet built) is planned under the
**Functional Source License (FSL-1.1-Apache-2.0)**: source-available, with use
restricted to non-competing purposes, and an automatic conversion to Apache-2.0
two years after each release. It will live in a separable component
(`crates/server`) with its own `LICENSE` so the Apache-2.0 core is never
affected.

## Repository structure

A single monorepo holds both the Apache-2.0 core and the (future)
FSL-licensed server component, each with its own `LICENSE` file. Licenses may
differ per directory. The server may move to its own repository later if needed;
there is no need to split early.

## Contributions

- Contributions are accepted under the **Developer Certificate of Origin (DCO)**:
  sign commits with `git commit -s` (a `Signed-off-by` line).
- **No CLA (Contributor License Agreement) is required.** Contributors keep their
  copyright. Contributions are inbound = outbound: code is contributed under the
  license of the component it is added to (Apache-2.0 for the core, FSL for the
  server component).

## Trademark

"Tellur" is intended to be protected as a trademark. The source licenses above
do not grant trademark rights. Details will be added when a trademark policy is
published.
