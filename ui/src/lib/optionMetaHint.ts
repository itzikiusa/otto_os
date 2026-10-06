// One-time hint for non-US keyboard layouts in the terminal (iteration-1
// follow-up, S19-12). With "Use Option as Meta key" on (the default — ⌥← /
// ⌥→ word jumps and ⌥⌫ for the agents' TUIs), ⌥ never composes a character,
// so on a German / French / Hebrew / … layout `@ [ ] { } | \ ~` can't be typed
// at all. The default stays; the FIRST time ⌥ would have composed one of
// those ASCII symbols, a toast says why nothing appeared and offers to turn
// the setting off. It never shows again (dismissed or acted on).
//
// Why ASCII punctuation is the signal: on the US layout every ⌥+key yields a
// non-ASCII glyph (å ∫ “ – ≠ ™ …), so a US user who relies on Meta never
// sees the hint; layouts that put ASCII symbols on ⌥ are exactly the ones the
// default breaks.

export const OPTION_META_HINT_KEY = 'otto_term_option_meta_hint_shown';

type KeyLike = Pick<KeyboardEvent, 'type' | 'key' | 'altKey' | 'metaKey' | 'ctrlKey'>;

/** Would this keydown have typed an ASCII symbol had ⌥ not been Meta? */
export function optionComposesAscii(e: KeyLike): boolean {
  if (e.type !== 'keydown' || !e.altKey || e.metaKey || e.ctrlKey) return false;
  if (e.key.length !== 1) return false;
  const c = e.key.charCodeAt(0);
  // Printable ASCII, not a letter/digit/space (⌥ on those is a Meta chord).
  return c > 0x20 && c < 0x7f && !/[A-Za-z0-9]/.test(e.key);
}

export interface OptionMetaHintDeps {
  optionAsMeta: () => boolean;
  shown: () => boolean;
  markShown: () => void;
  show: (key: string) => void;
}

/** Call from the terminal's key handler; shows the hint at most once ever. */
export function noteTerminalKey(e: KeyLike, deps: OptionMetaHintDeps): boolean {
  if (!deps.optionAsMeta() || deps.shown() || !optionComposesAscii(e)) return false;
  deps.markShown();
  deps.show(e.key);
  return true;
}
