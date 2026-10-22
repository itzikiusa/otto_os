// ⌘K "Connect to <name>" commands for a workspace's saved connections. The
// connections module owns them (architecture-2-05); the shell's palette
// sources (shell/paletteSources.svelte.ts) only decide WHEN to load them.

import { api } from '../../lib/api/client';
import type { Command } from '../../lib/commands.svelte';
import type { Connection, Id, Session } from '../../lib/api/types';
import { ws } from '../../lib/stores/workspace.svelte';
import { toasts } from '../../lib/toast.svelte';

/** One command per connection: open it as a session. */
export function connectionCommands(conns: Connection[]): Command[] {
  return conns.map((c) => ({
    id: `connect.${c.id}`,
    title: `Connect to ${c.name}`,
    group: 'Connections',
    keywords: `${c.kind} open`,
    run: async () => {
      const session = await api.post<Session>(`/connections/${c.id}/open`, {});
      ws.addSession(session);
      toasts.success('Connection opened', c.name);
    },
  }));
}

/** Fetch `wsId`'s connections and build their commands. */
export async function loadConnectionCommands(wsId: Id): Promise<Command[]> {
  return connectionCommands(await api.get<Connection[]>(`/workspaces/${wsId}/connections`));
}
