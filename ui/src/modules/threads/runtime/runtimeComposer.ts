// What a runtime thread's composer does that is not view state (kept out of
// RuntimeThread.svelte): the slash commands that act on the view (through
// the host's commands, hostCommands.ts), merging an edited queued message
// into the draft, and reading pasted / attached files into attachments.
import type { RuntimeHostCommands } from '../../../lib/runtimeThreadPort';
import type { ParsedSlashIntent } from '../../../lib/slashActions';
import { toastError } from '../../../lib/toastError';
import { runHostSlash } from './hostCommands';

/** Per file (images and PDFs). */
const MAX_ATTACHMENT_BYTES = 10 * 1024 * 1024;

import { runtimeDrafts, type ComposerFile } from '../../../lib/stores/runtimeDrafts.svelte';
export type { ComposerFile } from '../../../lib/stores/runtimeDrafts.svelte';

/** Read the files `takes` accepts as base64 attachments (oversized ones are
 *  refused with a toast). */
export async function readAttachments(files: Iterable<File>, takes: (f: File) => boolean): Promise<ComposerFile[]> {
  const out: ComposerFile[] = [];
  for (const f of files) {
    if (!takes(f)) continue;
    if (f.size > MAX_ATTACHMENT_BYTES) {
      toastError('File too large', new Error(`${f.name} is over 10 MB`));
      continue;
    }
    const data = new Uint8Array(await f.arrayBuffer());
    let bin = '';
    for (let i = 0; i < data.length; i += 0x8000) bin += String.fromCharCode(...data.subarray(i, i + 0x8000));
    out.push({ name: f.name || 'pasted image', mime: f.type, data_base64: btoa(bin) });
  }
  return out;
}

/** A slash command that acts on the view; `false` = send it to the agent. */
export function runViewSlash(intent: ParsedSlashIntent, sessionId: string, host: RuntimeHostCommands | undefined): boolean {
  if (intent.kind !== 'fresh-context') return runHostSlash(intent, sessionId, host);
  runtimeDrafts.for(sessionId).freshContext = true;
  return true;
}

/** "Edit" on a queued message puts it back in the composer without losing
 *  what is being typed there: unsent text stays, the message goes below it. */
export function mergeQueuedEdit(current: string, queued: string): string {
  const kept = current.replace(/\s+$/, '');
  return kept.trim() ? `${kept}\n${queued}` : queued;
}
