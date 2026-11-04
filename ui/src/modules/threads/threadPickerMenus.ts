// The meta row's access-mode menu. (Harness, model and effort live in the one
// ModelPickerPopover panel behind the composer's agent chip.)
import type { MenuItem } from '../../lib/contextmenu.svelte';
import { INTERACTION_MODES, type InteractionMode } from '../../lib/providerModels';

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
