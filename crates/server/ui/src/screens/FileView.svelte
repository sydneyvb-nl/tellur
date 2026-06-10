<script lang="ts">
  import { api, type AttrFile } from "../lib/api";
  import { pct } from "../lib/format";
  import { repoPath } from "../lib/router";
  import { sourceLink } from "../lib/source";

  let { org, repo, path }: { org: string; repo: string; path: string } = $props();

  let file = $state<AttrFile | null>(null);
  let sourceTemplate = $state<string | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let reloadKey = $state(0);

  $effect(() => {
    const o = org;
    const r = repo;
    const p = path;
    void reloadKey;
    let cancelled = false;
    loading = true;
    error = null;
    api
      .attributions(o, r)
      .then((res) => {
        if (cancelled) return;
        file = res.files.find((f) => f.file_path === p) ?? null;
        sourceTemplate = res.source_template;
      })
      .catch((e) => {
        if (!cancelled) error = e instanceof Error ? e.message : "failed to load";
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
</script>

<p class="crumb">
  <a href={repoPath(org, repo)}>{repo}</a> / {path}
</p>

{#if loading}
  <div class="panel skeleton"></div>
{:else if error}
  <div class="panel error">
    <p>{error}</p>
    <button onclick={() => (reloadKey += 1)}>Retry</button>
  </div>
{:else if !file}
  <div class="panel empty"><p class="muted">No attribution recorded for this file.</p></div>
{:else}
  <h1 class="mono">{file.file_path}</h1>
  <p class="muted">
    blob <code>{file.git_blob_sha}</code> · {file.ranges.length} attributed ranges
    <span class="legend">
      <span class="dot ai"></span> AI
      <span class="dot human"></span> human
      <span class="dot mixed"></span> mixed
    </span>
  </p>
  <p class="note muted">
    Provenance metadata only — the hub never stores or proxies source text.
    {#if sourceTemplate}Source links open the lines at your configured provider.{/if}
  </p>

  <table>
    <thead>
      <tr>
        <th>Lines</th><th>Origin</th><th>Agent / model</th>
        <th class="num">Conf.</th><th>Reviewed</th>
        {#if sourceTemplate}<th>Source</th>{/if}
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
              {#if sourceLink(sourceTemplate, { path: file.file_path, start: r.start_line, end: r.end_line, sha: file.git_blob_sha })}
                <a
                  class="src"
                  href={sourceLink(sourceTemplate, { path: file.file_path, start: r.start_line, end: r.end_line, sha: file.git_blob_sha })}
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  View ↗
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
