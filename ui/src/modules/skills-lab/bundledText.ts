// The words of the Skills Lab bundled-skill actions (see bundledActions.ts),
// kept free of API / store imports so unit tests can load them in plain Node.

import type { InstallBundledResp } from '../../lib/api/types';

/** Where the daemon keeps backups and deleted skills, as the person sees it. */
export const BACKUP_FOLDER = '<data dir>/library/skills-backup/';

/** The confirm for putting the bundled copy over a library copy, worded by
 *  its state; `null` = nothing to replace (first install, no question). */
export function replaceQuestion(
  name: string,
  state: string | undefined,
  version: number | undefined,
): [string, { title: string; confirmLabel: string; danger: boolean }] | null {
  if (!state || state === 'not_installed') return null;
  const v = version != null ? ` v${version}` : '';
  if (state === 'ahead' || state === 'modified') {
    return [
      `Your library copy of “${name}” was edited${state === 'ahead' ? ' and is ahead of the bundled version' : ' since Otto installed it'}. ` +
        `Replacing backs it up to ${BACKUP_FOLDER} first, then puts bundled${v} in its place. Cancel keeps your edited copy.`,
      { title: 'Replace your edited skill?', confirmLabel: 'Back up & replace', danger: true },
    ];
  }
  return [
    `Replace the library copy of “${name}” with bundled${v}? Your current copy is backed up to ${BACKUP_FOLDER} first.`,
    { title: 'Update from bundled', confirmLabel: 'Update', danger: false },
  ];
}

/** The success toast body for an install / update. */
export function installedNote(r: Pick<InstallBundledResp, 'backed_up' | 'backup_path'>, updating: boolean): string {
  if (r.backed_up) return r.backup_path ? `The previous copy was saved to ${r.backup_path}.` : 'The previous copy was backed up.';
  return updating ? 'Updated from bundled.' : 'You can edit it now.';
}

/** The confirm for deleting a library skill: where it goes, what else goes. */
export function deleteQuestion(name: string): [string, { title: string; confirmLabel: string; danger: boolean }] {
  return [
    `Delete “${name}” from the Otto library? Its folder moves to ${BACKUP_FOLDER} (the newest three per skill are kept), so you can put it back by hand. ` +
      'The copies Otto installed for Claude, Codex and Antigravity are removed too; skills you made yourself in those folders are not touched.',
    { title: 'Delete skill', confirmLabel: 'Delete', danger: true },
  ];
}
