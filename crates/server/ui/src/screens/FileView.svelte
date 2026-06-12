<script lang="ts">
  import { api, type AttrFile } from "../lib/api";
  import { pct } from "../lib/format";
  import { repoPath } from "../lib/router";
  import { sourceLink, rawUrl, sliceLines } from "../lib/source";
  import { t } from "../lib/i18n.svelte";

  let { org, repo, path }: { org: string; repo: string; path: string } = $props();

  let file = $state<AttrFile | null>(null);
  let sourceTemplate = $state<string | null>(null);
  let rawTemplate = $state<string | null>(null);
  // When set, the repo is private: fetch raw bytes through the hub's proxy
  // (same-origin, session-authed) instead of directly from the provider.
  let sourceProxy = $state(false);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let reloadKey = $state(0);

  // Inline source gutter (opt-in): the browser fetches raw bytes straight from
  // the provider — the hub stores/serves none.
  let showSource = $state(false);
  let sourceText = $state<string | null>(null);
  let sourceError = $state<string | null>(null);
  let sourceLoading = $state(false);
  const MAX_SOURCE_BYTES = 2_000_000;

  $effect(() => {
    const o = org;
    const r = repo;
    const p = path;
    void reloadKey;
    let cancelled = false;
    loading = true;
    error = null;
    // Reset inline-source state so navigating to another file never renders the
    // previous file's cached source (or keeps its fetch error).
    showSource = false;
    sourceText = null;
    sourceError = null;
    sourceLoading = false;
    api
      .attributions(o, r)
      .then((res) => {
        if (cancelled) return;
        file = res.files.find((f) => f.file_path === p) ?? null;
        sourceTemplate = res.source_template;
        rawTemplate = res.source_raw_template;
        sourceProxy = res.source_proxy;
      })
      .catch((e) => {
        if (!cancelled) error = e instanceof Error ? e.message : t("app.failed");
      })
      .finally(() => {
        if (!cancelled) loading = false;
      });
    return () => {
      cancelled = true;
    };
  });

  function originClass(origin: string): string {
    return origin.toLowerCase(); // ai | human | mixed | unknown → CSS class
  }

  async function toggleSource() {
    showSource = !showSource;
    if (!showSource || sourceText || sourceError || sourceLoading) return;
    if (!file) return;
    sourceLoading = true;
    try {
      if (sourceProxy) {
        // Private repo: the hub fetches the bytes for us (it holds the token).
        const res = await api.blob(org, repo, file.file_path);
        if (res.content.length > MAX_SOURCE_BYTES) throw new Error(t("fileView.tooLarge"));
        sourceText = res.content;
      } else {
        const url = rawUrl(rawTemplate, file.file_path);
        if (!url) {
          sourceError = t("fileView.noRawUrl");
          return;
        }
        // No credentials: a plain cross-origin GET to the provider, exactly as
        // the user's browser would do — nothing flows through the hub.
        const res = await fetch(url, { credentials: "omit" });
        if (!res.ok) throw new Error(`provider returned ${res.status}`);
        const len = Number(res.headers.get("content-length") ?? "0");
        if (len > MAX_SOURCE_BYTES) throw new Error(t("fileView.tooLarge"));
        const text = await res.text();
        if (text.length > MAX_SOURCE_BYTES) throw new Error(t("fileView.tooLarge"));
        sourceText = text;
      }
    } catch (e) {
      // Cross-origin/private repos may block the fetch — fall back to links.
      sourceError = e instanceof Error ? e.message : t("app.failed");
    } finally {
      sourceLoading = false;
    }
  }
</script>

<p class="crumb">
  <a href={repoPath(org, repo)}>{repo}</a> / {path}
</p>

