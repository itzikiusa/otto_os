// Dialog keyboard behavior (cell viewer / doc editor / review modal).
import { tick } from 'svelte';

/** Minimal dialog a11y as a Svelte action: focus moves into the dialog on
 *  open, Tab cycles inside it, Escape closes (a child that stopPropagation's
 *  Escape — the editor textareas — wins), and focus returns on close. */
export function dialogKeys(node: HTMLElement, onEscape: () => void) {
  const focusables = (): HTMLElement[] =>
    [
      ...node.querySelectorAll<HTMLElement>(
        'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ),
    ].filter((el) => el.offsetParent !== null);
  const prev = document.activeElement as HTMLElement | null;
  void tick().then(() => {
    if (!node.contains(document.activeElement)) focusables()[0]?.focus();
  });
  const onKey = (e: KeyboardEvent): void => {
    if (e.key === 'Escape') {
      e.stopPropagation();
      onEscape();
      return;
    }
    if (e.key !== 'Tab') return;
    const els = focusables();
    if (els.length === 0) return;
    const first = els[0];
    const last = els[els.length - 1];
    const active = document.activeElement;
    if (e.shiftKey && (active === first || !node.contains(active))) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && active === last) {
      e.preventDefault();
      first.focus();
    }
  };
  node.addEventListener('keydown', onKey);
  return {
    destroy() {
      node.removeEventListener('keydown', onKey);
      prev?.focus?.();
    },
  };
}
