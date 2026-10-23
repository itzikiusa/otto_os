// Keyboard for a focused pane resizer (the ARIA window-splitter pattern),
// shared by the graph's refs-sidebar and commit-list splitters: ←/→ move it
// (⇧ for a big step), Home/End jump to the ends, Enter resets. `mirror` flips
// the arrows under RTL for a splitter whose pane sits on the other side.
export function resizerKey(
  e: KeyboardEvent,
  get: () => number,
  set: (w: number) => void,
  def: number,
  mirror = false,
): void {
  const step = e.shiftKey ? 64 : 16;
  const rtl = mirror && document.dir === 'rtl' ? -1 : 1;
  if (e.key === 'ArrowRight') set(get() + step * rtl);
  else if (e.key === 'ArrowLeft') set(get() - step * rtl);
  else if (e.key === 'Home') set(0);
  else if (e.key === 'End') set(10_000);
  else if (e.key === 'Enter') set(def);
  else return;
  e.preventDefault();
}
