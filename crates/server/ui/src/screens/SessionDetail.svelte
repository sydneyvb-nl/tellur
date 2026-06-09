<script lang="ts">
  import { api, type StoredEvent } from "../lib/api";
  import { relativeTime } from "../lib/format";
  import { sessionsPath } from "../lib/router";

  let { org, id }: { org: string; id: string } = $props();

  let events = $state<StoredEvent[]>([]);
  let truncated = $state(false);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let reloadKey = $state(0);

  $effect(() => {
    const o = org;
    const sid = id;
    void reloadKey;
    let cancelled = false;
    loading = true;
    error = null;
    api
      .session(o, sid)
      .then((res) => {
        if (!cancelled) {
          events = res.events;
          truncated = res.truncated ?? false;
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

  // Best-effort extraction of a file path from a payload for context.
  function fileOf(payload: unknown): string | null {
    if (payload && typeof payload === "object") {
      const p = payload as Record<string, unknown>;
      const v = p["file"] ?? p["file_path"] ?? p["path"];
      if (typeof v === "string") return v;
    }
    return null;
  }
</script>

<p class="crumb"><a href={sessionsPath(org)}>Sessions</a> / {id}</p>

{#if loading}
  <div class="panel skeleton"></div>
{:else if error}
  <div class="panel error">
    <p>{error}</p>
    <button onclick={() => (reloadKey += 1)}>Retry</button>
  </div>
{:else}
  <h1 class="mono">{id}</h1>
  <p class="muted">
    {events.length} events{#if truncated} · showing the first {events.length} (truncated){/if}
  </p>
  <ol class="timeline">
    {#each events as e (e.id)}
      <li>
        <span class="tick"></span>
        <div class="row">
          <span class="evt mono">{e.type}</span>
          <span class="muted">by {e.actor}</span>
          {#if fileOf(e.payload)}<span class="file mono">{fileOf(e.payload)}</span>{/if}
          <span class="spacer"></span>
          <span class="muted" title={e.timestamp}>{relativeTime(e.timestamp)}</span>
        </div>
      </li>
    {/each}
  </ol>
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
  }
  .muted {
    color: var(--text-muted);
  }
  .timeline {
    list-style: none;
    margin: 16px 0 0;
    padding: 0 0 0 16px;
    border-left: 2px solid var(--border);
  }
  .timeline li {
    position: relative;
    padding: 6px 0;
  }
  .tick {
    position: absolute;
    left: -21px;
    top: 12px;
    width: 8px;
    height: 8px;
    border-radius: 999px;
    background: var(--accent);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
  }
  .file {
    font-size: 12px;
    color: var(--text-muted);
  }
  .spacer {
    flex: 1;
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
</style>
