// Roving-tabindex keyboard for a `role="tablist"` of native buttons
// (WAI-ARIA tabs, automatic activation): ←/→ move focus AND select (visual
// direction, so RTL flips), Home/End jump to the ends. Put it on every
// `role="tab"` button:
//
//   <div role="tablist" aria-label="Sections">
//     <button role="tab" aria-selected={on} tabindex={on ? 0 : -1} onkeydown={onTabKey} …>
//
// or ONCE on the tablist itself (the keydown bubbles from the focused tab):
//
//   <div role="tablist" aria-label="Sections" onkeydown={onTabKey}>
//
// Vertical tablists (`aria-orientation="vertical"`) use ↑/↓ instead. Disabled
// tabs (`:disabled` / `aria-disabled="true"`) are skipped. Selecting is done by
// clicking the target tab, so a tab's `onclick` stays the single place that
// switches view (guards, routing, lazy loads) for pointer and keyboard alike.

/** Index a tab key moves to from `index` in a list of `count` tabs, or -1 when
 *  the key is not a tablist key. `rtl` flips ←/→ (visual direction). */
export function nextTabIndex(
  key: string,
  index: number,
  count: number,
  opts: { vertical?: boolean; rtl?: boolean } = {},
): number {
  if (count <= 0) return -1;
  const prevKey = opts.vertical ? 'ArrowUp' : 'ArrowLeft';
  const nextKey = opts.vertical ? 'ArrowDown' : 'ArrowRight';
  const step = opts.vertical || !opts.rtl ? 1 : -1;
  if (key === nextKey) return (index + step + count) % count;
  if (key === prevKey) return (index - step + count) % count;
  if (key === 'Home') return 0;
  if (key === 'End') return count - 1;
  return -1;
}

export function onTabKey(event: KeyboardEvent): void {
  const origin = event.currentTarget as HTMLElement;
  // Attached to the tablist: the focused tab is the event target; attached to
  // a tab: it is the tab itself.
  const group = origin.getAttribute('role') === 'tablist' ? origin : origin.closest('[role="tablist"]');
  if (!(group instanceof HTMLElement)) return;
  const button = (event.target as Element | null)?.closest('[role="tab"]') ?? origin;
  const tabs = Array.from(group.querySelectorAll<HTMLElement>('[role="tab"]:not(:disabled):not([aria-disabled="true"])'));
  const index = tabs.indexOf(button as HTMLElement);
  if (index < 0) return;
  const next = nextTabIndex(event.key, index, tabs.length, {
    vertical: group.getAttribute('aria-orientation') === 'vertical',
    rtl: getComputedStyle(group).direction === 'rtl',
  });
  if (next < 0) return;
  event.preventDefault();
  tabs[next].focus();
  tabs[next].click();
}
