// The system Back (Android, a phone browser's Back) closes the top-most open
// sheet / drawer / menu before it leaves the page. On a touch screen each
// open overlay pushes one history entry with the SAME url — the hash router
// only listens to hashchange, so it never sees it — and Back pops it, closing
// that overlay. A normal close (✕, backdrop, a pick) takes its entry back off;
// one that closed after a navigation leaves a stale entry, which Back skips.

const closers = new Map<number, () => void>();
const live: number[] = [];
/** Opened while one of our own Back steps is still in flight: pushed once it lands. */
const queued: number[] = [];
let seq = 0;
let ignore = 0;
let hooked = false;

const here = (): number | undefined => (history.state as { ottoSheet?: number } | null)?.ottoSheet;
const push = (id: number): void => history.pushState({ ...((history.state as object | null) ?? {}), ottoSheet: id }, '', window.location.href);

function onPop(): void {
  if (ignore > 0) {
    const at = here();
    // Our own step landed on another closed overlay's entry: one more.
    if (at !== undefined && !closers.has(at)) history.back();
    else if (--ignore === 0) queued.splice(0).forEach(push);
    return;
  }
  const top = live.at(-1);
  const at = here();
  if (top !== undefined && at !== top) {
    live.pop();
    const close = closers.get(top);
    closers.delete(top);
    close?.();
  }
  // Landed on the entry of an overlay that already closed (a menu whose pick
  // opened this sheet): step over it, so no Back is spent on nothing.
  if (at !== undefined && !closers.has(at)) history.back();
}

/** While open, Back calls `close`. Returns the release for a normal close
 *  (call it on unmount; after Back closed it, it does nothing). */
export function backCloses(close: () => void): () => void {
  if (typeof window === 'undefined' || !window.matchMedia?.('(pointer: coarse)').matches) return () => {};
  if (!hooked) {
    hooked = true;
    window.addEventListener('popstate', onPop);
  }
  const id = ++seq;
  if (ignore > 0) queued.push(id);
  else push(id);
  live.push(id);
  closers.set(id, close);
  return () => {
    if (!closers.delete(id)) return;
    live.splice(live.indexOf(id), 1);
    const q = queued.indexOf(id);
    if (q >= 0) queued.splice(q, 1);
    // A task later: a pick that closes the sheet navigates asynchronously
    // (router guards), and stepping back first would undo that navigation.
    else
      setTimeout(() => {
        if (here() !== id) return;
        ignore++;
        history.back();
      }, 0);
  };
}
