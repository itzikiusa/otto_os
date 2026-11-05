// Open the session a background run drove — a personal agent's run is a
// Threads thread on a structured-runtime provider (`#/threads/<id>`), else a
// terminal session (`#/agents/<id>`). The run row only stores the id, so ask
// the runtime first: a thread id is not a terminal session.
import { api } from './api/client';
import type { RuntimeSession } from './api/types';
import { router } from './router.svelte';
import { ws } from './stores/workspace.svelte';

export async function openAgentSession(id: string): Promise<void> {
  if (!id) return;
  try {
    await api.get<RuntimeSession>(`/runtime-sessions/${id}`);
  } catch {
    ws.navigateToSession(id);
    return;
  }
  router.go(`threads/${id}`);
}
