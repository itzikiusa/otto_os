// One screen-reader announcer for the whole app.
//
// WebKit/VoiceOver only speak CHANGES to a live region that already exists,
// so a `role="status"` element inserted together with its text is often never
// read. The shell mounts `Announcer.svelte` once (beside `Toasts`): one polite
// region and one assertive region, always in the DOM. Callers announce at the
// state edge (`announce('Saved')` where the save resolves), never by rendering
// a status element.
//
// Repeating the same text must speak again ("Copied" twice is two events), but
// a region whose text doesn't change is silent. So every announce clears the
// channel first and sets the text on the NEXT frame — two separate DOM writes
// the reader sees as a change. A second announce on the same channel before
// that frame replaces the first (the newest status is the one worth hearing).

export type AnnounceChannel = 'polite' | 'assertive';

class Announcer {
  /** Text of the always-mounted polite region (`aria-live="polite"`). */
  polite = $state('');
  /** Text of the always-mounted assertive region (`role="alert"`). */
  assertive = $state('');
}

export const announcer = new Announcer();

const pending: Record<AnnounceChannel, (() => void) | null> = { polite: null, assertive: null };

/** Run `fn` on the next frame (a timer outside a browser), returning a canceller. */
function nextFrame(fn: () => void): () => void {
  if (typeof requestAnimationFrame === 'function') {
    const id = requestAnimationFrame(() => fn());
    return () => cancelAnimationFrame(id);
  }
  const id = setTimeout(fn, 16);
  return () => clearTimeout(id);
}

/** Focus the shell's page content (`#app-content`, tabindex -1) after the
 *  next render — for a control that removes itself on activation (a
 *  Dismiss), so focus never falls to <body>. */
export async function focusAppContent(): Promise<void> {
  await new Promise<void>((resolve) => nextFrame(resolve));
  if (typeof document !== 'undefined') document.getElementById('app-content')?.focus({ preventScroll: true });
}

/** Speak `text` to screen readers: politely by default, interrupting with
 *  `{ assertive: true }` (errors that need attention now). Empty text only
 *  clears the channel. */
export function announce(text: string, opts?: { assertive?: boolean }): void {
  const channel: AnnounceChannel = opts?.assertive ? 'assertive' : 'polite';
  pending[channel]?.();
  pending[channel] = null;
  announcer[channel] = '';
  if (!text) return;
  pending[channel] = nextFrame(() => {
    pending[channel] = null;
    announcer[channel] = text;
  });
}

/** An edge detector over `announce`: call it with the CURRENT message on every
 *  render (from an `$effect`) and it speaks only when the message is new —
 *  a re-render with the same text stays quiet. Null / '' means "the state is
 *  gone" and re-arms it, so the same failure after a recovery speaks again. */
export function announceEdge(): (text: string | null | undefined, opts?: { assertive?: boolean }) => void {
  let last = '';
  return (text, opts) => {
    if (!text) {
      last = '';
      return;
    }
    if (text === last) return;
    last = text;
    announce(text, opts);
  };
}

/** Action for a status line that is inserted with its state
 *  (`{#if failed}<p use:announceShown={{ text: 'Run failed', assertive: true }}>`):
 *  it announces when the line appears and again when its text changes, so the
 *  line itself needs no role. Pass the words a listener needs — usually the
 *  line's own text. */
export function announceShown(
  _node: Element,
  param: string | { text: string; assertive?: boolean },
): { update: (p: string | { text: string; assertive?: boolean }) => void } {
  const say = announceEdge();
  const run = (p: string | { text: string; assertive?: boolean }) =>
    typeof p === 'string' ? say(p) : say(p.text, { assertive: p.assertive });
  run(param);
  return { update: run };
}
