// What a runtime thread's composer does that is not view state (kept out of
// RuntimeThread.svelte): the slash commands that act on the Threads view,
// and reading pasted / attached files into prompt attachments.
import { harnessLabel } from '../../../lib/providers';
import type { ParsedSlashIntent } from '../../../lib/slashActions';
import type { RuntimeSession } from '../../../lib/api/types';
import { toastError } from '../../../lib/toastError';
import { threads } from '../threadsStore.svelte';
import { runtimeThreads } from './runtimeThreads.svelte';

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
export function runViewSlash(intent: ParsedSlashIntent, sessionId: string, session: RuntimeSession | null): boolean {
  switch (intent.kind) {
    case 'open-diff':
      threads.setInspectorTab('changes');
      return true;
    case 'open-files':
      threads.setInspectorTab('files');
      return true;
    case 'toggle-terminal':
      threads.toggleTerminal(sessionId);
      return true;
    case 'fresh-context':
      runtimeDrafts.for(sessionId).freshContext = true;
      return true;
    case 'new-thread':
      void newThread(session, intent.beside);
      return true;
    case 'side-chat':
      threads.openSideChat(sessionId, intent.question);
      return true;
    default:
      return false;
  }
}

async function newThread(session: RuntimeSession | null, beside: boolean): Promise<void> {
  if (!session) return;
  if (!beside) return threads.startNewThread({ workspaceId: session.workspace_id, cwd: session.cwd });
  try {
    const { provider, cwd, model } = session;
    const created = await runtimeThreads.start(session.workspace_id, {
      provider,
      cwd,
      title: `${harnessLabel(provider)} thread`,
      model: model ?? null,
      prompt: '',
    });
    threads.openBeside(created.id);
  } catch (e) {
    toastError('Couldn’t start thread', e);
  }
}
