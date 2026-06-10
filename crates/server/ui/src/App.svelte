<script lang="ts">
  import { onMount } from "svelte";
  import AppShell from "./components/AppShell.svelte";
  import Overview from "./screens/Overview.svelte";
  import Repos from "./screens/Repos.svelte";
  import RepoDetail from "./screens/RepoDetail.svelte";
  import FileView from "./screens/FileView.svelte";
  import Sessions from "./screens/Sessions.svelte";
  import SessionDetail from "./screens/SessionDetail.svelte";
  import Audit from "./screens/Audit.svelte";
  import Exports from "./screens/Exports.svelte";
  import { api, type Me } from "./lib/api";
  import { parseRoute, defaultPath, type Route } from "./lib/router";

  let me = $state<Me | null>(null);
  let route = $state<Route | null>(parseRoute(location.pathname));
  let error = $state<string | null>(null);
  let loading = $state(true);

  onMount(() => {
    const onpop = () => {
      route = parseRoute(location.pathname);
    };
    window.addEventListener("popstate", onpop);

    (async () => {
      try {
        me = await api.me();
        if (!route) {
          history.replaceState({}, "", defaultPath(me.org_id));
          route = parseRoute(location.pathname);
        }
      } catch (e) {
        error = e instanceof Error ? e.message : "failed to load";
      } finally {
        loading = false;
      }
    })();

    return () => window.removeEventListener("popstate", onpop);
  });
</script>

{#if loading}
  <div class="boot">Loading…</div>
{:else if error}
  <div class="boot err">{error}</div>
{:else if me}
  <AppShell org={route?.org ?? me.org_id} role={me.role} active={route?.name ?? ""}>
    {#if route && route.name === "overview"}
      <Overview org={route.org} />
    {:else if route && route.name === "repos"}
      <Repos org={route.org} />
    {:else if route && route.name === "repo"}
      <RepoDetail org={route.org} repo={route.repo} />
    {:else if route && route.name === "file"}
      <FileView org={route.org} repo={route.repo} path={route.path} />
    {:else if route && route.name === "sessions"}
      <Sessions org={route.org} />
    {:else if route && route.name === "session"}
      <SessionDetail org={route.org} id={route.id} />
    {:else if route && route.name === "audit"}
      <Audit org={route.org} />
    {:else if route && route.name === "exports"}
      <Exports org={route.org} />
    {:else}
      <div class="notfound">
        <h1>Not found</h1>
        <p><a href={defaultPath(me.org_id)}>Back to overview</a></p>
      </div>
    {/if}
  </AppShell>
{/if}

<style>
  .boot {
    display: grid;
    place-items: center;
    height: 100vh;
    color: var(--text-muted);
  }
  .boot.err {
    color: var(--risk);
  }
  .notfound {
    padding: 8px;
  }
</style>
