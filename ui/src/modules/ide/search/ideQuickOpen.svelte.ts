// Entry points of the IDE quick-open (IdeSearchEverywhere.svelte). One picker serves
// ⇧⇧ (All), ⌘P (Files), ⌘E (recent files), ⌘G (`:` line mode), ⌘O (Symbols) and
// Find / Replace in path. A request carries the starting query; the panel reads it
// on mount and again whenever `seq` moves while it is already open (⌘G pressed with
// the picker up retargets it instead of stacking a second one).

import { ide } from '../ideStore.svelte';
import type { QuickOpenTab } from './ideQuickOpenModel';

class QuickOpenRequest {
  seq = $state(0);
  query = $state('');
}

export const quickOpenRequest = new QuickOpenRequest();

function openQuickOpen(tab: QuickOpenTab, query = ''): void {
  quickOpenRequest.query = query;
  quickOpenRequest.seq += 1;
  ide.openSearchEverywhere(tab, { scopePath: null, replace: false });
}

/** ⇧⇧: everything — files, symbols, text and actions. */
export function openSearchEverywhereAll(): void {
  openQuickOpen('all');
}

/** ⌘P (⌘⇧O in JetBrains): files; an empty query lists recent files. */
export function openGoToFile(): void {
  openQuickOpen('files');
}

/** ⌘E (⌃R in Standard): the same picker on Files with an empty query = recent files. */
export function openRecentFiles(): void {
  openQuickOpen('files');
}

/** ⌘O (⌘⇧O in Standard): symbols. */
export function openGoToSymbol(): void {
  openQuickOpen('symbols');
}

/** ⌘G (⌃G in Standard) and the status strip's `Ln, Col`: the picker in `:` mode —
 *  type `42` or `42:7` and ⏎ to move the cursor in the active file. */
export function openGoToLine(): void {
  if (!ide.activePath) return;
  openQuickOpen('files', ':');
}
