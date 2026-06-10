<script lang="ts">
  import { onDestroy } from "svelte";
  import { api, type ComplianceSnapshot } from "../lib/api";
  import { count, relativeTime } from "../lib/format";

  let { org }: { org: string } = $props();

  // Stop the re-eval poll loop if the screen unmounts mid-run.
  let destroyed = false;
  onDestroy(() => {
    destroyed = true;
  });

  let snapshots = $state<ComplianceSnapshot[]>([]);
  let evaluated = $state(false);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let reloadKey = $state(0);

  // Re-evaluation job state (enqueue → poll → reload).
  let running = $state(false);
  let runError = $state<string | null>(null);

  $effect(() => {
    const o = org;
    void reloadKey;
    let cancelled = false;
    loading = true;
    error = null;
    api
      .compliance(o)
      .then((page) => {
        if (cancelled) return;
        // Most at-risk first: highest violation count, then most recent.
        snapshots = [...page.snapshots].sort(
          (a, b) => b.violations - a.violations || b.evaluated_at.localeCompare(a.evaluated_at),
        );
        evaluated = page.evaluated;
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

  const totals = $derived(
    snapshots.reduce(
      (acc, s) => ({
        violations: acc.violations + s.violations,
        high: acc.high + s.high,
        medium: acc.medium + s.medium,
        low: acc.low + s.low,
      }),
      { violations: 0, high: 0, medium: 0, low: 0 },
    ),
  );

  const lastEvaluated = $derived(
    snapshots.reduce<string | null>(
      (latest, s) => (latest && latest > s.evaluated_at ? latest : s.evaluated_at),
      null,
    ),
  );

  async function reevaluate() {
    if (running) return;
    running = true;
    runError = null;
    try {
      const { job_id } = await api.runCompliance(org);
      // Durable/background work: poll until a terminal state, never treating a
      // still-running job as success (it can legitimately outlive any fixed cap).
      // eslint-disable-next-line no-constant-condition
      while (true) {
        await new Promise((r) => setTimeout(r, 1500));
        if (destroyed) return;
        const job = await api.job(org, job_id);
        if (job.status === "completed") {
          reloadKey += 1;
          break;
        }
        if (job.status === "failed") {
          throw new Error(job.error ?? "evaluation failed");
        }
        // queued/running → keep polling.
      }
    } catch (e) {
      runError = e instanceof Error ? e.message : "evaluation failed";
    } finally {
      running = false;
    }
  }
</script>

<div class="head">
  <div>
    <h1>Policy compliance</h1>
    <p class="sub">
      Your <span class="mono">default</span> policy evaluated against every repo's
      recorded attribution.
      {#if lastEvaluated}
        Last run {relativeTime(lastEvaluated)}.
      {/if}
    </p>
  </div>
  <button class="primary" onclick={reevaluate} disabled={running}>
    {running ? "Evaluating…" : "Re-evaluate"}
  </button>
</div>

{#if runError}
  <div class="panel error"><p>{runError}</p></div>
{/if}

{#if loading}
  <div class="panel skeleton"></div>
{:else if error}
  <div class="panel error">
    <p>{error}</p>
    <button onclick={() => (reloadKey += 1)}>Retry</button>
  </div>
{:else if !evaluated}
  <div class="panel empty">
    <h2>No evaluation yet</h2>
    <p class="muted">
      Upload a policy named <span class="mono">default</span> (via
      <span class="mono">PUT /v1/orgs/{org}/policies/default</span> or the admin CLI),
      then run an evaluation to see per-repo compliance here.
    </p>
    <button class="primary" onclick={reevaluate} disabled={running}>
      {running ? "Evaluating…" : "Run evaluation"}
    </button>
  </div>
{:else}
  <section class="kpis">
    <div class="kpi">
      <div class="num">{count(snapshots.length)}</div>
      <div class="lbl">Repos evaluated</div>
    </div>
    <div class="kpi" class:clean={totals.violations === 0}>
      <div class="num">{count(totals.violations)}</div>
      <div class="lbl">Open violations</div>
    </div>
    <div class="kpi">
      <div class="sev-row">
        <span class="sev high">{count(totals.high)} high</span>
        <span class="sev medium">{count(totals.medium)} med</span>
        <span class="sev low">{count(totals.low)} low</span>
      </div>
      <div class="lbl">By severity</div>
    </div>
  </section>

  <table>
    <thead>
      <tr>
        <th>Repository</th>
        <th class="num">AI ranges</th>
        <th class="num">Violations</th>
        <th>Severity</th>
        <th class="num">Policy</th>
        <th>Evaluated</th>
      </tr>
    </thead>
    <tbody>
      {#each snapshots as s (s.repo_id)}
        <tr class:flagged={s.violations > 0}>
          <td>{s.repo_name}</td>
          <td class="num mono">{count(s.ai_ranges)}</td>
          <td class="num mono">
            {#if s.violations === 0}
              <span class="ok-dot" title="Compliant">✓</span>
            {:else}
              {count(s.violations)}
            {/if}
          </td>
          <td>
            {#if s.violations === 0}
              <span class="muted">—</span>
            {:else}
              <span class="chips">
                {#if s.high > 0}<span class="sev high">{s.high}</span>{/if}
                {#if s.medium > 0}<span class="sev medium">{s.medium}</span>{/if}
                {#if s.low > 0}<span class="sev low">{s.low}</span>{/if}
              </span>
            {/if}
          </td>
          <td class="num mono muted">v{s.policy_version}</td>
          <td class="muted" title={s.evaluated_at}>{relativeTime(s.evaluated_at)}</td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}

<style>
  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 20px;
  }
  h1 {
    font-size: 20px;
    margin: 0 0 4px;
  }
  .sub {
    color: var(--text-muted);
    font-size: 13px;
    margin: 0;
    max-width: 64ch;
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
  .kpi.clean {
    border-color: var(--ok);
  }
  .kpi .num {
    font-size: 28px;
    font-weight: 600;
    font-family: var(--font-mono);
  }
  .kpi .lbl {
    color: var(--text-muted);
    font-size: 12px;
    margin-top: 4px;
  }
  .sev-row {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    padding-top: 4px;
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
    padding: 10px 14px;
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
  tr.flagged td:first-child {
    box-shadow: inset 2px 0 0 var(--risk);
  }
  .num {
    text-align: right;
  }
  .muted {
    color: var(--text-muted);
  }
  .ok-dot {
    color: var(--ok);
  }
  .chips {
    display: inline-flex;
    gap: 6px;
  }
  .sev {
    font-size: 11px;
    font-family: var(--font-mono);
    border-radius: 999px;
    padding: 1px 8px;
    border: 1px solid transparent;
  }
  .sev.high {
    color: var(--risk);
    border-color: var(--risk);
  }
  .sev.medium {
    color: var(--warn);
    border-color: var(--warn);
  }
  .sev.low {
    color: var(--text-muted);
    border-color: var(--border);
  }
  .panel {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    padding: 16px;
  }
  .panel.empty {
    padding: 32px;
    text-align: center;
  }
  .panel.empty h2 {
    font-size: 15px;
    margin: 0 0 8px;
  }
  .panel.empty p {
    max-width: 56ch;
    margin: 0 auto 16px;
  }
  .panel.error {
    border-color: var(--risk);
    margin-bottom: 16px;
  }
  .skeleton {
    height: 120px;
  }
  button {
    background: var(--accent-weak);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    padding: 7px 14px;
    cursor: pointer;
    font-size: 13px;
    white-space: nowrap;
  }
  button.primary {
    background: var(--accent);
    color: #0b0f13;
    border-color: var(--accent);
    font-weight: 600;
  }
  button:disabled {
    opacity: 0.6;
    cursor: default;
  }
  @media (max-width: 768px) {
    .kpis {
      grid-template-columns: 1fr;
    }
  }
</style>
