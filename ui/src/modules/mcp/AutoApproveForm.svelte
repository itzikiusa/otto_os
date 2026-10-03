<script lang="ts">
  // New auto-approve rule: WHAT (one mutating tool, or every mutating tool in a
  // category) × WHERE (everywhere / one workspace / one agent session). An
  // irreversible tool needs the second explicit toggle; a category never
  // covers one. Every auto-approved call is still audited, naming the rule.
  import { untrack } from 'svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { mcpCpExtraApi } from './cp-api';
  import type {
    McpAutoApproveList,
    McpAutoApproveScope,
    McpAutoApproveTargetKind,
  } from '../../lib/api/types';

  interface Props {
    catalog: McpAutoApproveList['categories'];
    /** Pre-selected target (e.g. opened from a catalog row). */
    preset?: { kind: McpAutoApproveTargetKind; target: string } | null;
    /** Pre-selected scope (e.g. "Approve & always allow…" from an approval
     *  card defaults to the requesting session, else its workspace). */
    presetScope?: {
      scope: McpAutoApproveScope;
      workspace_id?: string | null;
      session_id?: string | null;
      /** Shown when the session isn't in the current workspace's list. */
      session_label?: string | null;
    } | null;
    /** Modal title / primary label overrides (the approval-card flow also approves). */
    title?: string;
    saveLabel?: string;
    onclose: () => void;
    onsaved: () => void | Promise<void>;
  }
  let {
    catalog,
    preset = null,
    presetScope = null,
    title = 'New auto-approve rule',
    saveLabel = 'Create rule',
    onclose,
    onsaved,
  }: Props = $props();

  const init = untrack(() => preset);
  const initScope = untrack(() => presetScope);
  let kind = $state<McpAutoApproveTargetKind>(init?.kind ?? 'category');
  let category = $state(init?.kind === 'category' ? init.target : (untrack(() => catalog)[0]?.category ?? ''));
  let tool = $state(init?.kind === 'tool' ? init.target.replace(/^otto\./, '') : '');
  let scope = $state<McpAutoApproveScope>(initScope?.scope ?? 'global');
  let workspaceId = $state(initScope?.workspace_id ?? untrack(() => ws.currentId) ?? '');
  let sessionId = $state(initScope?.session_id ?? '');
  let name = $state('');
  let ackIrreversible = $state(false);
  let saving = $state(false);

  const allTools = $derived(
    catalog.flatMap((group) =>
      group.tools.map((t) => ({ ...t, bare: t.name.replace(/^otto\./, ''), category: group.category })),
    ),
  );
  const selectedTool = $derived(allTools.find((t) => t.bare === tool) ?? null);
  const irreversible = $derived(kind === 'tool' && !!selectedTool?.irreversible);
  const categoryGroup = $derived(catalog.find((group) => group.category === category) ?? null);
  const excluded = $derived((categoryGroup?.tools ?? []).filter((t) => t.irreversible));
  const covered = $derived((categoryGroup?.tools ?? []).filter((t) => !t.irreversible));
  // The preset session stays choosable even when it lives in another workspace.
  const sessions = $derived.by(() => {
    const list = ws.agentSessions.map((s) => ({ id: s.id, label: `${s.title} · ${s.provider}` }));
    const pre = initScope?.session_id;
    if (pre && !list.some((s) => s.id === pre)) list.unshift({ id: pre, label: initScope?.session_label ?? pre });
    return list;
  });
  const workspaces = $derived(ws.workspaces);

  const target = $derived(kind === 'category' ? category : tool);
  const scopeReady = $derived(
    scope === 'global' || (scope === 'workspace' && !!workspaceId) || (scope === 'session' && !!sessionId),
  );
  const canSave = $derived(!!target && scopeReady && (!irreversible || ackIrreversible) && !saving);

  async function save(): Promise<void> {
    if (!canSave) return;
    saving = true;
    try {
      await mcpCpExtraApi.createAutoApprove({
        name: name.trim() || undefined,
        scope,
        workspace_id: scope === 'workspace' ? workspaceId : undefined,
        session_id: scope === 'session' ? sessionId : undefined,
        target_kind: kind,
        target,
        allow_irreversible: irreversible ? ackIrreversible : undefined,
      });
      toasts.success('Auto-approve rule created', kind === 'category' ? `${category} writes` : `otto.${tool}`);
      await onsaved();
      onclose();
    } catch (e) {
      toasts.error('Could not create the rule', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }
</script>

<Modal {title} width={560} {onclose}>
  <div class="form" data-testid="mcp-auto-approve-form">
    <fieldset class="field">
      <legend>What runs without asking</legend>
      <div class="seg">
        <label class="check">
          <input type="radio" name="aa-kind" value="category" bind:group={kind} />
          <span>Every mutating tool in a category</span>
        </label>
        <label class="check">
          <input type="radio" name="aa-kind" value="tool" bind:group={kind} />
          <span>One tool</span>
        </label>
      </div>
      {#if kind === 'category'}
        <select bind:value={category} aria-label="Category" data-testid="mcp-auto-approve-category">
          {#each catalog as group (group.category)}
            <option value={group.category}>{group.category} ({group.tools.length})</option>
          {/each}
        </select>
        {#if categoryGroup}
          <span class="hint">
            Covers {covered.map((t) => t.name).join(', ') || 'no reversible tools'}, and tools added to
            {category} later.
          </span>
          {#if excluded.length}
            <span class="hint warn-text">
              Never covers the irreversible {excluded.map((t) => t.name).join(', ')} — those keep asking
              unless you add a rule for that one tool.
            </span>
          {/if}
        {/if}
      {:else}
        <select bind:value={tool} aria-label="Tool" data-testid="mcp-auto-approve-tool">
          <option value="" disabled>Choose a mutating tool…</option>
          {#each catalog as group (group.category)}
            <optgroup label={group.category}>
              {#each group.tools as t (t.name)}
                <option value={t.name.replace(/^otto\./, '')}>{t.name}{t.irreversible ? ' — irreversible' : ''}</option>
              {/each}
            </optgroup>
          {/each}
        </select>
      {/if}
    </fieldset>

    <fieldset class="field">
      <legend>Where</legend>
      <div class="seg">
        <label class="check">
          <input type="radio" name="aa-scope" value="global" bind:group={scope} />
          <span>Everywhere</span>
        </label>
        <label class="check">
          <input type="radio" name="aa-scope" value="workspace" bind:group={scope} />
          <span>One workspace</span>
        </label>
        <label class="check">
          <input type="radio" name="aa-scope" value="session" bind:group={scope} />
          <span>One agent session</span>
        </label>
      </div>
      {#if scope === 'workspace'}
        <select bind:value={workspaceId} aria-label="Workspace">
          <option value="" disabled>Choose a workspace…</option>
          {#each workspaces as w (w.id)}<option value={w.id}>{w.name}</option>{/each}
        </select>
        <span class="hint">Applies to calls that land in this workspace (e.g. a PR on one of its repos).</span>
      {:else if scope === 'session'}
        {#if sessions.length}
          <select bind:value={sessionId} aria-label="Agent session">
            <option value="" disabled>Choose a session…</option>
            {#each sessions as s (s.id)}<option value={s.id}>{s.label}</option>{/each}
          </select>
          <span class="hint">Only calls made by this agent session. The rule is removed with the session.</span>
        {:else}
          <span class="hint">No agent sessions in {ws.current?.name ?? 'this workspace'}.</span>
        {/if}
      {/if}
    </fieldset>

    <label class="field">
      <span>Name</span>
      <input bind:value={name} placeholder="Optional — shown in the audit trail" />
    </label>

    {#if irreversible}
      <div class="danger-box" role="note">
        <strong>otto.{tool} is irreversible.</strong>
        <span>
          Nobody can undo it from Otto once it runs. Running it without asking needs this second,
          explicit confirmation.
        </span>
        <label class="check">
          <input type="checkbox" bind:checked={ackIrreversible} data-testid="mcp-auto-approve-ack" />
          <span>Run otto.{tool} without asking — I accept it cannot be undone</span>
        </label>
      </div>
    {/if}
    <p class="hint">Every auto-approved call is still recorded in Activity → Audit as “Auto approved”, naming this rule.</p>
  </div>

  {#snippet footer()}
    <button class="btn" onclick={onclose} disabled={saving}>Cancel</button>
    <button class="btn primary" onclick={() => void save()} disabled={!canSave} data-testid="mcp-auto-approve-save">
      {saving ? 'Saving…' : saveLabel}
    </button>
  {/snippet}
</Modal>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
    border: none;
    margin: 0;
    padding: 0;
    min-width: 0;
  }
  .field > span,
  legend {
    font-size: var(--fs-s);
    color: var(--text-dim);
    padding: 0;
    margin-block-end: 2px;
  }
  .seg {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 14px;
  }
  input:not([type='radio']):not([type='checkbox']),
  select {
    width: 100%;
    box-sizing: border-box;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    padding: 7px 9px;
    font-size: var(--fs-m);
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-m);
    color: var(--text);
  }
  .hint {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .warn-text {
    color: var(--warning);
  }
  .danger-box {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    border-radius: var(--radius-s);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    background: var(--danger-soft);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .danger-box strong {
    color: var(--danger);
  }
</style>
