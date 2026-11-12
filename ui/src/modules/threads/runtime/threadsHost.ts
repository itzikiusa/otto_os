// Threads as a host of the runtime conversation (lib/runtimeThreadPort.ts):
// its view-level slash commands act on the Threads inspector, terminal
// drawer, side chat and panes. Other hosts supply their own handlers.
import { harnessLabel } from '../../../lib/providers';
import type { RuntimeHostCommands } from '../../../lib/runtimeThreadPort';
import { toastError } from '../../../lib/toastError';
import { threads } from '../threadsStore.svelte';
import { runtimeThreads } from './runtimeThreads.svelte';

export const threadsHostCommands: RuntimeHostCommands = {
  openDiff: () => threads.setInspectorTab('changes'),
  openFiles: () => threads.setInspectorTab('files'),
  openTerminal: (id) => threads.toggleTerminal(id),
  newThread: (id, beside) => void newThread(id, beside),
  sideChat: (id, question) => threads.openSideChat(id, question),
};

async function newThread(id: string, beside: boolean): Promise<void> {
  const session = runtimeThreads.get(id);
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
