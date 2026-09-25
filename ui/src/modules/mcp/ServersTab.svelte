<script lang="ts">
  // The governed MCP server registry: each row shows transport, a health pill,
  // tool count, injection-risk badge, and an enabled toggle, with Discover /
  // Health check / Delete actions. "Add server" opens the create form.
  import ResourceAccess from '../../lib/components/ResourceAccess.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { mcpCpApi } from '../../lib/api/mcp';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import type { McpServerDetail } from '../../lib/api/types';
  import McpPill from './McpPill.svelte';
  import RulesDrawer from './RulesDrawer.svelte';
  import ServerForm from './ServerForm.svelte';
  import ToolsTab from './ToolsTab.svelte';

  interface Props {
    wsId: string;
    servers: McpServerDetail[];
    loading: boolean;
    /** Last list-load failure (human text) — inline with Retry, never "no servers". */
    error?: string | null;
    selectedServerId: string | null;
    onReload: () => Promise<void> | void;
    onPatch: (s: McpServerDetail) => void;
    onSelect: (id: string) => void;
  }
  let { wsId, servers, loading, error = null, onReload, onPatch }: Props = $props();

  let accessId = $state<string | null>(null);
  const can = (id: string, op: string) => resourceAccess.can('mcp_server',id,op,'mcp','admin');
  const loadedAccessIds = new Set<string>();
  $effect(() => {
    for (const server of servers) {
      if (loadedAccessIds.has(server.id)) continue;
      loadedAccessIds.add(server.id);
      void resourceAccess.load('mcp_server', server.id);
    }
  });
  let formOpen = $state(false);
  let rulesOpen = $state(false);
  let expandedId = $state<string | null>(null);
  /** Per-server in-flight action so the right buttons spin without blocking others. */
  let busy = $state<Record<string, string>>({});

  function setBusy(id: string, what: string | null): void {
    if (what) busy = { ...busy, [id]: what };
    else {
      const n = { ...busy };
      delete n[id];
      busy = n;
    }
  }

  async function discover(s: McpServerDetail): Promise<void> {
    setBusy(s.id, 'discover');
    try {
      const tools = await mcpCpApi.cpDiscover(s.id);
      toasts.success('Discovered tools', `${tools.length} tool${tools.length === 1 ? '' : 's'} from ${s.name}`);
      await onReload();
    } catch (e) {
      toasts.error('Discovery failed', e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(s.id, null);
    }
  }

  async function health(s: McpServerDetail): Promise<void> {
    setBusy(s.id, 'health');
    try {
      const updated = await mcpCpApi.cpHealth(s.id);
      onPatch(updated);
      if (updated.health_status === 'healthy') {
        toasts.success('Healthy', `${s.name} · ${updated.health_latency_ms ?? '?'}ms`);
      } else {
        toasts.warn('Unhealthy', updated.health_error ?? s.name);
      }
    } catch (e) {
      toasts.error('Health check failed', e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(s.id, null);
    }
  }

  async function toggleEnabled(s: McpServerDetail): Promise<void> {
    setBusy(s.id, 'toggle');
    try {
      const updated = await mcpCpApi.cpUpdate(s.id, { enabled: !s.enabled });
      onPatch(updated);
    } catch (e) {
      toasts.error('Could not update server', e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(s.id, null);
    }
  }

  async function remove(s: McpServerDetail): Promise<void> {
    const ok = await confirmer.ask(`Delete MCP server "${s.name}"? Its discovered tools and allowlist entries are deleted too.`, {
      title: 'Delete server',
      confirmLabel: 'Delete',
      danger: true,
    });
    if (!ok) return;
    setBusy(s.id, 'delete');
    try {
      await mcpCpApi.cpDelete(s.id);
      toasts.success('Server deleted', s.name);
      await onReload();
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(s.id, null);
    }
  }

  /** Tooltip for a greyed-out row action — never a disabled button with no reason. */
  const NO_CONFIGURE = 'You need configure access to this server';
  const ROOT_ONLY = 'Only an Otto admin can add MCP servers';

  function rowMenu(e: MouseEvent, s: McpServerDetail): void {
    const configure = can(s.id, 'configure');
    const items: MenuItem[] = [
      { label: busy[s.id] === 'health' ? 'Checking health…' : 'Check health', icon: 'radar', disabled: !configure, action: () => void health(s) },
      { label: expandedId === s.id ? 'Hide tools' : 'View tools', icon: 'eye', action: () => toggleExpanded(s.id) },
    ];
    if (auth.isRoot || can(s.id, 'manage_access')) items.push({ label: 'Manage access…', icon: 'key', action: () => (accessId = s.id) });
    items.push({ separator: true }, { label: 'Delete…', icon: 'trash', danger: true, disabled: !configure, action: () => void remove(s) });
    ctxMenu.show(e, items);
  }

  function toggleExpanded(id: string): void {
    expandedId = expandedId === id ? null : id;
  }
</script>

<div class="servers">
  <div class="bar">
    <span class="count">{servers.length} server{servers.length === 1 ? '' : 's'}</span>
    <span class="grow"></span>
    <button class="btn small" data-testid="mcp-rules-btn" onclick={() => (rulesOpen = true)}>Rules</button>
    <button class="btn small" onclick={() => void onReload()} title="Refresh">
      <Icon name="refresh" size={13} /> Refresh
    </button>
    {#if servers.length > 0 || error || loading}
    <button class="btn primary small" data-testid="mcp-add-server" disabled={!auth.isRoot} title={auth.isRoot ? undefined : ROOT_ONLY} onclick={() => (formOpen = true)}>
      <Icon name="plus" size={13} /> Add server
    </button>
    {/if}
  </div>

  {#if error}
    <!-- Inline error (nothing loaded) or a stale bar over the last good list. -->
    <LoadState what="MCP servers" {loading} {error} empty={servers.length === 0} onretry={() => void onReload()} />
  {/if}
  {#if error && servers.length === 0}
    <!-- rendered above -->
  {:else if loading && servers.length === 0}
    <LoadState what="MCP servers" loading empty rows={3} />
  {:else if servers.length === 0}
    <EmptyState
      icon="server"
      title="No external servers yet"
      body="Register an external MCP server to govern its tools with policies, approvals and audit. Otto's own server lives on the Otto server tab."
      actionLabel={auth.isRoot ? 'Add server' : undefined}
      actionIcon="plus"
      onaction={auth.isRoot ? () => (formOpen = true) : undefined}
    />
  {:else}
    <div class="grid">
      <div class="thead">
        <span>Name</span>
        <span>Transport</span>
        <span>Health</span>
        <span class="num">Tools</span>
        <span>Injection</span>
        <span>Enabled</span>
        <span class="actions-h"><span class="sr-only">Actions</span></span>
      </div>
      {#each servers as s (s.id)}
        {@const endpoint = s.transport === 'stdio' ? `${s.command} ${s.args.join(' ')}`.trim() : (s.url ?? '')}
        <div class="srow">
          <button
            class="name"
            onclick={() => toggleExpanded(s.id)}
            title={expandedId === s.id ? 'Hide tools' : 'View tools'}
            aria-expanded={expandedId === s.id}
          >
            <span class="nm">{s.name}</span>
            {#if s.has_secret}<Icon name="key" size={12} />{/if}
            {#if s.description}<span class="desc" title={s.description}>{s.description}</span>{/if}
            <span class="endpoint mono" title={endpoint}>{endpoint}</span>
          </button>
          <span class="cell" data-label="Transport"><span class="transport">{s.transport}</span></span>
          <span class="cell" data-label="Health">
            <McpPill kind="health" value={s.health_status} small />
            {#if s.health_latency_ms != null && s.health_status === 'healthy'}<span class="lat">{s.health_latency_ms}ms</span>{/if}
          </span>
          <span class="cell num" data-label="Tools">{s.tools_count}</span>
          <span class="cell" data-label="Injection"><McpPill kind="injection" value={s.injection_risk} small /></span>
          <span class="cell" data-label="Enabled">
            <button
              class="switch"
              class:on={s.enabled}
              role="switch"
              aria-checked={s.enabled}
              disabled={busy[s.id] === 'toggle' || !can(s.id,'configure')}
              onclick={() => void toggleEnabled(s)}
              aria-label={`Enable ${s.name}`}
              title={!can(s.id, 'configure') ? NO_CONFIGURE : s.enabled ? 'Enabled — click to disable' : 'Disabled — click to enable'}
            >
              <span class="knob"></span>
            </button>
          </span>
          <span class="cell actions">
            <button class="btn small" disabled={!!busy[s.id] || !can(s.id,'configure')} title={can(s.id,'configure') ? 'Fetch the tool list from this server' : NO_CONFIGURE} onclick={() => void discover(s)}>
              {busy[s.id] === 'discover' ? 'Discovering…' : 'Discover'}
            </button>
            <button
              class="icon-btn"
              aria-label={`More actions for ${s.name}`}
              title="More actions"
              disabled={!!busy[s.id]}
              onclick={(e) => rowMenu(e, s)}
            >
              <Icon name="more" size={14} />
            </button>
          </span>
          {#if expandedId === s.id}
            <div class="row-body" data-testid="mcp-server-tools">
              <ToolsTab {wsId} {servers} selectedServerId={s.id} onSelect={() => {}} embedded ondiscovered={() => void onReload()} />
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

{#if accessId}<Modal title="MCP server access" width={820} onclose={() => accessId=null}><ResourceAccess kind="mcp_server" resourceId={accessId} /></Modal>{/if}

{#if formOpen}
  <ServerForm {wsId} onclose={() => (formOpen = false)} onsaved={() => void onReload()} />
{/if}

{#if rulesOpen}
  <RulesDrawer {wsId} {servers} onClose={() => (rulesOpen = false)} />
{/if}

<style>
  .servers {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
  }
  .count {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .grow {
    flex: 1;
  }
  .grid {
    flex: 1;
    overflow: auto;
    min-height: 0;
  }
  .thead,
  .srow {
    display: grid;
    grid-template-columns: minmax(220px, 1fr) 80px 120px 56px 96px 64px 124px;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
  }
  .thead {
    position: sticky;
    top: 0;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    z-index: 1;
  }
  .srow {
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .srow:hover {
    background: var(--hover);
  }
  .num {
    text-align: right;
  }
  .name {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    text-align: start;
    padding: 0;
    min-width: 0;
    width: 100%;
  }
  .name .nm {
    font-weight: 600;
    font-size: var(--fs-m);
  }
  .name:hover .nm {
    color: var(--accent-text);
  }
  .desc {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .endpoint {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    opacity: 0.8;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .cell {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .transport {
    font-size: var(--fs-xs);
    font-family: var(--font-mono);
    color: var(--text-dim);
  }
  .lat {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .actions {
    gap: 4px;
    justify-content: flex-end;
  }
  .actions-h {
    text-align: start;
  }
  .row-body {
    grid-column: 1 / -1;
    min-width: 0;
    margin: 4px -14px -8px;
    border-top: 1px solid var(--border);
    background: var(--bg);
  }
  .switch {
    width: 30px;
    height: 17px;
    border-radius: 9px;
    border: none;
    background: color-mix(in srgb, var(--text-dim) 30%, transparent);
    position: relative;
    cursor: pointer;
    padding: 0;
    flex: none;
  }
  .switch.on {
    background: var(--accent-solid);
  }
  .switch:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .switch .knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 13px;
    height: 13px;
    border-radius: 50%;
    background: var(--accent-contrast);
    transition: left 120ms ease;
  }
  .switch.on .knob {
    left: 15px;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }

  @media (max-width: 1024px) {
    .thead {
      display: none;
    }
    .srow {
      grid-template-columns: 1fr;
      gap: 4px;
      padding: 12px 14px;
    }
    .actions {
      justify-content: flex-start;
    }
    /* Stacked: each value carries its column name. */
    .cell[data-label]::before {
      content: attr(data-label);
      min-width: 84px;
      font-size: var(--fs-xs);
      color: var(--text-dim);
    }
    .cell.num {
      text-align: start;
    }
  }
</style>
