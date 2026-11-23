// Per-composer caret (`selectionStart`, `selectionEnd`) and `scrollTop` memory
// shared across session tabs, the New-Thread stage, and `#/ide`
// <-> `#/threads` navigation. Stored in bounded in-memory maps cleared on auth
// identity transitions (`otto:ide-storage-cleared`).

import { trace } from '../composerTrace.ts';

export interface ComposerCursorState {
  selectionStart: number;
  selectionEnd: number;
  scrollTop: number;
  direction?: 'forward' | 'backward' | 'none';
}

export type ComposerViewStateArg =
  | string
  | {
      key: string;
      text?: string;
      hasText?: boolean;
    };

const MAX_COMPOSER_CURSOR_KEYS = 200;
const MAX_TRANSIENT_DRAFT_KEYS = 100;

const cursorStates = new Map<string, ComposerCursorState>();
const transientDrafts = new Map<string, string>();

function normalizeKey(raw: string | null | undefined): string {
  return (raw ?? '').trim();
}

export function saveComposerCursorState(
  key: string,
  patch: Partial<ComposerCursorState>,
): ComposerCursorState | null {
  const k = normalizeKey(key);
  if (!k) return null;
  const prev = cursorStates.get(k);
  const start =
    typeof patch.selectionStart === 'number' && Number.isFinite(patch.selectionStart)
      ? Math.max(0, Math.floor(patch.selectionStart))
      : (prev?.selectionStart ?? 0);
  const end =
    typeof patch.selectionEnd === 'number' && Number.isFinite(patch.selectionEnd)
      ? Math.max(start, Math.floor(patch.selectionEnd))
      : Math.max(start, prev?.selectionEnd ?? start);
  const scrollTop =
    typeof patch.scrollTop === 'number' && Number.isFinite(patch.scrollTop)
      ? Math.max(0, patch.scrollTop)
      : (prev?.scrollTop ?? 0);
  const next: ComposerCursorState = {
    selectionStart: start,
    selectionEnd: end,
    scrollTop,
    direction: patch.direction ?? prev?.direction ?? 'none',
  };
  if (cursorStates.has(k)) cursorStates.delete(k);
  cursorStates.set(k, next);
  if (cursorStates.size > MAX_COMPOSER_CURSOR_KEYS) {
    const oldest = cursorStates.keys().next().value;
    if (oldest !== undefined) cursorStates.delete(oldest);
  }
  return next;
}

export function getComposerCursorState(key: string): ComposerCursorState | null {
  const k = normalizeKey(key);
  if (!k) return null;
  const hit = cursorStates.get(k);
  return hit ? { ...hit } : null;
}

export function forgetComposerCursorState(key: string): void {
  const k = normalizeKey(key);
  if (!k) return;
  cursorStates.delete(k);
}

export function clearComposerCursorStates(): void {
  cursorStates.clear();
}

export function saveTransientComposerDraft(key: string, text: string): void {
  const k = normalizeKey(key);
  if (!k) return;
  if (!text) {
    transientDrafts.delete(k);
    return;
  }
  if (transientDrafts.has(k)) transientDrafts.delete(k);
  transientDrafts.set(k, text);
  if (transientDrafts.size > MAX_TRANSIENT_DRAFT_KEYS) {
    const oldest = transientDrafts.keys().next().value;
    if (oldest !== undefined) transientDrafts.delete(oldest);
  }
}

export function getTransientComposerDraft(key: string): string {
  const k = normalizeKey(key);
  return k ? (transientDrafts.get(k) ?? '') : '';
}

export function clearTransientComposerDrafts(): void {
  transientDrafts.clear();
}

export function clampComposerCursor(
  saved: ComposerCursorState,
  textLength: number,
): { selectionStart: number; selectionEnd: number; scrollTop: number; direction: 'forward' | 'backward' | 'none' } {
  const cap = Math.max(0, Math.floor(textLength));
  const selectionStart = Math.max(0, Math.min(saved.selectionStart, cap));
  const selectionEnd = Math.max(selectionStart, Math.min(saved.selectionEnd, cap));
  const scrollTop = Math.max(0, saved.scrollTop);
  return {
    selectionStart,
    selectionEnd,
    scrollTop,
    direction: saved.direction ?? 'none',
  };
}

