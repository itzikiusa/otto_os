// Keyboard ownership for custom modal surfaces (drawers and the palette).
// Inner controls handle their own keys first; only the top dialog traps Tab.
// Safari does not focus pointer-clicked buttons. Remember the activating
// control until focus or another user interaction moves on. File choosers and
// async reads can open a sheet well after the click's initial render flush.
let activatingControl: HTMLElement | null = null;
if (typeof document !== 'undefined') {
  document.addEventListener('pointerdown', () => { activatingControl = null; }, true);
  document.addEventListener('keydown', () => { activatingControl = null; }, true);
  document.addEventListener('focusin', () => { activatingControl = null; }, true);
  document.addEventListener('click', (event) => {
    const control = event.target instanceof Element
      ? event.target.closest<HTMLElement>('button, a[href], input, select, textarea, [tabindex]')
      : null;
    // A visible button may call hiddenFileInput.click(). Keep the real trigger.
    if (control && control.getClientRects().length > 0 && !control.matches(':disabled') && !control.closest('[inert]')) {
      activatingControl = control;
    }
  }, true);
}

export function dialogReturnTarget(): HTMLElement | null {
  const target = activatingControl?.isConnected ? activatingControl
    : document.activeElement instanceof HTMLElement ? document.activeElement : null;
  activatingControl = null;
  return target;
}

/** Capture a return scope as well as the trigger: asynchronous updates may
 * remove or disable that trigger while a child sheet is open. */
export function dialogFocusReturn(): () => void {
  const previous = dialogReturnTarget();
  const scope = previous?.closest<HTMLElement>('[role="dialog"][aria-modal="true"]');
  const usable = (el: HTMLElement) => el.isConnected && !el.closest('[inert]') &&
    !el.matches(':disabled') && el.getClientRects().length > 0;
  return () => queueMicrotask(() => {
    if (previous && usable(previous)) {
      previous.focus();
      if (document.activeElement === previous) return;
    }
    const container = scope?.isConnected ? scope : document.querySelector<HTMLElement>('main');
    const fallback = Array.from(container?.querySelectorAll<HTMLElement>(
      'a[href], button, input, select, textarea, [tabindex]',
    ) ?? []).find((el) => el.tabIndex >= 0 && usable(el));
    fallback?.focus();
  });
}

export function dialogFocus(node: HTMLElement, onEscape: () => void) {
  const restoreFocus = dialogFocusReturn();
  const controls = () => Array.from(node.querySelectorAll<HTMLElement>(
    'a[href], button, input, select, textarea, [tabindex]',
  )).filter((el) => el.tabIndex >= 0 && !el.matches(':disabled') && !el.closest('[inert]') && el.getClientRects().length > 0);
  const topmost = () => {
    const dialogs = Array.from(document.querySelectorAll<HTMLElement>('[role="dialog"][aria-modal="true"]'))
      .filter((el) => el.getClientRects().length > 0);
    return dialogs.at(-1) === node;
  };
  const frame = requestAnimationFrame(() => {
    if (topmost() && !node.contains(document.activeElement)) controls()[0]?.focus();
  });
  function onKey(event: KeyboardEvent): void {
    if (event.defaultPrevented || !topmost()) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopImmediatePropagation();
      onEscape();
    } else if (event.key === 'Tab') {
      const items = controls();
      const index = items.indexOf(document.activeElement as HTMLElement);
      if (!items.length || index < 0 || (event.shiftKey ? index === 0 : index === items.length - 1)) {
        event.preventDefault();
        (event.shiftKey ? items.at(-1) : items[0])?.focus();
      }
    }
  }
  window.addEventListener('keydown', onKey);
  return {
    destroy() {
      cancelAnimationFrame(frame);
      window.removeEventListener('keydown', onKey);
      queueMicrotask(() => {
        if (document.activeElement === document.body || node.contains(document.activeElement)) restoreFocus();
      });
    },
  };
}
