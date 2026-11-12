// Pure rules for the Connections hub's SSH / custom-CLI terminal tabs —
// kept import-free so they are unit-testable (unit/connectionsSshTabs.test.ts).
// The stateful half (liveness map, open/reconnect/close) is ssh-tabs.svelte.ts.

import type { SessionStatus } from '../../lib/api/types';

/** What the hub knows about a tab's PTY: still running, or the process ended
 *  (auth failure, dropped link, the user typed `exit`). */
export type SshLiveness = 'live' | 'exited';

/** A terminal status frame → liveness. `reconnectable` is a dormant shell the
 *  Terminal's own Resume wakes, so it is not a dead tab. */
export function livenessOf(status: SessionStatus): SshLiveness {
  return status === 'exited' ? 'exited' : 'live';
}

/** Opening a connection from the tree: spawn a first session, focus the live
 *  tab, or — when that tab's session has ended — reconnect instead of
 *  focusing the corpse. */
export function sshOpenAction(
  tabs: readonly { connId: string; sessionId: string }[],
  liveness: (sessionId: string) => SshLiveness | undefined,
  connId: string,
): 'open' | 'focus' | 'reconnect' {
  const tab = tabs.find((t) => t.connId === connId);
  if (!tab) return 'open';
  return liveness(tab.sessionId) === 'exited' ? 'reconnect' : 'focus';
}

/** The close confirmation, or null when there is nothing left to stop. An
 *  unknown liveness counts as live: asking once too often beats killing a
 *  remote shell unasked. */
export function sshCloseQuestion(
  tab: { name: string; kind: string },
  liveness: SshLiveness | undefined,
): string | null {
  if (liveness === 'exited') return null;
  const what = tab.kind === 'ssh' ? 'SSH session' : 'terminal session';
  return `Close the ${what} to ${tab.name}? Anything running in it stops.`;
}

/** Replace a tab's session id (Reconnect), keeping its place in the strip. */
export function withSession<T extends { connId: string; sessionId: string }>(
  tabs: readonly T[],
  connId: string,
  sessionId: string,
): T[] {
  return tabs.map((t) => (t.connId === connId ? { ...t, sessionId } : t));
}
