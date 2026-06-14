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

  show(e: MouseEvent, items: MenuItem[]): void {
    e.preventDefault();
    e.stopPropagation();
    this.items = items;
    this.x = e.clientX;
    this.y = e.clientY;
    this.open = true;
  }

  close(): void {
    this.open = false;
  }
}

export const ctxMenu = new ContextMenuStore();
