// Guards the Workbench page runs before replacing what a person typed or
// named. Kept free of runtime imports (the store and the confirm dialog are
// passed in) so the unit tests load the file directly.

/** The slice of the workbench store `readyToRestore` reads. */
export interface RestoreGuardStore {
  isDirty(id: string): boolean;
  save(id: string, checkpoint: boolean): Promise<void>;
  open: Record<string, { saving: boolean; remoteChanged: boolean; saveError: string | null } | undefined>;
}

export type AskFn = (
  body: string,
  opts: { title: string; confirmLabel: string; cancelLabel?: string; danger: boolean },
) => Promise<boolean>;

const SETTLE_POLL_MS = 50;
const SETTLE_LIMIT_MS = 15_000;
const pause = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/** Before History restores revision `seq` over the open tab: true when nothing
 *  unsaved would be lost. Unsaved edits are saved first (a checkpoint, so they
 *  really are in history, as the restore dialog promises) and the save — plus a
 *  queued re-save — is awaited. If they are STILL unsaved (save held by the
 *  remote-change banner, refused, or failed) the person is asked, in plain
 *  words, whether those edits may be discarded; the default is to cancel. */
export async function readyToRestore(
  store: RestoreGuardStore,
  id: string,
  seq: number,
  ask: AskFn,
  wait: (ms: number) => Promise<void> = pause,
): Promise<boolean> {
  if (!store.isDirty(id)) return true;
  if (!store.open[id]?.remoteChanged) {
    await store.save(id, true);
    // `save` returns at once when another save is in flight (it queues a
    // re-save); wait for the tab to settle before deciding.
    for (let waited = 0; store.open[id]?.saving && waited < SETTLE_LIMIT_MS; waited += SETTLE_POLL_MS) {
      await wait(SETTLE_POLL_MS);
    }
  }
  if (!store.isDirty(id)) return true;
  const o = store.open[id];
  const why = o?.remoteChanged
    ? 'Saving is paused because the file changed in another window'
    : o?.saveError
      ? `Saving failed (${o.saveError})`
      : 'They could not be saved';
  return ask(
    `${why}, so the edits you made in this tab are NOT in history. ` +
      `Restoring revision ${seq} replaces them, and they cannot be recovered. ` +
      'Cancel to keep them (copy them somewhere, or resolve the save first).',
    { title: 'Discard unsaved edits?', confirmLabel: 'Discard edits and restore', cancelLabel: 'Keep my edits', danger: true },
  );
}

/** Why a rename typed into the prompt can no longer be applied, or null when
 *  it can: the file is gone, or someone renamed it while the prompt was open
 *  (its metadata object is replaced on every autosave, so identity says
 *  nothing — the NAME is what the prompt started from). */
export function renameBlocked(now: { name: string } | undefined, startedFrom: string): string | null {
  if (!now) return 'The file was deleted or moved while you were renaming it.';
  if (now.name !== startedFrom) return `It was renamed to “${now.name}” while you were typing. Rename it again if you still want to.`;
  return null;
}
