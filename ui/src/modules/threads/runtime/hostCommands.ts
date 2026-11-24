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

/** How well `q` matches `n` (lower case): 0 prefix, 1 inside, 2 letters in order, -1 not at all. */
function fuzzyRank(n: string, q: string): number {
  if (!q) return 0;
  if (n.startsWith(q)) return 0;
  if (n.includes(q)) return 1;
  let i = 0;
  for (const ch of n) if (ch === q[i]) i++;
  return i === q.length ? 2 : -1;
}

type Entry = { c: ComposerSlashCommand; n: string; i: number; ours: boolean };
type Prepared = { entries: Entry[]; grouped: boolean; last: { q: string; hits: Entry[]; out: ComposerSlashCommand[] } | null };
/** The merged list (Otto's, then the agent's not taken by Otto, labelled),
 *  built once per commands change — never per key. */
const prepared = new WeakMap<object, { otto: readonly ComposerSlashCommand[]; agentName: string; host: RuntimeHostCommands | undefined; p: Prepared }>();

function prepare(otto: readonly ComposerSlashCommand[], agent: readonly ProviderCommand[], agentName: string, host: RuntimeHostCommands | undefined): Prepared {
  const hit = prepared.get(agent);
  if (hit && hit.otto === otto && hit.agentName === agentName && hit.host === host) return hit.p;
  const seen = new Set<string>();
  const theirs: ComposerSlashCommand[] = [];
  for (const c of agent) {
    if (!c.name || seen.has(c.name)) continue;
    seen.add(c.name);
    if (ottoConsumes(c.name, host)) continue;
    theirs.push({ name: c.name, description: c.description ?? '', source: 'agent', ...(c.hint ? { hint: c.hint } : {}) });
  }
  const grouped = theirs.length > 0;
  const entries: Entry[] = [
    ...otto.map((c, i) => ({ c: grouped ? { ...c, group: 'Otto' } : c, n: c.name.toLowerCase(), i, ours: true })),
    ...theirs.map((c, i) => ({ c: { ...c, group: `${agentName} commands` }, n: c.name.toLowerCase(), i, ours: false })),
  ];
  const p: Prepared = { entries, grouped, last: null };
  prepared.set(agent, { otto, agentName, host, p });
  return p;
}

/** The `/` popup for `text` (T3 Code's ComposerCommandMenu, MIT): Otto's
 *  commands first, then the agent's own (`commands.updated`) under
 *  "<Agent> commands", fuzzy-filtered and best match first in each group.
 *  An agent command Otto would take itself is left out — typing it never
 *  reaches the agent. One group shows no heading. Typing on narrows the last
 *  matches instead of scanning every command again (235 for Grok), and the
 *  same matches return the same array (the popup does not redraw). */
export function slashSuggestions(
  text: string,
  otto: readonly ComposerSlashCommand[],
  agent: readonly ProviderCommand[],
  agentName: string,
  host: RuntimeHostCommands | undefined,
): ComposerSlashCommand[] {
  const q = /^\/([\w:.-]*)$/.exec(text)?.[1].toLowerCase();
  if (q === undefined) return [];
  const p = prepare(otto, agent, agentName, host);
  if (p.last?.q === q) return p.last.out;
  const pool = p.last && q.startsWith(p.last.q) ? p.last.hits : p.entries;
  const ranked: { e: Entry; r: number }[] = [];
  for (const e of pool) {
    const r = fuzzyRank(e.n, q);
    if (r >= 0) ranked.push({ e, r });
  }
  ranked.sort((a, b) => Number(b.e.ours) - Number(a.e.ours) || a.r - b.r || a.e.i - b.e.i);
  const hits = ranked.map((x) => x.e);
  const out = hits.map((e) => e.c);
  const prev = p.last?.out;
  p.last = { q, hits, out: prev && prev.length === out.length && prev.every((c, i) => c === out[i]) ? prev : out };
  return p.last.out;
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
