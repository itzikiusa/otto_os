// Roving-tabindex keyboard for a `role="tablist"` of native buttons
// (WAI-ARIA tabs, automatic activation): ←/→ move focus AND select (visual
// direction, so RTL flips), Home/End jump to the ends. Put it on every
// `role="tab"` button:
//
//   <div role="tablist" aria-label="Sections">
//     <button role="tab" aria-selected={on} tabindex={on ? 0 : -1} onkeydown={onTabKey} …>
//
// Vertical tablists (`aria-orientation="vertical"`) use ↑/↓ instead.
export function onTabKey(event: KeyboardEvent): void {
  const button = event.currentTarget as HTMLElement;
  const group = button.closest('[role="tablist"]');
  if (!(group instanceof HTMLElement)) return;
  const tabs = Array.from(group.querySelectorAll<HTMLElement>('[role="tab"]:not(:disabled):not([aria-disabled="true"])'));
  const index = tabs.indexOf(button);
  if (index < 0) return;
  const vertical = group.getAttribute('aria-orientation') === 'vertical';
  const dir = getComputedStyle(group).direction === 'rtl' ? -1 : 1;
  const prevKey = vertical ? 'ArrowUp' : 'ArrowLeft';
  const nextKey = vertical ? 'ArrowDown' : 'ArrowRight';
  let next = -1;
  const step = vertical ? 1 : dir;
  if (event.key === nextKey) next = (index + step + tabs.length) % tabs.length;
  else if (event.key === prevKey) next = (index - step + tabs.length) % tabs.length;
  else if (event.key === 'Home') next = 0;
  else if (event.key === 'End') next = tabs.length - 1;
  if (next < 0) return;
  event.preventDefault();
  tabs[next].focus();
  tabs[next].click();
}