{#if loading}
  <div class="panel skeleton"></div>
{:else if error}
  <div class="panel error">
    <p>{error}</p>
    <button onclick={() => (reloadKey += 1)}>{t("common.retry")}</button>
  </div>
{:else if !file}
  <div class="panel empty"><p class="muted">{t("fileView.empty")}</p></div>
{:else}
  <h1 class="mono">{file.file_path}</h1>
  <p class="muted">
    {t("fileView.blob")} <code>{file.git_blob_sha}</code> · {t("fileView.attributedRanges", { n: file.ranges.length })}
    <span class="legend">
      <span class="dot ai"></span> {t("common.ai")}
      <span class="dot human"></span> {t("common.human")}
      <span class="dot mixed"></span> {t("common.mixed")}
    </span>
  </p>
  <p class="note muted">
    {t("fileView.note")}
    {#if sourceTemplate}{t("fileView.noteLinks")}{/if}
  </p>

  {#if rawTemplate}
    <div class="srcbar">
      <button class="toggle" onclick={toggleSource}>
        {showSource ? t("fileView.hideSource") : t("fileView.showSource")}
      </button>
      <span class="muted small">
        {sourceProxy ? t("fileView.fetchedNoteProxy") : t("fileView.fetchedNote")}
      </span>
    </div>
  {/if}

  {#if showSource}
    {#if sourceLoading}
      <div class="panel skeleton"></div>
    {:else if sourceError}
      <div class="panel warnpanel">
        <p class="muted">{t("fileView.loadError", { err: sourceError })}</p>
      </div>
    {:else if sourceText}
      <div class="source">
        {#each file.ranges as r (r.start_line + "-" + r.end_line)}
          <div class="range">
            <div class="range-head mono {originClass(r.origin)}">
              {t("fileView.rangeHead", { origin: r.origin, start: r.start_line, end: r.end_line })}
            </div>
            <pre class="code"><code>{#each sliceLines(sourceText, r.start_line, r.end_line) as ln (ln.n)}<span class="ln">{ln.n}</span>{ln.text}
{/each}</code></pre>
          </div>
        {/each}
      </div>
    {/if}
  {/if}

  <table>
    <thead>
      <tr>
        <th>{t("fileView.colLines")}</th><th>{t("fileView.colOrigin")}</th><th>{t("fileView.colAgent")}</th>
        <th class="num">{t("fileView.colConf")}</th><th>{t("fileView.colReviewed")}</th>
        {#if sourceTemplate}<th>{t("fileView.colSource")}</th>{/if}
      </tr>
    </thead>
    <tbody>
      {#each file.ranges as r (r.start_line + "-" + r.end_line)}
        <tr>
          <td class="mono">
            <span class="gutter {originClass(r.origin)}"></span>
            {r.start_line}–{r.end_line}
          </td>
          <td><span class="pill {originClass(r.origin)}">{r.origin}</span></td>
          <td class="mono small">{r.agent_id}{r.model_id ? ` · ${r.model_id}` : ""}</td>
          <td class="num mono">{pct(r.confidence)}</td>
          <td>
            {#if r.reviewer && r.reviewed_at}
              <span class="ok">✓ {r.reviewer}</span>
            {:else}
              <span class="muted">—</span>
            {/if}
          </td>
          {#if sourceTemplate}
            <td>
              {#if sourceLink(sourceTemplate, { path: file.file_path, start: r.start_line, end: r.end_line })}
                <a
                  class="src"
                  href={sourceLink(sourceTemplate, { path: file.file_path, start: r.start_line, end: r.end_line })}
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  {t("fileView.view")}
                </a>
              {:else}
                <span class="muted">—</span>
              {/if}
            </td>
          {/if}
        </tr>
      {/each}
    </tbody>
  </table>
{/if}

<style>
  .crumb {
    font-size: 12px;
    color: var(--text-muted);
    margin: 0 0 8px;
  }
  .crumb a {
    color: var(--accent);
  }
  h1 {
    font-size: 16px;
    margin: 0 0 4px;
    word-break: break-all;
  }
  .muted {
    color: var(--text-muted);
  }
  .note {
    font-size: 12px;
    margin: 0 0 16px;
  }
  .legend {
    margin-left: 12px;
  }
  .dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 999px;
    margin: 0 2px 0 8px;
  }
  .dot.ai,
  .pill.ai,
  .gutter.ai {
    background: var(--ai);
  }
  .dot.human,
  .pill.human,
  .gutter.human {
    background: var(--human);
  }
  .dot.mixed,
  .pill.mixed,
  .gutter.mixed {
    background: var(--mixed);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    overflow: hidden;
  }
  th,
  td {
    text-align: left;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    font-size: 13px;
  }
  th {
    color: var(--text-muted);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  tbody tr:last-child td {
    border-bottom: none;
  }
  .num {
    text-align: right;
  }
  .small {
    font-size: 12px;
    color: var(--text-muted);
  }
  .gutter {
    display: inline-block;
    width: 3px;
    height: 14px;
    border-radius: 2px;
    vertical-align: middle;
    margin-right: 8px;
  }
  .pill {
    color: #06120d;
    font-size: 11px;
    padding: 1px 8px;
    border-radius: 999px;
    text-transform: capitalize;
  }
  .ok {
    color: var(--ok);
  }
  .src {
    color: var(--accent);
    font-size: 12px;
  }
  .srcbar {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 12px;
  }
  .toggle {
    background: var(--accent-weak);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    padding: 5px 12px;
    cursor: pointer;
    font-size: 13px;
  }
  .warnpanel {
    border-color: var(--warn);
    margin-bottom: 12px;
  }
  .source {
    margin-bottom: 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .range {
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    overflow: hidden;
  }
  .range-head {
    font-size: 11px;
    text-transform: capitalize;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    border-left: 3px solid var(--border);
    color: var(--text-muted);
    background: var(--surface-2);
  }
  .range-head.ai {
    border-left-color: var(--ai);
  }
  .range-head.human {
    border-left-color: var(--human);
  }
  .range-head.mixed {
    border-left-color: var(--mixed);
  }
  .code {
    margin: 0;
    padding: 10px 12px;
    background: var(--surface);
    overflow-x: auto;
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 1.5;
    white-space: pre;
  }
  .ln {
    display: inline-block;
    width: 3em;
    color: var(--text-muted);
    user-select: none;
    text-align: right;
    margin-right: 12px;
  }
  .panel {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    padding: var(--card-pad);
  }
  .panel.error {
    border-color: var(--risk);
  }
  .skeleton {
    height: 120px;
  }
  button {
    background: var(--accent-weak);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    padding: 6px 12px;
    cursor: pointer;
  }
</style>
