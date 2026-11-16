// The meta row's access-mode menu. (Harness, model and effort live in the one
// ModelPickerPopover panel behind the composer's agent chip.)
import type { MenuItem } from '../../lib/contextmenu.svelte';
import { INTERACTION_MODES, formatInteractionModeLabel, type InteractionMode } from '../../lib/providerModels';

/** A terminal agent's mode in the access chip's words (RuntimeAccessChip):
 *  "Auto-accept edits", not "Accept edits". */
export function modeLabel(m: InteractionMode): string {
  return m === 'accept-edits' ? 'Auto-accept edits' : formatInteractionModeLabel(m);
}

export function buildModeMenu(mode: InteractionMode, onMode: (m: InteractionMode) => void): MenuItem[] {
  return INTERACTION_MODES.map(
    (m): MenuItem => ({
      label: modeLabel(m.id),
      hint: m.id === 'plan' ? '⇧⇥' : undefined,
      title: m.hint,
      checked: mode === m.id,
      action: () => onMode(m.id),
    }),
  );
}
