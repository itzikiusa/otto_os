// Guards for writes that must not surprise the person: workspace-wide writes (Replace in
// Path, Rename symbol) that would replace unsaved edits, and a format request the daemon
// skipped on purpose. Split from `ideLspAndSearchHelpers.ts` to keep it under the size cap.

import { plural } from '../../../lib/plural';
import { toasts } from '../../../lib/toast.svelte';
import type { IdeBufferPerfController } from '../store/ideBufferPerf';
import type { IdeOpenTab } from '../store/ideTypes';

/** Open tabs inside `paths` (every open tab when null) that still hold unsaved edits after
 *  the pre-operation save: their save failed or is in conflict. A workspace write over
 *  them would replace those edits, so the caller skips them or stops. */
export function stillDirtyTabsIn(
  host: { tabs: IdeOpenTab[]; resolvePath(p: string): string },
  bufPerf: Pick<IdeBufferPerfController, 'isTabDirty'>,
  paths: Iterable<string> | null,
): IdeOpenTab[] {
  const within = paths ? new Set([...paths].map((p) => host.resolvePath(p))) : null;
  return host.tabs.filter((t) => !t.readOnly && bufPerf.isTabDirty(t) && (!within || within.has(t.path)));
}

export function namesOf(tabs: { name: string }[]): string {
  const shown = tabs.slice(0, 3).map((t) => `“${t.name}”`).join(', ');
  return tabs.length > 3 ? `${shown} and ${tabs.length - 3} more` : shown;
}

export function toastKeptTabs(kept: string[]): void {
  if (kept.length === 0) return;
  toasts.info(
    `Kept your unsaved edits in ${plural(kept.length, 'open file')}`,
    'They changed on disk too: saving will ask whether to overwrite or reload.',
  );
}

/** What `/ide/lsp/format` answers when it skipped formatting on purpose (a project whose
 *  Prettier config runs code): `formatter` is the message ("Formatting skipped: … Allow it
 *  in …"), `changed` false. Also accepts explicit `skipped` / `message` / `hint` fields. */
export interface IdeFormatAnswer { changed?: boolean; formatter?: string; skipped?: boolean; message?: string; hint?: string }

const formatSkipShown = new Set<string>();

/** True when the answer is a deliberate skip: the buffer stays as it is, it is not an
 *  error, and the message (with the hint it carries) shows once per file, quietly. */
export function noteFormatSkipped(path: string, resp: IdeFormatAnswer): boolean {
  const text = (resp.message ?? resp.formatter ?? '').trim();
  if (!resp.skipped && !/^formatting skipped/i.test(text)) return false;
  if (formatSkipShown.has(path)) return true;
  formatSkipShown.add(path);
  const cut = text.indexOf('. ');
  const title = resp.hint || cut < 0 ? text : text.slice(0, cut + 1);
  const hint = resp.hint ?? (cut < 0 ? undefined : text.slice(cut + 2));
  toasts.info(title || 'Formatting skipped', hint);
  return true;
}
