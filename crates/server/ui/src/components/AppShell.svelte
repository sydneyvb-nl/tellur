<script lang="ts">
  import type { Snippet } from "svelte";
  import {
    auditPath,
    defaultPath,
    exportsPath,
    peoplePath,
    policiesPath,
    reposPath,
    sessionsPath,
  } from "../lib/router";
  import { applyPref, loadPref, nextPref, type ThemePref } from "../lib/theme";
  import {
    applyDensity,
    loadDensity,
    toggleDensity,
    type Density,
  } from "../lib/density";
  import { t, getLocale, setLocale } from "../lib/i18n.svelte";
  import { nextLocale } from "../lib/i18n";

  let {
    org,
    role,
    active = "",
    children,
  }: { org: string; role: string; active?: string; children: Snippet } = $props();

  let themePref = $state<ThemePref>(loadPref());
  const themeLabel = { system: "theme.system", light: "theme.light", dark: "theme.dark" };
  function cycleTheme() {
    themePref = nextPref(themePref);
    applyPref(themePref);
  }

  let density = $state<Density>(loadDensity());
  const densityLabel = { comfortable: "density.comfortable", compact: "density.compact" };
  function flipDensity() {
    density = toggleDensity(density);
    applyDensity(density);
  }

  function flipLocale() {
    setLocale(nextLocale(getLocale()));
  }

  // Items without a ready screen are shown disabled until their phase lands.
  // `admin` items are hidden entirely for non-admins (the API enforces this too).
  const nav = $derived(
    [
      { key: "overview", labelKey: "nav.overview", href: defaultPath(org), ready: true, admin: false },
      { key: "repos", labelKey: "nav.repos", href: reposPath(org), ready: true, admin: false },
      { key: "sessions", labelKey: "nav.sessions", href: sessionsPath(org), ready: true, admin: false },
      { key: "policies", labelKey: "nav.policies", href: policiesPath(org), ready: true, admin: true },
      { key: "people", labelKey: "nav.people", href: peoplePath(org), ready: true, admin: true },
      { key: "exports", labelKey: "nav.exports", href: exportsPath(org), ready: true, admin: true },
      { key: "audit", labelKey: "nav.audit", href: auditPath(org), ready: true, admin: true },
    ].filter((item) => !item.admin || role === "admin"),
  );

  // Sub-screens map to their section for nav highlighting.
  const activeKey = $derived(
    active === "repo" || active === "file"
      ? "repos"
      : active === "session"
        ? "sessions"
        : active,
  );
</script>

<a class="skip" href="#main">{t("shell.skip")}</a>
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
            aria-current={item.key === activeKey ? "page" : undefined}
            aria-disabled={!item.ready}
            class:soon={!item.ready}
          >
            {t(item.labelKey)}
            {#if !item.ready}<span class="tag">{t("nav.soon")}</span>{/if}
          </a>
        </li>
      {/each}
    </ul>
  </nav>

  <div class="frame">
    <header class="topbar">
      <div class="org" title={t("shell.org")}>{org}</div>
      <div class="spacer"></div>
      <button
        class="ghost kbd-hint"
        title="{t('palette.label')} (⌘K / Ctrl-K)"
        onclick={() =>
          window.dispatchEvent(
            new KeyboardEvent("keydown", { key: "k", metaKey: true }),
          )}
      >
        {t("shell.search")} <kbd>⌘K</kbd>
      </button>
      <button
        class="ghost"
        title={t("shell.langTitle")}
        aria-label={t("shell.langTitle")}
        onclick={flipLocale}
      >
        {getLocale().toUpperCase()}
      </button>
      <button
        class="ghost"
        title="{t('shell.densityTitle')}: {t(densityLabel[density])}"
        aria-label="{t('shell.densityTitle')}"
        onclick={flipDensity}
      >
        {t(densityLabel[density])}
      </button>
      <button
        class="ghost"
        title="{t('shell.themeTitle')}: {t(themeLabel[themePref])}"
        aria-label="{t('shell.themeTitle')}"
        onclick={cycleTheme}
      >
        {t(themeLabel[themePref])}
      </button>
      <div class="role" title={t("shell.role")}>{role}</div>
      <a class="signout" href="/auth/logout">{t("shell.signOut")}</a>
    </header>
    <main class="content" id="main">
      {@render children()}
    </main>
  </div>
</div>

<style>
  /* Visually-hidden until focused: keyboard users can jump past the nav. */
  .skip {
    position: absolute;
    left: 8px;
    top: -40px;
    z-index: 200;
    background: var(--accent);
    color: #0b0f13;
    padding: 8px 12px;
    border-radius: var(--radius-control);
    transition: top 0.15s ease;
  }
  .skip:focus {
    top: 8px;
  }
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
  .ghost {
    background: transparent;
    color: var(--text-muted);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    padding: 4px 10px;
    font-size: 12px;
    cursor: pointer;
  }
  .ghost:hover {
    color: var(--text);
    background: var(--surface-2);
  }
  .kbd-hint {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .kbd-hint kbd {
    font-family: var(--font-mono);
    font-size: 10px;
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0 4px;
  }
  /* Visible keyboard focus across all interactive elements (a11y). */
  :global(a:focus-visible),
  :global(button:focus-visible),
  :global(input:focus-visible),
  :global(select:focus-visible) {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
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
