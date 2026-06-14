// In-app confirmation dialog. Native window.confirm() does not work reliably
// inside the Tauri WKWebView (it silently returns false), which previously made
// every "delete" no-op. Use `confirmer.ask(...)` instead; it resolves a promise
// from a real in-app modal (see ConfirmDialog.svelte, mounted once in App).

class ConfirmStore {
  open = $state(false);
  title = $state('Confirm');
  message = $state('');
  confirmLabel = $state('Delete');
  danger = $state(true);
  private resolver: ((v: boolean) => void) | null = null;

  ask(
    message: string,
    opts?: { title?: string; confirmLabel?: string; danger?: boolean },
  ): Promise<boolean> {
    this.message = message;
    this.title = opts?.title ?? 'Confirm';
    this.confirmLabel = opts?.confirmLabel ?? 'Delete';
    this.danger = opts?.danger ?? true;
    this.open = true;
    return new Promise<boolean>((resolve) => {
      this.resolver = resolve;
    });
  }

  resolve(value: boolean): void {
    this.open = false;
    const r = this.resolver;
    this.resolver = null;
    r?.(value);
  }
}

export const confirmer = new ConfirmStore();
