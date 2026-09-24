// Context menu store — show a native-feeling right-click menu anywhere in the app.
//
// Two ways in:
//   • `ctxMenu.show(e, items)` — from an event. A right-click (`contextmenu`)
//     opens at the cursor. A CLICK on a button (a "⋯" / "New ▾" trigger) or a
//     keyboard activation anchors the menu UNDER that control instead, exactly
//     like `showAt(e.currentTarget, …)`.
//   • `ctxMenu.showAt(el, items, { align })` — anchored to an element: below
//     it, its start edge (`align: 'start'`, default) or end edge (`'end'`)
//     aligned with the trigger's, mirrored in RTL. ContextMenu.svelte measures
//     the real menu, flips above only when that fits, and clamps into the
//     viewport otherwise.
// Either way focus moves into the menu (item 0) and returns to the trigger —
// or to whatever had focus — when it closes.

export interface MenuItem {
  label?: string;       // omit for a separator
  icon?: string;        // Icon name
  danger?: boolean;
  disabled?: boolean;
  /** A checkable row (sort mode, view, zoom…): `true` draws a check mark in
   *  the icon slot, `false` keeps the slot empty so labels stay aligned. */
  checked?: boolean;
  separator?: boolean;  // true → render a divider
  /** In a filterable menu: always visible, never filtered or capped (use for
   *  the fixed action rows above a long data-driven list). */
  pinned?: boolean;
  action?: () => void;
}

export interface MenuOptions {
  /** Show a search input pinned at the top; typing filters non-pinned items. */
  filter?: boolean;
  filterPlaceholder?: string;
  /** Max non-pinned items shown at once (0 = unlimited). The rest collapse
   *  into a "+N more" hint until the query narrows the list. */
  maxVisible?: number;
}

export interface AnchorOptions extends MenuOptions {
  /** Which trigger edge the menu lines up with (logical: mirrored in RTL). */
  align?: 'start' | 'end';
}

/** The trigger's box, captured when the menu opens. */
export interface AnchorRect {
  top: number;
  bottom: number;
  left: number;
  right: number;
}

const TRIGGER = 'button, a[href], [role="button"], [aria-haspopup], summary';

function visibleRect(el: Element | null | undefined): DOMRect | null {
  if (!el || !el.isConnected) return null;
  const r = el.getBoundingClientRect();
  return r.width > 0 || r.height > 0 ? r : null;
}

class ContextMenuStore {
  open = $state(false);
  x = $state(0);
  y = $state(0);
  /** Set for an anchored menu (showAt / button click); null → open at x/y. */
  anchor: AnchorRect | null = $state(null);
  align: 'start' | 'end' = $state('start');
  items: MenuItem[] = $state([]);
  filter = $state(false);
  filterPlaceholder = $state('Filter…');
  maxVisible = $state(0);
  query = $state('');
  /** Bumped on every open, so a menu re-opened in the same tick (a ⋯ row
   *  whose action opens another menu) is re-measured and re-focused. */
  seq = $state(0);

  /** Where focus goes back to on close (not reactive). */
  private returnTo: HTMLElement | null = null;
  /** The last trigger — the fallback anchor for a menu opened by a control
   *  that is itself hidden (a button collapsed into PageHeader's ⋯ menu). */
  private lastTrigger: HTMLElement | null = null;

  private reset(items: MenuItem[], opts?: MenuOptions): void {
    this.items = items;
    this.filter = opts?.filter ?? false;
    this.filterPlaceholder = opts?.filterPlaceholder ?? 'Filter…';
    this.maxVisible = opts?.maxVisible ?? 0;
    this.query = '';
  }

  private rememberFocus(trigger: HTMLElement | null): void {
    // WebKit does not focus a clicked button, so prefer the trigger itself.
    const active = document.activeElement;
    const inMenu = active instanceof HTMLElement && !!active.closest('.ctx-menu');
    this.returnTo =
      trigger ?? (active instanceof HTMLElement && active !== document.body && !inMenu ? active : this.returnTo);
  }

  show(e: MouseEvent | KeyboardEvent, items: MenuItem[], opts?: MenuOptions): void {
    e.preventDefault();
    e.stopPropagation();
    const el = e.currentTarget instanceof HTMLElement ? e.currentTarget : null;
    const keyboard = !('clientX' in e) || (e.type === 'click' && (e as MouseEvent).detail === 0);
    // A button-opened menu belongs under its button, not at the pointer. (A
    // tall clickable card keeps the pointer position: "under it" is far away.)
    const small = !!el && el.getBoundingClientRect().height <= 48;
    if (el && (keyboard || (e.type === 'click' && small && el.matches(TRIGGER)))) {
      this.showAt(el, items, opts);
      return;
    }
    this.reset(items, opts);
    this.anchor = null;
    this.x = (e as MouseEvent).clientX ?? 0;
    this.y = (e as MouseEvent).clientY ?? 0;
    this.rememberFocus(null);
    this.seq++;
    this.open = true;
  }

  showAt(el: Element | null | undefined, items: MenuItem[], opts?: AnchorOptions): void {
    const trigger = el instanceof HTMLElement ? el : null;
    let target: HTMLElement | null = trigger;
    let r = visibleRect(trigger);
    if (!r) {
      // Hidden trigger (e.g. clicked from PageHeader's ⋯ menu): anchor to the
      // control that opened the previous menu instead.
      r = visibleRect(this.lastTrigger);
      target = r ? this.lastTrigger : null;
    }
    this.reset(items, opts);
    this.align = opts?.align ?? 'start';
    if (r) {
      this.anchor = { top: r.top, bottom: r.bottom, left: r.left, right: r.right };
    } else {
      // Nothing visible to hang it on: centre-top of the window.
      this.anchor = null;
      this.x = Math.round(window.innerWidth / 2 - 100);
      this.y = Math.round(window.innerHeight / 4);
    }
    if (target) this.lastTrigger = target;
    this.rememberFocus(target);
    this.seq++;
    this.open = true;
  }

  close(): void {
    if (!this.open) return;
    this.open = false;
    // Hand focus back only when it is still ours to give: in the menu (about
    // to unmount) or nowhere. An action that focused something else keeps it.
    const active = document.activeElement;
    const ours = !active || active === document.body || (active instanceof HTMLElement && !!active.closest('.ctx-menu'));
    const back = this.returnTo;
    this.returnTo = null;
    if (ours && back?.isConnected) back.focus({ preventScroll: true });
  }
}

export const ctxMenu = new ContextMenuStore();
