// Toast store — bottom-right notifications, auto-dismiss.

export interface Toast {
  id: number;
  level: 'info' | 'warn' | 'error' | 'success';
  title: string;
  body?: string;
}

let nextId = 1;

class ToastStore {
  toasts: Toast[] = $state([]);

  push(level: Toast['level'], title: string, body?: string, ttlMs = 4500): void {
    const id = nextId++;
    this.toasts = [...this.toasts, { id, level, title, body }];
    setTimeout(() => this.dismiss(id), ttlMs);
  }

  info(title: string, body?: string): void {
    this.push('info', title, body);
  }
  success(title: string, body?: string): void {
    this.push('success', title, body);
  }
  warn(title: string, body?: string): void {
    this.push('warn', title, body);
  }
  error(title: string, body?: string): void {
    this.push('error', title, body, 7000);
  }

  dismiss(id: number): void {
    this.toasts = this.toasts.filter((t) => t.id !== id);
  }
}

export const toasts = new ToastStore();
