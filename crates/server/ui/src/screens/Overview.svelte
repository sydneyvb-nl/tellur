<script lang="ts">
  import { api, type Overview } from "../lib/api";
  import { count, pct, relativeTime } from "../lib/format";
  import { repoPath } from "../lib/router";
  import Trend from "../components/Trend.svelte";
  import { t } from "../lib/i18n.svelte";

  let { org }: { org: string } = $props();

  let data = $state<Overview | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let reloadKey = $state(0);

  $effect(() => {
    const currentOrg = org;
    void reloadKey;
    let cancelled = false;
    loading = true;
    error = null;
    // One round-trip: totals, rollups, activity, ranked repos, recent feed.
    api
      .overview(currentOrg)
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

<h1>{t("overview.title")}</h1>

{#if loading}
  <div class="kpis">
    {#each Array(5) as _unused}
      <div class="kpi skeleton"></div>
    {/each}
  </div>
{:else if error}
  <div class="panel error">
    <p>{error}</p>
    <button onclick={() => (reloadKey += 1)}>{t("common.retry")}</button>
  </div>
{:else if data}
  {#if data.totals.events === 0}
    <div class="panel empty">
      <p>{t("overview.emptyTitle")}</p>
      <p class="muted">
        {t("overview.emptyHintPre")}
        <code>tellur notes push</code> {t("overview.emptyHintPost")}
      </p>
    </div>
  {:else}
    <section class="kpis" aria-label="Key metrics">
      <div class="kpi">
        <div class="num mono">{count(data.totals.events)}</div>
        <div class="lbl">{t("overview.kpiEvents")}</div>
      </div>
      <div class="kpi">
        <div class="num mono">{count(data.totals.sessions)}</div>
        <div class="lbl">{t("overview.kpiSessions")}</div>
      </div>
      <div class="kpi">
        <div class="num mono">{count(data.totals.repos)}</div>
        <div class="lbl">{t("overview.kpiRepos")}</div>
      </div>
      <div class="kpi">
        <div class="num mono">{data.ai_share === null ? "—" : pct(data.ai_share)}</div>
        <div class="lbl">{t("overview.kpiAiLines")}</div>
      </div>
      <div
        class="kpi"
        class:warn={data.review_coverage !== null && data.review_coverage < 0.5}
      >
        <div class="num mono">
          {data.review_coverage === null ? "—" : pct(data.review_coverage)}
        </div>
        <div class="lbl">{t("overview.kpiReviewed")}</div>
      </div>
    </section>

    <Trend buckets={data.activity} label={t("trend.activity", { days: 30 })} />

    <div class="cols">
      <section class="panel">
        <h2>{t("overview.reposByGap")}</h2>
        {#if data.repos.length === 0}
          <p class="muted">{t("overview.noRepos")}</p>
        {:else}
          <ul class="rows">
            {#each data.repos.slice(0, 6) as r (r.id)}
              <li>
                <a class="repo" href={repoPath(org, r.id)}>{r.name}</a>
                <span class="spacer"></span>
                {#if r.review_gap_lines > 0}
                  <span class="gap mono">
                    {t("overview.unreviewed", { n: count(r.review_gap_lines) })}
                  </span>
                {:else if r.ai_lines > 0}
                  <span class="clear mono">{t("overview.reviewed")}</span>
                {:else}
                  <span class="muted mono">{t("overview.noAi")}</span>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      </section>

      <section class="panel">
        <h2>{t("overview.recent")}</h2>
        {#if data.recent_events.length === 0}
          <p class="muted">{t("overview.noRecent")}</p>
        {:else}
          <ul class="rows">
            {#each data.recent_events as e (e.id)}
              <li>
                <span class="evt mono">{e.type}</span>
                <span class="muted">{t("common.by", { actor: e.actor })}</span>
                <span class="spacer"></span>
                <span class="muted">{relativeTime(e.timestamp)}</span>
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    </div>
  {/if}
{/if}

<style>
  h1 {
    font-size: 20px;
    margin: 0 0 16px;
  }
  h2 {
    font-size: 13px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
    margin: 0 0 12px;
  }
  .kpis {
    display: grid;
    grid-template-columns: repeat(5, 1fr);
    gap: 12px;
    margin-bottom: 16px;
  }
  .kpi {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    padding: var(--card-pad);
  }
  .kpi.warn {
    border-color: var(--warn);
  }
  .kpi .num {
    font-size: 26px;
    font-weight: 600;
  }
  .kpi .lbl {
    color: var(--text-muted);
    font-size: 12px;
    margin-top: 4px;
  }
  .skeleton {
    height: 76px;
    background: linear-gradient(
      90deg,
      var(--surface),
      var(--surface-2),
      var(--surface)
    );
  }
  .cols {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
    margin-bottom: 12px;
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
  .rows {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  .rows li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 0;
    border-bottom: 1px solid var(--border);
    font-size: 13px;
  }
  .rows li:last-child {
    border-bottom: none;
  }
  .repo {
    color: var(--accent);
  }
  .gap {
    font-size: 12px;
    color: var(--warn);
  }
  .clear {
    font-size: 12px;
    color: var(--ok);
  }
  .spacer {
    flex: 1;
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
  @media (max-width: 1024px) {
    .kpis {
      grid-template-columns: repeat(2, 1fr);
    }
  }
  @media (max-width: 768px) {
    .cols {
      grid-template-columns: 1fr;
    }
  }
</style>
