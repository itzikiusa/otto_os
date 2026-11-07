// One delete confirmation for a personal agent. The main agent — the one you
// talk to, which runs the others — asks you to type its name, so a stray
// click (or script) can't wipe its routines and whole chat.

import type { PersonalAgent } from '../../lib/api/types';
import { confirmer } from '../../lib/confirm.svelte';
import { personalAgents } from '../../lib/stores/personalAgents.svelte';

export async function confirmDeleteAgent(a: PersonalAgent, isMain: boolean): Promise<boolean> {
  if (!isMain) {
    return confirmer.ask(`Delete personal agent “${a.name}”? Its routines, memory and run history go with it.`, {
      title: 'Delete personal agent',
      danger: true,
      confirmLabel: 'Delete',
    });
  }
  const n = personalAgents.schedulesByAgent[a.id]?.length ?? 0;
  const routines = n === 1 ? 'its 1 routine' : `its ${n} routines`;
  const typed = await confirmer.promptText(
    `“${a.name}” is your main agent — you talk to it and it runs your other agents. Deleting it deletes ${routines}, its chat and its run history. Type its name to confirm.`,
    { title: 'Delete your main agent', confirmLabel: 'Delete main agent', placeholder: 'Type the name to confirm', mustMatch: a.name.trim(), danger: true },
  );
  // OK stays disabled until the typed name matches (`mustMatch`).
  return typed !== null;
}
