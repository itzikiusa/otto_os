// `use:rowMenu` — the keyboard and touch way into a row's right-click menu.
//
// Many rows/tree nodes open their actions with `oncontextmenu` only, which a
// keyboard user (no secondary click) and a touch screen (no right button)
// can't reach. This action adds the two standard paths and routes BOTH into
// the element's existing `oncontextmenu` handler by dispatching a synthetic
// `contextmenu` event on it — so the handler (usually `ctxMenu.show(e, …)`)
// stays the single source of the menu:
//
//   • keyboard — the ContextMenu ("Menu") key or ⇧F10 while the element
//     itself has focus (not a field inside it). The menu opens under the
//     element's start edge, as for a ⋯ button.
//   • touch / pen — a long press (500 ms, < 10 px of movement). The click
//     that ends the press is swallowed so the row doesn't also open.
//
// Platforms whose long press already fires a native `contextmenu` (Android)
// keep theirs; the synthetic one is only sent when none arrived.
//
//   <tr tabindex="0" onclick={open} oncontextmenu={(e) => menu(e, row)} use:rowMenu>

const LONG_PRESS_MS = 500;
const SLOP_PX = 10;

/** Is this keydown the platform's "open the context menu" chord? */
export function isMenuKey(e: Pick<KeyboardEvent, 'key' | 'shiftKey' | 'metaKey' | 'ctrlKey' | 'altKey'>): boolean {
  if (e.metaKey || e.ctrlKey || e.altKey) return false;
  return e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey);
}

/** Where a keyboard-opened menu goes: the element's start edge, just below
 *  its top line (mirrored in RTL). */
export function keyboardMenuPoint(rect: { left: number; right: number; top: number; bottom: number }, rtl: boolean): { x: number; y: number } {
  return { x: Math.round(rtl ? rect.right - 8 : rect.left + 8), y: Math.round(Math.min(rect.bottom, rect.top + 24)) };
}

function fire(node: HTMLElement, x: number, y: number): void {
  node.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: x, clientY: y, button: 2 }));
}

export function rowMenu(node: HTMLElement): { destroy(): void } {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let start: { x: number; y: number } | null = null;
  let nativeSeen = false;
  let swallowClick = false;

  const cancel = (): void => {
    if (timer) clearTimeout(timer);
    timer = null;
    start = null;
  };

  function onKeydown(e: KeyboardEvent): void {
    if (e.target !== node || !isMenuKey(e)) return;
    e.preventDefault();
    e.stopPropagation();
    const rtl = getComputedStyle(node).direction === 'rtl';
    const p = keyboardMenuPoint(node.getBoundingClientRect(), rtl);
    fire(node, p.x, p.y);
  }

  function onPointerDown(e: PointerEvent): void {
    if (e.pointerType === 'mouse' || !e.isPrimary) return;
    cancel();
    nativeSeen = false;
    swallowClick = false;
    start = { x: e.clientX, y: e.clientY };
    timer = setTimeout(() => {
      timer = null;
      if (!start || nativeSeen) return;
      swallowClick = true;
      fire(node, start.x, start.y);
      start = null;
    }, LONG_PRESS_MS);
  }

  function onPointerMove(e: PointerEvent): void {
    if (start && Math.hypot(e.clientX - start.x, e.clientY - start.y) > SLOP_PX) cancel();
  }

  function onNativeMenu(e: Event): void {
    // A real (trusted) contextmenu during the press: the platform handles it.
    if (e.isTrusted && timer) {
      nativeSeen = true;
      cancel();
    }
  }

  function onClickCapture(e: MouseEvent): void {
    if (!swallowClick) return;
    swallowClick = false;
    e.preventDefault();
    e.stopPropagation();
  }

  node.addEventListener('keydown', onKeydown);
  node.addEventListener('pointerdown', onPointerDown);
  node.addEventListener('pointermove', onPointerMove);
  node.addEventListener('pointerup', cancel);
  node.addEventListener('pointercancel', cancel);
  node.addEventListener('contextmenu', onNativeMenu, true);
  node.addEventListener('click', onClickCapture, true);
  return {
    destroy() {
      cancel();
      node.removeEventListener('keydown', onKeydown);
      node.removeEventListener('pointerdown', onPointerDown);
      node.removeEventListener('pointermove', onPointerMove);
      node.removeEventListener('pointerup', cancel);
      node.removeEventListener('pointercancel', cancel);
      node.removeEventListener('contextmenu', onNativeMenu, true);
      node.removeEventListener('click', onClickCapture, true);
    },
  };
}
