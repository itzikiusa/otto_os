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
//
// Lists that need something else take options instead of forking the handler:
//
//   onkeydown={tabKeys({ activate: 'click' })}   // onclick owns focus (async
//                                                // route guards, discard dialogs)
//   onkeydown={tabKeys({ activate: 'manual' })}  // arrows move focus only;
//                                                // Enter/Space select
//   onkeydown={tabKeys({ wrap: false, orientation: 'vertical' })}

export type TabOrientation = 'horizontal' | 'vertical';

export interface TabKeyOptions {
  /** Arrow axis. Default: the tablist's `aria-orientation` (horizontal). */
  orientation?: TabOrientation;
  /** Whether ←/→ wrap at the ends. Default true (WAI-ARIA tabs pattern). */
  wrap?: boolean;
  /** `'focus'` (default) moves focus then clicks — automatic activation.
   *  `'click'` only clicks; the tab's onclick decides where focus lands (use it
   *  when activation is async or may be refused, so focus never lands on a
   *  tab that didn't take). `'manual'` only moves focus; Enter/Space (the
   *  button's own click) select. */
  activate?: 'focus' | 'click' | 'manual';
}

/** Index a tab key moves to from `index` in a list of `count` tabs, or -1 when
 *  the key is not a tablist key (or would step past an end with `wrap: false`).
 *  `rtl` flips ←/→ (visual direction). */
export function nextTabIndex(
  key: string,
  index: number,
  count: number,
  opts: { vertical?: boolean; rtl?: boolean; wrap?: boolean } = {},
): number {
  if (count <= 0) return -1;
  const wrap = opts.wrap ?? true;
  const prevKey = opts.vertical ? 'ArrowUp' : 'ArrowLeft';
  const nextKey = opts.vertical ? 'ArrowDown' : 'ArrowRight';
  const step = opts.vertical || !opts.rtl ? 1 : -1;
  const move = (delta: number): number => {
    const raw = index + delta;
    if (wrap) return (raw + count) % count;
    return raw < 0 || raw >= count ? -1 : raw;
  };
  if (key === nextKey) return move(step);
  if (key === prevKey) return move(-step);
  if (key === 'Home') return 0;
  if (key === 'End') return count - 1;
  return -1;
}

/** The shared handler. Use it bare (`onkeydown={onTabKey}`) for the default
 *  behavior, or via {@link tabKeys} to pass options. */
export function onTabKey(event: KeyboardEvent, opts: TabKeyOptions = {}): void {
  if (event.defaultPrevented || event.metaKey || event.ctrlKey || event.altKey) return;
  const origin = event.currentTarget as HTMLElement;
  // Attached to the tablist: the focused tab is the event target; attached to
  // a tab: it is the tab itself.
  const group = origin.getAttribute('role') === 'tablist' ? origin : origin.closest('[role="tablist"]');
  if (!(group instanceof HTMLElement)) return;
  const tabs = Array.from(group.querySelectorAll<HTMLElement>('[role="tab"]:not(:disabled):not([aria-disabled="true"])'));
  // The focused tab, or — when focus sits on the tablist container itself —
  // the selected one.
  const focused = (event.target as Element | null)?.closest('[role="tab"]');
  let index = focused instanceof HTMLElement ? tabs.indexOf(focused) : -1;
  if (index < 0) index = tabs.findIndex((t) => t.getAttribute('aria-selected') === 'true');
  if (index < 0) return;
  const vertical = (opts.orientation ?? group.getAttribute('aria-orientation')) === 'vertical';
  const next = nextTabIndex(event.key, index, tabs.length, {
    vertical,
    rtl: getComputedStyle(group).direction === 'rtl',
    wrap: opts.wrap,
  });
  if (next < 0) return;
  event.preventDefault();
  const activate = opts.activate ?? 'focus';
  if (activate !== 'click') tabs[next].focus();
  if (activate !== 'manual') tabs[next].click();
}

/** {@link onTabKey} bound to options: `onkeydown={tabKeys({ activate: 'click' })}`. */
export function tabKeys(opts: TabKeyOptions): (event: KeyboardEvent) => void {
  return (event) => onTabKey(event, opts);
}
