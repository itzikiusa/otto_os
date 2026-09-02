// In-app confirmation + prompt dialog. Native window.confirm()/prompt() do not
// work inside the Tauri WKWebView (they silently return false/null), which made
// every "delete" no-op and every "new/rename" (which used prompt) do nothing.
// Use `confirmer.ask(...)` for a yes/no and `confirmer.promptText(...)` for a
// line of text instead; both resolve a promise from a real in-app modal (see
// ConfirmDialog.svelte, mounted once in App).

/** One button in a `choose(...)` dialog. */
export interface ChoiceOption {
  label: string;
  value: string;
  kind?: 'primary' | 'danger' | 'normal';
}

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
  // Choice mode: N labeled buttons + an optional "remember" checkbox.
  choices: ChoiceOption[] | null = $state(null);
  checkboxLabel = $state('');
  checkboxChecked = $state(false);
  private resolver: ((v: boolean | string | null) => void) | null = null;
  private choiceResolver: ((v: { value: string | null; remember: boolean }) => void) | null = null;

  ask(
    message: string,
    opts?: { title?: string; confirmLabel?: string; danger?: boolean },
  ): Promise<boolean> {
    this.isPrompt = false;
    this.choices = null;
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
    this.choices = null;
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

  /**
   * Offer several labeled actions (plus Cancel) and an optional "remember"
   * checkbox. Resolves the picked option's `value` (`null` on cancel/dismiss)
   * and whether the checkbox was ticked.
   */
  choose(
    message: string,
    opts: { title?: string; options: ChoiceOption[]; checkboxLabel?: string },
  ): Promise<{ value: string | null; remember: boolean }> {
    this.isPrompt = false;
    this.message = message;
    this.title = opts.title ?? 'Confirm';
    this.choices = opts.options;
    this.checkboxLabel = opts.checkboxLabel ?? '';
    this.checkboxChecked = false;
    this.open = true;
    return new Promise((resolve) => {
      this.choiceResolver = resolve;
    });
  }

  /** Choice-mode resolution — one of the offered option values. */
  pick(value: string): void {
    this.open = false;
    const r = this.choiceResolver;
    this.choiceResolver = null;
    const remember = this.checkboxChecked;
    this.choices = null;
    r?.({ value, remember });
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

  /** Backdrop / X / Cancel — false for a confirm, null for a prompt/choice. */
  dismiss(): void {
    if (this.choiceResolver) {
      this.open = false;
      const cr = this.choiceResolver;
      this.choiceResolver = null;
      this.choices = null;
      cr?.({ value: null, remember: false });
      return;
    }
    const wasPrompt = this.isPrompt;
    this.open = false;
    const r = this.resolver;
    this.resolver = null;
    r?.(wasPrompt ? null : false);
  }
}

export const confirmer = new ConfirmStore();
