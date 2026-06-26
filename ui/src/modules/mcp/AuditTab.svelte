<script lang="ts">
  // The governed-call audit ledger — every invoke (UI tester, gateway, inward
  // read-only server, outward otto.* tools) writes one redacted row here. The
  // table is filterable by server, tool, and decision; rows show the decision,
  // ok/error, latency, bytes, and time.
  import Icon from '../../lib/components/Icon.svelte';
  import { mcpCpApi } from '../../lib/api/mcp';
  import { toasts } from '../../lib/toast.svelte';
  import type { McpCallLogRow, McpServerDetail } from '../../lib/api/types';
  import McpPill from './McpPill.svelte';

  interface Props {
    servers: McpServerDetail[];
  }
  let { servers }: Props = $props();

  let rows = $state<McpCallLogRow[]>([]);
  let loading = $state(false);
  let fServer = $state('');
  let fTool = $state('');
  let fDecision = $state('');

  async function load(): Promise<void> {
    loading = true;
    try {
      rows = await mcpCpApi.cpAudit({
        server_id: fServer || undefined,
        tool: fTool.trim() || undefined,
        decision: fDecision || undefined,
        limit: 200,
      });
    } catch (e) {
      toasts.error('Failed to load audit log', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    // Server + decision selects apply immediately; the tool box applies on submit.
    void fServer;
    void fDecision;
    void load();
  });

  function fmtBytes(b: number | null): string {
    if (b == null) return '—';
    if (b < 1024) return `${b} B`;
    if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`;
    return `${(b / 1024 / 1024).toFixed(1)} MB`;
  }
</script>

<div class="audit">
  <div class="bar">
    <select bind:value={fServer}>
      <option value="">All servers</option>
      {#each servers as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
    </select>
    <input
      bind:value={fTool}
      placeholder="Filter tool…"
      class="mono"
      onkeydown={(e) => e.key === 'Enter' && void load()}
    />
    <select bind:value={fDecision}>
      <option value="">All decisions</option>
      <option value="allowed">allowed</option>
      <option value="approved">approved</option>
      <option value="denied">denied</option>
      <option value="dry_run">dry_run</option>
      <option value="pending_approval">pending_approval</option>
      <option value="error">error</option>
    </select>
    <button class="btn small" onclick={() => void load()}>Apply</button>
    <span class="grow"></span>
    <span class="count">{rows.length} row{rows.length === 1 ? '' : 's'}</span>
  </div>

  {#if loading && rows.length === 0}
    <p class="muted pad">Loading…</p>
  {:else if rows.length === 0}
    <div class="empty">
      <Icon name="note" size={22} />
      <p>No audit rows match. Run a tool from the Tools tester to populate it.</p>
    </div>
  {:else}
    <div class="grid">
      <div class="thead">
        <span>Time</span>
        <span>Server</span>
        <span>Tool</span>
        <span>Decision</span>
        <span>Dir</span>
        <span class="num">OK</span>
        <span class="num">Latency</span>
        <span class="num">Bytes</span>
      </div>
      {#each rows as r (r.id)}
        <div class="arow">
          <span class="cell when">{new Date(r.created_at).toLocaleString()}</span>
          <span class="cell">{r.server_name ?? '—'}</span>
          <span class="cell mono">{r.tool}{#if r.dry_run}<span class="dry">dry</span>{/if}</span>
          <span class="cell"><McpPill kind="decision" value={r.decision} small /></span>
          <span class="cell"><McpPill kind="direction" value={r.direction} small /></span>
          <span class="cell num">
            {#if r.ok}<Icon name="check" size={13} />{:else}<span class="bad" title={r.error ?? 'error'}><Icon name="x" size={13} /></span>{/if}
          </span>
          <span class="cell num">{r.latency_ms != null ? `${r.latency_ms}ms` : '—'}</span>
          <span class="cell num">{fmtBytes(r.bytes)}</span>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .audit {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }
  .grow {
    flex: 1;
  }
  .count {
    font-size: 12px;
    color: var(--text-dim);
  }
  select,
  input {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 6px);
    color: var(--text);
    padding: 6px 8px;
    font-size: 12.5px;
  }
  .grid {
    overflow: auto;
  }
  .thead,
  .arow {
    display: grid;
    grid-template-columns: 170px minmax(110px, 1fr) minmax(140px, 1.4fr) 130px 70px 40px 80px 80px;
    align-items: center;
    gap: 8px;
    padding: 7px 14px;
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
  .arow {
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
    font-size: 12.5px;
  }
  .arow:hover {
    background: color-mix(in srgb, var(--text-dim) 5%, transparent);
  }
  .cell {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .num {
    justify-content: flex-end;
    text-align: right;
  }
  .when {
    font-size: 11px;
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }
  .dry {
    margin-inline-start: 5px;
    font-size: 9px;
    text-transform: uppercase;
    color: #3b82f6;
  }
  .bad {
    color: var(--status-exited, #ff5f57);
    display: inline-flex;
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
    gap: 10px;
    color: var(--text-dim);
    text-align: center;
    padding: 36px 24px;
  }

  @media (max-width: 760px) {
    .thead {
      display: none;
    }
    .arow {
      grid-template-columns: 1fr 1fr;
      gap: 4px 8px;
    }
  }
</style>
