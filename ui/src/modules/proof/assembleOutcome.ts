// What to tell the person after `POST /proof-packs/{id}/assemble`
// (error-handling-6-02): the daemon reports what it assembled, so the page
// never says "Proof assembled" for a folder that isn't a repository.
import type { AssembleSummary } from '../../lib/api/types/proof';
import { plural } from '../../lib/plural.ts';

export interface AssembleOutcome {
  tone: 'success' | 'info' | 'error';
  title: string;
  body: string;
}

export function assembleOutcome(a: AssembleSummary | null | undefined): AssembleOutcome {
  const failedCmds = (a?.commands ?? []).filter((c) => c.error);
  const cmdNote = failedCmds.length
    ? ` Couldn’t record ${failedCmds.map((c) => `“${c.cmd}” (${c.error})`).join(', ')}.`
    : '';
  const diff = a?.diff;
  if (!diff) {
    return failedCmds.length
      ? { tone: 'error', title: 'Proof not fully assembled', body: cmdNote.trim() }
      : { tone: 'info', title: 'Proof rechecked', body: 'No working folder was given, so nothing new was assembled.' };
  }
  switch (diff.status) {
    case 'not_a_repo':
      return { tone: 'info', title: 'Nothing to assemble', body: `No git repository at that folder — nothing to assemble.${cmdNote}` };
    case 'diff_failed':
      return { tone: 'error', title: 'Couldn’t assemble proof', body: `These git commands failed: ${diff.commands.join('; ')}.${cmdNote}` };
    case 'no_changes':
      return { tone: failedCmds.length ? 'error' : 'info', title: 'No changes to assemble', body: `The working tree matches its base, so no diff was recorded.${cmdNote}` };
    case 'assembled':
      return {
        tone: failedCmds.length ? 'error' : 'success',
        title: 'Proof assembled',
        body: `Recorded the diff of ${plural(diff.files_changed, 'changed file')}.${cmdNote}`,
      };
  }
}
