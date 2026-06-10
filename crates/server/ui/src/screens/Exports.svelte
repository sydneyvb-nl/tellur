<script lang="ts">
  import { api, type Job, type ExportKind } from "../lib/api";
  import { relativeTime } from "../lib/format";

  let { org }: { org: string } = $props();

  let jobs = $state<Job[]>([]);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let loading = $state(true);
  let starting = $state(false);
  let downloading = $state<string | null>(null);
  // Bumped to (re)start the polling loop — e.g. after queueing a new export,
  // even when the page previously had no active jobs and polling had stopped.
  let pollKey = $state(0);

  // Poll while any job is still in flight so status updates live.
  const POLL_MS = 3000;

  async function refresh() {
    try {
      const res = await api.jobs(org);
      jobs = res.jobs;
      error = null;
    } catch (e) {
      error = e instanceof Error ? e.message : "failed to load jobs";
    } finally {
      loading = false;
    }
  }

  function active(list: Job[]): boolean {
    return list.some((j) => j.status === "queued" || j.status === "running");
  }

  $effect(() => {
    const o = org;
    void o;
    void pollKey; // restart the loop on demand
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      if (cancelled) return;
      await refresh();
      if (!cancelled && active(jobs)) timer = setTimeout(tick, POLL_MS);
    };
    loading = true;
    void tick();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  });

  async function start(kind: ExportKind) {
    starting = true;
    notice = null;
    try {
      await api.startExport(org, kind);
      notice = `Queued ${kind} export.`;
      // Restart the polling loop so the newly queued job is tracked to completion
      // even if the page had no active jobs before.
      pollKey += 1;
    } catch (e) {
      error = e instanceof Error ? e.message : "failed to start export";
    } finally {
      starting = false;
    }
  }

  // Fetch a completed job's full result and save it as a JSON file.
  async function download(job: Job) {
    downloading = job.id;
    try {
      const full = await api.job(org, job.id);
      // `result` is inlined as parsed JSON once completed.
      const result = (full as unknown as { result: unknown }).result;
      const blob = new Blob([JSON.stringify(result, null, 2)], {
        type: "application/json",
      });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `${job.kind.replace(/\./g, "-")}-${job.id}.json`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (e) {
      error = e instanceof Error ? e.message : "failed to download";
    } finally {
      downloading = null;
    }
  }

  function statusClass(s: string): string {
    if (s === "completed") return "ok";
    if (s === "failed") return "bad";
    return "pending";
  }
</script>

<h1>Exports</h1>
<p class="muted">
  Generate a portable snapshot of your org's activity log, tamper-evident audit
  trail, or a full compliance <strong>evidence pack</strong> (every repo's SLSA
  provenance + latest policy compliance + audit-chain status). Exports run as
  background jobs; large orgs may take a moment.
</p>

<div class="actions">
  <button class="primary" onclick={() => start("evidence")} disabled={starting}>
    Evidence pack
  </button>
  <button onclick={() => start("events")} disabled={starting}>Export events</button>
  <button onclick={() => start("audit")} disabled={starting}>Export audit log</button>
  {#if notice}<span class="notice">{notice}</span>{/if}
</div>

{#if error}
  <div class="panel error"><p>{error}</p></div>
{/if}

{#if loading}
  <div class="panel skeleton"></div>
{:else if jobs.length === 0}
  <div class="panel empty"><p class="muted">No exports yet.</p></div>
{:else}
  <table>
    <thead>
      <tr>
        <th>Kind</th><th>Status</th><th>Created</th><th>Updated</th><th></th>
      </tr>
    </thead>
    <tbody>
      {#each jobs as j (j.id)}
        <tr>
          <td class="mono">{j.kind}</td>
          <td><span class="status {statusClass(j.status)}">{j.status}</span></td>
          <td class="muted" title={j.created_at}>{relativeTime(j.created_at)}</td>
          <td class="muted" title={j.updated_at}>{relativeTime(j.updated_at)}</td>
          <td class="right">
            {#if j.status === "completed"}
              <button class="link" onclick={() => download(j)} disabled={downloading === j.id}>
                {downloading === j.id ? "Preparing…" : "Download"}
              </button>
            {:else if j.status === "failed"}
              <span class="muted" title={j.error ?? ""}>error</span>
            {/if}
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}

<style>
  h1 {
    font-size: 20px;
    margin: 0 0 4px;
  }
  .muted {
    color: var(--text-muted);
  }
  p.muted {
    max-width: 60ch;
    margin: 0 0 16px;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 16px;
  }
  .notice {
    font-size: 12px;
    color: var(--accent);
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
  .right {
    text-align: right;
  }
  .status {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    border-radius: 999px;
    padding: 2px 10px;
    border: 1px solid var(--border);
  }
  .status.ok {
    color: var(--accent);
    border-color: var(--accent);
  }
  .status.bad {
    color: var(--risk);
    border-color: var(--risk);
  }
  .status.pending {
    color: var(--text-muted);
  }
  .panel {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    padding: var(--card-pad);
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
    padding: 6px 12px;
    cursor: pointer;
  }
  button.primary {
    background: var(--accent);
    color: #0b0f13;
    border-color: var(--accent);
    font-weight: 600;
  }
  button.link {
    background: none;
    border: none;
    color: var(--accent);
    padding: 0;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
