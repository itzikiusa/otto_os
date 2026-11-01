// Guard against macOS AppKit / WKWebView leaking NSEvent Function-Key Unicodes
// (Apple Private Use Area U+F700..U+F8FF: arrows, F1..F35, Home, End, Forward
// Delete, PageUp/Down) into editable text controls as missing-glyph boxes `□`.
//
// In Tauri's WKWebView, certain responder transitions cause AppKit's
// `interpretKeyEvents:` to forward function keys to `insertText:` (`beforeinput`
// with `data: "\uF703"`) instead of executing the navigation selector
// (`moveRight:`). We intercept `beforeinput` in the capture phase to cancel the
// bogus insertion, apply the intended caret movement when `keydown` did not
// already move the selection, and scrub any residual U+F700..U+F8FF codepoints
// on `input` as a belt-and-suspenders fallback.

export const MAC_FUNCTION_KEY_CHAR_RE = /[\uF700-\uF8FF]/;
const MAC_FUNCTION_KEY_GLOBAL_RE = /[\uF700-\uF8FF]/g;

// NSEvent.h Function-Key Unicodes
export const NS_UP_ARROW = 0xf700;
export const NS_DOWN_ARROW = 0xf701;
export const NS_LEFT_ARROW = 0xf702;
export const NS_RIGHT_ARROW = 0xf703;
export const NS_DELETE_FORWARD = 0xf728;
export const NS_HOME = 0xf729;
export const NS_BEGIN = 0xf72a;
export const NS_END = 0xf72b;

export interface TextEditState {
  value: string;
  selectionStart: number;
  selectionEnd: number;
  selectionDirection?: 'forward' | 'backward' | 'none';
}

export interface KeyModifiers {
  shiftKey?: boolean;
  metaKey?: boolean;
  altKey?: boolean;
}

/** True when `s` contains any macOS AppKit function-key PUA character (`U+F700..U+F8FF`). */
export function hasMacFunctionKeyChar(s: string | null | undefined): boolean {
  return typeof s === 'string' && s.length > 0 && MAC_FUNCTION_KEY_CHAR_RE.test(s);
}

/** Strip any `U+F700..U+F8FF` characters from `state`, preserving caret position. */
export function stripMacFunctionKeyChars(state: TextEditState): TextEditState {
  if (!hasMacFunctionKeyChar(state.value)) return state;
  const beforeStart = state.value.slice(0, state.selectionStart).replace(MAC_FUNCTION_KEY_GLOBAL_RE, '');
  const beforeEnd = state.value.slice(0, state.selectionEnd).replace(MAC_FUNCTION_KEY_GLOBAL_RE, '');
  const clean = state.value.replace(MAC_FUNCTION_KEY_GLOBAL_RE, '');
  return {
    value: clean,
    selectionStart: beforeStart.length,
    selectionEnd: beforeEnd.length,
    selectionDirection: state.selectionDirection,
  };
}

function prevCodePointOffset(s: string, index: number): number {
  if (index <= 0) return 0;
  if (index >= 2) {
    const lo = s.charCodeAt(index - 1);
    const hi = s.charCodeAt(index - 2);
    if (lo >= 0xdc00 && lo <= 0xdfff && hi >= 0xd800 && hi <= 0xdbff) {
      return index - 2;
    }
  }
  return index - 1;
}

function nextCodePointOffset(s: string, index: number): number {
  if (index >= s.length) return s.length;
  const hi = s.charCodeAt(index);
  if (hi >= 0xd800 && hi <= 0xdbff && index + 1 < s.length) {
    const lo = s.charCodeAt(index + 1);
    if (lo >= 0xdc00 && lo <= 0xdfff) return index + 2;
  }
  return index + 1;
}

function lineStartOffset(s: string, index: number): number {
  if (index <= 0) return 0;
  const nl = s.lastIndexOf('\n', index - 1);
  return nl < 0 ? 0 : nl + 1;
}

function lineEndOffset(s: string, index: number): number {
  const nl = s.indexOf('\n', index);
  return nl < 0 ? s.length : nl;
}

function prevWordOffset(s: string, index: number): number {
  let i = index;
  while (i > 0 && /\s/.test(s[i - 1])) i--;
  while (i > 0 && !/\s/.test(s[i - 1])) i--;
  return i;
}

function nextWordOffset(s: string, index: number): number {
  let i = index;
  while (i < s.length && /\s/.test(s[i])) i++;
  while (i < s.length && !/\s/.test(s[i])) i++;
  return i;
}

