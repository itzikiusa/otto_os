// In-app confirmation + prompt dialog. Native window.confirm()/prompt() do not
// work inside the Tauri WKWebView (they silently return false/null), which made
// every "delete" no-op and every "new/rename" (which used prompt) do nothing.
// Use `confirmer.ask(...)` for a yes/no and `confirmer.promptText(...)` for a
// line of text instead; both resolve a promise from a real in-app modal (see
// ConfirmDialog.svelte, mounted once in App).

class ConfirmStore {
  open = $state(false);
  title = $state('Confirm');
  message = $state('');
  confirmLabel = $state('Delete');
  danger = $state(true);
  // Prompt (text-input) mode.
  isPrompt = $state(false);
  inputValue = $state('');
  placeholder = $state('');
  private resolver: ((v: boolean | string | null) => void) | null = null;

  ask(
    message: string,
    opts?: { title?: string; confirmLabel?: string; danger?: boolean },
  ): Promise<boolean> {
    this.isPrompt = false;
    this.message = message;
    this.title = opts?.title ?? 'Confirm';
    this.confirmLabel = opts?.confirmLabel ?? 'Delete';
    this.danger = opts?.danger ?? true;
    this.open = true;
    return new Promise<boolean>((resolve) => {
      this.resolver = resolve as (v: boolean | string | null) => void;
    });
  }

  /**
   * Prompt for a single line of text. Resolves the trimmed value, or `null` if
   * the user cancels or leaves it empty.
   */
  promptText(
    message: string,
    opts?: { title?: string; confirmLabel?: string; initial?: string; placeholder?: string },
  ): Promise<string | null> {
    this.isPrompt = true;
    this.message = message;
    this.title = opts?.title ?? 'Enter a value';
    this.confirmLabel = opts?.confirmLabel ?? 'OK';
    this.danger = false;
    this.inputValue = opts?.initial ?? '';
    this.placeholder = opts?.placeholder ?? '';
    this.open = true;
    return new Promise<string | null>((resolve) => {
      this.resolver = resolve as (v: boolean | string | null) => void;
    });
  }

  /** Confirm-mode resolution (true on confirm). */
  resolve(value: boolean): void {
    this.open = false;
    const r = this.resolver;
    this.resolver = null;
    r?.(value);
  }

  /** Prompt-mode OK — resolve the trimmed input (null when empty). */
  submit(): void {
    const v = this.inputValue.trim();
    this.open = false;
    const r = this.resolver;
    this.resolver = null;
    r?.(v ? v : null);
  }

  /** Backdrop / X / Cancel — false for a confirm, null for a prompt. */
  dismiss(): void {
    const wasPrompt = this.isPrompt;
    this.open = false;
    const r = this.resolver;
    this.resolver = null;
    r?.(wasPrompt ? null : false);
  }
}

export const confirmer = new ConfirmStore();
