// Global-shortcut chords in the shell's syntax (shortcuts.rs / global-hotkey,
// e.g. "Cmd+Ctrl+Shift+2"): record one from a keydown, show one as glyphs.
// Shared by the Settings pages that change a system-wide shortcut.

/** A keydown → "Cmd+Shift+K". null while only modifiers are held, or with no
 *  modifier at all (a bare letter would shadow normal typing system-wide). */
export function chordFromKey(e: KeyboardEvent): string | null {
  if (['Meta', 'Control', 'Alt', 'Shift'].includes(e.key)) return null;
  const mods = [e.metaKey && 'Cmd', e.ctrlKey && 'Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift'].filter(Boolean);
  if (!mods.length) return null;
  // The PHYSICAL key: with ⇧ or ⌥ held, `e.key` is the shifted/option
  // character ("@", "™"), which the shell can't parse as a chord.
  const c = e.code;
  let key = /^Key[A-Z]$/.test(c)
    ? c.slice(3)
    : /^Digit[0-9]$/.test(c)
      ? c.slice(5)
      : /^F\d{1,2}$/.test(c)
        ? c
        : e.key.length === 1
          ? e.key.toUpperCase()
          : e.key;
  if (key === ' ' || c === 'Space') key = 'Space';
  return [...mods, key].join('+');
}

/** "Cmd+Ctrl+Shift+2" → "⌘⌃⇧2"; "" → "—". */
export function prettyChord(a: string): string {
  if (!a) return '—';
  return a
    .split('+')
    .map((part) => ({ Cmd: '⌘', Ctrl: '⌃', Alt: '⌥', Shift: '⇧' })[part] ?? part)
    .join('');
}
