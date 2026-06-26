<script lang="ts">
  // Policy-as-code rules (global + this workspace). List / create / edit /
  // delete, export the whole ruleset to JSON, import a ruleset (append or
  // replace), and an Evaluate preview that shows the decision a (server, tool)
  // would get under the current rules.
  import Icon from '../../lib/components/Icon.svelte';
  import { mcpCpApi } from '../../lib/api/mcp';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type {
    CreateMcpPolicyReq,
    McpEvaluatePreview,
    McpPolicy,
    McpServerDetail,
  } from '../../lib/api/types';
  import McpPill from './McpPill.svelte';
  import PolicyForm from './PolicyForm.svelte';

  interface Props {
    wsId: string;
    servers: McpServerDetail[];
  }
  let { wsId, servers }: Props = $props();

  let policies = $state<McpPolicy[]>([]);
  let loading = $state(false);
  let editing = $state<McpPolicy | null>(null);
  let formOpen = $state(false);

  async function load(): Promise<void> {
    loading = true;
    try {
      policies = await mcpCpApi.cpPolicies(wsId);
    } catch (e) {
      toasts.error('Failed to load policies', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void wsId;
    void load();
  });

  function openNew(): void {
    editing = null;
    formOpen = true;
  }
  function openEdit(p: McpPolicy): void {
    editing = p;
    formOpen = true;
  }

  async function remove(p: McpPolicy): Promise<void> {
    if (!(await confirmer.ask(`Delete policy "${p.name}"?`, { title: 'Delete policy', danger: true, confirmLabel: 'Delete' })))
      return;
    try {
      await mcpCpApi.cpDeletePolicy(p.id);
      toasts.success('Policy deleted', p.name);
      await load();
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function exportJson(): Promise<void> {
    try {
      const doc = await mcpCpApi.cpExportPolicies();
      const blob = new Blob([JSON.stringify(doc, null, 2)], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `mcp-policies-${new Date().toISOString().slice(0, 10)}.json`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (e) {
      toasts.error('Export failed', e instanceof Error ? e.message : String(e));
    }
  }

  // ---- import ----
  let importOpen = $state(false);
  let importText = $state('');
  let importReplace = $state(false);
  let importing = $state(false);

  async function doImport(): Promise<void> {
    let parsed: unknown;
    try {
      parsed = JSON.parse(importText);
    } catch (e) {
      toasts.error('Invalid JSON', e instanceof Error ? e.message : String(e));
      return;
    }
    // Accept either the exported {version, policies} doc or a bare policies array.
    const list = Array.isArray(parsed)
      ? parsed
      : ((parsed as { policies?: unknown }).policies ?? []);
    if (!Array.isArray(list)) {
      toasts.error('Expected a policies array or an exported {version, policies} document');
      return;
    }
    importing = true;
    try {
      const res = await mcpCpApi.cpImportPolicies({
        policies: list as CreateMcpPolicyReq[],
        replace: importReplace,
      });
      toasts.success(
        'Policies imported',
        `${res.imported} imported${res.replaced ? ' (replaced existing)' : ''}`,
      );
      importOpen = false;
      importText = '';
      await load();
    } catch (e) {
      toasts.error('Import failed', e instanceof Error ? e.message : String(e));
    } finally {
      importing = false;
    }
  }

  // ---- evaluate preview ----
  let evalServerId = $state('');
  let evalTool = $state('');
  let evalResult = $state<McpEvaluatePreview | null>(null);
  let evaluating = $state(false);

  $effect(() => {
    if (!evalServerId && servers.length) evalServerId = servers[0].id;
  });

  async function evaluate(): Promise<void> {
    if (!evalServerId || !evalTool.trim()) {
      toasts.error('Pick a server and enter a tool name');
      return;
    }
    evaluating = true;
    evalResult = null;
    try {
      evalResult = await mcpCpApi.cpEvaluate({
        server_id: evalServerId,
        tool: evalTool.trim(),
        workspace_id: wsId,
      });
    } catch (e) {
      toasts.error('Evaluate failed', e instanceof Error ? e.message : String(e));
    } finally {
      evaluating = false;
    }
  }

  function matchSummary(m: unknown): string {
    if (!m || typeof m !== 'object') return 'any';
    const entries = Object.entries(m as Record<string, unknown>);
    if (entries.length === 0) return 'any';
    return entries.map(([k, v]) => `${k}=${typeof v === 'object' ? JSON.stringify(v) : String(v)}`).join('  ');
  }
</script>

<div class="pol">
  <div class="bar">
    <span class="count">{policies.length} rule{policies.length === 1 ? '' : 's'}</span>
    <span class="grow"></span>
    <button class="btn small" onclick={() => void exportJson()}><Icon name="arrowDown" size={13} /> Export</button>
    <button class="btn small" onclick={() => (importOpen = !importOpen)}><Icon name="arrowUp" size={13} /> Import</button>
    <button class="btn primary small" onclick={openNew}><Icon name="plus" size={13} /> New policy</button>
  </div>

  {#if importOpen}
    <div class="import-panel">
      <textarea
        bind:value={importText}
        rows="5"
        class="mono"
        spellcheck="false"
        placeholder={'Paste an exported {"version":1,"policies":[…]} document or a bare [ … ] array'}
      ></textarea>
      <div class="import-actions">
        <label class="check">
          <input type="checkbox" bind:checked={importReplace} />
          <span>Replace all existing rules</span>
        </label>
        <span class="grow"></span>
        <button class="btn small" onclick={() => (importOpen = false)}>Cancel</button>
        <button class="btn primary small" onclick={() => void doImport()} disabled={importing}>
          {importing ? 'Importing…' : 'Import'}
        </button>
      </div>
    </div>
  {/if}

  <!-- Evaluate preview -->
  <div class="evaluate">
    <div class="eval-row">
      <Icon name="gauge" size={14} />
      <span class="el">Evaluate</span>
      <select bind:value={evalServerId}>
        {#if servers.length === 0}<option value="">No servers</option>{/if}
        {#each servers as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
      </select>
      <input bind:value={evalTool} placeholder="tool name" class="mono" />
      <button class="btn small" onclick={() => void evaluate()} disabled={evaluating || servers.length === 0}>
        {evaluating ? '…' : 'Preview decision'}
      </button>
      {#if evalResult}
        <McpPill kind="decision" value={evalResult.policy_decision === 'allow' ? 'allowed' : evalResult.policy_decision} />
        <McpPill kind="risk" value={evalResult.risk_label} small />
        <McpPill kind="injection" value={evalResult.injection_risk} small />
        {#if evalResult.reason}<span class="reason">{evalResult.reason}</span>{/if}
      {/if}
    </div>
  </div>

  {#if loading && policies.length === 0}
    <p class="muted pad">Loading…</p>
  {:else if policies.length === 0}
    <div class="empty">
      <Icon name="split" size={22} />
      <p>No policy rules. Calls fall through to allowlists + per-tool permission.</p>
      <button class="btn primary" onclick={openNew}>Create a rule</button>
    </div>
  {:else}
    <div class="grid">
      <div class="thead">
        <span>Name</span>
        <span>Scope</span>
        <span class="num">Prio</span>
        <span>Effect</span>
        <span>Match</span>
        <span>On</span>
        <span></span>
      </div>
      {#each policies as p (p.id)}
        <div class="prow">
          <div class="pname">
            <span class="nm">{p.name}</span>
            {#if p.reason}<span class="desc">{p.reason}</span>{/if}
          </div>
          <span class="cell"><span class="scope">{p.workspace_id == null ? 'global' : 'workspace'}</span></span>
          <span class="cell num">{p.priority}</span>
          <span class="cell"><McpPill kind="decision" value={p.effect === 'allow' ? 'allowed' : p.effect === 'deny' ? 'denied' : p.effect === 'require_approval' ? 'pending_approval' : 'dry_run'} small /></span>
          <span class="cell match mono">{matchSummary(p.match)}</span>
          <span class="cell">{#if p.enabled}<Icon name="check" size={14} />{:else}<span class="off">off</span>{/if}</span>
          <span class="cell actions">
            <button class="btn xs" onclick={() => openEdit(p)}>Edit</button>
            <button class="btn xs danger" onclick={() => void remove(p)}>Delete</button>
          </span>
        </div>
      {/each}
    </div>
  {/if}
</div>

{#if formOpen}
  <PolicyForm
    {wsId}
    policy={editing}
    onclose={() => (formOpen = false)}
    onsaved={() => void load()}
  />
{/if}

<style>
  .pol {
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
  }
  .count {
    font-size: 12px;
    color: var(--text-dim);
  }
  .grow {
    flex: 1;
  }
  .import-panel {
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 8px;
    background: var(--surface);
  }
  .import-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .evaluate {
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }
  .eval-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    color: var(--text-dim);
  }
  .eval-row .el {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
  }
  .reason {
    font-size: 12px;
    color: var(--text-dim);
  }
  .grid {
    overflow: auto;
  }
  .thead,
  .prow {
    display: grid;
    grid-template-columns: minmax(180px, 1.6fr) 90px 50px 120px minmax(160px, 1.6fr) 50px 130px;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
  }
  .thead {
    border-bottom: 1px solid var(--border);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--text-dim);
  }
  .prow {
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .prow:hover {
    background: color-mix(in srgb, var(--text-dim) 5%, transparent);
  }
  .num {
    text-align: right;
  }
  .pname {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .pname .nm {
    font-size: 13px;
    font-weight: 600;
  }
  .desc {
    font-size: 11px;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cell {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .scope {
    font-size: 11px;
    color: var(--text-dim);
  }
  .match {
    font-size: 11px;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .off {
    font-size: 11px;
    color: var(--text-dim);
  }
  .actions {
    gap: 4px;
  }
  select,
  input,
  textarea {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 6px);
    color: var(--text);
    padding: 6px 8px;
    font-size: 12.5px;
  }
  textarea {
    width: 100%;
    resize: vertical;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .check {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
    color: var(--text);
  }
  .btn.xs {
    font-size: 11px;
    padding: 3px 8px;
  }
  .btn.danger {
    color: var(--status-exited, #ff5f57);
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
</style>
