<script lang="ts">
  import { toastError } from '../../lib/toastError';
  // Policy-as-code rules (global + this workspace). List / create / edit /
  // delete, export the whole ruleset to JSON, import a ruleset (append or
  // replace), and an Evaluate preview that shows the decision a (server, tool)
  // would get under the current rules.
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { plural } from '../../lib/plural';
  import { loadErrorText } from '../../lib/loadError';
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
  /** Failed load — inline with Retry, never the empty state. */
  let loadError = $state<string | null>(null);
  let editing = $state<McpPolicy | null>(null);
  let formOpen = $state(false);

  async function load(): Promise<void> {
    loading = true;
    try {
      policies = await mcpCpApi.cpPolicies(wsId);
      loadError = null;
    } catch (e) {
      loadError = loadErrorText(e);
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
      toastError('Couldn’t delete', e);
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
      toastError('Couldn’t export', e);
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
      toastError('Invalid JSON', e);
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
    if (importing) return;
    const replace = importReplace;
    importing = true;
    try {
      if (replace) {
        // Replacement is instance-wide, even if this workspace's list is empty.
        const affected = await mcpCpApi.cpPolicies();
        const workspaces = new Set(affected.flatMap((rule) => rule.workspace_id ? [rule.workspace_id] : []));
        const globals = affected.filter((rule) => !rule.workspace_id).length;
        if (!(await confirmer.ask(
          `Replace all ${affected.length} existing rules across every workspace with ${list.length} imported rules? This deletes ${globals} global rules and the rules in ${workspaces.size} workspaces.`,
          { title: 'Replace policy rules everywhere', danger: true, confirmLabel: 'Replace all rules' },
        ))) return;
      }
      const res = await mcpCpApi.cpImportPolicies({
        policies: list as CreateMcpPolicyReq[],
        replace,
      });
      toasts.success(
        'Policies imported',
        `${res.imported} imported${res.replaced ? ' (replaced existing)' : ''}`,
      );
      importOpen = false;
      importText = '';
      await load();
    } catch (e) {
      toastError('Couldn’t import', e);
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
      toastError('Couldn’t evaluate', e);
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
    <span class="count">{plural(policies.length, 'rule')}</span>
    <span class="grow"></span>
    <button class="btn small" onclick={() => void exportJson()}><Icon name="arrowDown" size={13} /> Export</button>
    <button class="btn small" onclick={() => (importOpen = !importOpen)}><Icon name="arrowUp" size={13} /> Import</button>
    <button class="btn primary small" onclick={openNew}><Icon name="plus" size={13} /> New policy</button>
  </div>

  {#if importOpen}
    <div class="import-panel">
      <textarea dir="ltr"
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
      <input dir="ltr" bind:value={evalTool} placeholder="tool name" class="mono" />
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

  <LoadState what="policies" {loading} error={loadError} empty={policies.length === 0} onretry={() => void load()}>
    {#snippet emptyView()}
      <EmptyState icon="split" title="No policy rules yet" body="Without rules, calls fall through to allowlists and per-tool permission." actionLabel="Create a rule" onaction={openNew} />
    {/snippet}
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
            {#if p.reason}<span class="desc" title={p.reason}>{p.reason}</span>{/if}
          </div>
          <!-- `.cl`: the column name, announced on every row and shown inline once
               the header row is hidden (phone). -->
          <span class="cell"><span class="cl">Scope</span><span class="scope">{p.workspace_id == null ? 'Global' : 'Workspace'}</span></span>
          <span class="cell num"><span class="cl">Priority</span>{p.priority}</span>
          <span class="cell"><span class="cl">Effect</span><McpPill kind="decision" value={p.effect === 'allow' ? 'allowed' : p.effect === 'deny' ? 'denied' : p.effect === 'require_approval' ? 'pending_approval' : 'dry_run'} small /></span>
          <span class="cell mono match-cell"><span class="cl">Match</span><span class="match" title={matchSummary(p.match)}>{matchSummary(p.match)}</span></span>
          <span class="cell"><span class="cl">Enabled</span>{#if p.enabled}<Icon name="check" size={14} /><span class="sr-only">On</span>{:else}<span class="off">Off</span>{/if}</span>
          <span class="cell actions">
            <button class="btn small" onclick={() => openEdit(p)}>Edit</button>
            <button class="btn small danger" onclick={() => void remove(p)}>Delete…</button>
          </span>
        </div>
      {/each}
    </div>
  </LoadState>
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
    font-size: var(--fs-s);
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
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
  }
  .reason {
    font-size: var(--fs-s);
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
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
  }
  .prow {
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .prow:hover {
    background: var(--hover);
  }
  .num {
    text-align: end;
  }
  /* `.cell` is a flex box — text-align doesn't move its content. */
  .cell.num {
    justify-content: flex-end;
  }
  .pname {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .pname .nm {
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .desc {
    font-size: var(--fs-xs);
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
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .match {
    min-width: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .off {
    font-size: var(--fs-xs);
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
    border-radius: var(--radius-s);
    color: var(--text);
    padding: 6px 8px;
    font-size: var(--fs-m);
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
    font-size: var(--fs-m);
    color: var(--text);
  }
  .cl {
    position: absolute;
    inline-size: 1px;
    block-size: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  /* Phone: the header row goes; each rule stacks — name across the top, then
     labelled cells two to a line, actions at the end (as ToolsTab does). */
  @media (max-width: 640px) {
    .thead {
      display: none;
    }
    .prow {
      grid-template-columns: repeat(2, minmax(0, 1fr));
      gap: 6px 12px;
    }
    .pname,
    .match-cell,
    .actions {
      grid-column: 1 / -1;
    }
    .cell.num {
      justify-content: flex-start;
    }
    .cl {
      position: static;
      inline-size: auto;
      block-size: auto;
      overflow: visible;
      clip-path: none;
      font-size: var(--fs-xs);
      color: var(--text-dim);
    }
  }
</style>
