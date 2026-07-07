// Context menu store — show a native-feeling right-click menu anywhere in the app.

export interface MenuItem {
  label?: string;       // omit for a separator
  icon?: string;        // Icon name
  danger?: boolean;
  disabled?: boolean;
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

class ContextMenuStore {
  open = $state(false);
  x = $state(0);
  y = $state(0);
  items: MenuItem[] = $state([]);
  filter = $state(false);
  filterPlaceholder = $state('Filter…');
  maxVisible = $state(0);
  query = $state('');

  show(e: MouseEvent | KeyboardEvent, items: MenuItem[], opts?: MenuOptions): void {
    e.preventDefault();
    e.stopPropagation();
    this.items = items;
    this.filter = opts?.filter ?? false;
    this.filterPlaceholder = opts?.filterPlaceholder ?? 'Filter…';
    this.maxVisible = opts?.maxVisible ?? 0;
    this.query = '';
    if ('clientX' in e) {
      // Pointer / contextmenu: open at the cursor.
      this.x = e.clientX;
      this.y = e.clientY;
    } else {
      // Keyboard activation (Enter/Space on a focused row): anchor the menu to
      // the focused element so it's positioned sensibly without a cursor.
      const el = e.currentTarget as HTMLElement | null;
      const r = el?.getBoundingClientRect();
      this.x = r ? r.left : 0;
      this.y = r ? r.bottom : 0;
    }
    this.open = true;
  }

  close(): void {
    this.open = false;
  }
}

export const ctxMenu = new ContextMenuStore();
