<script lang="ts">
  import { newId } from '../../lib/id';
  // Shared review-agent configuration modal for BOTH PR ReviewPanel and
  // LocalReviewPanel so local-only repositories (and repos without PRs) can
  // configure global or per-repo review agents directly.
  import { onDestroy, tick } from 'svelte';
  import { api, isAbortError } from '../../lib/api/client';
  import type {
    RepoReviewBinding,
    RepoReviewConfigResp,
    ReviewAgentCfg,
    ReviewConfig,
    ReviewConfigPreset,
  } from '../../lib/api/types';
  import { toastError } from '../../lib/toastError';
  import { toasts } from '../../lib/toast.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';
  import {
    advancedOf,
    mergeReviewConfig,
    syncProviders,
    type ReviewAdvanced,
  } from './reviewConfigMerge';
  import ReviewConfigAdvanced from './ReviewConfigAdvanced.svelte';
  import {
    JSON_INSTR,
    REVIEWER_PRESETS,
    presetFromSkill,
    type LibrarySkillMeta,
    type ReviewerPreset,
  } from './reviewSkillPresets';
  import {
    normalizeReviewAgent,
    sameReviewConfig,
    toggleAgentProvider,
  } from './gitUiHelpers';

  interface Props {
    repoId: string;
    onclose: () => void;
    /** Notifies the host whenever the repo's effective config binding changes. */
    onupdated?: (repoCfg: RepoReviewConfigResp) => void;
  }
  let { repoId, onclose, onupdated }: Props = $props();

  let disposed = false;
  let loadAbort: AbortController | null = null;
  onDestroy(() => {
    disposed = true;
    loadAbort?.abort();
  });

  let configLoading = $state(true);
  let configSaving = $state(false);
  let crossProviderCheck = $state(false);
  let editAgents: ReviewAgentCfg[] = $state([]);
  let editSummarizer: ReviewAgentCfg = $state({
    name: 'Summarizer',
    provider: defaultAgentProvider(),
    providers: [],
    model: '',
    prompt: '',
  });
  let editPresets: ReviewAgentCfg[] = $state([]);
  let configScope: 'global' | 'repo' = $state('global');
  let globalCfg: ReviewConfig | null = $state(null);
  let repoCfg: RepoReviewConfigResp | null = $state(null);
  let namedPresets: ReviewConfigPreset[] = $state([]);
  // '' = repo-only custom config (no preset backing it).
  let selectedPresetId = $state('');
  let presetSaveOpen = $state(false);
  let presetNameDraft = $state('');
  // Loaded config being edited: save lays edits over it so fields the modal
  // doesn't show (set via API / MCP / preset) survive.
  let editBase: ReviewConfig | null = null;
  let editAdvanced: ReviewAdvanced = $state(advancedOf(null));
  let skillPresets: ReviewerPreset[] = $state([]);

  let addAgentBtnEl = $state<HTMLButtonElement | null>(null);
  let addPresetBtnEl = $state<HTMLButtonElement | null>(null);
  let presetsListEl = $state<HTMLDivElement | null>(null);
  let agentsListEl = $state<HTMLDivElement | null>(null);

  const PROVIDER_OPTIONS = $derived(agentProviders());

  function seedEditFields(cfg: ReviewConfig): void {
    const fallback = defaultAgentProvider();
    editBase = cfg;
    editAdvanced = advancedOf(cfg);
    crossProviderCheck = cfg.cross_provider_check ?? false;
    editAgents = cfg.agents.map((a) => normalizeReviewAgent({ ...a }, fallback));
    editSummarizer = { ...cfg.summarizer };
  }

  $effect(() => {
    const rid = repoId;
    loadAbort?.abort();
    const ctl = new AbortController();
    loadAbort = ctl;
    configLoading = true;
    void Promise.all([
      api.get<ReviewConfig>('/settings/pr-review', ctl.signal),
      api.get<ReviewConfigPreset[]>('/settings/pr-review/presets', ctl.signal),
      api.get<RepoReviewConfigResp>(`/repos/${rid}/review-config`, ctl.signal),
      api.get<LibrarySkillMeta[]>('/library/skills', ctl.signal).catch(() => [] as LibrarySkillMeta[]),
    ])
      .then(([cfg, presets, binding, skills]) => {
        if (disposed || ctl.signal.aborted || rid !== repoId) return;
        const fallback = defaultAgentProvider();
        globalCfg = cfg;
        namedPresets = presets;
        repoCfg = binding;
        skillPresets = skills
          .filter((s) => s.category === 'review')
          .map((s) => presetFromSkill(s));
        // Per-agent presets live on the GLOBAL config regardless of scope.
        editPresets = (cfg.custom_presets ?? []).map((p) =>
          normalizeReviewAgent({ ...p }, fallback),
        );
        // Open on the scope the repo actually reviews with.
        configScope = binding.scope === 'global' ? 'global' : 'repo';
        selectedPresetId = binding.preset_id ?? '';
        seedEditFields(configScope === 'global' ? cfg : binding.config);
      })
      .catch((e) => {
        if (disposed || ctl.signal.aborted || isAbortError(e)) return;
        toastError('Couldn’t load config', e);
        onclose();
      })
      .finally(() => {
        if (!disposed && !ctl.signal.aborted && loadAbort === ctl) {
          configLoading = false;
        }
      });
    return () => {
      ctl.abort();
    };
  });

  function setConfigScope(scope: 'global' | 'repo'): void {
    if (scope === configScope) return;
    configScope = scope;
    if (scope === 'global') {
      if (globalCfg) seedEditFields(globalCfg);
    } else if (repoCfg) {
      selectedPresetId = repoCfg.preset_id ?? '';
      seedEditFields(repoCfg.config);
    }
  }

  function pickNamedPreset(id: string): void {
    selectedPresetId = id;
    const p = namedPresets.find((x) => x.id === id);
    if (p) seedEditFields(p.config);
  }

  function buildConfigFromEditState(presets: ReviewAgentCfg[]): ReviewConfig {
    const sync = (a: ReviewAgentCfg): ReviewAgentCfg => syncProviders(a, defaultAgentProvider());
    return mergeReviewConfig(
      editBase,
      {
        cross_provider_check: crossProviderCheck,
        agents: editAgents.map(sync),
        summarizer: editSummarizer,
        custom_presets: presets.map(sync),
      },
      editAdvanced,
    );
  }

  function currentEditConfig(includePresets: boolean): ReviewConfig {
    const base = buildConfigFromEditState(includePresets ? editPresets : []);
    return includePresets ? base : { ...base, custom_presets: [] };
  }

  function toggleProvider(agentIdx: number, p: string): void {
    const next = toggleAgentProvider(editAgents[agentIdx].providers, p, defaultAgentProvider());
    editAgents = editAgents.map((a, i) => (i === agentIdx ? { ...a, providers: next } : a));
  }

  async function useGlobalConfig(): Promise<void> {
    configSaving = true;
    try {
      const next = await api.del<RepoReviewConfigResp>(`/repos/${repoId}/review-config`);
      if (disposed) return;
      repoCfg = next;
      onupdated?.(next);
      configScope = 'global';
      selectedPresetId = '';
      if (globalCfg) seedEditFields(globalCfg);
      toasts.success('Repo now uses the global review config');
    } catch (e) {
      toastError('Couldn’t revert to global', e);
    } finally {
      if (!disposed) configSaving = false;
    }
  }

  async function saveAsNamedPreset(): Promise<void> {
    const name = presetNameDraft.trim();
    if (name === '') return;
    configSaving = true;
    try {
      const cfg = currentEditConfig(false);
      const existing = namedPresets.find((p) => p.name === name);
      const id = existing?.id ?? newId();
      const next = existing
        ? namedPresets.map((p) => (p.id === id ? { ...p, config: cfg } : p))
        : [...namedPresets, { id, name, config: cfg }];
      namedPresets = await api.put<ReviewConfigPreset[]>('/settings/pr-review/presets', next);
      const body: RepoReviewBinding = { preset_id: id };
      const updated = await api.put<RepoReviewConfigResp>(`/repos/${repoId}/review-config`, body);
      if (disposed) return;
      repoCfg = updated;
      onupdated?.(updated);
      selectedPresetId = id;
      presetSaveOpen = false;
      presetNameDraft = '';
      toasts.success(
        existing ? `Preset “${name}” updated` : `Preset “${name}” created`,
        'This repo now uses it',
      );
    } catch (e) {
      toastError('Couldn’t save preset', e);
    } finally {
      if (!disposed) configSaving = false;
    }
  }

  function addAgent(): void {
    const p = defaultAgentProvider();
    editAgents = [...editAgents, { name: 'New agent', provider: p, providers: [p], model: '', prompt: '' }];
  }

  async function removeAgent(i: number): Promise<void> {
    editAgents = editAgents.filter((_, idx) => idx !== i);
    await tick();
    const btns = agentsListEl?.querySelectorAll<HTMLButtonElement>('.cfg-remove');
    if (btns && btns.length > 0) {
      btns[Math.min(i, btns.length - 1)]?.focus();
    } else {
      addAgentBtnEl?.focus();
    }
  }

  async function persistPresets(presets: ReviewAgentCfg[]): Promise<void> {
    try {
      const synced = presets.map((a) => syncProviders(a, defaultAgentProvider()));
      const cfg: ReviewConfig =
        configScope === 'repo' && globalCfg
          ? { ...globalCfg, custom_presets: synced }
          : buildConfigFromEditState(presets);
      await api.put('/settings/pr-review', cfg);
      if (!disposed) globalCfg = cfg;
    } catch (e) {
      toastError('Couldn’t persist presets', e);
    }
  }

  async function saveAsPreset(i: number): Promise<void> {
    const agent = editAgents[i];
    const effectiveProviders =
      agent.providers && agent.providers.length > 0
        ? agent.providers
        : [agent.provider || defaultAgentProvider()];
    const copy: ReviewAgentCfg = {
      name: agent.name,
      provider: effectiveProviders[0],
      providers: effectiveProviders,
      model: agent.model,
      prompt: agent.prompt,
      ...(agent.skill ? { skill: agent.skill } : {}),
    };
    const existing = editPresets.findIndex((p) => p.name === copy.name);
    const next =
      existing >= 0
        ? editPresets.map((p, idx) => (idx === existing ? copy : p))
        : [...editPresets, copy];
    editPresets = next;
    await persistPresets(next);
    if (!disposed) toasts.success('Saved to presets');
  }

  function addPresetToAgents(preset: ReviewAgentCfg): void {
    editAgents = [...editAgents, { ...preset }];
  }

  async function removePreset(i: number): Promise<void> {
    const next = editPresets.filter((_, idx) => idx !== i);
    editPresets = next;
    // Restore focus before/after async persist so removing the chip never
    // drops focus to <body>.
    await tick();
    const delBtns = presetsListEl?.querySelectorAll<HTMLButtonElement>('.cfg-preset-del');
    if (delBtns && delBtns.length > 0) {
      delBtns[Math.min(i, delBtns.length - 1)]?.focus();
    } else {
      addPresetBtnEl?.focus();
    }
    await persistPresets(next);
  }

  function addPreset(p: ReviewerPreset): void {
    const prov = defaultAgentProvider();
    editAgents = [
      ...editAgents,
      { name: p.name, provider: prov, providers: [prov], model: '', prompt: `${p.focus} ${JSON_INSTR}` },
    ];
  }

  function openPresetMenu(e: MouseEvent): void {
    const taken = new Set(editAgents.map((a) => a.name));
    const lensPresets = skillPresets.length > 0 ? skillPresets : REVIEWER_PRESETS;
    const builtinItems = lensPresets.map((p) => ({
      label: p.name,
      disabled: taken.has(p.name),
      action: () => addPreset(p),
    }));
    const customItems = editPresets.map((p) => ({
      label: p.name,
      disabled: taken.has(p.name),
      action: () => addPresetToAgents(p),
    }));
    const items =
      customItems.length > 0
        ? [...builtinItems, { label: '', separator: true, action: () => {} }, ...customItems]
        : builtinItems;
    ctxMenu.show(e, items, { filter: items.length > 12, maxVisible: 12 });
  }

  async function saveConfig(): Promise<void> {
    configSaving = true;
    try {
      if (configScope === 'global') {
        const cfg = currentEditConfig(true);
        await api.put('/settings/pr-review', cfg);
        if (disposed) return;
        globalCfg = cfg;
        toasts.success('Review config saved', 'Applies to every repo without an override');
      } else {
        const cfg = currentEditConfig(false);
        const chosen = namedPresets.find((p) => p.id === selectedPresetId);
        const body: RepoReviewBinding =
          chosen && sameReviewConfig(cfg, chosen.config, defaultAgentProvider())
            ? { preset_id: chosen.id }
            : { config: cfg };
        const updated = await api.put<RepoReviewConfigResp>(`/repos/${repoId}/review-config`, body);
        if (disposed) return;
        repoCfg = updated;
        onupdated?.(updated);
        selectedPresetId = updated.preset_id ?? '';
        toasts.success(
          'Repo review config saved',
          updated.scope === 'preset'
            ? `Uses preset “${updated.preset_name}”`
            : 'Custom config for this repo only',
        );
      }
      onclose();
    } catch (e) {
      toastError('Couldn’t save config', e);
    } finally {
      if (!disposed) configSaving = false;
    }
  }
