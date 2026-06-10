<script lang="ts">
  import {
    api,
    type MemberInfo,
    type GroupInfo,
    type SsoStatus,
  } from "../lib/api";
  import { count, relativeTime } from "../lib/format";

  let { org }: { org: string } = $props();

  let members = $state<MemberInfo[]>([]);
  let groups = $state<GroupInfo[]>([]);
  let sso = $state<SsoStatus | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let reloadKey = $state(0);

  $effect(() => {
    const o = org;
    void reloadKey;
    let cancelled = false;
    loading = true;
    error = null;
    Promise.all([api.members(o), api.groups(o), api.ssoStatus(o)])
      .then(([m, g, s]) => {
        if (cancelled) return;
        // Inactive members last, then by name.
        members = [...m.members].sort(
          (a, b) =>
            Number(b.active) - Number(a.active) ||
            a.display_name.localeCompare(b.display_name),
        );
        groups = g.groups;
        sso = s;
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
</script>

<h1>People &amp; Access</h1>

{#if loading}
  <div class="panel skeleton"></div>
{:else if error}
  <div class="panel error">
    <p>{error}</p>
    <button onclick={() => (reloadKey += 1)}>Retry</button>
  </div>
{:else}
  {#if sso}
    <section class="status">
      <div class="stat">
        <div class="stat-h">Single sign-on</div>
        {#if sso.oidc_enabled}
          <div class="badge ok">Enabled</div>
          <div class="meta mono" title={sso.oidc_issuer ?? ""}>{sso.oidc_issuer}</div>
        {:else}
          <div class="badge off">Not configured</div>
          <div class="meta">OIDC is not set up on this hub.</div>
        {/if}
      </div>
      <div class="stat">
        <div class="stat-h">SCIM provisioning</div>
        {#if sso.scim_configured}
          <div class="badge ok">Active</div>
          <div class="meta">
            Token issued
            {#if sso.scim_token_created_at}{relativeTime(sso.scim_token_created_at)}{/if}
          </div>
        {:else}
          <div class="badge off">Not configured</div>
          <div class="meta">No SCIM token has been minted.</div>
        {/if}
      </div>
      <div class="stat">
        <div class="stat-h">Members</div>
        <div class="big mono">{count(sso.members_active)}<span class="of">/ {count(sso.members_total)}</span></div>
        <div class="meta">{count(sso.members_sso_bound)} bound to SSO</div>
      </div>
      <div class="stat">
        <div class="stat-h">Groups</div>
        <div class="big mono">{count(sso.scim_groups)}</div>
        <div class="meta">SCIM-managed</div>
      </div>
    </section>
  {/if}

  <section class="block">
    <h2>Members</h2>
    {#if members.length === 0}
      <p class="muted">No members yet.</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th>Member</th><th>Role</th><th>Email</th><th>SSO</th><th>Status</th>
          </tr>
        </thead>
        <tbody>
          {#each members as m (m.id)}
            <tr class:inactive={!m.active}>
              <td>{m.display_name}</td>
              <td><span class="role {m.role}">{m.role}</span></td>
              <td class="muted mono">{m.email ?? "—"}</td>
              <td>
                {#if m.sso_bound}<span class="dot ok" title="Bound to an SSO identity"></span> Bound
                {:else}<span class="muted">—</span>{/if}
              </td>
              <td>
                {#if m.active}<span class="status-pill active">Active</span>
                {:else}<span class="status-pill off">Deactivated</span>{/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>

  <section class="block">
    <h2>Groups</h2>
    {#if groups.length === 0}
      <p class="muted">No SCIM groups. Groups named <span class="mono">tellur-admin</span>,
        <span class="mono">tellur-contributor</span>, or <span class="mono">tellur-viewer</span>
        drive member roles automatically.</p>
    {:else}
      <table>
        <thead>
          <tr><th>Group</th><th>Maps to role</th><th class="num">Members</th></tr>
        </thead>
        <tbody>
          {#each groups as g (g.id)}
            <tr>
              <td class="mono">{g.display_name}</td>
              <td>
                {#if g.maps_to_role}<span class="role {g.maps_to_role}">{g.maps_to_role}</span>
                {:else}<span class="muted">informational</span>{/if}
              </td>
              <td class="num mono">{count(g.members.length)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>
{/if}

<style>
  h1 {
    font-size: 20px;
    margin: 0 0 16px;
  }
  h2 {
    font-size: 13px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
    margin: 0 0 12px;
  }
  .status {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 12px;
    margin-bottom: 24px;
  }
  .stat {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    padding: 14px 16px;
  }
  .stat-h {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
    margin-bottom: 8px;
  }
  .badge {
    display: inline-block;
    font-size: 11px;
    border-radius: 999px;
    padding: 2px 10px;
    border: 1px solid var(--border);
  }
  .badge.ok {
    color: var(--accent);
    border-color: var(--accent);
  }
  .badge.off {
    color: var(--text-muted);
  }
  .big {
    font-size: 26px;
    font-weight: 600;
  }
  .big .of {
    font-size: 15px;
    color: var(--text-muted);
    margin-left: 2px;
  }
  .meta {
    font-size: 12px;
    color: var(--text-muted);
    margin-top: 6px;
    word-break: break-all;
  }
  .block {
    margin-bottom: 24px;
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
  tr.inactive td {
    opacity: 0.55;
  }
  .num {
    text-align: right;
  }
  .muted {
    color: var(--text-muted);
  }
  .role {
    font-size: 11px;
    text-transform: capitalize;
    border-radius: 999px;
    padding: 1px 10px;
    border: 1px solid var(--border);
    color: var(--text-muted);
  }
  .role.admin {
    color: var(--accent);
    border-color: var(--accent);
  }
  .role.contributor {
    color: var(--text);
  }
  .dot {
    display: inline-block;
    width: 7px;
    height: 7px;
    border-radius: 999px;
    margin-right: 4px;
  }
  .dot.ok {
    background: var(--accent);
  }
  .status-pill {
    font-size: 11px;
    border-radius: 999px;
    padding: 1px 10px;
    border: 1px solid var(--border);
  }
  .status-pill.active {
    color: var(--ok);
    border-color: var(--ok);
  }
  .status-pill.off {
    color: var(--risk);
    border-color: var(--risk);
  }
  .panel {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    padding: var(--card-pad);
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
  @media (max-width: 1024px) {
    .status {
      grid-template-columns: repeat(2, 1fr);
    }
  }
  @media (max-width: 600px) {
    .status {
      grid-template-columns: 1fr;
    }
  }
</style>
