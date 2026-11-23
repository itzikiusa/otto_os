// Shared guard for global window shortcuts in Vault (`NoteView`, `VaultPage`).
// Prevents global ⌘/Ctrl shortcuts from firing while a modal dialog is open,
// and stops Ctrl-only chords (Ctrl+E / Ctrl+N / Ctrl+O) from hijacking
// readline/terminal keybindings inside the embedded RefineDrawer PTY or inputs.

export function shouldIgnoreVaultShortcut(
  e: Pick<KeyboardEvent, 'metaKey' | 'ctrlKey' | 'target'>,
  opts: { allowInEditor?: boolean; doc?: Document | null } = {},
): boolean {
  const doc = opts.doc ?? (typeof document !== 'undefined' ? document : null);
  if (!doc) return false;
  if (doc.querySelector('[role="dialog"][aria-modal="true"]')) return true;
  // Ctrl-only chords (no ⌘): never hijack terminal PTYs or text inputs.
  if (!e.metaKey && e.ctrlKey) {
    const raw = e.target;
    const isEl = (v: unknown): v is Element => !!v && typeof (v as Element).closest === 'function';
    const target = isEl(raw) ? raw : isEl(doc.activeElement) ? doc.activeElement : null;
    if (!target) return false;
    if (target.closest('.xterm')) return true;
    if (opts.allowInEditor) return false;
    if (target.closest('input, textarea, select, [contenteditable="true"], [contenteditable="plaintext-only"], .cm-editor')) {
      return true;
    }
  }
  return false;
}
