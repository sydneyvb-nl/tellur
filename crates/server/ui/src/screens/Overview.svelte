<script lang="ts">
  import { api, type Overview } from "../lib/api";
  import { count, pct, relativeTime } from "../lib/format";
  import { repoPath, reposPath } from "../lib/router";
  import Trend from "../components/Trend.svelte";
  import { t } from "../lib/i18n.svelte";

  let { org }: { org: string } = $props();
  let data = $state<Overview | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let reloadKey = $state(0);

  const reviewGap = $derived(
    data ? Math.max(0, data.totals.ai_lines - data.totals.reviewed_ai_lines) : 0,
  );
  const reposAtRisk = $derived(
    data ? data.repos.filter((repo) => repo.review_gap_lines > 0).length : 0,
  );

  $effect(() => {
    const currentOrg = org;
    void reloadKey;
    let cancelled = false;
    loading = true;
    error = null;
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

<header class="page-head">
  <div>
    <p class="eyebrow">{t("overview.eyebrow")}</p>
    <h1>{t("overview.title")}</h1>
    <p class="lede">{t("overview.lede")}</p>
  </div>
  {#if data}
    <p class="freshness">{t("overview.updated", { time: relativeTime(data.generated_at) })}</p>
  {/if}
</header>

{#if loading}
  <div class="metric-strip skeleton" aria-label={t("common.loading")}></div>
{:else if error}
  <div class="notice error">
    <strong>{t("overview.loadFailed")}</strong>
    <p>{error}</p>
    <button onclick={() => (reloadKey += 1)}>{t("common.retry")}</button>
  </div>
{:else if data}
  {#if data.totals.events === 0}
    <div class="notice empty">
      <strong>{t("overview.emptyTitle")}</strong>
      <p>{t("overview.emptyHintPre")} <code>tellur connect</code>.</p>
    </div>
  {:else}
    <section class="decision" aria-labelledby="decision-title">
      <div class="decision-copy">
        <p class="eyebrow">{t("overview.attention")}</p>
        <h2 id="decision-title">
          {#if data.totals.ai_lines === 0}
            {t("overview.noAiHeadline")}
          {:else if reviewGap > 0}
            {t("overview.reviewGapHeadline", { n: count(reviewGap) })}
          {:else}
            {t("overview.clearHeadline")}
          {/if}
        </h2>
        <p>
          {#if data.totals.ai_lines === 0}
            {t("overview.noAiContext")}
          {:else}
            {t("overview.reviewGapContext", {
              repos: count(reposAtRisk),
              coverage: data.review_coverage === null ? "—" : pct(data.review_coverage),
            })}
          {/if}
        </p>
      </div>
      <a class="primary" href={reposPath(org)}>{t("overview.reviewRepos")}</a>
    </section>

    <section class="metric-strip" aria-label={t("overview.keyMetrics")}>
      <div class="metric critical">
        <span class="value mono">{count(reviewGap)}</span>
        <span class="label">{t("overview.kpiGap")}</span>
      </div>
      <div class="metric">
        <span class="value mono">{data.review_coverage === null ? "—" : pct(data.review_coverage)}</span>
        <span class="label">{t("overview.kpiReviewed")}</span>
      </div>
      <div class="metric">
        <span class="value mono">{data.ai_share === null ? "—" : pct(data.ai_share)}</span>
        <span class="label">{t("overview.kpiAiLines")}</span>
      </div>
      <div class="metric">
        <span class="value mono">{count(data.totals.repos)}</span>
        <span class="label">{t("overview.kpiRepos")}</span>
      </div>
    </section>

    <div class="workspace">
      <section class="risk-list" aria-labelledby="risk-title">
        <div class="section-head">
          <div>
            <p class="eyebrow">{t("overview.priority")}</p>
            <h2 id="risk-title">{t("overview.reposByGap")}</h2>
          </div>
          <a href={reposPath(org)}>{t("overview.allRepos")}</a>
        </div>
        {#if data.repos.length === 0}
          <p class="muted">{t("overview.noRepos")}</p>
        {:else}
          <ol>
            {#each [...data.repos].sort((a, b) => b.review_gap_lines - a.review_gap_lines).slice(0, 6) as repo (repo.id)}
              <li>
                <a class="repo-row" href={repoPath(org, repo.id)}>
                  <span class="repo-copy">
                    <strong>{repo.name}</strong>
                    <span>{repo.ai_share === null ? "—" : pct(repo.ai_share)} {t("overview.aiShareSuffix")}</span>
                  </span>
                  <span class:clear={repo.review_gap_lines === 0} class="gap mono">
                    {repo.review_gap_lines > 0
                      ? t("overview.unreviewed", { n: count(repo.review_gap_lines) })
                      : t("overview.reviewed")}
                  </span>
                  <span aria-hidden="true">→</span>
                </a>
              </li>
            {/each}
          </ol>
        {/if}
      </section>

      <aside class="evidence" aria-labelledby="evidence-title">
        <p class="eyebrow">{t("overview.evidence")}</p>
        <h2 id="evidence-title">{t("overview.activityTitle")}</h2>
        <Trend buckets={data.activity} label={t("trend.activity", { days: 30 })} />
        <dl>
          <div><dt>{t("overview.kpiEvents")}</dt><dd class="mono">{count(data.totals.events)}</dd></div>
          <div><dt>{t("overview.kpiSessions")}</dt><dd class="mono">{count(data.totals.sessions)}</dd></div>
        </dl>
      </aside>
    </div>
  {/if}
{/if}

<style>
  .page-head, .decision, .section-head, .repo-row, dl div { display: flex; align-items: center; }
  .page-head { justify-content: space-between; gap: 24px; margin-bottom: 24px; }
  h1 { font-size: clamp(26px, 3vw, 38px); letter-spacing: -0.03em; margin: 2px 0 4px; }
  h2 { font-size: 17px; margin: 2px 0 0; letter-spacing: -0.01em; }
  .eyebrow { color: var(--accent); font: 600 11px/1.4 var(--font-mono); letter-spacing: .08em; text-transform: uppercase; margin: 0; }
  .lede, .freshness, .muted { color: var(--text-muted); }
  .lede { margin: 0; max-width: 620px; }
  .freshness { font-size: 12px; white-space: nowrap; }
  .decision { justify-content: space-between; gap: 28px; padding: 24px 0; border-block: 1px solid var(--border); animation: reveal .32s ease-out both; }
  .decision-copy h2 { font-size: clamp(22px, 3vw, 32px); }
  .decision-copy p:last-child { color: var(--text-muted); margin: 8px 0 0; }
  .primary { background: var(--accent); color: #07130e; padding: 10px 14px; border-radius: var(--radius-control); font-weight: 650; white-space: nowrap; }
  .metric-strip { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); border-bottom: 1px solid var(--border); }
  .metric { display: flex; flex-direction: column; gap: 4px; padding: 24px 20px 24px 0; min-width: 0; }
  .metric + .metric { border-left: 1px solid var(--border); padding-left: 20px; }
  .value { font-size: clamp(24px, 3vw, 34px); font-weight: 650; letter-spacing: -.04em; }
  .critical .value { color: var(--warn); }
  .label { color: var(--text-muted); font-size: 12px; }
  .workspace { display: grid; grid-template-columns: minmax(0, 1.5fr) minmax(280px, .75fr); gap: 40px; padding-top: 28px; }
  .section-head { justify-content: space-between; gap: 16px; margin-bottom: 12px; }
  .section-head > a { color: var(--accent); font-size: 12px; }
  ol { list-style: none; padding: 0; margin: 0; border-top: 1px solid var(--border); }
  li { border-bottom: 1px solid var(--border); }
  .repo-row { gap: 16px; padding: 14px 4px; transition: background .16s ease, transform .16s ease; }
  .repo-row:hover { background: var(--surface); transform: translateX(3px); }
  .repo-copy { display: flex; flex-direction: column; flex: 1; min-width: 0; }
  .repo-copy span { color: var(--text-muted); font-size: 12px; }
  .gap { color: var(--warn); font-size: 12px; white-space: nowrap; }
  .gap.clear { color: var(--ok); }
  .evidence { min-width: 0; border-left: 1px solid var(--border); padding-left: 28px; }
  .evidence :global(.trend) { margin-top: 14px; }
  dl { margin: 16px 0 0; }
  dl div { justify-content: space-between; padding: 8px 0; border-bottom: 1px solid var(--border); }
  dt { color: var(--text-muted); }
  dd { margin: 0; }
  .notice { border: 1px solid var(--border); padding: 18px; }
  .notice.error { border-color: var(--risk); }
  .notice p { color: var(--text-muted); }
  button { background: var(--accent-weak); color: var(--text); border: 1px solid var(--border); border-radius: var(--radius-control); padding: 7px 12px; }
  .skeleton { height: 110px; background: linear-gradient(90deg,var(--surface),var(--surface-2),var(--surface)); }
  @keyframes reveal { from { opacity: 0; transform: translateY(6px); } to { opacity: 1; transform: translateY(0); } }
  @media (max-width: 980px) { .workspace { grid-template-columns: 1fr; } .evidence { border-left: 0; border-top: 1px solid var(--border); padding: 24px 0 0; } }
  @media (max-width: 720px) { .page-head, .decision { align-items: flex-start; flex-direction: column; } .metric-strip { grid-template-columns: repeat(2,minmax(0,1fr)); } .metric:nth-child(3) { border-left: 0; padding-left: 0; border-top: 1px solid var(--border); } .metric:nth-child(4) { border-top: 1px solid var(--border); } }
</style>
