// Menus behind the new-thread composer's one agent chip (Panda-style: harness,
// model and effort live in a single picker instead of a row of cards + three
// pills) and the meta row's access-mode chip. Pure builders over ctxMenu
// items, so the stage stays a layout file.
import type { MetaResp } from '../../lib/api/types';
import { confirmer } from '../../lib/confirm.svelte';
import type { MenuItem } from '../../lib/contextmenu.svelte';
import {
  effortsForModel,
  formatEffortLabel,
  INTERACTION_MODES,
  modelsForProvider,
  supportsEffortSelection,
  supportsModelSelection,
  type EffortLevel,
  type InteractionMode,
} from '../../lib/providerModels';
import { providerReadiness } from '../../lib/providers';
import { harnessLabel } from './threadsHelpers';

type Meta = Pick<MetaResp, 'provider_models' | 'model_flags'> | null | undefined;

/** Agent chip: every harness (checked = current), then the model / effort
 *  sub-pickers for the current one, then the rarely used launch options. */
export function buildAgentMenu(opts: {
  harnesses: readonly string[];
  provider: string;
  modelLabel: string;
  effort: EffortLevel | null;
  meta: Meta;
  openBeside: boolean | null;
  onProvider: (p: string) => void;
  onOpenModel: () => void;
  onOpenEffort: () => void;
  onToggleBeside: () => void;
  onEmpty: () => void;
}): MenuItem[] {
  const items: MenuItem[] = opts.harnesses.map((h): MenuItem => {
    const ready = providerReadiness(h);
    // Panda 0.3.84: a harness that can't run is dimmed with the reason, not pickable.
    return {
      label: harnessLabel(h),
      hint: ready.available ? undefined : 'Not installed',
      title: ready.available ? undefined : ready.message,
      disabled: !ready.available && opts.provider !== h,
      checked: opts.provider === h,
      action: () => opts.onProvider(h),
    };
  });
  const tuning: MenuItem[] = [];
  if (supportsModelSelection(opts.provider, opts.meta)) {
    tuning.push({ label: 'Model…', icon: 'sparkle', hint: opts.modelLabel, action: opts.onOpenModel });
  }
  if (supportsEffortSelection(opts.provider)) {
    tuning.push({ label: 'Reasoning effort…', icon: 'gauge', hint: formatEffortLabel(opts.effort), action: opts.onOpenEffort });
  }
  if (tuning.length > 0) items.push({ separator: true }, ...tuning);
  items.push({ separator: true });
  if (opts.openBeside !== null) {
    items.push({ label: 'Open beside current thread', checked: opts.openBeside, action: opts.onToggleBeside });
  }
  items.push({ label: 'Start without a prompt', icon: 'play', action: opts.onEmpty });
  return items;
}

export function buildModelMenu(opts: {
  provider: string;
  pinnedModel: string | null;
  meta: Meta;
  onModel: (m: string | null) => void;
}): MenuItem[] {
  const catalog = modelsForProvider(opts.provider, opts.meta);
  return [
    { label: 'Default model', hint: 'Provider default', checked: opts.pinnedModel === null, pinned: true, action: () => opts.onModel(null) },
    { separator: true, pinned: true },
    ...catalog.map(
      (m): MenuItem => ({
        label: m.label,
        hint: m.id !== m.label ? m.id : undefined,
        title: m.description || m.id,
        checked: opts.pinnedModel === m.id,
        action: () => opts.onModel(m.id),
      }),
    ),
    { separator: true, pinned: true },
    {
      label: 'Custom model…',
      icon: 'edit',
      pinned: true,
      action: () => {
        void confirmer
          .promptText('Enter an exact model ID or alias supported by this CLI harness.', {
            title: 'Custom model',
            confirmLabel: 'Use model',
            initial: opts.pinnedModel ?? '',
            placeholder: 'Model ID',
            validate: (v) => (v.trim() ? null : 'Model ID is required'),
          })
          .then((picked) => {
            if (picked && picked.trim()) opts.onModel(picked.trim());
          });
      },
    },
  ];
}

export function buildEffortMenu(opts: {
  provider: string;
  model: string | null;
  effort: EffortLevel | null;
  meta: Meta;
  onEffort: (e: EffortLevel | null) => void;
}): MenuItem[] {
  return [
    { label: 'Default effort', hint: 'Model default', checked: opts.effort === null, action: () => opts.onEffort(null) },
    { separator: true },
    ...effortsForModel(opts.provider, opts.model, opts.meta).map(
      (lvl): MenuItem => ({
        label: formatEffortLabel(lvl),
        checked: opts.effort === lvl,
        action: () => opts.onEffort(lvl),
      }),
    ),
  ];
}

export function buildModeMenu(mode: InteractionMode, onMode: (m: InteractionMode) => void): MenuItem[] {
  return INTERACTION_MODES.map(
    (m): MenuItem => ({
      label: m.label,
      hint: m.id === 'plan' ? '⇧⇥' : undefined,
      title: m.hint,
      checked: mode === m.id,
      action: () => onMode(m.id),
    }),
  );
}
