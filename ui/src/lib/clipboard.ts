// The single clipboard-write path for the whole UI.
//
// Why this exists: `navigator.clipboard` is a SECURE-CONTEXT API. Otto is
// routinely reached from a browser on another machine — the `0.0.0.0` TLS
// listener with its auto-generated self-signed cert, a LAN reverse proxy, an
// older WKWebView — and on any origin the browser de-privileges, the async
// Clipboard API is simply absent. Reading `.writeText` off an undefined
// `navigator.clipboard` throws a SYNCHRONOUS TypeError, so the usual
// `navigator.clipboard.writeText(x).catch(...)` doesn't even catch it: the
// throw happens while evaluating the expression, before a promise exists.
//
// The legacy `document.execCommand('copy')` path has none of those
// constraints — no secure context, no permission prompt — it only needs the
// call to happen inside a user gesture, which every Copy button already is.
// So: prefer the modern API, fall back to the legacy one, and never let a
// copy button become a silent no-op just because the page isn't on localhost.
//
// Deliberately NOT here: clipboard READS. `navigator.clipboard.readText()` is
// the one operation with no legacy equivalent, and it needs both a secure
// context and a permission grant. Everything that consumes the clipboard
// (terminal paste, image paste) rides the DOM `paste` event instead, which
// hands over `clipboardData` in any context — see `Terminal.svelte`.

/** Copy `text` to the clipboard. Prefers the async Clipboard API (secure
 *  contexts only); falls back to a hidden-textarea `execCommand('copy')` for
 *  http / cert-degraded origins and older webviews. Never throws — returns
 *  whether the copy is believed to have succeeded. */
export async function copyText(text: string): Promise<boolean> {
  // `document.hasFocus()` is part of the GUARD, not an optimisation. Chrome
  // rejects `writeText` with `NotAllowedError: Document is not focused` on an
  // unfocused document, and awaiting that rejection is fatal to the fallback:
  // once we have awaited, the user-gesture window has closed, so the
  // `execCommand` path in the catch fails too and BOTH routes die silently.
  // Checking first means the unfocused case never awaits at all — `legacyCopy`
  // runs synchronously, still inside the gesture that called us.
  const canUseAsync =
    typeof navigator !== 'undefined' &&
    !!navigator.clipboard &&
    window.isSecureContext &&
    (typeof document === 'undefined' || document.hasFocus());
  if (canUseAsync) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // Refused anyway (permission policy, a focus race, a cert-degraded
      // origin). Try the legacy path even though the gesture may be spent — it
      // sometimes still lands, and a failed attempt costs nothing.
    }
  }
  return legacyCopy(text);
}

/** `copyText` for the many call sites shaped as `try { copy } catch { toast }`:
 *  throws when the copy could not be made, so an existing catch block keeps
 *  reporting failure to the user instead of silently claiming success. The
 *  message is the toast body those blocks show, so it says what to do next
 *  (ux-design-6-07), not only what broke. */
export async function copyTextOrThrow(text: string): Promise<void> {
  if (!(await copyText(text))) {
    throw new Error(COPY_FAILED_BODY);
  }
}

/** The toast body every clipboard failure shows: what to do next, not the
 *  cause again (content.md: "Clipboard failures say what to do next"). */
export const COPY_FAILED_BODY = 'The clipboard isn’t available here. Select the text and copy it manually.';

/** A Copy button's whole job: copy, then say so. Success toasts `Copied
 *  {what}` (with the text itself as the body when it is short enough to read
 *  in a toast); failure toasts `Couldn’t copy` + the manual next step. Returns
 *  whether it copied, for a caller that also flips a "Copied ✓" label. */
export async function copyWithToast(text: string, what: string): Promise<boolean> {
  const ok = await copyText(text);
  // Loaded on use, not at the top: this file stays rune-free so plain-Node
  // importers (the code-block helpers and their unit tests) can load it.
  const { toasts } = await import('./toast.svelte');
  if (ok) toasts.success(`Copied ${what}`, text.length > 80 ? undefined : text);
  else toasts.error('Couldn’t copy', COPY_FAILED_BODY);
  return ok;
}

/** Hidden-textarea `execCommand('copy')`. Works on insecure origins and in
 *  webviews without the async API; must run inside a user gesture. */
function legacyCopy(text: string): boolean {
  if (typeof document === 'undefined') return false;
  const ta = document.createElement('textarea');
  ta.value = text;
  // Off-screen but focusable and NOT `display:none` — a hidden element can't
  // hold a selection, and iOS Safari refuses to copy from a zero-size box.
  ta.setAttribute('readonly', '');
  ta.style.position = 'fixed';
  ta.style.top = '0';
  ta.style.left = '0';
  ta.style.width = '1px';
  ta.style.height = '1px';
  ta.style.padding = '0';
  ta.style.border = 'none';
  ta.style.opacity = '0';
  // Restore focus afterwards: stealing it from a terminal/editor mid-gesture
  // would drop the caret the user is about to keep typing into.
  const prev = document.activeElement as HTMLElement | null;
  document.body.appendChild(ta);
  try {
    ta.focus();
    ta.select();
    ta.setSelectionRange(0, ta.value.length); // iOS ignores select() alone
    return document.execCommand('copy');
  } catch {
    return false;
  } finally {
    ta.remove();
    prev?.focus?.();
  }
}
