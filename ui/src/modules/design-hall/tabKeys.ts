/** Automatic-activation tabs, including visual arrow direction in RTL. */
export function onTabKey(event: KeyboardEvent): void {
  const button = event.currentTarget as HTMLButtonElement;
  const group = button.closest('[role="tablist"]');
  if (!group) return;
  const tabs = Array.from(group.querySelectorAll<HTMLButtonElement>('[role="tab"]:not(:disabled)'));
  const index = tabs.indexOf(button);
  const direction = getComputedStyle(group).direction === 'rtl' ? -1 : 1;
  let next = -1;
  if (event.key === 'ArrowRight') next = (index + direction + tabs.length) % tabs.length;
  else if (event.key === 'ArrowLeft') next = (index - direction + tabs.length) % tabs.length;
  else if (event.key === 'Home') next = 0;
  else if (event.key === 'End') next = tabs.length - 1;
  if (next < 0) return;
  event.preventDefault();
  tabs[next]?.focus();
  tabs[next]?.click();
}
