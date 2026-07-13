# Examples

Copy-paste examples for adopting Tellur in CI.

## `github-actions-team-report.yml`

A GitHub Actions workflow that posts a [`tellur team report`](../../README.md#team-reports-no-server)
comment on every pull request — the no-server (Tier 0) team-mode path.

Usage:

1. Copy it to `.github/workflows/tellur-team-report.yml` in your repository.
2. Make sure contributors publish their authorship notes with `tellur notes push`
   (or via `tellur notes install-config` for automatic fetch/rewrite).
3. Open a PR — the workflow fetches `refs/notes/ai`, aggregates the range, and
   upserts a single comment with AI involvement by tool/model/author plus
   commit- and line-level provenance coverage.

It degrades honestly: if no notes exist, actual added PR lines are reported as
unknown with a `provenance unavailable` warning. Missing evidence is never
presented as 0% AI involvement.

See also the per-PR risk-report workflow in
[`.github/workflows/tellur.yml`](../../.github/workflows/tellur.yml).
