<script lang="ts">
  import { api, type RepoDetail, type AttrFile } from "../lib/api";
  import { count, pct, relativeTime } from "../lib/format";
  import { reposPath, filePath } from "../lib/router";

  let { org, repo }: { org: string; repo: string } = $props();

  let data = $state<RepoDetail | null>(null);
  let files = $state<AttrFile[]>([]);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let reloadKey = $state(0);

  $effect(() => {
    const currentOrg = org;
    const currentRepo = repo;
    void reloadKey;
    let cancelled = false;
    loading = true;
    error = null;
    Promise.all([
      api.repo(currentOrg, currentRepo),
      api.attributions(currentOrg, currentRepo),
    ])
      .then(([d, a]) => {
        if (!cancelled) {
          data = d;
          files = a.files;
        }
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
</script>

<p class="crumb"><a href={reposPath(org)}>Repositories</a> / {repo}</p>

{#if loading}
  <div class="panel skeleton"></div>
{:else if error}
  <div class="panel error">
    <p>{error}</p>
    <button onclick={() => (reloadKey += 1)}>Retry</button>
  </div>
{:else if data}
  <h1>{data.name}</h1>
  <p class="muted">
    {count(data.event_count)} events · {data.attributed_files} attributed files ·
    {#if data.last_activity}last activity {relativeTime(data.last_activity)}{:else}no
      activity{/if}
  </p>

  <section class="kpis">
    <div class="kpi">
      <div class="num mono">{data.ai_share === null ? "—" : pct(data.ai_share)}</div>
      <div class="lbl">AI-attributed lines</div>
    </div>
    <div class="kpi">
      <div class="num mono">
        {data.review_coverage === null ? "—" : pct(data.review_coverage)}
      </div>
      <div class="lbl">AI lines reviewed</div>
    </div>
    <div class="kpi">
      <div class="num mono">{count(data.lines.ai)}</div>
      <div class="lbl">AI lines</div>
    </div>
  </section>

  <section class="panel">
    <h2>Attributed files</h2>
    {#if files.length === 0}
      <p class="muted">No attribution recorded yet.</p>
    {:else}
      <ul class="files">
        {#each files as f (f.file_path)}
          <li>
            <a class="mono" href={filePath(org, repo, f.file_path)}>{f.file_path}</a>
            <span class="muted">{f.ranges.length} ranges</span>
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <section class="panel">
    <h2>Contributors</h2>
    {#if data.contributors.length === 0}
      <p class="muted">None recorded.</p>
    {:else}
      <ul class="chips">
        {#each data.contributors as c (c)}
          <li class="chip">{c}</li>
        {/each}
      </ul>
    {/if}
  </section>
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
    font-size: 20px;
    margin: 0 0 4px;
  }
  h2 {
    font-size: 13px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
    margin: 0 0 12px;
  }
  .muted {
    color: var(--text-muted);
    margin: 0 0 16px;
  }
  .kpis {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 12px;
    margin-bottom: 12px;
  }
  .kpi,
  .panel {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    padding: var(--card-pad);
  }
  .panel.error {
    border-color: var(--risk);
  }
  .kpi .num {
    font-size: 28px;
    font-weight: 600;
  }
  .kpi .lbl {
    color: var(--text-muted);
    font-size: 12px;
    margin-top: 4px;
  }
  .skeleton {
    height: 120px;
  }
  .files {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .files li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 0;
    border-bottom: 1px solid var(--border);
    font-size: 13px;
  }
  .files li:last-child {
    border-bottom: none;
  }
  .files a {
    color: var(--accent);
    word-break: break-all;
  }
  .chips {
    list-style: none;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin: 0;
    padding: 0;
  }
  .chip {
    font-size: 12px;
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 2px 10px;
    color: var(--text-muted);
  }
  button {
    background: var(--accent-weak);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    padding: 6px 12px;
    cursor: pointer;
  }
  @media (max-width: 768px) {
    .kpis {
      grid-template-columns: 1fr;
    }
  }
</style>
