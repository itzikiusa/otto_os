<script lang="ts">
  // Budgets editor (opt-in spend caps per workspace or provider), in a sheet
  // opened from the Usage details card. A local copy of usage.budgets.config,
  // saved on demand; enforcement and hard blocking stay opt-in, and turning
  // blocking on confirms what it will refuse right away.
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { guardUnsaved } from '../../lib/leaveGuard';
  import { auth } from '../../lib/stores/auth.svelte';
  import { usage, type UsageBudgetConfig } from '../../lib/api/usage.svelte';
  import { ws, SCRATCH_WORKSPACE_ID } from '../../lib/stores/workspace.svelte';

  let { onclose }: { onclose: () => void } = $props();

  // svelte-ignore state_referenced_locally
  let budgetCfg: UsageBudgetConfig = $state(
    usage.budgets ? structuredClone($state.snapshot(usage.budgets.config)) : { enforce: false, block_on_exceed: false, window_days: 30, workspaces: [], providers: [] },
  );
  let budgetsDirty = $state(false);
  let budgetValidated = $state(false);
  $effect(() => guardUnsaved(() => budgetsDirty, { what: 'your budgets', allow: (to) => to === 'usage' || to.startsWith('usage/') }));
  const budgetWindowInvalid = $derived(!Number.isInteger(budgetCfg.window_days) || budgetCfg.window_days < 1 || budgetCfg.window_days > 3650);
  const budgetValidation = $derived.by(() => {
    if (budgetWindowInvalid) return 'Choose a whole number of days from 1 to 3650.';
    if ([...budgetCfg.workspaces, ...budgetCfg.providers].some((b) => b.monthly_usd != null && (!Number.isFinite(b.monthly_usd) || b.monthly_usd < 0)))
      return 'Caps must be zero or a positive amount in US dollars.';
    const providers = budgetCfg.providers.filter((b) => b.provider && b.monthly_usd > 0).map((b) => b.provider);
    if (new Set(providers).size !== providers.length) return 'Use one cap per provider. Remove the duplicate row or choose another provider.';
    const workspaces = budgetCfg.workspaces.filter((b) => b.workspace_id && b.monthly_usd > 0).map((b) => b.workspace_id);
    if (new Set(workspaces).size !== workspaces.length) return 'Use one cap per workspace. Remove the duplicate row or choose another workspace.';
    return '';
  });

  // Workspace name for a budget row's id; the hidden scratch workspace is not
  // in `ws.workspaces`, so name it explicitly.
  const wsName = (id: string): string => ws.workspaces.find((w) => w.id === id)?.name ?? (id === SCRATCH_WORKSPACE_ID ? (ws.scratch?.name ?? 'Scratch') : id);
  // Provider choices: installed CLIs + any already used.
  const providerChoices = $derived.by(() => {
    const set = new Set<string>((usage.summary?.providers ?? []).map((p) => p.provider));
    for (const t of auth.meta?.providers ?? []) set.add(t);
    return [...set].filter(Boolean);
  });

  function addWsBudget(): void {
    budgetsDirty = true;
    budgetCfg.workspaces = [...budgetCfg.workspaces, { workspace_id: '', monthly_usd: 0 }];
  }
  function addProviderBudget(): void {
    budgetsDirty = true;
    budgetCfg.providers = [...budgetCfg.providers, { provider: '', monthly_usd: 0 }];
  }
  function removeWsBudget(i: number): void {
    budgetsDirty = true;
    budgetCfg.workspaces = budgetCfg.workspaces.filter((_, j) => j !== i);
  }
  function removeProviderBudget(i: number): void {
    budgetsDirty = true;
    budgetCfg.providers = budgetCfg.providers.filter((_, j) => j !== i);
  }

  async function close(): Promise<void> {
    if (budgetsDirty && !(await confirmer.ask('Your budget changes are not saved.', { title: 'Discard changes?', confirmLabel: 'Discard', cancelLabel: 'Keep editing', danger: true }))) return;
    budgetsDirty = false;
    onclose();
  }

  async function saveBudgets(): Promise<void> {
    if (usage.savingBudgets || budgetValidation) return;
    const submitted = JSON.stringify(budgetCfg);
    const cfg: UsageBudgetConfig = {
      ...budgetCfg,
      window_days: budgetCfg.window_days || 30,
      workspaces: budgetCfg.workspaces.filter((b) => b.workspace_id && b.monthly_usd > 0),
      providers: budgetCfg.providers.filter((b) => b.provider && b.monthly_usd > 0),
    };
    if (cfg.enforce && cfg.block_on_exceed && !usage.budgets?.config.block_on_exceed) {
      const over = (usage.budgets?.rows ?? []).filter((r) => r.exceeded).map((r) => (r.scope === 'workspace' ? wsName(r.key) : (r.label ?? r.key)));
      const detail = over.length ? `Already over budget and blocked immediately: ${over.join(', ')}.` : 'New work in any scope that exceeds its cap will be refused until spend drops or the cap is raised.';
      if (!(await confirmer.ask(detail, { title: 'Enable hard budget blocking?', confirmLabel: 'Enable blocking', danger: true }))) return;
    }
    // Edits made while the save was in flight stay open and unsaved.
    if ((await usage.saveBudgets(cfg)) && JSON.stringify(budgetCfg) === submitted) {
      budgetsDirty = false;
      onclose();
    }
  }
</script>

