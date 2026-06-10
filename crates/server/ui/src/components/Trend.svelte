<script lang="ts">
  import type { ActivityBucket } from "../lib/api";
  import { dailyTotals, maxCount } from "../lib/series";

  let { buckets, label = "Activity" }: { buckets: ActivityBucket[]; label?: string } =
    $props();

  const totals = $derived(dailyTotals(buckets));
  const peak = $derived(maxCount(totals));
  // Simple bespoke bar chart; one accent, hover shows the day/count via title.
</script>

<section class="panel">
  <h2>{label}</h2>
  {#if totals.length === 0}
    <p class="muted">No activity in range.</p>
  {:else}
    <div class="chart" role="img" aria-label={`${label}: ${totals.length} days`}>
      {#each totals as t (t.day)}
        <div class="col" title={`${t.day}: ${t.count}`}>
          <div
            class="bar"
            style={`height:${peak > 0 ? Math.max(2, (t.count / peak) * 100) : 0}%`}
          ></div>
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .panel {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    padding: var(--card-pad);
    margin-bottom: 12px;
  }
  h2 {
    font-size: 13px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
    margin: 0 0 12px;
  }
  .chart {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    height: 96px;
  }
  .col {
    flex: 1;
    display: flex;
    align-items: flex-end;
    height: 100%;
  }
  .bar {
    width: 100%;
    background: var(--accent);
    border-radius: 2px 2px 0 0;
    min-height: 2px;
  }
  .muted {
    color: var(--text-muted);
  }
</style>