export function restoreComposerCursorOnElement(
  node: Pick<HTMLTextAreaElement, 'value' | 'scrollTop' | 'setSelectionRange'>,
  key: string,
): boolean {
  const saved = getComposerCursorState(key);
  if (!saved) return false;
  const clamped = clampComposerCursor(saved, node.value.length);
  try {
    node.setSelectionRange(clamped.selectionStart, clamped.selectionEnd, clamped.direction);
  } catch {
    /* non-focusable or detached input */
  }
  node.scrollTop = clamped.scrollTop;
  return true;
}

function parseViewStateArg(arg: ComposerViewStateArg): { key: string; hasText: boolean | undefined } {
  if (typeof arg === 'string') return { key: normalizeKey(arg), hasText: undefined };
  const hasText = typeof arg.hasText === 'boolean' ? arg.hasText : typeof arg.text === 'string' ? arg.text.length > 0 : undefined;
  return { key: normalizeKey(arg.key), hasText };
}

/** Svelte action preserving `<textarea>` selection range and scroll offset per key
 *  without forcing synchronous layout reflows (`clientHeight`/`scrollTop`) on keystrokes. */
export function composerViewState(
  node: HTMLTextAreaElement,
  initial: ComposerViewStateArg,
): { update: (next: ComposerViewStateArg) => void; destroy: () => void } {
  let parsed = parseViewStateArg(initial);
  let currentKey = parsed.key;
  let lastHadText = parsed.hasText ?? (node.value ?? '').length > 0;
  let interacted = false;
  let restoring = false;
  let captureRaf = 0;

  const cancelCaptureRaf = (): void => {
    if (captureRaf && typeof cancelAnimationFrame === 'function') {
      cancelAnimationFrame(captureRaf);
    }
    captureRaf = 0;
  };

  // `why` tags each programmatic caret move for the composer trace (lib/composerTrace.ts).
  const applyRestore = (targetKey: string, why: string): void => {
    if (!targetKey || targetKey !== currentKey || interacted) return;
    if (!getComposerCursorState(targetKey)) return;
    if (node.value.length === 0) return;
    restoring = true;
    try {
      const from = [node.selectionStart, node.selectionEnd];
      restoreComposerCursorOnElement(node, targetKey);
      trace('caret-restore', { why, from: from.join(','), to: `${node.selectionStart},${node.selectionEnd}`, vlen: node.value.length });
    } finally {
      restoring = false;
    }
  };

  const scheduleRestore = (targetKey: string, why: string): void => {
    if (!targetKey) return;
    applyRestore(targetKey, why);
    queueMicrotask(() => applyRestore(targetKey, `${why}+microtask`));
    if (typeof requestAnimationFrame === 'function') {
      requestAnimationFrame(() => applyRestore(targetKey, `${why}+frame`));
    }
  };

  const writeSelection = (includeScroll: boolean): void => {
    if (restoring || !currentKey) return;
    if (!interacted && node.value.length === 0 && getComposerCursorState(currentKey)) return;
    const patch: Partial<ComposerCursorState> = {
      selectionStart: node.selectionStart ?? 0,
      selectionEnd: node.selectionEnd ?? 0,
      direction: node.selectionDirection ?? 'none',
    };
    if (includeScroll && node.isConnected && node.clientHeight > 0) {
      patch.scrollTop = node.scrollTop;
    }
    saveComposerCursorState(currentKey, patch);
  };

  const scheduleSelectionCapture = (markInteracted: boolean): void => {
    if (restoring || !currentKey) return;
    if (markInteracted) interacted = true;
    if (typeof requestAnimationFrame !== 'function') {
      writeSelection(false);
      return;
    }
    if (captureRaf) return;
    captureRaf = requestAnimationFrame(() => {
      captureRaf = 0;
      writeSelection(false);
    });
  };

  // Typing counts as touching the composer BEFORE its text changes. The
  // textarea's `bind:value` listener runs ahead of this action's `input`
  // listener, and the microtask checkpoint between them runs `update()`: a
  // composer that got its focus from the page (a thread just opened, no tap
  // yet) saw its first letter as "a draft arrived" and put the caret back at
  // the empty composer's saved {0,0} — "i also saw" went out as "also sawi".
  const onBeforeInput = (): void => {
    interacted = true;
  };
  const onInput = (): void => scheduleSelectionCapture(true);
  const onKeyUp = (e: KeyboardEvent): void => {
    // Printable edits already fire `input`; only navigation keys move caret without `input`.
    if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) return;
    scheduleSelectionCapture(true);
  };
  const onPointerUp = (): void => scheduleSelectionCapture(true);
  const onClick = (): void => scheduleSelectionCapture(true);
  const onSelect = (): void => {
    if (restoring || !interacted) return;
    scheduleSelectionCapture(false);
  };
  const onFocus = (): void => {
    if (!interacted && currentKey) {
      applyRestore(currentKey, 'focus');
    }
  };
  const onBlur = (): void => {
    cancelCaptureRaf();
    writeSelection(true);
  };
  const onScroll = (): void => {
    if (restoring || !currentKey) return;
    saveComposerCursorState(currentKey, { scrollTop: node.scrollTop });
  };

  node.addEventListener('beforeinput', onBeforeInput);
  node.addEventListener('input', onInput);
  node.addEventListener('keyup', onKeyUp);
  node.addEventListener('pointerup', onPointerUp);
  node.addEventListener('click', onClick);
  node.addEventListener('select', onSelect);
  node.addEventListener('focus', onFocus);
  node.addEventListener('blur', onBlur);
  node.addEventListener('scroll', onScroll, { passive: true });

  trace('caret-mount', { vlen: node.value.length, saved: !!getComposerCursorState(currentKey) });
  scheduleRestore(currentKey, 'mount');

  return {
    update(next: ComposerViewStateArg): void {
      const nextParsed = parseViewStateArg(next);
      const nextHasText = nextParsed.hasText ?? (node.value ?? '').length > 0;
      if (nextParsed.key !== currentKey) {
        cancelCaptureRaf();
        if (interacted) writeSelection(true);
        currentKey = nextParsed.key;
        interacted = false;
        lastHadText = nextHasText;
        trace('caret-key', { vlen: node.value.length });
        scheduleRestore(currentKey, 'key');
        return;
      }
      if (!interacted && !lastHadText && nextHasText) {
        lastHadText = true;
        scheduleRestore(currentKey, 'draft');
        return;
      }
      if (interacted && !nextHasText && lastHadText) {
        trace('caret-empty');
        cancelCaptureRaf();
        saveComposerCursorState(currentKey, { selectionStart: 0, selectionEnd: 0, scrollTop: 0 });
      }
      lastHadText = nextHasText;
    },
    destroy(): void {
      trace('caret-destroy', { interacted, vlen: node.value.length });
      cancelCaptureRaf();
      if (!restoring && currentKey && (interacted || (node.value.length > 0 && !getComposerCursorState(currentKey)))) {
        writeSelection(true);
      }
      node.removeEventListener('beforeinput', onBeforeInput);
      node.removeEventListener('input', onInput);
      node.removeEventListener('keyup', onKeyUp);
      node.removeEventListener('pointerup', onPointerUp);
      node.removeEventListener('click', onClick);
      node.removeEventListener('select', onSelect);
      node.removeEventListener('focus', onFocus);
      node.removeEventListener('blur', onBlur);
      node.removeEventListener('scroll', onScroll);
    },
  };
}

if (typeof window !== 'undefined' && typeof window.addEventListener === 'function') {
  window.addEventListener('otto:ide-storage-cleared', () => {
    clearComposerCursorStates();
    clearTransientComposerDrafts();
  });
}
