<script lang="ts">
  import { onMount, tick } from "svelte";
  import { buildCommands, filterCommands } from "../lib/commands";
  import { navigate } from "../lib/router";

  let { org, role }: { org: string; role: string } = $props();

  let open = $state(false);
  let query = $state("");
  let selected = $state(0);
  let input = $state<HTMLInputElement | null>(null);

  const commands = $derived(buildCommands(org, role));
  const results = $derived(filterCommands(commands, query));

  // Keep the highlight in range as the result set changes. Clamps both ends so a
  // negative index (e.g. ArrowDown on an empty result set) can't survive into a
  // later matching query and break Enter.
  $effect(() => {
    if (results.length === 0) selected = 0;
    else selected = Math.min(Math.max(selected, 0), results.length - 1);
  });

  async function show() {
    open = true;
    query = "";
    selected = 0;
    await tick();
    input?.focus();
  }

  function hide() {
    open = false;
  }

  function choose(i: number) {
    const cmd = results[i];
    if (!cmd) return;
    hide();
    navigate(cmd.path);
  }

  onMount(() => {
    const onkey = (e: KeyboardEvent) => {
      // Cmd/Ctrl-K toggles the palette from anywhere.
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        if (open) hide();
        else void show();
        return;
      }
      if (!open) return;
      if (e.key === "Escape") {
        e.preventDefault();
        hide();
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        selected = Math.min(selected + 1, results.length - 1);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        selected = Math.max(selected - 1, 0);
      } else if (e.key === "Enter") {
        e.preventDefault();
        choose(selected);
      }
    };
    window.addEventListener("keydown", onkey);
    return () => window.removeEventListener("keydown", onkey);
  });
</script>

{#if open}
  <!-- Backdrop: click to dismiss. Full keyboard operation (open/close/navigate)
       is handled on window in onMount, so the mouse handler is enhancement-only. -->
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="backdrop" onclick={hide} role="presentation">
    <div
      class="palette"
      role="dialog"
      aria-modal="true"
      aria-label="Command palette"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
    >
      <input
        bind:this={input}
        bind:value={query}
        type="text"
        placeholder="Jump to…"
        aria-label="Search commands"
        autocomplete="off"
        spellcheck="false"
      />
      {#if results.length === 0}
        <p class="none">No matches</p>
      {:else}
        <ul role="listbox" aria-label="Commands">
          {#each results as cmd, i (cmd.id)}
            <!-- Options are keyboard-operable via the focused input (arrows/enter);
                 mouse handlers are enhancement-only. -->
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <li
              role="option"
              aria-selected={i === selected}
              class:active={i === selected}
              onmousemove={() => (selected = i)}
              onclick={() => choose(i)}
            >
              <span class="label">{cmd.label}</span>
              {#if cmd.hint}<span class="hint">{cmd.hint}</span>{/if}
            </li>
          {/each}
        </ul>
      {/if}
      <div class="foot">
        <kbd>↑</kbd><kbd>↓</kbd> navigate · <kbd>↵</kbd> open · <kbd>esc</kbd> close
      </div>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding-top: 12vh;
    z-index: 100;
  }
  .palette {
    width: min(560px, 92vw);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.4);
    overflow: hidden;
  }
  input {
    width: 100%;
    border: none;
    border-bottom: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    font-size: 15px;
    padding: 14px 16px;
    outline: none;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 6px;
    max-height: 320px;
    overflow-y: auto;
  }
  li {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 9px 12px;
    border-radius: var(--radius-control);
    cursor: pointer;
    font-size: 13px;
  }
  li.active {
    background: var(--accent-weak);
  }
  .label {
    color: var(--text);
  }
  .hint {
    font-size: 11px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .none {
    color: var(--text-muted);
    padding: 18px 16px;
    margin: 0;
    font-size: 13px;
  }
  .foot {
    border-top: 1px solid var(--border);
    padding: 8px 14px;
    font-size: 11px;
    color: var(--text-muted);
  }
  kbd {
    font-family: var(--font-mono);
    font-size: 10px;
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0 4px;
    margin: 0 1px;
  }
</style>
