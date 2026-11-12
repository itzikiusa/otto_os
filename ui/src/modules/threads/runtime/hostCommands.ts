// The runtime composer's view-level slash commands, carried out by whoever
// hosts the conversation (lib/runtimeThreadPort.ts: Threads, a personal
// agent's chat, a pull request's column…). Pure — no stores — so the
// filtering is unit-tested (unit/threadsHostCommands.test.ts).
import type { RuntimeHostCommands } from '../../../lib/runtimeThreadPort';
import type { ComposerSlashCommand, ParsedSlashIntent } from '../../../lib/slashActions';

/** The commands the runtime composer offers, and the host handler each one
 *  needs (null: the thread's own — `/clear` arms a fresh context, `/review`
 *  is a prompt — so it works in every host). */
export const VIEW_SLASH: Readonly<Record<string, keyof RuntimeHostCommands | null>> = {
  diff: 'openDiff',
  files: 'openFiles',
  terminal: 'openTerminal',
  new: 'newThread',
  clear: null,
  fork: 'newThread',
  review: null,
  btw: 'sideChat',
};

/** The commands this host can carry out, in `commands` order: one without a
 *  handler is left out rather than offered as a no-op. */
export function availableSlash(commands: readonly ComposerSlashCommand[], host: RuntimeHostCommands | undefined): ComposerSlashCommand[] {
  return commands.filter((c) => {
    if (!Object.prototype.hasOwnProperty.call(VIEW_SLASH, c.name)) return false;
    const need = VIEW_SLASH[c.name];
    return need === null || typeof host?.[need] === 'function';
  });
}

/** Hand a host-level intent to the host; `false` when it has no handler (the
 *  composer then sends the text to the agent as typed). */
export function runHostSlash(intent: ParsedSlashIntent, sessionId: string, host: RuntimeHostCommands | undefined): boolean {
  switch (intent.kind) {
    case 'open-diff':
      return call(host?.openDiff, () => host!.openDiff!(sessionId));
    case 'open-files':
      return call(host?.openFiles, () => host!.openFiles!(sessionId));
    case 'toggle-terminal':
      return call(host?.openTerminal, () => host!.openTerminal!(sessionId));
    case 'new-thread':
      return call(host?.newThread, () => host!.newThread!(sessionId, intent.beside));
    case 'side-chat':
      return call(host?.sideChat, () => host!.sideChat!(sessionId, intent.question));
    default:
      return false;
  }
}

function call(handler: unknown, run: () => void): boolean {
  if (typeof handler !== 'function') return false;
  run();
  return true;
}