function verticalLineOffset(s: string, index: number, dir: -1 | 1): number {
  const curStart = lineStartOffset(s, index);
  const col = index - curStart;
  if (dir < 0) {
    if (curStart === 0) return 0;
    const prevEnd = curStart - 1;
    const prevStart = lineStartOffset(s, prevEnd);
    return Math.min(prevEnd, prevStart + col);
  }
  const curEnd = lineEndOffset(s, index);
  if (curEnd >= s.length) return s.length;
  const nextStart = curEnd + 1;
  const nextEnd = lineEndOffset(s, nextStart);
  return Math.min(nextEnd, nextStart + col);
}

/**
 * Pure state transition for a misfired macOS function-key `beforeinput` payload.
 * Returns `{ next, mutatedValue }` when handled, or `null` if the codepoint is
 * a non-editing function key (e.g. F1..F12) whose insertion should simply be suppressed.
 */
export function applyMacFunctionKeyFallback(
  state: TextEditState,
  data: string,
  mods: KeyModifiers = {},
  multiline = true,
  editable = true,
): { next: TextEditState; mutatedValue: boolean } | null {
  if (!data) return null;
  let cur: TextEditState = state;
  let anyHandled = false;
  let mutatedValue = false;

  for (let i = 0; i < data.length; i++) {
    const code = data.charCodeAt(i);
    if (code < 0xf700 || code > 0xf8ff) continue;

    const { value, selectionStart: start, selectionEnd: end } = cur;
    const dir = cur.selectionDirection ?? 'none';
    const anchor = dir === 'backward' ? end : start;
    const head = dir === 'backward' ? start : end;

    if (code === NS_DELETE_FORWARD) {
      if (!editable) continue;
      if (start !== end) {
        cur = {
          value: value.slice(0, start) + value.slice(end),
          selectionStart: start,
          selectionEnd: start,
          selectionDirection: 'none',
        };
        anyHandled = true;
        mutatedValue = true;
      } else if (start < value.length) {
        const delTo = mods.altKey ? nextWordOffset(value, start) : nextCodePointOffset(value, start);
        cur = {
          value: value.slice(0, start) + value.slice(delTo),
          selectionStart: start,
          selectionEnd: start,
          selectionDirection: 'none',
        };
        anyHandled = true;
        mutatedValue = true;
      }
      continue;
    }

    let target: number | null = null;
    if (code === NS_LEFT_ARROW) {
      if (!mods.shiftKey && start !== end) {
        target = start;
      } else if (mods.metaKey) {
        target = lineStartOffset(value, head);
      } else if (mods.altKey) {
        target = prevWordOffset(value, head);
      } else {
        target = prevCodePointOffset(value, head);
      }
    } else if (code === NS_RIGHT_ARROW) {
      if (!mods.shiftKey && start !== end) {
        target = end;
      } else if (mods.metaKey) {
        target = lineEndOffset(value, head);
      } else if (mods.altKey) {
        target = nextWordOffset(value, head);
      } else {
        target = nextCodePointOffset(value, head);
      }
    } else if (code === NS_HOME || code === NS_BEGIN) {
      target = multiline && !mods.metaKey ? lineStartOffset(value, head) : 0;
    } else if (code === NS_END) {
      target = multiline && !mods.metaKey ? lineEndOffset(value, head) : value.length;
    } else if (code === NS_UP_ARROW) {
      target = mods.metaKey ? 0 : multiline ? verticalLineOffset(value, head, -1) : 0;
    } else if (code === NS_DOWN_ARROW) {
      target = mods.metaKey ? value.length : multiline ? verticalLineOffset(value, head, 1) : value.length;
    }

    if (target === null) continue;
    anyHandled = true;
    if (mods.shiftKey) {
      cur =
        target < anchor
          ? { value, selectionStart: target, selectionEnd: anchor, selectionDirection: 'backward' }
          : { value, selectionStart: anchor, selectionEnd: target, selectionDirection: 'forward' };
    } else {
      cur = { value, selectionStart: target, selectionEnd: target, selectionDirection: 'none' };
    }
  }

  return anyHandled ? { next: cur, mutatedValue } : null;
}

interface KeydownSnapshot {
  key: string;
  start: number;
  end: number;
  len: number;
  shiftKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  at: number;
}

const NAVIGATION_KEYS = new Set([
  'ArrowLeft',
  'ArrowRight',
  'ArrowUp',
  'ArrowDown',
  'Home',
  'End',
  'PageUp',
  'PageDown',
  'Delete',
]);

function isSelectableTextControl(
  el: EventTarget | null,
): el is HTMLTextAreaElement | HTMLInputElement {
  if (typeof HTMLTextAreaElement !== 'undefined' && el instanceof HTMLTextAreaElement) return true;
  if (typeof HTMLInputElement !== 'undefined' && el instanceof HTMLInputElement) {
    return ['text', 'search', 'url', 'tel', 'password', ''].includes(el.type);
  }
  return false;
}

