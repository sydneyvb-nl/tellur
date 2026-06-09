<script lang="ts">
  import { api, type Dashboard, type ActivityBucket } from "../lib/api";
  import { count, relativeTime } from "../lib/format";
  import Trend from "../components/Trend.svelte";

  let { org }: { org: string } = $props();

  let data = $state<Dashboard | null>(null);
  let activity = $state<ActivityBucket[]>([]);
  let error = $state<string | null>(null);
  let loading = $state(true);
  // Bumped to force a reload (e.g. the Retry button), since re-assigning the
  // same `org` would not re-trigger the effect.
  let reloadKey = $state(0);

  $effect(() => {
    // Reload whenever the org changes or a reload is requested.
    const currentOrg = org;
    void reloadKey;
    let cancelled = false;
    loading = true;
    error = null;
    Promise.all([api.dashboard(currentOrg), api.activity(currentOrg, 30, "type")])
      .then(([d, a]) => {
        if (!cancelled) {
          data = d;
          activity = a.buckets;
        }
      })
      .catch((e) => {
        if (!cancelled) error = e instanceof Error ? e.message : "failed to load";
      })
      .finally(() => {
        if (!cancelled) loading = false;
      });
    // If org/reloadKey changes before this resolves, ignore the stale result.
    return () => {
      cancelled = true;
    };
  });

  function topTypes(d: Dashboard): [string, number][] {
    return Object.entries(d.report.by_type)
      .sort((a, b) => b[1] - a[1])
      .slice(0, 5);
  }
</script>

<h1>Overview</h1>

{#if loading}
  <div class="kpis">
    {#each Array(3) as _unused}
      <div class="kpi skeleton"></div>
    {/each}
  </div>
{:else if error}
  <div class="panel error">
    <p>{error}</p>
    <button onclick={() => (reloadKey += 1)}>Retry</button>
  </div>
{:else if data}
  {#if data.report.total_events === 0}
    <div class="panel empty">
      <p>No activity yet for this org.</p>
      <p class="muted">
        Connect a repo and push provenance, e.g.
        <code>tellur notes push</code> or the hub ingest API.
      </p>
    </div>
  {:else}
    <section class="kpis" aria-label="Key metrics">
      <div class="kpi">
        <div class="num mono">{count(data.report.total_events)}</div>
        <div class="lbl">Events</div>
      </div>
      <div class="kpi">
        <div class="num mono">{count(data.report.distinct_sessions)}</div>
        <div class="lbl">Sessions</div>
      </div>
      <div class="kpi">
        <div class="num mono">{count(data.report.repos.length)}</div>
        <div class="lbl">Repositories</div>
      </div>
    </section>

    <Trend buckets={activity} label="Activity (30 days)" />

    <div class="cols">
      <section class="panel">
        <h2>Repositories</h2>
        {#if data.report.repos.length === 0}
          <p class="muted">No repositories.</p>
        {:else}
          <ul class="rows">
            {#each data.report.repos as r (r.id)}
              <li>
                <span class="repo">{r.name}</span>
                <span class="muted mono">{count(r.event_count)} events</span>
              </li>
            {/each}
          </ul>
        {/if}
      </section>

      <section class="panel">
        <h2>Recent activity</h2>
        {#if data.recent_events.length === 0}
          <p class="muted">No recent events.</p>
        {:else}
          <ul class="rows">
            {#each data.recent_events as e (e.id)}
              <li>
                <span class="evt mono">{e.type}</span>
                <span class="muted">by {e.actor}</span>
                <span class="spacer"></span>
                <span class="muted">{relativeTime(e.timestamp)}</span>
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    </div>

    {#if topTypes(data).length > 0}
      <section class="panel">
        <h2>Event types</h2>
        <ul class="rows">
          {#each topTypes(data) as [type, n] (type)}
            <li>
              <span class="evt mono">{type}</span>
              <span class="spacer"></span>
              <span class="muted mono">{count(n)}</span>
            </li>
          {/each}
        </ul>
      </section>
    {/if}
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
    grid-template-columns: repeat(3, 1fr);
    gap: 12px;
    margin-bottom: 16px;
  }
  .kpi {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    padding: 16px;
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
    padding: 16px;
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
  @media (max-width: 768px) {
    .kpis,
    .cols {
      grid-template-columns: 1fr;
    }
  }
</style>
