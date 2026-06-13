# Bringing the Tellur GitHub App live

This is the operator walkthrough for **P2** of the GitHub-App plan
([`docs/proposals/GITHUB_APP.md`](proposals/GITHUB_APP.md)): authenticating the
hub's private-repo **source proxy** with short-lived GitHub **App installation
tokens** instead of a manually-pasted PAT.

It is **optional and GitHub-only.** Without it, private GitHub repos still work
via a stored Personal Access Token, and GitLab/Bitbucket/self-managed always use
the PAT path. Set the App up when you want auto-rotating, least-privilege,
per-repo tokens with no human-managed secret in the hub database.

> Scope: P2 is **source access only**. The webhook-driven notes harvester (P3)
> and PR Check Runs (P4) are not built yet, so this guide intentionally leaves the
> App's webhook disabled and asks only for read permissions.

---

## 1. Register the GitHub App

On GitHub: **Settings → Developer settings → GitHub Apps → New GitHub App**
(an org App lives under the org's settings; a personal App under your account).

- **GitHub App name** — e.g. `tellur-hub` (must be globally unique).
- **Homepage URL** — your hub URL (any valid URL is fine).
- **Webhook → Active** — **uncheck it.** P2 does not use webhooks; leave this off
  until P3.
- **Repository permissions:**
  - **Contents** → **Read-only** (reads file bytes for the source proxy).
  - **Metadata** → **Read-only** (mandatory; GitHub selects it automatically).
  - Leave everything else **No access**. (Checks: write and Pull requests: read
    are only needed for P4.)
- **Where can this GitHub App be installed?** — "Only on this account" is fine for
  a single org; choose "Any account" only if multiple orgs will install it.

Click **Create GitHub App**.

## 2. Capture the App ID and a private key

On the new App's **General** page:

- Copy the **App ID** (a number) → this becomes `TELLUR_GITHUB_APP_ID`.
- Under **Private keys**, click **Generate a private key**. GitHub downloads a
  `*.pem` file (PKCS#1 or PKCS#8 RSA). **This is a high-value secret** — see
  [Security](#security-notes). Keep it; GitHub does not let you re-download it.

## 3. Install the App on your repositories

On the App's **Install App** page, install it on the target account/org and choose
**Only select repositories** → pick the repos the hub should be able to read (or
"All repositories"). The installation is what grants the hub access; an App with no
installation can mint nothing.

## 4. Configure the hub

The hub reads the App config from the environment. The App is enabled **only when
both** the ID and a key are present:

| Variable | Required | Meaning |
| --- | --- | --- |
| `TELLUR_GITHUB_APP_ID` | yes | The numeric App ID from step 2. |
| `TELLUR_GITHUB_APP_PRIVATE_KEY` | one of these two | The PEM **contents** (inline). |
| `TELLUR_GITHUB_APP_PRIVATE_KEY_FILE` | one of these two | Path to the `.pem` file on the hub host. |
| `TELLUR_GITHUB_API_BASE` | no | API base for **GitHub Enterprise Server** (e.g. `https://ghe.example.com/api/v3`). Default `https://api.github.com`. |

Prefer the **`_FILE`** form so the key never sits in shell history or process
args. Example for the Docker Compose hub (`dist/docker/`):

```yaml
services:
  tellur-server:
    environment:
      TELLUR_GITHUB_APP_ID: "123456"
      TELLUR_GITHUB_APP_PRIVATE_KEY_FILE: /run/secrets/tellur_github_app_key
    secrets:
      - tellur_github_app_key
secrets:
  tellur_github_app_key:
    file: ./tellur-hub.private-key.pem   # chmod 600; keep out of git
```

Or inline (e.g. systemd `EnvironmentFile`, a secret store that injects env):

```bash
export TELLUR_GITHUB_APP_ID=123456
export TELLUR_GITHUB_APP_PRIVATE_KEY="$(cat tellur-hub.private-key.pem)"
```

**Restart the hub** so it re-reads the environment. On boot you should see:

```
INFO tellur_server: GitHub App source access enabled app_id=123456
```

If that line is absent, the App is off — re-check the two variables.

## 5. Connect a repo's source

The App provides the *token*; you still tell the hub *which provider URL* to fetch.
As an org admin, set the repo's source connection — either in the dashboard
(**Repo → Source connection** card) or from the CLI:

```bash
tellur-server admin set-repo-source \
  --org <org> --repo <repo> \
  --link  'https://github.com/OWNER/REPO/blob/main/{path}#L{start}-L{end}' \
  --raw   'https://api.github.com/repos/OWNER/REPO/contents/{path}?ref=main'
```

Notes:
- For **private** GitHub repos use the **Contents API** raw template
  (`https://api.github.com/repos/OWNER/REPO/contents/{path}?ref=BRANCH`). The hub
  sends `Accept: application/vnd.github.raw` so it gets raw bytes. `raw.githubusercontent.com`
  is best for **public** repos. On **GitHub Enterprise** use the Contents API on
  your configured host
  (`https://<ghe-host>/api/v3/repos/OWNER/REPO/contents/{path}?ref=BRANCH`); that
  host is auto-allowlisted from `TELLUR_GITHUB_API_BASE`.
- With the App enabled you do **not** need to store a token for GitHub repos — the
  hub mints an installation token per fetch. A stored PAT, if present, is ignored
  for GitHub repos while the App is configured (and used as the fallback if minting
  fails).
- `OWNER`/`REPO` must be concrete (no `{...}` placeholders) — that's how the hub
  detects a GitHub repo and resolves the installation.

## 6. Verify end to end

1. Open an attributed file in the dashboard and toggle **Show source** on a
   **private** repo — the gutter should render real lines.
2. Or call the proxy directly with any viewer+ token:

   ```bash
   curl -H "Authorization: Bearer <member-token>" \
     "https://hub.example.com/v1/orgs/<org>/repos/<repo>/blob?path=src/main.rs"
   ```

   A `200` with `{"path":...,"content":"..."}` confirms the App-minted token
   fetched private bytes. The token itself is never in the response.
3. Check the hub logs: no `GitHub App token mint failed; falling back to stored
   token` warning means the App path is working.

## Troubleshooting

| Symptom | Likely cause |
| --- | --- |
| No "GitHub App source access enabled" at startup | `TELLUR_GITHUB_APP_ID` or the key var is unset/empty; the hub wasn't restarted. |
| Log: `invalid GitHub App private key (expected an RSA PEM)` | The key isn't an RSA PEM, or `_PRIVATE_KEY` got truncated (newlines lost). Use `_PRIVATE_KEY_FILE`. |
| Log: `GitHub App installation lookup failed` / token mint failed | The App isn't installed on that repo, or the repo wasn't selected in the installation; the proxy falls back to the stored PAT (often none → next row). |
| Proxy returns `404`/empty for a private repo | No usable token (App not installed there and no PAT), or the `OWNER/REPO`/branch in the raw template is wrong. |
| `source host '…' is not in the allowed provider list` | The raw template host isn't on the SSRF allowlist — use `api.github.com` / `raw.githubusercontent.com` (or, on GHES, the host of `TELLUR_GITHUB_API_BASE`). |
| GitHub Enterprise Server | Set `TELLUR_GITHUB_API_BASE` to your `…/api/v3` base. The hub then allowlists that host and recognises **Contents API** templates on it — connect GHES repos with `--raw 'https://<ghe-host>/api/v3/repos/OWNER/REPO/contents/{path}?ref=BRANCH'` (a `raw.<host>` subdomain is **not** supported). |

## Security notes

- The **App private key** can mint read tokens for **every repo the App is
  installed on**. Treat it like a root credential: store it in a secret manager or
  a `0600` file, never commit it, and scope the installation to only the repos the
  hub needs.
- Installation tokens are short-lived (≈1h) and cached in-process; the hub never
  returns them to any client and never persists them.
- **Rotate** by generating a new private key on the App and updating the hub
  (remove the old key from the App afterwards). **Revoke everything** by
  uninstalling the App from the account.
- The outbound fetch is still bounded by the hub's SSRF host allowlist
  (`api.github.com` / `raw.githubusercontent.com` / GitLab / Bitbucket), https-only,
  2 MB cap. See [`docs/THREAT_MODEL.md`](THREAT_MODEL.md).
