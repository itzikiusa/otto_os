<script lang="ts">
  // T3Code / PandaCode-style interactive control buttons for Model, Reasoning
  // Effort, and Execution Mode (Plan / Accept edits / Full access). Shared
  // between NewThreadStage and the live thread Composer.
  import { confirmer } from '../confirm.svelte';
  import { ctxMenu, type MenuItem } from '../contextmenu.svelte';
  import {
    effortsForModel,
    formatEffortLabel,
    formatEffortShort,
    formatInteractionModeLabel,
    formatModelLabel,
    INTERACTION_MODES,
    modelsForProvider,
    supportsEffortSelection,
    supportsInteractionMode,
    supportsModelSelection,
    type EffortLevel,
    type InteractionMode,
  } from '../providerModels';
  import { auth } from '../stores/auth.svelte';
  import Icon from './Icon.svelte';

  interface Props {
    provider: string;
    pinnedModel: string | null;
    effectiveModel?: string | null;
    effort: EffortLevel | null;
    interactionMode: InteractionMode;
    disabled?: boolean;
    /** Which pickers to show (Threads splits model/effort into the composer box
     *  and the access mode into the meta row). Default: all three. */
    parts?: readonly ('model' | 'effort' | 'mode')[];
    /** Borderless chips for use inside a composer box. */
    quiet?: boolean;
    onmodelchange: (model: string | null) => void;
    oneffortchange: (effort: EffortLevel | null) => void;
    onmodechange: (mode: InteractionMode) => void;
  }

  let {
    provider,
    pinnedModel,
    effectiveModel = null,
    effort,
    interactionMode,
    disabled = false,
    parts = ['model', 'effort', 'mode'],
    quiet = false,
    onmodelchange,
    oneffortchange,
    onmodechange,
  }: Props = $props();

  let modelBtnEl = $state<HTMLButtonElement | null>(null);
  let effortBtnEl = $state<HTMLButtonElement | null>(null);
  let modeBtnEl = $state<HTMLButtonElement | null>(null);

  const meta = $derived(auth.meta);
  const canPickModel = $derived(parts.includes('model') && supportsModelSelection(provider, meta));
  const canPickEffort = $derived(parts.includes('effort') && supportsEffortSelection(provider));
  const canPickMode = $derived(parts.includes('mode') && supportsInteractionMode(provider));

  const catalog = $derived(modelsForProvider(provider, meta));
  const shownModelId = $derived(pinnedModel ?? effectiveModel ?? null);
  const modelLabel = $derived(formatModelLabel(provider, shownModelId, meta));
  const supportedEfforts = $derived(effortsForModel(provider, shownModelId, meta));
  const effortLabel = $derived(formatEffortShort(effort));
  const modeLabel = $derived(formatInteractionModeLabel(interactionMode));

  async function promptCustomModel(): Promise<void> {
    const picked = await confirmer.promptText(
      'Enter an exact model ID or alias supported by this CLI harness.',
      {
        title: 'Custom model',
        confirmLabel: 'Use model',
        initial: pinnedModel ?? '',
        placeholder: provider === 'codex' ? 'e.g. gpt-5.4' : provider === 'agy' ? 'e.g. gemini-4-argon-high' : 'e.g. claude-opus-5-5',
        validate: (v) => (v.trim() ? null : 'Model ID is required'),
      },
    );
    if (picked !== null && picked.trim() !== '') {
      onmodelchange(picked.trim());
    }
  }

  export function openModelPicker(): void {
    if (!canPickModel || disabled) return;
    const items: MenuItem[] = [
      {
        label: 'Default model',
        hint: 'Provider default',
        checked: pinnedModel === null,
        pinned: true,
        action: () => onmodelchange(null),
      },
      { separator: true, pinned: true },
      ...catalog.map(
        (m): MenuItem => ({
          label: m.label,
          hint: m.id !== m.label ? m.id : (m.default_effort ? `${m.default_effort} default` : undefined),
          title: m.description || m.id,
          checked: pinnedModel === m.id || (pinnedModel === null && effectiveModel === m.id),
          action: () => onmodelchange(m.id),
        }),
      ),
      { separator: true, pinned: true },
      {
        label: 'Custom model…',
        icon: 'edit',
        pinned: true,
        action: () => void promptCustomModel(),
      },
    ];
    if (modelBtnEl) {
      ctxMenu.showAt(modelBtnEl, items, {
        filter: catalog.length > 6,
        filterPlaceholder: 'Filter models…',
      });
    }
  }

  export function openEffortPicker(): void {
    if (!canPickEffort || disabled) return;
    const items: MenuItem[] = [
      {
        label: 'Default effort',
        hint: 'Model default',
        checked: effort === null,
        action: () => oneffortchange(null),
      },
      { separator: true },
      ...supportedEfforts.map(
        (lvl): MenuItem => ({
          label: formatEffortLabel(lvl),
          hint: lvl,
          checked: effort === lvl,
          action: () => oneffortchange(lvl),
        }),
      ),
    ];
    if (effortBtnEl) {
      ctxMenu.showAt(effortBtnEl, items);
    }
  }

  export function openModePicker(): void {
    if (!canPickMode || disabled) return;
    const items: MenuItem[] = INTERACTION_MODES.map(
      (m): MenuItem => ({
        label: m.label,
        hint: m.id === 'plan' ? '⇧⇥' : undefined,
        title: m.hint,
        checked: interactionMode === m.id,
        action: () => onmodechange(m.id),
      }),
    );
    if (modeBtnEl) {
      ctxMenu.showAt(modeBtnEl, items);
    }
  }