/** Install the capture-phase WKWebView function-key guard on `doc`. Idempotent. */
export function installMacFunctionKeyGuard(doc: Document = document): () => void {
  const snapshots = new WeakMap<HTMLTextAreaElement | HTMLInputElement, KeydownSnapshot>();

  const onKeyDown = (e: KeyboardEvent): void => {
    if (!isSelectableTextControl(e.target)) return;
    const el = e.target;
    // xterm's hidden helper textarea already translated the keydown into VT
    // escape sequences; never synthesize DOM text edits on its buffer.
    if (el.classList?.contains('xterm-helper-textarea')) return;
    try {
      snapshots.set(el, {
        key: e.key,
        start: el.selectionStart ?? 0,
        end: el.selectionEnd ?? 0,
        len: el.value.length,
        shiftKey: e.shiftKey,
        metaKey: e.metaKey,
        altKey: e.altKey,
        at: performance.now(),
      });
    } catch {
      /* input types without selectionStart */
    }
  };

  const onBeforeInput = (e: Event): void => {
    const ie = e as InputEvent;
    const data = ie.data;
    if (!hasMacFunctionKeyChar(data) || !data) return;

    // Always block the PUA box character from entering the DOM / text buffer.
    e.preventDefault();
    e.stopImmediatePropagation();

    if (!isSelectableTextControl(e.target)) return;
    const el = e.target;
    // xterm's hidden helper textarea already translated the keydown into VT
    // escape sequences; never synthesize DOM text edits on its buffer.
    if (el.classList?.contains('xterm-helper-textarea')) return;
    let start = 0;
    let end = 0;
    let dir: 'forward' | 'backward' | 'none' = 'none';
    try {
      start = el.selectionStart ?? 0;
      end = el.selectionEnd ?? 0;
      dir = (el.selectionDirection as 'forward' | 'backward' | 'none' | null) ?? 'none';
    } catch {
      return;
    }

    const snap = snapshots.get(el);
    const now = performance.now();
    // Skip fallback only when a navigation keydown just fired on this exact buffer
    // AND WebKit already advanced the selection before dispatching `beforeinput`.
    const webkitAlreadyMovedSelection =
      !!snap &&
      now - snap.at <= 80 &&
      NAVIGATION_KEYS.has(snap.key) &&
      snap.len === el.value.length &&
      (snap.start !== start || snap.end !== end);
    if (webkitAlreadyMovedSelection) return;

    const mods: KeyModifiers =
      snap && now - snap.at <= 80 && NAVIGATION_KEYS.has(snap.key)
        ? { shiftKey: snap.shiftKey, metaKey: snap.metaKey, altKey: snap.altKey }
        : {};
    const multiline = typeof HTMLTextAreaElement !== 'undefined' && el instanceof HTMLTextAreaElement;
    const editable = !el.readOnly && !el.disabled;
    const applied = applyMacFunctionKeyFallback(
      { value: el.value, selectionStart: start, selectionEnd: end, selectionDirection: dir },
      data,
      mods,
      multiline,
      editable,
    );
    if (!applied) return;

    if (applied.mutatedValue) {
      el.value = applied.next.value;
      el.setSelectionRange(
        applied.next.selectionStart,
        applied.next.selectionEnd,
        applied.next.selectionDirection ?? 'none',
      );
      el.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'deleteContentForward' }));
    } else {
      el.setSelectionRange(
        applied.next.selectionStart,
        applied.next.selectionEnd,
        applied.next.selectionDirection ?? 'none',
      );
    }
  };

  const onInput = (e: Event): void => {
    if (!isSelectableTextControl(e.target)) return;
    const el = e.target;
    snapshots.delete(el);
    if (!hasMacFunctionKeyChar(el.value)) return;
    let start = el.value.length;
    let end = el.value.length;
    try {
      start = el.selectionStart ?? start;
      end = el.selectionEnd ?? end;
    } catch {
      /* ignore */
    }
    const cleaned = stripMacFunctionKeyChars({
      value: el.value,
      selectionStart: start,
      selectionEnd: end,
    });
    el.value = cleaned.value;
    try {
      el.setSelectionRange(cleaned.selectionStart, cleaned.selectionEnd);
    } catch {
      /* ignore */
    }
    if (el.classList?.contains('xterm-helper-textarea') && cleaned.value === '') {
      e.stopImmediatePropagation();
    }
  };

  doc.addEventListener('keydown', onKeyDown, true);
  doc.addEventListener('beforeinput', onBeforeInput, true);
  doc.addEventListener('input', onInput, true);
  return () => {
    doc.removeEventListener('keydown', onKeyDown, true);
    doc.removeEventListener('beforeinput', onBeforeInput, true);
    doc.removeEventListener('input', onInput, true);
  };
}
