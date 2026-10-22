// The shell's one-shot global installers (architecture-2-05), installed once
// from App.svelte's onMount instead of as loose `$effect`s in the app root —
// where one stray reactive read used to re-run boot code (c14b1d5e). Returns
// one teardown for all of them (sign-out unmounts the shell).

import { badgeWriter, tauriBadgeSink } from '../lib/dockBadge';
import { isEmbedded } from '../lib/desktop';
import { isTauri } from '../lib/stores/ui.svelte';
import { ws } from '../lib/stores/workspace.svelte';
import { installPaletteSources, type PaletteDeps } from './paletteSources.svelte';

export function installShell(deps: PaletteDeps): () => void {
  const stopPalette = installPaletteSources(deps);
  const stopBadge = $effect.root(() => {
    // Dock badge = sessions waiting on you (A1). Once per window; the shell
    // command itself only honours the main window. Unmount (sign-out) → 0.
    if (!isTauri || isEmbedded) return;
    const badge = badgeWriter(tauriBadgeSink);
    $effect(() => badge.set(ws.needsYouCount));
    return () => badge.flush(0);
  });
  return () => {
    stopBadge();
    stopPalette();
  };
}
