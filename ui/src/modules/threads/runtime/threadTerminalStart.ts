import type { RuntimeSession, Session } from '../../../lib/api/types';
import { SCRATCH_WORKSPACE_ID, ws } from '../../../lib/stores/workspace.svelte';

// Shared across pane instances: changing layout or A → B → A while a request
// is pending must not create another shell for A. Failures release the slot.
const starting = new Map<string, Promise<Session>>();

export function startThreadTerminal(runtime: RuntimeSession): Promise<Session> {
  const pending = starting.get(runtime.id);
  if (pending) return pending;
  const scratch = runtime.workspace_id === SCRATCH_WORKSPACE_ID && ws.currentId !== SCRATCH_WORKSPACE_ID;
  const attempt = ws.createSessionQuiet({
    kind: 'agent', provider: 'shell', title: `Terminal · ${runtime.title}`, cwd: runtime.cwd,
    meta: { surface: 'threads', threads_terminal: true, terminal_of: runtime.id },
  }, { scratch }).finally(() => { starting.delete(runtime.id); });
  starting.set(runtime.id, attempt);
  return attempt;
}
