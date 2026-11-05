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

/** Rail order: the main agent first, then enabled before paused, then name. */
export function railOrder<T extends Pick<PersonalAgent, 'id' | 'name' | 'enabled'>>(agents: T[], isMain: (id: string) => boolean): T[] {
  return [...agents].sort(
    (a, b) =>
      Number(isMain(b.id)) - Number(isMain(a.id)) ||
      Number(b.enabled) - Number(a.enabled) ||
      a.name.localeCompare(b.name),
  );
}

/** "Try asking" chips on an agent's first open — no tokens are spent until
 *  one is picked. */
export function starterPrompts(main: boolean): string[] {
  return main
    ? ['What are my agents up to?', 'Set up an agent that watches my pull requests', 'Make me a morning routine: every weekday at 9, brief me on what needs me']
    : ['What can you do for me?', 'Set up a routine for yourself', 'Run your routine now and tell me what you find'];
}

/** The intro line shown before the first message. */
export function introLine(name: string, main: boolean): string {
  return main
    ? `Hey! I’m ${name}, your main agent. I can manage your other agents. Task me with anything — I’ll make sure it gets to the right agent, and I’ll follow up when they report back.`
    : `Hi, I’m ${name}. Tell me what you need, or set me up with a routine — my results show up right here.`;
}
