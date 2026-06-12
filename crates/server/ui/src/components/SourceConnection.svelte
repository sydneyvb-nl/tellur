<script lang="ts">
  import { api, type SourceConfig, type SourceUpdate } from "../lib/api";
  import { buildTemplates, type Provider } from "../lib/source";
  import { t } from "../lib/i18n.svelte";

  let { org, repo, role }: { org: string; repo: string; role: string } = $props();

  let config = $state<SourceConfig | null>(null);
  let loading = $state(true);
  let editing = $state(false);
  let saving = $state(false);
  let error = $state<string | null>(null);

  // Form state.
  let provider = $state<Provider>("github");
  let slug = $state("");
  let branch = $state("main");
  let isPrivate = $state(false);
  let token = $state("");
  let advanced = $state(false);
  let linkTmpl = $state("");
  let rawTmpl = $state("");

  let isAdmin = $derived(role === "admin");

  // Live preview from the guided fields (also what an empty Advanced box inherits).
  let generated = $derived(buildTemplates(provider, slug || "owner/repo", branch, isPrivate));

  $effect(() => {
    if (!isAdmin) {
      loading = false;
      return;
    }
    const o = org;
    const r = repo;
    let cancelled = false;
    loading = true;
    api
      .getSource(o, r)
      .then((c) => {
        if (!cancelled) config = c;
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

  function openConnect() {
    provider = "github";
    slug = "";
    branch = "main";
    isPrivate = false;
    token = "";
    advanced = false;
    linkTmpl = "";
    rawTmpl = "";
    error = null;
    editing = true;
  }

  function openEdit() {
    // Edit prefills Advanced with the stored templates (works for any provider).
    advanced = true;
    linkTmpl = config?.source_template ?? "";
    rawTmpl = config?.source_raw_template ?? "";
    isPrivate = config?.token_configured ?? false;
    token = "";
    error = null;
    editing = true;
  }

  function toggleAdvanced() {
    advanced = !advanced;
    if (advanced && !linkTmpl && !rawTmpl) {
      linkTmpl = generated.link;
      rawTmpl = generated.raw;
    }
  }

  async function save() {
    error = null;
    let link: string;
    let raw: string;
    if (advanced) {
      link = linkTmpl.trim();
      raw = rawTmpl.trim();
    } else {
      if (!slug.trim() || !slug.includes("/")) {
        error = t("source.slugRequired");
        return;
      }
      ({ link, raw } = generated);
    }
    const body: SourceUpdate = {
      template: link || null,
      raw_template: raw || null,
    };
    // A non-empty token sets it; otherwise the server preserves the existing one.
    if (token.trim()) body.token = token.trim();
    saving = true;
    try {
      config = await api.setSource(org, repo, body);
      editing = false;
    } catch (e) {
      error = e instanceof Error ? e.message : t("app.failed");
    } finally {
      saving = false;
    }
  }

  async function disconnect() {
    saving = true;
    error = null;
    try {
      config = await api.setSource(org, repo, {
        template: null,
        raw_template: null,
        clear_token: true,
      });
    } catch (e) {
      error = e instanceof Error ? e.message : t("app.failed");
    } finally {
      saving = false;
    }
  }

  let connected = $derived(!!(config && (config.source_template || config.source_raw_template)));
</script>

{#if isAdmin}
  <section class="panel">
    <h2>{t("source.title")}</h2>
    {#if loading}
      <p class="muted">…</p>
    {:else if editing}
      <div class="form">
        {#if !advanced}
          <label class="field">
            <span>{t("source.provider")}</span>
            <select bind:value={provider}>
              <option value="github">GitHub</option>
              <option value="gitlab">GitLab</option>
              <option value="bitbucket">Bitbucket</option>
            </select>
          </label>
          <label class="field">
            <span>{t("source.slug")}</span>
            <input bind:value={slug} placeholder="acme/myapp" autocomplete="off" />
          </label>
          <label class="field">
            <span>{t("source.branch")}</span>
            <input bind:value={branch} placeholder="main" autocomplete="off" />
          </label>
          <label class="check">
            <input type="checkbox" bind:checked={isPrivate} />
            <span>{t("source.privateToggle")}</span>
          </label>
          {#if isPrivate && provider !== "github"}
            <p class="hint warn">{t("source.privateGithubOnly")}</p>
          {/if}
          {#if isPrivate}
            <label class="field">
              <span>{t("source.token")}</span>
              <input
                type="password"
                bind:value={token}
                placeholder={config?.token_configured ? "••••••" : ""}
                autocomplete="off"
              />
            </label>
            <p class="hint">
              {t("source.tokenHelp")}
              {#if config?.token_configured}{t("source.tokenKeep")}{/if}
            </p>
          {/if}
          <div class="preview mono">
            <div><b>{t("source.linkTmpl")}:</b> {generated.link}</div>
            <div><b>{t("source.rawTmpl")}:</b> {generated.raw}</div>
          </div>
        {:else}
          <label class="field">
            <span>{t("source.linkTmpl")}</span>
            <input class="mono" bind:value={linkTmpl} placeholder="https://…/{'{path}'}#L{'{start}'}-L{'{end}'}" autocomplete="off" />
          </label>
          <label class="field">
            <span>{t("source.rawTmpl")}</span>
            <input class="mono" bind:value={rawTmpl} placeholder="https://…/{'{path}'}" autocomplete="off" />
          </label>
          <label class="check">
            <input type="checkbox" bind:checked={isPrivate} />
            <span>{t("source.privateToggle")}</span>
          </label>
          {#if isPrivate}
            <label class="field">
              <span>{t("source.token")}</span>
              <input
                type="password"
                bind:value={token}
                placeholder={config?.token_configured ? "••••••" : ""}
                autocomplete="off"
              />
            </label>
            <p class="hint">
              {t("source.tokenHelp")}
              {#if config?.token_configured}{t("source.tokenKeep")}{/if}
            </p>
          {/if}
        {/if}

        {#if error}<p class="hint err">{error}</p>{/if}

        <div class="actions">
          <button class="primary" onclick={save} disabled={saving}>{t("source.save")}</button>
          <button onclick={() => (editing = false)} disabled={saving}>{t("source.cancel")}</button>
          <button class="link" onclick={toggleAdvanced} type="button">{t("source.advanced")}</button>
        </div>
      </div>
    {:else if connected}
      <p class="muted">
        {config?.token_configured ? t("source.connectedPrivate") : t("source.connectedPublic")}
      </p>
      {#if config?.source_template}<p class="tmpl mono">{config.source_template}</p>{/if}
      <div class="actions">
        <button onclick={openEdit}>{t("source.edit")}</button>
        <button class="danger" onclick={disconnect} disabled={saving}>{t("source.disconnect")}</button>
      </div>
    {:else}
      <p class="muted">{t("source.none")}</p>
      <button class="primary" onclick={openConnect}>{t("source.connect")}</button>
    {/if}
  </section>
{/if}

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
  .muted {
    color: var(--text-muted);
    margin: 0 0 12px;
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-width: 560px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field > span {
    font-size: 12px;
    color: var(--text-muted);
  }
  input,
  select {
    background: var(--surface-2);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    padding: 7px 10px;
    font: inherit;
  }
  input.mono {
    font-family: var(--font-mono);
    font-size: 12px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
  }
  .check input {
    width: auto;
  }
  .hint {
    font-size: 12px;
    color: var(--text-muted);
    margin: 0;
  }
  .hint.warn {
    color: var(--warn);
  }
  .hint.err {
    color: var(--risk);
  }
  .preview {
    font-size: 11px;
    color: var(--text-muted);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    padding: 8px 10px;
    word-break: break-all;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .tmpl {
    font-size: 11px;
    color: var(--text-muted);
    word-break: break-all;
    margin: 0 0 12px;
  }
  .actions {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
  }
  button {
    background: var(--accent-weak);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    padding: 6px 12px;
    cursor: pointer;
    font: inherit;
  }
  button.primary {
    background: var(--accent);
    color: #06120d;
    border-color: transparent;
  }
  button.danger {
    color: var(--risk);
  }
  button.link {
    background: transparent;
    border-color: transparent;
    color: var(--accent);
    margin-left: auto;
  }
  button:disabled {
    opacity: 0.6;
    cursor: default;
  }
</style>
