<script lang="ts">
  import { api, type StoredEvent } from "../lib/api";
  import { relativeTime } from "../lib/format";
  import { sessionsPath } from "../lib/router";
  import { t } from "../lib/i18n.svelte";
  import {
    eventCategory,
    eventDetail,
    sessionStats,
    formatDuration,
    type Category,
  } from "../lib/timeline";

  let { org, id }: { org: string; id: string } = $props();

  let events = $state<StoredEvent[]>([]);
  let truncated = $state(false);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let reloadKey = $state(0);

  // Filters.
  let mutedCats = $state<Set<Category>>(new Set());
  let actorFilter = $state<string>("");
  let query = $state<string>("");

  $effect(() => {
    const o = org;
    const sid = id;
    void reloadKey;
    let cancelled = false;
    loading = true;
    error = null;
    mutedCats = new Set();
    actorFilter = "";
    query = "";
    api
      .session(o, sid)
      .then((res) => {
        if (!cancelled) {
          events = res.events;
          truncated = res.truncated ?? false;
        }
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

  let stats = $derived(sessionStats(events));

  // Category metadata: a glyph + a CSS accent class per category.
  const META: Record<Category, { glyph: string; cls: string }> = {
    prompt: { glyph: "❝", cls: "c-prompt" },
    file: { glyph: "✎", cls: "c-file" },
    command: { glyph: "›_", cls: "c-command" },
    tool: { glyph: "⚙", cls: "c-tool" },
    test: { glyph: "✓", cls: "c-test" },
    git: { glyph: "⎇", cls: "c-git" },
    session: { glyph: "◷", cls: "c-session" },
    policy: { glyph: "⚑", cls: "c-policy" },
    other: { glyph: "•", cls: "c-other" },
  };

  function catLabel(cat: Category): string {
    return t(`timeline.cat.${cat}`);
  }

  // "file.write" → "File write".
  function humanize(type: string): string {
    const s = type.replace(/[._]/g, " ");
    return s.charAt(0).toUpperCase() + s.slice(1);
  }

  function prettyPayload(payload: unknown): string {
    try {
      return JSON.stringify(payload, null, 2);
    } catch {
      return String(payload);
    }
  }

  function toggleCat(cat: Category) {
    const next = new Set(mutedCats);
    if (next.has(cat)) next.delete(cat);
    else next.add(cat);
    mutedCats = next;
  }

  let filtered = $derived(
    events.filter((e) => {
      if (mutedCats.has(eventCategory(e.type))) return false;
      if (actorFilter && e.actor !== actorFilter) return false;
      if (query.trim()) {
        const q = query.trim().toLowerCase();
        const d = eventDetail(e);
        const hay = [e.type, e.actor, d.file, d.command, d.tool, d.prompt]
          .filter(Boolean)
          .join(" ")
          .toLowerCase();
        if (!hay.includes(q)) return false;
      }
      return true;
    }),
  );
</script>

<p class="crumb"><a href={sessionsPath(org)}>{t("nav.sessions")}</a> / {id}</p>

{#if loading}
  <div class="panel skeleton"></div>
{:else if error}
  <div class="panel error">
    <p>{error}</p>
    <button onclick={() => (reloadKey += 1)}>{t("common.retry")}</button>
  </div>
{:else}
  <!-- Summary header -->
  <section class="summary">
    <h1 class="mono" title={id}>{id}</h1>
    <div class="kpis">
      <span class="kpi"><b>{stats.count}</b> {t("timeline.events")}</span>
      {#if stats.durationMs > 0}
        <span class="kpi"><b>{formatDuration(stats.durationMs)}</b> {t("timeline.duration")}</span>
      {/if}
      <span class="kpi"><b>{stats.files}</b> {t("timeline.files")}</span>
      {#if stats.prompts > 0}
        <span class="kpi"><b>{stats.prompts}</b> {t("timeline.prompts")}</span>
      {/if}
    </div>
    <div class="actors">
      {#each stats.actors as a (a)}
        <button
          class="chip actor"
          class:active={actorFilter === a}
          onclick={() => (actorFilter = actorFilter === a ? "" : a)}
        >{a}</button>
      {/each}
    </div>
    {#if truncated}<p class="warn">{t("sessionDetail.truncated", { n: events.length })}</p>{/if}
  </section>

  <!-- Filter bar -->
  <div class="filters">
    <div class="cats">
      {#each stats.categories as cat (cat)}
        <button
          class="chip cat {META[cat].cls}"
          class:off={mutedCats.has(cat)}
          onclick={() => toggleCat(cat)}
          title={catLabel(cat)}
        >
          <span class="g">{META[cat].glyph}</span>{catLabel(cat)}
        </button>
      {/each}
    </div>
    <input class="search" placeholder={t("timeline.search")} bind:value={query} />
  </div>

  <!-- Timeline -->
  {#if filtered.length === 0}
    <div class="panel empty"><p class="muted">{t("timeline.noMatch")}</p></div>
  {:else}
    <ol class="timeline">
      {#each filtered as e (e.id)}
        {@const cat = eventCategory(e.type)}
        {@const d = eventDetail(e)}
        <li class={META[cat].cls}>
          <span class="node" aria-hidden="true">{META[cat].glyph}</span>
          <div class="card">
            <div class="head">
              <span class="title">{humanize(e.type)}</span>
              <span class="chip mini">{e.actor}</span>
              <span class="spacer"></span>
              <time class="when" title={e.timestamp}>{relativeTime(e.timestamp)}</time>
            </div>

            {#if d.prompt}
              <blockquote class="prompt">{d.prompt}</blockquote>
            {:else if d.promptHashed}
              <p class="hashed">{t("timeline.promptHashed")}</p>
            {/if}

            {#if d.file}<div class="ctx mono">{d.file}</div>{/if}
            {#if d.command}
              <div class="ctx cmd mono">
                <span>{d.command}</span>
                {#if d.exitCode !== null}
                  <span class="exit" class:bad={d.exitCode !== 0}>exit {d.exitCode}</span>
                {/if}
              </div>
            {/if}
            {#if d.tool && !d.command}<div class="ctx mono">{d.tool}</div>{/if}

            <details class="raw">
              <summary>{t("timeline.raw")}</summary>
              <pre class="mono">{prettyPayload(e.payload)}</pre>
            </details>
          </div>
        </li>
      {/each}
    </ol>
  {/if}
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
  .summary {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    padding: var(--card-pad);
    margin-bottom: 12px;
  }
  h1 {
    font-size: 15px;
    margin: 0 0 10px;
    word-break: break-all;
  }
  .kpis {
    display: flex;
    flex-wrap: wrap;
    gap: 16px;
    font-size: 13px;
    color: var(--text-muted);
  }
  .kpi b {
    color: var(--text);
    font-size: 16px;
    margin-right: 2px;
  }
  .actors {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 12px;
  }
  .warn {
    color: var(--warn);
    font-size: 12px;
    margin: 10px 0 0;
  }
  .chip {
    font: inherit;
    font-size: 12px;
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 2px 10px;
    background: var(--surface-2);
    color: var(--text-muted);
    cursor: pointer;
  }
  .chip.actor.active {
    background: var(--accent);
    color: #06120d;
    border-color: transparent;
  }
  .chip.mini {
    cursor: default;
    padding: 1px 8px;
    font-size: 11px;
  }
  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 14px;
  }
  .cats {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip.cat .g {
    margin-right: 5px;
    opacity: 0.9;
  }
  .chip.cat.off {
    opacity: 0.4;
    text-decoration: line-through;
  }
  .search {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    padding: 6px 10px;
    font: inherit;
    color: var(--text);
    min-width: 180px;
    flex: 1;
    max-width: 280px;
  }

  .timeline {
    list-style: none;
    margin: 0;
    padding: 0 0 0 28px;
    position: relative;
  }
  .timeline::before {
    content: "";
    position: absolute;
    left: 10px;
    top: 6px;
    bottom: 6px;
    width: 2px;
    background: var(--border);
  }
  .timeline li {
    position: relative;
    padding: 0 0 12px;
  }
  .node {
    position: absolute;
    left: -28px;
    top: 2px;
    width: 22px;
    height: 22px;
    border-radius: 999px;
    display: grid;
    place-items: center;
    font-size: 12px;
    background: var(--surface);
    border: 2px solid var(--border);
    color: var(--text-muted);
    z-index: 1;
  }
  .card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    border-left: 3px solid var(--border);
    padding: 8px 12px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
  }
  .title {
    font-weight: 600;
  }
  .spacer {
    flex: 1;
  }
  .when {
    color: var(--text-muted);
    font-size: 12px;
    white-space: nowrap;
  }
  .prompt {
    margin: 8px 0 4px;
    padding: 8px 12px;
    border-left: 3px solid var(--accent);
    background: var(--accent-weak);
    border-radius: 0 var(--radius-control) var(--radius-control) 0;
    font-size: 13px;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .hashed {
    margin: 6px 0 0;
    font-size: 12px;
    color: var(--text-muted);
    font-style: italic;
  }
  .ctx {
    margin-top: 6px;
    font-size: 12px;
    color: var(--text-muted);
    word-break: break-all;
  }
  .ctx.cmd {
    display: flex;
    align-items: center;
    gap: 8px;
    background: var(--surface-2);
    border-radius: var(--radius-control);
    padding: 4px 8px;
  }
  .exit {
    color: var(--ok);
    font-size: 11px;
    margin-left: auto;
  }
  .exit.bad {
    color: var(--risk);
  }
  .raw {
    margin-top: 8px;
  }
  .raw summary {
    cursor: pointer;
    font-size: 12px;
    color: var(--text-muted);
  }
  .raw pre {
    margin: 8px 0 0;
    padding: 10px;
    background: var(--surface-2);
    border-radius: var(--radius-control);
    overflow-x: auto;
    font-size: 12px;
    line-height: 1.5;
  }

  /* Category accents — node ring + card edge. */
  li.c-prompt .node { border-color: var(--accent); color: var(--accent); }
  li.c-prompt .card { border-left-color: var(--accent); }
  li.c-file .node { border-color: var(--ai); color: var(--ai); }
  li.c-file .card { border-left-color: var(--ai); }
  li.c-command .node,
  li.c-tool .node { border-color: var(--warn); color: var(--warn); }
  li.c-command .card,
  li.c-tool .card { border-left-color: var(--warn); }
  li.c-test .node { border-color: var(--ok); color: var(--ok); }
  li.c-test .card { border-left-color: var(--ok); }
  li.c-git .node { border-color: var(--human); color: var(--human); }
  li.c-git .card { border-left-color: var(--human); }
  li.c-policy .node { border-color: var(--risk); color: var(--risk); }
  li.c-policy .card { border-left-color: var(--risk); }
  .chip.cat.c-prompt { color: var(--accent); }
  .chip.cat.c-file { color: var(--ai); }
  .chip.cat.c-command,
  .chip.cat.c-tool { color: var(--warn); }
  .chip.cat.c-test { color: var(--ok); }
  .chip.cat.c-git { color: var(--human); }
  .chip.cat.c-policy { color: var(--risk); }

  .panel {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    padding: var(--card-pad);
  }
  .panel.error {
    border-color: var(--risk);
  }
  .muted {
    color: var(--text-muted);
  }
  .skeleton {
    height: 140px;
  }
  button.chip {
    line-height: 1.6;
  }
  button:not(.chip) {
    background: var(--accent-weak);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    padding: 6px 12px;
    cursor: pointer;
  }
</style>
