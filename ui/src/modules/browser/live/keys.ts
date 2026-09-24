// Remote live browser — keyboard routing. No imports (unit-tested in node).
//
// Every keydown that reaches the live view's focus sink is routed ONE way:
//
//   app      — Otto owns it. The global key map (lib/keys.ts) runs on window
//              CAPTURE, before the view sees the event, and calls
//              preventDefault on every chord it handles (⌘K, ⌘T, ⌘W, ⌘J, ⌘1,
//              ⌘[ ⌘], ⌃Tab, ⌃1…9, ⌘± zoom, ⌘⇧R…). So `defaultPrevented` IS the
//              pass-through list — it can't drift from KEYMAP. System chords
//              (⌘Q ⌘H ⌘M ⌘` ⌘⇥) are also left alone.
//   url      — ⌘L: focus Otto's address bar (browser convention).
//   reload   — ⌘R: reload the remote page (not the Otto UI).
//   paste    — ⌘V: let the native paste event fire; the view reads the
//              clipboard text from it and sends an insert-text message.
//   release  — Esc: hand the keyboard back to Otto (blur the live view).
//              ⇧Esc sends a plain Escape to the page instead.
//   forward  — everything else goes to the remote page.

export interface KeyLike {
  key: string;
  code: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  repeat?: boolean;
  isComposing?: boolean;
  defaultPrevented?: boolean;
}

export type KeyRoute = 'app' | 'url' | 'reload' | 'paste' | 'release' | 'forward' | 'ignore';

/** CDP `Input.dispatchKeyEvent` modifier bit field: Alt=1 Ctrl=2 Meta=4 Shift=8. */
export function modifierMask(e: Pick<KeyLike, 'altKey' | 'ctrlKey' | 'metaKey' | 'shiftKey'>): number {
  return (e.altKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.metaKey ? 4 : 0) | (e.shiftKey ? 8 : 0);
}

/** The platform's "command" modifier: ⌘ on a Mac viewer, Ctrl elsewhere
 *  (a Windows/Linux viewer reaching Otto remotely presses Ctrl+C for copy). */
function primary(e: KeyLike, isMac: boolean): boolean {
  return isMac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
}

const SYSTEM_META_KEYS = new Set(['q', 'h', 'm', '`', 'tab']);

export function routeKey(e: KeyLike, isMac: boolean): KeyRoute {
  if (e.defaultPrevented) return 'app';
  // Composition keystrokes belong to the IME; the composed text arrives via
  // compositionend → insert text. (Safari reports keyCode 229 as key
  // "Process" instead of setting isComposing; a phone's soft keyboard sends
  // "Unidentified" keydowns and the real text via the `input` event.)
  if (e.isComposing || e.key === 'Process' || e.key === 'Dead' || e.key === 'Unidentified') return 'ignore';
  const k = e.key.toLowerCase();
  if (e.key === 'Escape') return e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey ? 'forward' : 'release';
  if (e.metaKey && SYSTEM_META_KEYS.has(k)) return 'app';
  if (primary(e, isMac) && !e.altKey && !e.shiftKey) {
    if (k === 'l') return 'url';
    if (k === 'r') return 'reload';
    if (k === 'v') return 'paste';
  }
  return 'forward';
}

/** Windows virtual key codes for the keys CDP can't infer from `key`/`code`
 *  (it needs them for Enter/Backspace/arrows to act in form fields). */
const VK: Record<string, number> = {
  Backspace: 8, Tab: 9, Enter: 13, Shift: 16, Control: 17, Alt: 18, Pause: 19,
  CapsLock: 20, Escape: 27, ' ': 32, PageUp: 33, PageDown: 34, End: 35, Home: 36,
  ArrowLeft: 37, ArrowUp: 38, ArrowRight: 39, ArrowDown: 40, Insert: 45, Delete: 46,
  Meta: 91, ContextMenu: 93,
  F1: 112, F2: 113, F3: 114, F4: 115, F5: 116, F6: 117, F7: 118, F8: 119, F9: 120,
  F10: 121, F11: 122, F12: 123,
};
const VK_BY_CODE: Record<string, number> = {
  Semicolon: 186, Equal: 187, Comma: 188, Minus: 189, Period: 190, Slash: 191,
  Backquote: 192, BracketLeft: 219, Backslash: 220, BracketRight: 221, Quote: 222,
};

export function virtualKeyCode(e: Pick<KeyLike, 'key' | 'code'>): number {
  if (e.key in VK) return VK[e.key];
  const m = /^Key([A-Z])$/.exec(e.code);
  if (m) return m[1].charCodeAt(0);
  const d = /^(?:Digit|Numpad)([0-9])$/.exec(e.code);
  if (d) return 48 + Number(d[1]);
  if (e.code in VK_BY_CODE) return VK_BY_CODE[e.code];
  if (e.key.length === 1) return e.key.toUpperCase().charCodeAt(0);
  return 0;
}

/** The text a keydown types, or undefined for a non-text key or a chord
 *  (⌘/Ctrl+letter types nothing — the command, if any, does the work). */
export function keyText(e: KeyLike): string | undefined {
  if (e.metaKey || e.ctrlKey) return undefined;
  if (e.key === 'Enter') return '\r';
  if (e.key.length === 1) return e.key;
  return undefined;
}

/** A key message body (sans `type`/`action`) — the contract's `key` frame.
 *  The daemon maps ⌘A/⌘C/⌘V/⌘X/⌘Z onto Chromium's editing commands itself. */
export interface KeyPayload {
  key: string;
  code: string;
  text?: string;
  modifiers: number;
  key_code: number;
  location: number;
  repeat: boolean;
}

export function keyPayload(e: KeyLike & { location?: number }, action: 'down' | 'up'): KeyPayload {
  // ⇧Esc is "send Escape to the page": strip the shift that routed it here.
  const ev = e.key === 'Escape' ? { ...e, shiftKey: false } : e;
  const out: KeyPayload = {
    key: ev.key,
    code: ev.code,
    modifiers: modifierMask(ev),
    key_code: virtualKeyCode(ev),
    location: ev.location ?? 0,
    repeat: !!ev.repeat,
  };
  if (action === 'down') {
    const text = keyText(ev);
    if (text !== undefined) out.text = text;
  }
  return out;
}
