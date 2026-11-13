// Skills Lab → Skills → Bundled: the bundled-catalogue actions that used to
// live in Settings → Skills — the ONE "install / update / replace from
// bundled" question, the result notes, the delete question and "Install all".
// Installs are root-only server-side and always back up an existing copy
// (an edited one even when a caller asks not to).

import { contextApi } from '../../lib/api/context';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { toastError } from '../../lib/toastError';
import { plural } from '../../lib/plural';
import type { KeptProviderCopy } from '../../lib/api/types';
import { BACKUP_FOLDER, keptCopiesNote } from './bundledText';

export { BACKUP_FOLDER, deleteQuestion, installedNote, replaceQuestion } from './bundledText';

/** An agent CLI already had its own skill of that name: Otto left it alone.
 *  `kept`: Otto's own copies there that the person had edited, moved aside. */
export function noteUserOwned(paths: string[] | undefined, kept?: KeptProviderCopy[]): void {
  noteKeptCopies(kept);
  if (!paths || paths.length === 0) return;
  toasts.info(
    'Kept your own skills',
    `These agent CLI skill folders already hold a skill you own, so Otto did not replace them:\n${paths.join('\n')}`,
  );
}

/** Say where an edited agent-CLI copy went (after a save, install or delete). */
export function noteKeptCopies(kept: KeptProviderCopy[] | undefined): void {
  const note = keptCopiesNote(kept);
  if (note) toasts.info(...note);
}

/** "Install all bundled skills" (optionally one category). Asks first, skips
 *  current and edited copies, names failures. Resolves true when it ran. */
export async function installAllBundled(category: string | null): Promise<boolean> {
  const what = category ? `bundled ${category} skills` : 'bundled skills';
  const ok = await confirmer.ask(
    `Install every ${what} that is missing from your library and update the ones with a newer bundled version. ` +
      `Existing copies are backed up to ${BACKUP_FOLDER} first; copies you edited are left as they are.`,
    { title: `Install all ${what}`, confirmLabel: 'Install all', danger: false },
  );
  if (!ok) return false;
  try {
    const r = await contextApi.installAllBundled(category ?? undefined);
    const n = r.installed.length;
    const failed = r.failed ?? [];
    const edited = r.modified ?? [];
    if (failed.length > 0) {
      toasts.error(
        `${plural(failed.length, 'skill')} didn’t install`,
        failed.map((f) => `${f.name}: ${f.error}`).join('\n') + (n > 0 ? `\nInstalled ${plural(n, 'other')}.` : ''),
      );
    } else if (n === 0) {
      toasts.info('Nothing to install', `Every ${what.replace('bundled ', '')} in your library is current or carries your own edits.`);
    } else {
      const b = r.backed_up.length;
      toasts.success(`Installed ${plural(n, 'skill')}`, b > 0 ? `${b} existing cop${b === 1 ? 'y was' : 'ies were'} backed up first.` : undefined);
    }
    if (edited.length > 0) {
      toasts.info(
        `Kept ${plural(edited.length, 'edited skill')}`,
        `You changed these since Otto installed them, so Install all left them alone. Open one and use Replace with bundled to update it:\n${edited.join('\n')}`,
      );
    }
    noteUserOwned(r.user_owned, r.kept_copies);
    return true;
  } catch (e) {
    toastError(`Couldn’t install the ${what}`, e);
    return true;
  }
}
