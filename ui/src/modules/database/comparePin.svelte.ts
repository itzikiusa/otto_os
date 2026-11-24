// The "Compare results" source (ConnectionComparison.svelte): module state, so
// the pin survives leaving the workbench for the connection list to open the
// other side. Toggled from the workbench header's ⋯ and the ⌘K command.
import { database } from '../../lib/stores/database.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { ws } from '../../lib/stores/workspace.svelte';

export const comparePin = $state<{ value: { connId: string; tabId: number; scope: string } | null }>({ value: null });

/** A pin is only valid for the identity / access it was made under. */
export function compareScope(): string {
  return JSON.stringify([ws.currentId, auth.me?.id, database.accessRevision]);
}

/** Pin the active query tab's results beside the editor, or clear the pin. */
export function toggleCompare(): void {
  if (comparePin.value) {
    comparePin.value = null;
    return;
  }
  const tab = database.tab;
  if (!database.selectedConnId || !tab) return;
  database.setMainTab('query');
  comparePin.value = { connId: database.selectedConnId, tabId: tab.id, scope: compareScope() };
}
