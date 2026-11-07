// Pure helpers for the Personal Agents home (rail + conversation + panel).

import type { PersonalAgent } from '../../lib/api/types';

/** An agent's one-line description: its persona's first real line (template
 *  markers and headings dropped), as the daemon's `about_line` reads it. */
export function about(a: Pick<PersonalAgent, 'soul_md'>): string {
  const line = a.soul_md
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => !l.startsWith('<!--'))
    .map((l) => l.replace(/^#+/, '').trim())
    .find((l) => l !== '');
  if (!line) return 'A personal agent — chat with it to set it up.';
  const chars = [...line];
  return chars.length > 160 ? `${chars.slice(0, 160).join('')}…` : line;
}

/** Rail order: the main agent first, then by name. Stable — pausing an
 *  agent never moves its row (the selected row used to jump). */
export function railOrder<T extends Pick<PersonalAgent, 'id' | 'name'>>(agents: T[], isMain: (id: string) => boolean): T[] {
  return [...agents].sort((a, b) => Number(isMain(b.id)) - Number(isMain(a.id)) || a.name.localeCompare(b.name));
}

/** Explicit links always win; a missing linked agent must never show another chat. */
export function focusAgent<T extends Pick<PersonalAgent, 'id'>>(agents: T[], id: string, primaryId: string | null): T | null {
  if (id) return agents.find((agent) => agent.id === id) ?? null;
  return agents.find((agent) => agent.id === primaryId) ?? agents[0] ?? null;
}

/** A time in the viewer's local time — THE one format of this module:
 *  "Today, 09:00", "Tomorrow, 09:00", else "Sun 8 Nov, 09:00". */
export function nextRunWords(iso: string | Date, now = new Date()): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  // 24-hour, like every routine's own "at 09:00".
  const time = d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' });
  const day = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((day(d) - day(now)) / 86_400_000);
  if (days === 0) return `Today, ${time}`;
  if (days === 1) return `Tomorrow, ${time}`;
  if (days === -1) return `Yesterday, ${time}`;
  const date = d.toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' }).replace(',', '');
  return `${date}, ${time}`;
}

export { firstLine, plainText } from '../../lib/plainText.ts';

/** "Try asking" chips on an agent's first open — no tokens are spent until
 *  one is sent. */
export function starterPrompts(main: boolean, hasRoutine = false, paused = false): string[] {
  if (main) return ['What are my agents up to?', 'Set up an agent that watches my pull requests', 'Every weekday at 9, brief me on what needs me'];
  // Paused with a routine: nothing to run or set up — the banner offers Resume.
  if (hasRoutine && paused) return ['What can you do for me?'];
  return ['What can you do for me?', hasRoutine ? 'Run your routine now and tell me what you find' : 'Set up a routine for yourself'];
}

/** The intro line shown before the first message. */
export function introLine(name: string, main: boolean): string {
  return main
    ? `Hey! I’m ${name}, your main agent. I can manage your other agents. Task me with anything — I’ll make sure it gets to the right agent, and I’ll follow up when they report back.`
    : `Hi, I’m ${name}. Tell me what you need, or set me up with a routine — my results show up right here.`;
}

/** The model list an agent's Setup offers: the newest of each family
 *  ("claude-opus-5-5" over "claude-opus-4-8"), plus `keep` (the agent's saved
 *  model) when it is an older one, so a 12-entry catalog reads as 4 choices. */
export function currentModels<T extends { id: string }>(models: T[], keep = ''): T[] {
  const version = (id: string): number[] =>
    (id.match(/\d+/g) ?? []).filter((d) => d.length < 6).map(Number);
  const newer = (a: number[], b: number[]): boolean => {
    for (let i = 0; i < Math.max(a.length, b.length); i++) {
      if ((a[i] ?? 0) !== (b[i] ?? 0)) return (a[i] ?? 0) > (b[i] ?? 0);
    }
    return false;
  };
  const best = new Map<string, T>();
  for (const m of models) {
    const family = m.id.replace(/-\d.*$/, '');
    const cur = best.get(family);
    if (!cur || newer(version(m.id), version(cur.id))) best.set(family, m);
  }
  const kept = new Set([...best.values()].map((m) => m.id));
  return models.filter((m) => kept.has(m.id) || m.id === keep);
}

/** A routine's directive as {name, instruction}: a short first line followed
 *  by a blank line is its name ("Morning PR brief\n\nList my PRs…" — what the
 *  form and `otto.routine_create` store); anything else has no name. */
export function splitDirective(directive: string): { name: string; instruction: string } {
  const m = /^([^\n]{1,60})\n\s*\n([\s\S]+)$/.exec(directive.trim());
  return m ? { name: m[1].trim(), instruction: m[2].trim() } : { name: '', instruction: directive.trim() };
}

/** The directive stored for a name + instruction (the name as its first line). */
export function joinDirective(name: string, instruction: string): string {
  const n = name.trim().replace(/\s+/g, ' ');
  return n ? `${n}\n\n${instruction.trim()}` : instruction.trim();
}
