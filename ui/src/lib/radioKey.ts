/** Native button radio groups share one tab stop and move selection with focus. */
export function radioKey(event: KeyboardEvent): void {
  if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
  const group = (event.currentTarget as HTMLElement).closest('[role="radiogroup"]');
  if (!(group instanceof HTMLElement)) return;
  const buttons = [...group.querySelectorAll<HTMLButtonElement>('button[role="radio"]:not(:disabled)')];
  const index = buttons.indexOf(event.target as HTMLButtonElement);
  if (index < 0) return;
  event.preventDefault();
  const rtl = getComputedStyle(group).direction === 'rtl';
  const forward = event.key === 'ArrowDown' || event.key === (rtl ? 'ArrowLeft' : 'ArrowRight');
  const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (index + (forward ? 1 : -1) + buttons.length) % buttons.length;
  buttons[next].focus();
  buttons[next].click();
}
