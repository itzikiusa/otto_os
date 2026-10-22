// The one confirm for closing several sessions at once — the ⌘K palette's
// "close …" and the sidebar's bulk Archive / Delete ask the same question the
// same way (ux-design-3-03): the singular or plural title, up to five titles
// plus "…and N more", and a note when live agents will stop mid-turn.
// Pure (no store imports) so it is unit-tested; callers pass the result to
// `confirmer.ask(message, opts)`.

import { plural } from '../lib/plural';

export interface CloseAsk {
  /** Titles of the sessions being closed, in display order. */
  titles: string[];
  /** How many of them are working right now (an agent mid-turn). */
  working: number;
  /** Delete (history gone) rather than archive. */
  permanent: boolean;
}

export interface CloseConfirm {
  message: string;
  title: string;
  confirmLabel: string;
  danger: boolean;
}

export function closeConfirm({ titles, working, permanent }: CloseAsk): CloseConfirm {
  const n = titles.length;
  const shown = titles.slice(0, 5).map((t) => `• ${t}`).join('\n');
  const more = n > 5 ? `\n…and ${n - 5} more` : '';
  const busyNote =
    working > 0 ? `\n\n${plural(working, 'session')} ${working === 1 ? 'is' : 'are'} working right now and will stop mid-turn.` : '';
  if (permanent) {
    return {
      message: `Delete ${plural(n, 'session')} and ${n === 1 ? 'its' : 'their'} history permanently?\n\n${shown}${more}${busyNote}`,
      title: n === 1 ? 'Delete session?' : 'Delete sessions?',
      confirmLabel: 'Delete',
      danger: true,
    };
  }
  return {
    message: `Archive ${plural(n, 'session')}? ${n === 1 ? 'It stops and keeps its' : 'They stop and keep their'} history (resumable from the Archived list).\n\n${shown}${more}${busyNote}`,
    title: n === 1 ? 'Archive session?' : `Archive ${n} sessions?`,
    confirmLabel: n === 1 ? 'Archive session' : `Archive ${n} sessions`,
    danger: false,
  };
}
