// "Otto edited · Undo" (patterns.md §7: prefer an undo over a confirm). Every
// Ask Otto commit leaves the scene's previous document as an `agent` version
// (GET /canvas/scenes/{id}/versions, newest first), so Undo restores the
// newest one through the same store call the Versions picker uses — and the
// restore itself is kept as a version, so the undo is undoable too.
import { canvas } from '../../lib/stores/canvas.svelte';
import { toasts } from '../../lib/toast.svelte';

/** Toast an agent edit of `sceneId` with an Undo action. */
export function toastAgentEdit(sceneId: string | null, title: string, body?: string): void {
  if (!sceneId) {
    toasts.success(title, body);
    return;
  }
  toasts.push('success', title, body, 8000, { action: { label: 'Undo', run: () => undoAgentEdit(sceneId) } });
}

async function undoAgentEdit(sceneId: string): Promise<void> {
  const before = (await canvas.listVersions(sceneId)).find((v) => v.origin === 'agent');
  if (!before) {
    toasts.info('Nothing to undo', 'This scene has no saved version from before Otto’s edit.');
    return;
  }
  await canvas.restoreVersion(sceneId, before.id);
  toasts.success('Undid Otto’s edit', 'The scene is back to its version from before the edit.');
}
