// Context menu store — show a native-feeling right-click menu anywhere in the app.

export interface MenuItem {
  label?: string;       // omit for a separator
  icon?: string;        // Icon name
  danger?: boolean;
  disabled?: boolean;
  separator?: boolean;  // true → render a divider
  action?: () => void;
}

class ContextMenuStore {
  open = $state(false);
  x = $state(0);
  y = $state(0);
  items: MenuItem[] = $state([]);

  show(e: MouseEvent | KeyboardEvent, items: MenuItem[]): void {
    e.preventDefault();
    e.stopPropagation();
    this.items = items;
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