</script>

<div class="tmb-bar" class:quiet role="group" aria-label="Model, reasoning effort, and execution mode">
  {#if canPickModel}
    <button
      bind:this={modelBtnEl}
      type="button"
      class="tmb-ctl"
      aria-haspopup="menu"
      aria-label="Model: {modelLabel}"
      title="Switch model (/model){shownModelId ? ` — ${shownModelId}` : ''}"
      data-testid="picker-model"
      {disabled}
      onclick={() => openModelPicker()}
    >
      <Icon name="sparkle" size={12} />
      <span class="tmb-label ellipsis">{modelLabel}</span>
      <Icon name="chevronDown" size={12} />
    </button>
  {/if}

  {#if canPickEffort}
    <button
      bind:this={effortBtnEl}
      type="button"
      class="tmb-ctl"
      class:tmb-active={effort !== null}
      aria-haspopup="menu"
      aria-label="Reasoning effort: {formatEffortLabel(effort)}"
      title="Reasoning effort (/effort): {formatEffortLabel(effort)}"
      data-testid="picker-effort"
      {disabled}
      onclick={() => openEffortPicker()}
    >
      <Icon name="gauge" size={12} />
      <span class="tmb-label">{effortLabel}</span>
      <Icon name="chevronDown" size={12} />
    </button>
  {/if}

  {#if canPickMode}
    <button
      bind:this={modeBtnEl}
      type="button"
      class="tmb-ctl"
      class:tmb-plan={interactionMode === 'plan'}
      class:tmb-edits={interactionMode === 'accept-edits'}
      aria-haspopup="menu"
      aria-label="Execution mode: {modeLabel}"
      title="Execution mode (/mode · ⇧⇥ toggles Plan): {modeLabel}"
      data-testid="picker-mode"
      {disabled}
      onclick={() => openModePicker()}
    >
      <Icon
        name={interactionMode === 'plan' ? 'compass' : interactionMode === 'accept-edits' ? 'shield' : 'zap'}
        size={12}
      />
      <span class="tmb-label">{modeLabel}</span>
      <Icon name="chevronDown" size={12} />
    </button>
  {/if}
</div>

<style>
  .tmb-bar {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    min-width: 0;
  }
  .tmb-ctl {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 24px;
    padding-inline: 8px 6px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-xs);
    font-weight: 500;
    cursor: pointer;
    min-width: 0;
    max-width: 200px;
    white-space: nowrap;
  }
  .tmb-ctl:hover:not(:disabled) {
    background: var(--hover);
    border-color: var(--accent-line);
  }
  .tmb-ctl:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  .tmb-ctl:disabled {
    opacity: var(--disabled-opacity);
    cursor: default;
  }
  .tmb-ctl.tmb-active {
    border-color: var(--accent-line);
    color: var(--accent-text);
    background: var(--accent-faint);
  }
  .tmb-ctl.tmb-plan {
    border-color: color-mix(in srgb, var(--warning) 48%, var(--border));
    background: var(--warning-soft);
    color: var(--warning);
    font-weight: 600;
  }
  .tmb-ctl.tmb-edits {
    border-color: var(--accent-line);
    background: var(--accent-faint);
    color: var(--accent-text);
  }
  .tmb-bar.quiet .tmb-ctl {
    border-color: transparent;
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .tmb-bar.quiet .tmb-ctl:hover:not(:disabled) {
    background: var(--hover);
    border-color: transparent;
    color: var(--text);
  }
  .tmb-label {
    min-width: 0;
  }
</style>
