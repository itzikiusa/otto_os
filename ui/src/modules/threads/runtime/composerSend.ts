// The composer's ONE primary button and what a send does while the agent
// works, after T3 Code's (MIT, Copyright (c) 2026 T3 Tools Inc.)
// apps/web/src/components/chat/composerSend.logic.ts: idle → Send; working
// with an empty draft → Stop; working with a draft → the person's mid-turn
// delivery (Queue by default; Steer from the toolbar ⋯ › View), and ⌘↵
// (Ctrl+↵ off macOS) the other one. Steering goes the queue bar's way
// (`delivery: 'steer'`, the daemon's deliver_busy); an agent without the
// `steer` capability only queues, and the button says so. Pure
// (unit/runtimeComposerSend.test.ts).

export type MidTurn = 'queue' | 'steer';
export type PrimaryAction = 'send' | 'stop' | MidTurn;

export function primaryAction(o: { running: boolean; canSend: boolean; pref: MidTurn; canSteer: boolean }): PrimaryAction {
  if (!o.running) return 'send';
  if (!o.canSend) return 'stop';
  return o.pref === 'steer' && o.canSteer ? 'steer' : 'queue';
}

/** A send while the turn runs: the preference, or (`alt`, ⌘↵) the other
 *  one. `refused`: steering was asked of an agent that can't — it queues. */
export function midTurnDelivery(pref: MidTurn, alt: boolean, canSteer: boolean): { delivery: MidTurn; refused: boolean } {
  const want: MidTurn = alt ? (pref === 'queue' ? 'steer' : 'queue') : pref;
  return want === 'steer' && !canSteer ? { delivery: 'queue', refused: true } : { delivery: want, refused: false };
}

/** ⌘↵ (Ctrl+↵ off macOS) asks for the other delivery. On a touch-first
 *  device ⌘↵ is the only keyboard send (enterSends.ts), so it stays the default. */
export function isAltSend(e: { key: string; metaKey: boolean; ctrlKey: boolean; shiftKey: boolean }, mac: boolean, touch: boolean): boolean {
  return !touch && e.key === 'Enter' && !e.shiftKey && (mac ? e.metaKey : e.ctrlKey);
}

/** The primary button's name and tooltip (Stop keeps its own, RuntimeComposerActions). */
export function primaryLabel(action: PrimaryAction, o: { agent: string; canSteer: boolean; mac: boolean }): { label: string; title: string } {
  const other = o.mac ? '⌘↵' : 'Ctrl+↵';
  if (action === 'steer') {
    const label = 'Steer — the agent reads it at its next step';
    return { label, title: `${label} (↵ · ${other} queues)` };
  }
  if (action === 'queue') {
    const label = 'Queue — sends when this turn ends';
    return { label, title: o.canSteer ? `${label} (↵ · ${other} steers)` : `${label}. ${o.agent} can’t take a message mid-turn` };
  }
  return { label: 'Send message', title: 'Send (↵)' };
}
