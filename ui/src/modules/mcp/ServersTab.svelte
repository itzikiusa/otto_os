<script lang="ts">
  // The governed MCP server registry: each row shows transport, a health pill,
  // tool count, injection-risk badge, and an enabled toggle, with Discover /
  // Health check / Delete actions. "Add server" opens the create form.
  import Icon from '../../lib/components/Icon.svelte';
  import { mcpCpApi } from '../../lib/api/mcp';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { McpServerDetail } from '../../lib/api/types';
  import McpPill from './McpPill.svelte';
  import ServerForm from './ServerForm.svelte';

  interface Props {
    wsId: string;
    servers: McpServerDetail[];
    loading: boolean;
    selectedServerId: string | null;
    onReload: () => Promise<void> | void;
    onPatch: (s: McpServerDetail) => void;
    onSelect: (id: string) => void;
  }
  let { wsId, servers, loading, onReload, onPatch, onSelect }: Props = $props();

  let formOpen = $state(false);
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
    const ok = await confirmer.ask(`Remove MCP server "${s.name}"? Its discovered tools and allowlist entries go too.`, {
      title: 'Remove server',
      confirmLabel: 'Remove',
      danger: true,
    });
    if (!ok) return;
    setBusy(s.id, 'delete');
    try {
      await mcpCpApi.cpDelete(s.id);
      toasts.success('Server removed', s.name);
      await onReload();
    } catch (e) {
      toasts.error('Remove failed', e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(s.id, null);
    }
  }
</script>

<div class="servers">
  <div class="bar">
    <span class="count">{servers.length} server{servers.length === 1 ? '' : 's'}</span>
    <span class="grow"></span>
    <button class="btn small" onclick={() => void onReload()} title="Refresh">
      <Icon name="refresh" size={13} />
    </button>
    <button class="btn primary small" onclick={() => (formOpen = true)}>
      <Icon name="plus" size={13} /> Add server
    </button>
  </div>

  {#if loading && servers.length === 0}
    <p class="muted pad">Loading…</p>
  {:else if servers.length === 0}
    <div class="empty">
      <Icon name="plug" size={26} />
      <p>No MCP servers registered in this workspace yet.</p>
      <button class="btn primary" onclick={() => (formOpen = true)}>Add a server</button>
    </div>
  {:else}
    <div class="grid">
      <div class="thead">
        <span>Name</span>
        <span>Transport</span>
        <span>Health</span>
        <span class="num">Tools</span>
        <span>Injection</span>
        <span>Enabled</span>
        <span class="actions-h">Actions</span>
      </div>
      {#each servers as s (s.id)}
        <div class="srow">
          <button class="name" onclick={() => onSelect(s.id)} title="View tools">
            <span class="nm">{s.name}</span>
            {#if s.has_secret}<Icon name="key" size={11} />{/if}
            {#if s.description}<span class="desc">{s.description}</span>{/if}
            <span class="endpoint mono">{s.transport === 'stdio' ? `${s.command} ${s.args.join(' ')}`.trim() : (s.url ?? '')}</span>
          </button>
          <span class="cell"><span class="transport">{s.transport}</span></span>
          <span class="cell">
            <McpPill kind="health" value={s.health_status} small />
            {#if s.health_latency_ms != null && s.health_status === 'healthy'}<span class="lat">{s.health_latency_ms}ms</span>{/if}
          </span>
          <span class="cell num">{s.tools_count}</span>
          <span class="cell"><McpPill kind="injection" value={s.injection_risk} small /></span>
          <span class="cell">
            <button
              class="switch"
              class:on={s.enabled}
              role="switch"
              aria-checked={s.enabled}
              disabled={busy[s.id] === 'toggle'}
              onclick={() => void toggleEnabled(s)}
              title={s.enabled ? 'Enabled — click to disable' : 'Disabled — click to enable'}
            >
              <span class="knob"></span>
            </button>
          </span>
          <span class="cell actions">
            <button class="btn xs" disabled={!!busy[s.id]} onclick={() => void discover(s)}>
              {busy[s.id] === 'discover' ? '…' : 'Discover'}
            </button>
            <button class="btn xs" disabled={!!busy[s.id]} onclick={() => void health(s)}>
              {busy[s.id] === 'health' ? '…' : 'Health'}
            </button>
            <button class="btn xs danger" disabled={!!busy[s.id]} onclick={() => void remove(s)}>
              {busy[s.id] === 'delete' ? '…' : 'Delete'}
            </button>
          </span>
        </div>
      {/each}
    </div>
  {/if}
</div>

{#if formOpen}
  <ServerForm {wsId} onclose={() => (formOpen = false)} onsaved={() => void onReload()} />
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
    font-size: 12px;
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
    grid-template-columns: minmax(220px, 2fr) 80px 140px 60px 90px 60px minmax(190px, auto);
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
  }
  .thead {
    position: sticky;
    top: 0;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--text-dim);
    z-index: 1;
  }
  .srow {
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .srow:hover {
    background: color-mix(in srgb, var(--text-dim) 5%, transparent);
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
    font-size: 13px;
  }
  .name:hover .nm {
    color: var(--accent);
  }
  .desc {
    font-size: 11px;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .endpoint {
    font-size: 10.5px;
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
    font-size: 11px;
    font-family: var(--font-mono);
    color: var(--text-dim);
  }
  .lat {
    font-size: 10px;
    color: var(--text-dim);
  }
  .actions {
    gap: 4px;
    flex-wrap: wrap;
  }
  .actions-h {
    text-align: start;
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
    background: var(--status-working, #28c840);
  }
  .switch .knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 13px;
    height: 13px;
    border-radius: 50%;
    background: #fff;
    transition: left 120ms ease;
  }
  .switch.on .knob {
    left: 15px;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .muted {
    color: var(--text-dim);
  }
  .pad {
    padding: 16px;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    color: var(--text-dim);
    text-align: center;
    padding: 40px 24px;
  }
  .btn.xs {
    font-size: 11px;
    padding: 3px 8px;
  }
  .btn.danger {
    color: var(--status-exited, #ff5f57);
  }

  @media (max-width: 760px) {
    .thead {
      display: none;
    }
    .srow {
      grid-template-columns: 1fr;
      gap: 4px;
      padding: 12px 14px;
    }
  }
</style>
