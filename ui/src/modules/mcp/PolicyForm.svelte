<script lang="ts">
  // Create / edit one policy-as-code rule. `match` is the matcher object
  // (all fields optional, AND-combined): server_id, server_name, tool, tool_glob,
  // risk_label, min_injection_risk, mutating, direction, caller_kind,
  // workspace_id. Evaluation is most-restrictive-wins; priority only orders
  // display and which reason is shown.
  import { untrack } from 'svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { mcpCpApi } from '../../lib/api/mcp';
  import { toasts } from '../../lib/toast.svelte';
  import type { McpPolicy, McpPolicyEffect } from '../../lib/api/types';

  interface Props {
    wsId: string;
    policy: McpPolicy | null;
    onclose: () => void;
    onsaved: () => void;
  }
  let { wsId, policy, onclose, onsaved }: Props = $props();

  // The form edits a snapshot of `policy` (the modal is re-created per open), so
  // capture it once — untracked — to seed the fields without a reactive read.
  const init = untrack(() => policy);

  let name = $state(init?.name ?? '');
  let enabled = $state(init?.enabled ?? true);
  let priority = $state(init?.priority ?? 100);
  let effect = $state<McpPolicyEffect>(init?.effect ?? 'require_approval');
  let reason = $state(init?.reason ?? '');
  let global = $state(init ? init.workspace_id == null : false);
  let matchText = $state(
    init ? JSON.stringify(init.match ?? {}, null, 2) : '{\n  "tool_glob": "*"\n}',
  );
  let saving = $state(false);

  async function save(): Promise<void> {
    if (!name.trim()) {
      toasts.error('A policy name is required');
      return;
    }
    let match: unknown = {};
    if (matchText.trim()) {
      try {
        match = JSON.parse(matchText);
      } catch (e) {
        toasts.error('Match must be valid JSON', e instanceof Error ? e.message : String(e));
        return;
      }
    }
    saving = true;
    try {
      if (policy) {
        await mcpCpApi.cpUpdatePolicy(policy.id, {
          name: name.trim(),
          enabled,
          priority,
          match,
          effect,
          reason: reason.trim() || null,
        });
      } else {
        await mcpCpApi.cpCreatePolicy({
          workspace_id: global ? null : wsId,
          name: name.trim(),
          enabled,
          priority,
          match,
          effect,
          reason: reason.trim() || null,
        });
      }
      toasts.success(policy ? 'Policy updated' : 'Policy created', name.trim());
      onsaved();
      onclose();
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }
</script>

<Modal title={policy ? 'Edit policy' : 'New policy'} width={560} {onclose}>
  <div class="form">
    <label class="field">
      <span>Name</span>
      <input bind:value={name} placeholder="e.g. Approve all dangerous writes" />
    </label>

    <div class="row3">
      <label class="field">
        <span>Effect</span>
        <select bind:value={effect}>
          <option value="allow">allow</option>
          <option value="deny">deny</option>
          <option value="require_approval">require_approval</option>
          <option value="require_dry_run">require_dry_run</option>
        </select>
      </label>
      <label class="field">
        <span>Priority</span>
        <input type="number" bind:value={priority} />
      </label>
      <label class="check tall">
        <input type="checkbox" bind:checked={enabled} />
        <span>Enabled</span>
      </label>
    </div>

    {#if !policy}
      <label class="check">
        <input type="checkbox" bind:checked={global} />
        <span>Global rule (applies to every workspace)</span>
      </label>
    {/if}

    <label class="field">
      <span>Match (JSON)</span>
      <textarea bind:value={matchText} rows="7" class="mono" spellcheck="false"></textarea>
      <span class="hint">
        Keys (all optional, AND-combined): server_id, server_name, tool, tool_glob, risk_label,
        min_injection_risk, mutating, direction, caller_kind, workspace_id.
      </span>
    </label>

    <label class="field">
      <span>Reason</span>
      <input bind:value={reason} placeholder="Shown when this rule decides (optional)" />
    </label>
  </div>

  {#snippet footer()}
    <button class="btn" onclick={onclose} disabled={saving}>Cancel</button>
    <button class="btn primary" onclick={() => void save()} disabled={saving}>
      {saving ? 'Saving…' : policy ? 'Save' : 'Create'}
    </button>
  {/snippet}
</Modal>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field > span {
    font-size: 12px;
    color: var(--text-dim);
  }
  .row3 {
    display: grid;
    grid-template-columns: 1.4fr 0.8fr auto;
    gap: 12px;
    align-items: end;
  }
  input,
  select,
  textarea {
    width: 100%;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 6px);
    color: var(--text);
    padding: 7px 9px;
    font-size: 13px;
  }
  textarea {
    resize: vertical;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: 12px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--text);
  }
  .check.tall {
    height: 36px;
  }
  .check input {
    width: auto;
  }
  .hint {
    font-size: 11px;
    color: var(--text-dim);
  }
</style>