<Modal title="Budgets" width={560} onclose={() => void close()}>
  <p class="dim small">Spend caps per workspace or provider. They only warn until you turn on enforcement.</p>
      <div class="budget-editor" id="usage-budget-editor">
  <label class="checkbox-row">
    <input type="checkbox" bind:checked={budgetCfg.enforce} onchange={() => (budgetsDirty = true)} />
    <span>Enforce budgets: warn prominently when a cap is exceeded</span>
  </label>
  <label class="checkbox-row" class:disabled={!budgetCfg.enforce} title={budgetCfg.enforce ? undefined : 'Turn on enforcement first'}>
    <input
      type="checkbox"
      bind:checked={budgetCfg.block_on_exceed}
      disabled={!budgetCfg.enforce}
      onchange={() => (budgetsDirty = true)}
    />
    <span>Block new work in a scope that is over its cap (otherwise warn only)</span>
  </label>
  <div class="window-row">
    <label for="budget-window">Compare spend over the last</label>
    <input
      id="budget-window"
      class="input num-in"
      type="number"
      min="1"
      max="3650"
      step="1"
      aria-invalid={budgetValidated && budgetWindowInvalid}
      aria-describedby={budgetValidated && budgetValidation ? 'budget-validation' : undefined}
      onblur={() => (budgetValidated = true)}
      bind:value={budgetCfg.window_days}
      oninput={() => (budgetsDirty = true)}
    />
    <span class="dim">days</span>
  </div>

  <div class="editor-section">
    <div class="editor-head">
      <span id="budget-ws-head">Per workspace</span>
      <button class="btn small ghost" onclick={addWsBudget}><Icon name="plus" size={12} /> Add workspace cap</button>
    </div>
    {#each budgetCfg.workspaces as b, i (i)}
      <div class="editor-line">
        <select class="input" aria-label="Workspace" bind:value={b.workspace_id} onchange={() => (budgetsDirty = true)}>
          <option value="">Choose a workspace…</option>
          {#each ws.workspaces as w (w.id)}
            <option value={w.id}>{w.name}</option>
          {/each}
        </select>
        <span class="usd">$</span>
        <input
          class="input num-in"
          type="number"
          min="0"
          step="1"
          placeholder="0"
          aria-label="Cap in US dollars"
          bind:value={b.monthly_usd}
          aria-invalid={budgetValidated && b.monthly_usd != null && (!Number.isFinite(b.monthly_usd) || b.monthly_usd < 0)}
          aria-describedby={budgetValidated && budgetValidation ? 'budget-validation' : undefined}
          onblur={() => (budgetValidated = true)}
          oninput={() => (budgetsDirty = true)}
        />
        <button class="icon-btn rm-btn" onclick={() => removeWsBudget(i)} title="Remove this cap" aria-label="Remove this workspace cap">
          <Icon name="trash" size={14} />
        </button>
      </div>
    {:else}
      <p class="dim small">No workspace caps.</p>
    {/each}
  </div>

  <div class="editor-section">
    <div class="editor-head">
      <span>Per provider</span>
      <button class="btn small ghost" onclick={addProviderBudget}><Icon name="plus" size={12} /> Add provider cap</button>
    </div>
    {#each budgetCfg.providers as b, i (i)}
      <div class="editor-line">
        <select class="input" aria-label="Provider" bind:value={b.provider} onchange={() => (budgetsDirty = true)}>
          <option value="">Choose a provider…</option>
          {#each providerChoices as p (p)}
            <option value={p}>{p}</option>
          {/each}
        </select>
        <span class="usd">$</span>
        <input
          class="input num-in"
          type="number"
          min="0"
          step="1"
          placeholder="0"
          aria-label="Cap in US dollars"
          bind:value={b.monthly_usd}
          aria-invalid={budgetValidated && b.monthly_usd != null && (!Number.isFinite(b.monthly_usd) || b.monthly_usd < 0)}
          aria-describedby={budgetValidated && budgetValidation ? 'budget-validation' : undefined}
          onblur={() => (budgetValidated = true)}
          oninput={() => (budgetsDirty = true)}
        />
        <button class="icon-btn rm-btn" onclick={() => removeProviderBudget(i)} title="Remove this cap" aria-label="Remove this provider cap">
          <Icon name="trash" size={14} />
        </button>
      </div>
    {:else}
      <p class="dim small">No provider caps.</p>
    {/each}
  </div>

  {#if budgetValidated && budgetValidation}<p id="budget-validation" use:announceShown={{ text: budgetValidation, assertive: true }}>{budgetValidation}</p>{/if}
  <div class="editor-actions">
    <span class="dim small">Rows without a scope or a cap above $0 are dropped on save.</span>
    <button class="btn" onclick={close}>{budgetsDirty ? 'Discard changes' : 'Cancel'}</button>
    <button
      class="btn primary"
      disabled={usage.savingBudgets || !budgetsDirty || !!budgetValidation}
      title={budgetValidation || (budgetsDirty ? undefined : 'No changes to save')}
      onclick={saveBudgets}
    >
      {usage.savingBudgets ? 'Saving…' : 'Save budgets'}
    </button>
  </div>
      </div>
</Modal>

<style>
  .budget-editor {
    margin-top: 12px;
    padding-top: 12px;
    border-top: 1px solid var(--separator);
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-width: 640px;
  }
  .checkbox-row.disabled {
    opacity: var(--disabled-opacity);
  }
  .window-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-m);
  }
  .editor-section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .editor-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text-dim);
  }
  .editor-line {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .editor-line select {
    flex: 1;
    min-width: 0;
  }
  .usd {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .num-in {
    width: 96px;
    text-align: end;
    font-variant-numeric: tabular-nums;
  }
  .rm-btn:hover {
    color: var(--danger);
    background: var(--danger-soft);
  }
  .editor-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 12px;
    flex-wrap: wrap;
  }
  .dim {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-s);
  }
  #budget-validation { color: var(--danger); }
</style>
