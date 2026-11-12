// Shell dialogs that open only from a menu or a click, loaded on first open
// (same pattern as overlays.svelte.ts: ONE module-level `lazyComponent`
// rendered through `<LazyMount quiet onerror>`, which resets the open flag
// when the chunk fails so the next open retries).
import { lazyComponent } from '../lib/lazy-component.svelte';

/** Navigator ▸ workspace menu ▸ "Removed workspaces…". */
export const RemovedWorkspacesChunk = lazyComponent(() => import('./RemovedWorkspaces.svelte'));
/** The status-bar / More-sheet machine chip's details dialog. */
export const MachineDetailsChunk = lazyComponent(() => import('./MachineDetails.svelte'));
