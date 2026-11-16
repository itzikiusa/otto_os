// What the Threads page folds away while you type with the on-screen keyboard
// up (lib/viewportFit.ts measures it). The composer — and what the agent waits
// on, the attention dock — always stay; everything else gives its height back
// to the conversation, the least needed first, so a phone on its side
// (~200 px left above the keyboard) still shows a few lines of what you are
// replying to. All of it returns the moment the keyboard closes.

export interface TypingChrome {
  /** The page's header row (thread title, list, ⋯). */
  header: boolean;
  /** The plan line and the queued-messages fold above the composer. */
  banners: boolean;
  /** The terminal drawer under the conversation. */
  terminal: boolean;
  /** The composer on one row (text + attach + send; model / access hidden),
   *  the attention dock as its one-line head. */
  tight: boolean;
}

export const ALL_CHROME: TypingChrome = { header: true, banners: true, terminal: true, tight: false };

/** Visible heights (CSS px, keyboard up) under which each piece folds. */
export const TYPING_FOLD = {
  /** The terminal drawer (~240 px) would leave no conversation at all. */
  terminal: 560,
  /** Plan line + queue fold: ~70 px, about three lines of conversation. */
  banners: 420,
  /** Header and the two-row composer: a phone on its side, an iPhone SE. */
  tight: 340,
} as const;

/**
 * @param keyboardOpen the on-screen keyboard is up
 * @param composerFocused focus is in the thread's composer or its attention
 *   dock (typing into the terminal drawer keeps the drawer)
 * @param visibleHeight what is left above the keyboard, CSS px
 */
export function typingChrome(keyboardOpen: boolean, composerFocused: boolean, visibleHeight: number): TypingChrome {
  if (!keyboardOpen || visibleHeight <= 0) return ALL_CHROME;
  const tight = visibleHeight < TYPING_FOLD.tight;
  return {
    header: !tight,
    banners: !(composerFocused && visibleHeight < TYPING_FOLD.banners),
    terminal: !(composerFocused && visibleHeight < TYPING_FOLD.terminal),
    tight: composerFocused && tight,
  };
}

export type TypingFocus = 'composer' | 'dock' | null;

/** Where the caret is inside `el`: the thread's composer, the attention dock
 *  above it (a question's answer, a secret), or elsewhere (null). Focus to
 *  nowhere (Safari blurs the field when Send is tapped) keeps the typing
 *  layout through the tap, so Send does not move under the finger, then reads
 *  where focus really went. Returns the cleanup. */
export function trackComposerFocus(el: HTMLElement, set: (f: TypingFocus) => void): () => void {
  const where = (t: EventTarget | null): TypingFocus => {
    const at = t instanceof Element ? t : null;
    return at?.closest('.rt-composer') ? 'composer' : at?.closest('.rt-dock') ? 'dock' : null;
  };
  let settle: ReturnType<typeof setTimeout> | undefined;
  const onIn = (e: FocusEvent): void => {
    clearTimeout(settle);
    set(where(e.target));
  };
  const onOut = (e: FocusEvent): void => {
    clearTimeout(settle);
    if (e.relatedTarget) set(where(e.relatedTarget));
    else settle = setTimeout(() => set(where(document.activeElement)), 400);
  };
  el.addEventListener('focusin', onIn);
  el.addEventListener('focusout', onOut);
  return () => {
    clearTimeout(settle);
    el.removeEventListener('focusin', onIn);
    el.removeEventListener('focusout', onOut);
  };
}
