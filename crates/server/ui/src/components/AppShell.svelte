<script lang="ts">
  import type { Snippet } from "svelte";
  import { defaultPath, reposPath, sessionsPath } from "../lib/router";

  let {
    org,
    role,
    active = "",
    children,
  }: { org: string; role: string; active?: string; children: Snippet } = $props();

  // Items without a ready screen are shown disabled until their phase lands.
  const nav = $derived([
    { key: "overview", label: "Overview", href: defaultPath(org), ready: true },
    { key: "repos", label: "Repositories", href: reposPath(org), ready: true },
    { key: "sessions", label: "Sessions", href: sessionsPath(org), ready: true },
    { key: "policies", label: "Policies", href: "#", ready: false },
  ]);

  // Sub-screens map to their section for nav highlighting.
  const activeKey = $derived(
    active === "repo" || active === "file"
      ? "repos"
      : active === "session"
        ? "sessions"
        : active,
  );
</script>

<div class="shell">
  <nav class="rail" aria-label="Primary">
    <div class="brand">
      <span class="dot" aria-hidden="true"></span>
      <span class="name">Tellur</span>
    </div>
    <ul>
      {#each nav as item (item.key)}
        <li>
          <a
            href={item.href}
            class:active={item.key === activeKey}
            aria-disabled={!item.ready}
            class:soon={!item.ready}
          >
            {item.label}
            {#if !item.ready}<span class="tag">soon</span>{/if}
          </a>
        </li>
      {/each}
    </ul>
  </nav>

  <div class="frame">
    <header class="topbar">
      <div class="org" title="Organization">{org}</div>
      <div class="spacer"></div>
      <div class="role" title="Your role">{role}</div>
      <a class="signout" href="/auth/logout">Sign out</a>
    </header>
    <main class="content">
      {@render children()}
    </main>
  </div>
</div>

<style>
  .shell {
    display: grid;
    grid-template-columns: 240px 1fr;
    height: 100vh;
  }
  .rail {
    border-right: 1px solid var(--border);
    background: var(--surface);
    padding: 16px 12px;
    overflow-y: auto;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px 16px;
    font-weight: 600;
  }
  .brand .dot {
    width: 10px;
    height: 10px;
    border-radius: 999px;
    background: var(--accent);
  }
  .rail ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .rail a {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 10px;
    border-radius: var(--radius-control);
    color: var(--text-muted);
    font-size: 13px;
  }
  .rail a:hover {
    background: var(--surface-2);
    color: var(--text);
  }
  .rail a.active {
    background: var(--accent-weak);
    color: var(--text);
  }
  .rail a.soon {
    pointer-events: none;
    opacity: 0.5;
  }
  .tag {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 0 6px;
  }
  .frame {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .topbar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 20px;
    border-bottom: 1px solid var(--border);
  }
  .org {
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--text-muted);
  }
  .spacer {
    flex: 1;
  }
  .role {
    font-size: 12px;
    color: var(--text-muted);
    text-transform: capitalize;
  }
  .signout {
    font-size: 12px;
    color: var(--text-muted);
  }
  .signout:hover {
    color: var(--text);
  }
  .content {
    padding: 24px;
    overflow-y: auto;
  }
  @media (max-width: 768px) {
    .shell {
      grid-template-columns: 1fr;
    }
    .rail {
      display: none;
    }
  }
</style>
