// While the on-screen keyboard is up (`html[data-kb="open"]`, lib/viewportFit.ts)
// the page is taller than what shows, so a vertical drag that nothing under
// the finger can scroll goes on to the page and the browser pans the visual
// viewport: the app slid up under the finger and lib/viewportFit.ts then put
// it back (--app-top, window.scrollTo) — the screen jumped. Seen with a real
// keyboard and real touches on Android Chrome from the composer, the
// conversation at its end, the queue bar and the header alike, so it is
// held for the whole document, not per composer. Anything under the finger
// that can still scroll that way takes the drag: a long draft, a list, the
// conversation, and — for a mostly sideways drag — anything that scrolls
// sideways (a code block with wrapping off). It holds from the moment the
// keyboard starts opening (a text field has focus and the visible area
// shrank), before data-kb: a drag right after the tap panned up to 80 px in
// landscape. A pinch and a zoomed page (keyboard closed) are left alone.
// main.ts installs it on touch devices.

import { trace } from './composerTrace.ts';
import { isTextEntry } from './viewportFit.ts';

/** Whether `el` can scroll further down/up for a vertical drag by `dy`, or
 *  (`sideways`) scrolls sideways at all, whatever the writing direction. */
function scrollsToward(el: Element, dy: number, sideways: boolean): boolean {
  const s = getComputedStyle(el);
  if (sideways) return el.scrollWidth > el.clientWidth + 1 && /auto|scroll/.test(s.overflowX);
  if (el.scrollHeight <= el.clientHeight + 1 || !/auto|scroll/.test(s.overflowY)) return false;
  return dy > 0 ? el.scrollTop > 0 : el.scrollTop + el.clientHeight < el.scrollHeight - 1;
}

/** The keyboard is up — or still opening: a text field has focus and, at no zoom, the
 *  visible area is already shorter than the page (before viewportFit flips data-kb). */
function keyboardUp(doc: Document): boolean {
  if (doc.documentElement.dataset.kb === 'open') return true;
  const w = doc.defaultView;
  const vv = w?.visualViewport;
  return !!vv && Math.abs(vv.scale - 1) < 0.01 && vv.height < w.innerHeight - 1 && isTextEntry(doc.activeElement);
}

export function holdTouchScroll(doc: Document): void {
  let x = 0;
  let y = 0;
  // The composer trace (lib/composerTrace.ts) gets each change of decision in a gesture.
  let decided = '';
  const note = (d: string, data: Record<string, unknown>): void => {
    if (d !== decided) trace('hold', { d, ...data });
    decided = d;
  };
  const onStart = (e: TouchEvent): void => {
    x = e.touches[0]?.clientX ?? 0;
    y = e.touches[0]?.clientY ?? 0;
    decided = '';
  };
  const onMove = (e: TouchEvent): void => {
    const t = e.touches[0];
    if (!t || !keyboardUp(doc)) return;
    if (e.touches.length > 1 || !e.cancelable) return note(e.cancelable ? 'pinch' : 'not-cancelable', {});
    const dx = t.clientX - x;
    const dy = t.clientY - y;
    x = t.clientX;
    y = t.clientY;
    if (!dy) return;
    const sideways = Math.abs(dx) > Math.abs(dy);
    for (let el = e.target instanceof Element ? e.target : null; el && el !== doc.documentElement; el = el.parentElement) {
      if (scrollsToward(el, dy, sideways)) return note('let', { dx, dy, by: el });
    }
    note('cancel', { dx, dy });
    e.preventDefault();
  };
  doc.addEventListener('touchstart', onStart, { passive: true });
  doc.addEventListener('touchmove', onMove, { passive: false });
}
