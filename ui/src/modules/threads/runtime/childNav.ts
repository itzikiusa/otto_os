// Opening a subagent — its row, the waiting line, the details' Subagents
// section, the sidebar — goes through here (T3 Code's useSideThreadTrigger,
// MIT, Copyright (c) 2026 T3 Tools Inc.): a tab in the right panel while the
// parent stays put; `main` (⌘/Ctrl-click) opens it in the main view
// instead. One per-device setting (Settings in the subagent's tab) flips the
// default. A phone's right panel is a full-height sheet with Back.
import { router } from '../../../lib/router.svelte';
import { threads } from '../threadsStore.svelte';
import { threadsChrome } from '../threadsChrome.svelte';

/** Where a click opens a subagent: the setting, flipped by ⌘/Ctrl-click. */
export function opensInMain(modifier: boolean, setting: 'panel' | 'main'): boolean {
  return (setting === 'main') !== modifier;
}

export function openChildThread(id: string, modifier = false): void {
  if (router.module !== 'threads') {
    router.go(`threads/${id}`);
    return;
  }
  if (opensInMain(modifier, threadsChrome.subagentsIn)) {
    threads.selectThread(id);
    return;
  }
  threadsChrome.subagentId = id;
  threads.setInspectorTab('subagent');
}

/** A click's "open elsewhere" modifier. */
export const mainViewKey = (e: MouseEvent | KeyboardEvent): boolean => e.metaKey || e.ctrlKey;
