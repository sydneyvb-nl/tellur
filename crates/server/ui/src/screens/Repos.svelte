<script lang="ts">
  import { api, type Dashboard } from "../lib/api";
  import { count } from "../lib/format";
  import { repoPath } from "../lib/router";
  import { t } from "../lib/i18n.svelte";

  let { org }: { org: string } = $props();

  let data = $state<Dashboard | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let reloadKey = $state(0);

  $effect(() => {
    const currentOrg = org;
    void reloadKey;
    let cancelled = false;
    loading = true;
    error = null;
    api
      .dashboard(currentOrg)
      .then((d) => {
        if (!cancelled) data = d;
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
</script>

<h1>{t("repos.title")}</h1>

{#if loading}
  <div class="panel skeleton"></div>
{:else if error}
  <div class="panel error">
    <p>{error}</p>
    <button onclick={() => (reloadKey += 1)}>{t("common.retry")}</button>
  </div>
{:else if data}
  {#if data.report.repos.length === 0}
    <div class="panel empty">
      <p class="muted">{t("repos.empty")}</p>
    </div>
  {:else}
    <table>
      <thead>
        <tr><th>{t("repos.colRepo")}</th><th class="num">{t("repos.colEvents")}</th></tr>
      </thead>
      <tbody>
        {#each data.report.repos as r (r.id)}
          <tr>
            <td><a href={repoPath(org, r.id)}>{r.name}</a></td>
            <td class="num mono">{count(r.event_count)}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
{/if}

<style>
  h1 {
    font-size: 20px;
    margin: 0 0 16px;
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
    padding: var(--row-pad-y) var(--row-pad-x);
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
  td a {
    color: var(--accent);
  }
  .num {
    text-align: right;
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
  .muted {
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
</style>
