<script lang="ts">
  // TermKeysBar — phone-only accessory bar shown above the soft keyboard.
  // Provides keys that a mobile soft keyboard typically cannot produce.
  //
  // Every button calls `sendSeq(bytes)` which is passed in by Terminal.svelte —
  // that keeps the send path identical to what the physical keyboard uses
  // (sendJson({ type:'input', data: textToBase64(…) }), same readOnly guard,
  // same WS socket, respects can_input scoping from the server).
  //
  // The bar is only rendered when `viewport.isPhone`; Terminal.svelte gates the
  // mount so the desktop sees nothing.

  interface Props {
    /** Write a string of bytes to the PTY (same path Terminal.svelte's onData uses). */
    sendSeq: (seq: string) => void;
    /** Mirror Terminal.svelte's readOnly prop so buttons grey out for viewer shares. */
    readOnly?: boolean;
  }

  let { sendSeq, readOnly = false }: Props = $props();

  // Ctrl sticky-modifier state: when true, the next letter key press sends ctrl+letter.
  let ctrlActive = $state(false);

  function send(seq: string): void {
    if (readOnly) return;
    sendSeq(seq);
  }

  function toggleCtrl(): void {
    if (readOnly) return;
    ctrlActive = !ctrlActive;
  }

  // Keys that, when ctrl is active, produce their control-character equivalent.
  function sendCtrl(letter: string): void {
    if (readOnly) return;
    ctrlActive = false;
    // Ctrl+A–Z → \x01–\x1a
    const code = letter.toUpperCase().charCodeAt(0) - 64;
    sendSeq(String.fromCharCode(code));
  }

  // Shift+Enter: sends ESC+CR (\x1b\r) — the same sequence the custom key handler
  // in Terminal.svelte emits for a physical Shift+Enter (see Terminal.svelte ~line 404-423).
  function sendShiftEnter(): void {
    send('\x1b\r');
  }

  type KeyDef =
    | { label: string; seq: string; kind?: 'normal' | 'arrow' | 'sym' }
    | { label: string; action: () => void; kind?: 'normal' | 'ctrl'; active?: boolean };

  // Row 1: Esc, Tab, Ctrl (sticky), Ctrl-C, Shift+Enter
  const row1: KeyDef[] = [
    { label: 'Esc',    seq: '\x1b',   kind: 'normal' },
    { label: 'Tab',    seq: '\x09',   kind: 'normal' },
    { label: 'Ctrl',   action: toggleCtrl, kind: 'ctrl', get active() { return ctrlActive; } },
    { label: 'Ctrl-C', seq: '\x03',   kind: 'normal' },
    { label: '⇧↵',   action: sendShiftEnter, kind: 'normal' },
  ];

  // Row 2: arrow keys + common symbols
  const row2: KeyDef[] = [
    { label: '↑', seq: '\x1b[A', kind: 'arrow' },
    { label: '↓', seq: '\x1b[B', kind: 'arrow' },
    { label: '←', seq: '\x1b[D', kind: 'arrow' },
    { label: '→', seq: '\x1b[C', kind: 'arrow' },
    { label: '|',   seq: '|',    kind: 'sym' },
    { label: '/',   seq: '/',    kind: 'sym' },
    { label: '~',   seq: '~',    kind: 'sym' },
  ];

  function handleKey(k: KeyDef): void {
    if ('action' in k) {
      k.action();
    } else if ('seq' in k) {
      send(k.seq);
    }
  }
</script>

<div class="keys-bar" role="toolbar" aria-label="Terminal key shortcuts">
  <div class="keys-row">
    {#each row1 as k (k.label)}
      <button
        class="key-btn {k.kind ?? 'normal'}"
        class:active={'active' in k && k.active}
        class:disabled={readOnly}
        onclick={() => handleKey(k)}
        disabled={readOnly}
        aria-label={k.label}
        aria-pressed={'active' in k ? (k.active ?? false) : undefined}
      >{k.label}</button>
    {/each}
  </div>
  <div class="keys-row">
    {#each row2 as k (k.label)}
      <button
        class="key-btn {k.kind ?? 'normal'}"
        class:disabled={readOnly}
        onclick={() => handleKey(k)}
        disabled={readOnly}
        aria-label={k.label}
      >{k.label}</button>
    {/each}
  </div>
</div>

<style>
  .keys-bar {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 4px 6px;
    background: var(--surface-2, #1e1e24);
    border-top: 1px solid var(--border, #333);
    /* Prevent the bar itself from receiving touch-scroll events (it shouldn't scroll). */
    touch-action: none;
    user-select: none;
    -webkit-user-select: none;
  }

  .keys-row {
    display: flex;
    flex-direction: row;
    gap: 4px;
    justify-content: flex-start;
    flex-wrap: wrap;
  }

  .key-btn {
    /* iOS/Android: tap targets must be ≥44×44px (WCAG 2.5.5 / HIG). */
    min-height: 44px;
    min-width: 44px;
    padding: 0 10px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-s, 6px);
    border: 1px solid var(--border, #444);
    background: var(--surface, #28282e);
    color: var(--text, #e8e8e0);
    font-size: 14px;
    font-family: 'SF Mono', SFMono-Regular, Menlo, monospace;
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
    transition: background 0.1s;
    /* Prevent double-tap zoom on individual buttons */
    touch-action: manipulation;
    flex-shrink: 0;
  }

  .key-btn:active:not(:disabled) {
    background: var(--accent, #0066cc);
    color: #fff;
  }

  .key-btn.ctrl {
    background: var(--surface-2, #1e1e24);
    font-weight: 600;
  }

  .key-btn.ctrl.active {
    background: var(--accent, #0066cc);
    color: #fff;
    border-color: var(--accent, #0066cc);
  }

  .key-btn.arrow {
    min-width: 44px;
    font-size: 18px;
  }

  .key-btn.sym {
    font-size: 16px;
    min-width: 44px;
  }

  .key-btn:disabled,
  .key-btn.disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
</style>