</script>

<Modal title="Configure review agents" width={580} dismissable={!configSaving} {onclose}>
  {#snippet children()}
    {#if configLoading}
      <Skeleton rows={3} height={36} label="review configuration" />
    {:else}
      <p class="dim cfg-note">
        Otto assesses the change first, selects relevant lenses and assigns one enabled provider per lens. The final verifier checks claims against the code.
      </p>

      <label class="cfg-note cfg-cross-label">
        <input type="checkbox" bind:checked={crossProviderCheck} />
        Cross-provider check — run selected lenses on every enabled provider
      </label>

      <!-- Scope: the global default vs. a config bound to THIS repository. -->
      <div class="segmented cfg-scope" role="group" aria-label="Review config scope">
        <button class:active={configScope === 'global'} aria-pressed={configScope === 'global'} onclick={() => setConfigScope('global')}>
          Global default
        </button>
        <button class:active={configScope === 'repo'} aria-pressed={configScope === 'repo'} onclick={() => setConfigScope('repo')}>
          This repository
        </button>
      </div>
      {#if configScope === 'global'}
        {#if repoCfg && repoCfg.scope !== 'global'}
          <p class="dim cfg-scope-note">
            Heads-up: this repo has its own config
            ({repoCfg.scope === 'preset' ? `preset “${repoCfg.preset_name ?? repoCfg.preset_id}”` : 'custom'})
            — reviews here ignore the global agents until you revert it under “This repository”.
          </p>
        {/if}
      {:else}
        <div class="cfg-repo-row">
          <select
            aria-label="Review preset"
            class="cfg-select cfg-preset-select"
            value={selectedPresetId}
            onchange={(e) => pickNamedPreset((e.currentTarget as HTMLSelectElement).value)}
          >
            <option value="">Custom — this repo only</option>
            {#each namedPresets as p (p.id)}
              <option value={p.id}>{p.name}</option>
            {/each}
          </select>
          <button class="btn small ghost" onclick={() => (presetSaveOpen = !presetSaveOpen)}>
            Save as preset…
          </button>
          {#if repoCfg && repoCfg.scope !== 'global'}
            <button class="btn small ghost" disabled={configSaving} onclick={useGlobalConfig}>
              Use global
            </button>
          {/if}
        </div>
        {#if presetSaveOpen}
          <div class="cfg-repo-row cfg-preset-save-row">
            <input
              dir="auto"
              aria-label="Preset name"
              class="cfg-input"
              bind:value={presetNameDraft}
              placeholder="Preset name (e.g. Backend services)"
              onkeydown={(e) => { if (e.key === 'Enter') void saveAsNamedPreset(); }}
            />
            <button
              class="btn small primary"
              disabled={configSaving || presetNameDraft.trim() === ''}
              onclick={saveAsNamedPreset}
            >
              Save &amp; use
            </button>
          </div>
        {/if}
        <p class="dim cfg-scope-note">
          {#if selectedPresetId === ''}
            These agents apply only to this repository.
          {:else}
            Editing a preset here saves the changes as a repo-only custom config —
            use “Save as preset…” with the same name to update the preset itself.
          {/if}
        </p>
      {/if}

      <h3 class="cfg-section-title">Review agents</h3>
      <div bind:this={agentsListEl}>
        {#each editAgents as agent, i (i)}
          {@const agentProviders = agent.providers && agent.providers.length > 0 ? agent.providers : [agent.provider || defaultAgentProvider()]}
          <div class="cfg-agent-row card">
            <div class="cfg-agent-fields">
              <div class="cfg-field">
                <span class="cfg-label">Name</span>
                <input dir="auto" aria-label="Reviewer name" class="cfg-input" bind:value={editAgents[i].name} />
              </div>
              <div class="cfg-field">
                <span class="cfg-label">Run on (CLIs)</span>
                <div class="cfg-provider-checks">
                  {#each PROVIDER_OPTIONS as p (p)}
                    <label class="cfg-check-label">
                      <input
                        type="checkbox"
                        checked={agentProviders.includes(p)}
                        onchange={() => toggleProvider(i, p)}
                      />
                      {p}
                    </label>
                  {/each}
                </div>
              </div>
              <div class="cfg-field">
                <span class="cfg-label">Lens / instructions</span>
                <textarea dir="auto" aria-label="Reviewer lens / instructions" class="cfg-textarea" rows={3} bind:value={editAgents[i].prompt}></textarea>
              </div>
            </div>
            <div class="cfg-agent-actions">
              <button
                class="btn small ghost cfg-save-preset"
                title="Save as preset"
                onclick={() => void saveAsPreset(i)}
              ><Icon name="star" size={12} /> Save as preset</button>
              <button
                class="icon-btn cfg-remove"
                onclick={() => void removeAgent(i)}
                aria-label="Remove agent {editAgents[i].name || i + 1}"
                title="Remove agent"
              ><Icon name="x" size={12} /></button>
            </div>
          </div>
        {/each}
      </div>
      <div class="cfg-add-row">
        <button class="btn small ghost cfg-add-btn" bind:this={addAgentBtnEl} onclick={addAgent}><Icon name="plus" size={12} /> Add agent</button>
        <button class="btn small ghost cfg-add-btn" bind:this={addPresetBtnEl} onclick={openPresetMenu}><Icon name="plus" size={12} /> Add preset <Icon name="chevronDown" size={12} /></button>
      </div>

      <!-- Your presets section -->
      {#if editPresets.length > 0}
        <h3 class="cfg-section-title cfg-presets-title">Your presets</h3>
        <div class="cfg-presets-list" bind:this={presetsListEl}>
          {#each editPresets as preset, i (i)}
            <div class="cfg-preset-pill">
              <span class="cfg-preset-name">{preset.name}</span>
              <button
                class="cfg-preset-action"
                title="Add to agents"
                onclick={() => addPresetToAgents(preset)}
                aria-label="Add preset {preset.name} to agents"
              ><Icon name="plus" size={12} /></button>
              <button
                class="cfg-preset-action cfg-preset-del"
                title="Delete preset"
                onclick={() => void removePreset(i)}
                aria-label="Delete preset {preset.name}"
              ><Icon name="x" size={12} /></button>
            </div>
          {/each}
        </div>
      {/if}

      <h3 class="cfg-section-title cfg-summ-title">Summarizer</h3>
      <div class="cfg-agent-row card">
        <div class="cfg-agent-fields">
          <div class="cfg-field">
            <span class="cfg-label">Name</span>
            <input dir="auto" aria-label="Summarizer name" class="cfg-input" bind:value={editSummarizer.name} />
          </div>
          <div class="cfg-field">
            <span class="cfg-label">Provider <span class="dim">(uses its default model)</span></span>
            <select aria-label="Summarizer provider" class="cfg-select" bind:value={editSummarizer.provider}>
              {#each PROVIDER_OPTIONS as p (p)}
                <option value={p}>{p}</option>
              {/each}
            </select>
          </div>
          <div class="cfg-field">
            <span class="cfg-label">Merge / dedupe instructions</span>
            <textarea dir="auto" aria-label="Summarizer merge / dedupe instructions" class="cfg-textarea" rows={3} bind:value={editSummarizer.prompt}></textarea>
          </div>
        </div>
      </div>
      <ReviewConfigAdvanced bind:value={editAdvanced} />
    {/if}
  {/snippet}
  {#snippet footer()}
    <button class="btn ghost" disabled={configSaving} onclick={onclose}>Cancel</button>
    <button class="btn primary" disabled={configSaving || configLoading} onclick={saveConfig}>
      {configSaving ? 'Saving…' : 'Save'}
    </button>
  {/snippet}
</Modal>

<style>
  .cfg-note {
    font-size: var(--fs-s);
    margin: 0 0 14px;
  }
  .cfg-cross-label {
    display: flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
  }
  .cfg-scope {
    margin-block-end: 10px;
  }
  .cfg-repo-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-block-end: 8px;
  }
  .cfg-preset-select {
    flex: 1;
    min-width: 0;
  }
  .cfg-preset-save-row .cfg-input {
    flex: 1;
    min-width: 0;
  }
  .cfg-scope-note {
    font-size: var(--fs-xs);
    margin: 0 0 14px;
  }
  .cfg-add-row {
    display: flex;
    gap: 8px;
  }
  .cfg-section-title {
    font-size: var(--fs-s);
    font-weight: 600;
    margin: 0 0 8px;
  }
  .cfg-summ-title {
    margin-block-start: 18px;
  }
  .cfg-agent-row {
    padding: 10px 12px;
    margin-block-end: 8px;
    display: flex;
    gap: 8px;
    align-items: flex-start;
  }
  .cfg-agent-fields {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .cfg-agent-actions {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex-shrink: 0;
    align-items: flex-end;
  }
  .cfg-save-preset {
    font-size: var(--fs-xs);
    padding: 2px 6px;
    white-space: nowrap;
  }
  .cfg-field {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .cfg-label {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-weight: 500;
  }
  .cfg-input,
  .cfg-select,
  .cfg-textarea {
    width: 100%;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-s);
    padding: 4px 8px;
    box-sizing: border-box;
  }
  .cfg-textarea {
    resize: vertical;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    line-height: 1.5;
  }
  .cfg-remove {
    flex-shrink: 0;
  }
  .cfg-provider-checks {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
    padding: 4px 0;
  }
  .cfg-check-label {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-s);
    cursor: pointer;
    user-select: none;
  }
  .cfg-presets-title {
    margin-block-start: 16px;
  }
  .cfg-presets-list {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-block-end: 4px;
  }
  .cfg-preset-pill {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding-block: 2px;
    padding-inline: 10px 8px;
    font-size: var(--fs-xs);
  }
  .cfg-preset-name {
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cfg-preset-action {
    background: none;
    border: none;
    cursor: pointer;
    padding: 0 2px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    line-height: 1;
    display: inline-flex;
    align-items: center;
  }
  .cfg-preset-action:hover {
    color: var(--text);
  }
  .cfg-preset-del:hover {
    color: var(--danger);
  }

  @media (max-width: 1024px) {
    .cfg-preset-action {
      min-width: var(--hit-min);
      min-height: var(--hit-min);
      display: inline-flex;
      align-items: center;
      justify-content: center;
    }
  }
  @media (max-width: 640px) {
    .cfg-agent-row { flex-direction: column; }
    .cfg-agent-actions {
      flex-direction: row;
      align-items: center;
      align-self: stretch;
    }
    .cfg-add-row { flex-wrap: wrap; }
    .cfg-add-btn { flex: 1 1 auto; }
    .cfg-check-label { padding: 4px 0; }
  }
</style>
