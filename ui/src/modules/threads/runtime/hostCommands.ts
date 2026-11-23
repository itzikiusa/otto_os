// The runtime composer's view-level slash commands, carried out by whoever
// hosts the conversation (lib/runtimeThreadPort.ts: Threads, a personal
// agent's chat, a pull request's column…). Pure — no stores — so the
// filtering is unit-tested (unit/threadsHostCommands.test.ts).
import type { ProviderCommand } from '../../../lib/api/types';
import type { RuntimeHostCommands } from '../../../lib/runtimeThreadPort';
import { parseInteractiveSlashInput, type ComposerSlashCommand, type ParsedSlashIntent } from '../../../lib/slashActions.ts';

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

/** What a view intent needs from the host (runHostSlash). */
const HOST_FOR: Partial<Record<ParsedSlashIntent['kind'], keyof RuntimeHostCommands>> = {
  'open-diff': 'openDiff', 'open-files': 'openFiles', 'toggle-terminal': 'openTerminal', 'new-thread': 'newThread', 'side-chat': 'sideChat',
};

/** Otto takes `/name` itself (it never reaches the agent): a view command
 *  this host carries out, `/clear`, or a prompt Otto writes (`/review`). */
export function ottoConsumes(name: string, host: RuntimeHostCommands | undefined): boolean {
  const intent = parseInteractiveSlashInput(`/${name}`);
  if (!intent) return false;
  if (intent.kind === 'send-prompt' || intent.kind === 'fresh-context') return true;
  const need = HOST_FOR[intent.kind];
  return !!need && typeof host?.[need] === 'function';
}

/** How well `q` matches `name`: 0 prefix, 1 inside, 2 letters in order, -1 not at all. */
function fuzzyRank(name: string, q: string): number {
  if (!q) return 0;
  const n = name.toLowerCase();
  if (n.startsWith(q)) return 0;
  if (n.includes(q)) return 1;
  let i = 0;
  for (const ch of n) if (ch === q[i]) i++;
  return i === q.length ? 2 : -1;
}

/** The `/` popup for `text` (T3 Code's ComposerCommandMenu, MIT): Otto's
 *  commands first, then the agent's own (`commands.updated`) under
 *  "<Agent> commands", fuzzy-filtered and best match first in each group.
 *  An agent command Otto would take itself is left out — typing it never
 *  reaches the agent. One group shows no heading. */
export function slashSuggestions(
  text: string,
  otto: readonly ComposerSlashCommand[],
  agent: readonly ProviderCommand[],
  agentName: string,
  host: RuntimeHostCommands | undefined,
): ComposerSlashCommand[] {
  const q = /^\/([\w:.-]*)$/.exec(text)?.[1].toLowerCase();
  if (q === undefined) return [];
  const pick = <T extends { name: string }>(list: readonly T[]): T[] =>
    list.map((c, i) => ({ c, i, r: fuzzyRank(c.name, q) })).filter((x) => x.r >= 0).sort((a, b) => a.r - b.r || a.i - b.i).map((x) => x.c);
  const ours = pick(otto);
  const seen = new Set<string>();
  const theirs: ComposerSlashCommand[] = pick(agent.filter((c) => {
    const keep = !!c.name && !seen.has(c.name) && !ottoConsumes(c.name, host);
    seen.add(c.name);
    return keep;
  })).map((c) => ({ name: c.name, description: c.description ?? '', source: 'agent', ...(c.hint ? { hint: c.hint } : {}) }));
  if (!theirs.length) return ours;
  const label = (g: string) => (c: ComposerSlashCommand): ComposerSlashCommand => ({ ...c, group: g });
  return [...ours.map(label('Otto')), ...theirs.map(label(`${agentName} commands`))];
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
