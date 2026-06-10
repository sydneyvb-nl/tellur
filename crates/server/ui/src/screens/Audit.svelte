<script lang="ts">
  import { api, type AuditRecord } from "../lib/api";
  import { relativeTime } from "../lib/format";

  let { org }: { org: string } = $props();

  // Live input state (bound to the form). Editing these must NOT trigger a
  // fetch — only `applied` does, so the effect can't read them reactively.
  let actor = $state("");
  let action = $state("");
  let rangeDays = $state(30);

  // The snapshot the effect actually queries; updated only on Apply/retry.
  type Filters = { actor?: string; action?: string; rangeDays: number };
  let applied = $state<Filters>({ rangeDays: 30 });

  let records = $state<AuditRecord[]>([]);
  let chainIntact = $state<boolean | null>(null);
  let nextBefore = $state<number | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let loadingMore = $state(false);

  $effect(() => {
    const o = org;
    const opts = applied; // sole reactive dependency besides org
    let cancelled = false;
    loading = true;
    error = null;
    api
      .audit(o, opts)
      .then((page) => {
        if (cancelled) return;
        records = page.records;
        chainIntact = page.chain_intact;
        nextBefore = page.next_before;
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

  async function loadMore() {
    if (nextBefore == null || loadingMore) return;
    loadingMore = true;
    try {
      const page = await api.audit(org, { ...applied, before: nextBefore });
      records = [...records, ...page.records];
      nextBefore = page.next_before;
    } catch (e) {
      error = e instanceof Error ? e.message : "failed to load more";
    } finally {
      loadingMore = false;
    }
  }

  function apply(e?: Event) {
    e?.preventDefault();
    // New object identity → effect reruns once, here and only here.
    applied = {
      actor: actor.trim() || undefined,
      action: action.trim() || undefined,
      rangeDays,
    };
  }

  function shortHash(h: string): string {
    return h.length > 12 ? h.slice(0, 12) : h;
  }
</script>

<div class="head">
  <h1>Audit log</h1>
  {#if chainIntact === true}
    <span class="badge ok" title="The tamper-evident hash chain verifies">
      Chain verified
    </span>
  {:else if chainIntact === false}
    <span class="badge bad" title="The audit hash chain failed verification">
      Chain broken
    </span>
  {/if}
</div>

<form class="filters" onsubmit={apply}>
  <input placeholder="Actor (member id)" bind:value={actor} />
  <input placeholder="Action (e.g. policy.update)" bind:value={action} />
  <select bind:value={rangeDays}>
    <option value={7}>Last 7 days</option>
    <option value={30}>Last 30 days</option>
    <option value={90}>Last 90 days</option>
    <option value={365}>Last year</option>
  </select>
  <button type="submit">Apply</button>
</form>

{#if loading}
  <div class="panel skeleton"></div>
{:else if error}
  <div class="panel error">
    <p>{error}</p>
    <button onclick={() => apply()}>Retry</button>
  </div>
{:else if records.length === 0}
  <div class="panel empty"><p class="muted">No audit entries match.</p></div>
{:else}
  <table>
    <thead>
      <tr>
        <th>When</th><th>Actor</th><th>Action</th><th>Detail</th><th>Entry hash</th>
      </tr>
    </thead>
    <tbody>
      {#each records as r (r.seq)}
        <tr>
          <td class="muted" title={r.ts}>{relativeTime(r.ts)}</td>
          <td class="mono">{r.actor_member_id ?? "—"}</td>
          <td><span class="action">{r.action}</span></td>
          <td class="detail">{r.detail}</td>
          <td class="mono muted" title={r.entry_hash}>{shortHash(r.entry_hash)}</td>
        </tr>
      {/each}
    </tbody>
  </table>
  {#if nextBefore != null}
    <div class="more">
      <button onclick={loadMore} disabled={loadingMore}>
        {loadingMore ? "Loading…" : "Load more"}
      </button>
    </div>
  {/if}
{/if}

<style>
  .head {
    display: flex;
    align-items: center;
    gap: 12px;
    margin: 0 0 16px;
  }
  h1 {
    font-size: 20px;
    margin: 0;
  }
  .badge {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    border-radius: 999px;
    padding: 2px 10px;
    border: 1px solid var(--border);
  }
  .badge.ok {
    color: var(--accent);
    border-color: var(--accent);
  }
  .badge.bad {
    color: var(--risk);
    border-color: var(--risk);
  }
  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-bottom: 16px;
  }
  input,
  select {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    padding: 6px 10px;
    font-size: 13px;
  }
  input {
    min-width: 200px;
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
    vertical-align: top;
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
  .action {
    font-family: var(--font-mono);
    font-size: 12px;
  }
  .detail {
    color: var(--text-muted);
    word-break: break-word;
  }
  .muted {
    color: var(--text-muted);
  }
  .more {
    margin-top: 12px;
    text-align: center;
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
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
