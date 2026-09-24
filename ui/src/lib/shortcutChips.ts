// Cheat-sheet chord → <kbd> chips for the `?` overlay (ShortcutsOverlay.svelte).
// DOM-free so node:test can import it (lib/keys.ts touches window/document).

/** One rendered piece of a cheat-sheet chord: a `<kbd>` chip, or the plain
 *  separator between alternatives ("⌘U / ⌘⇧U"). */
export interface ShortcutChip {
  kind: 'key' | 'sep';
  text: string;
}

/** Split a display chord into cheat-sheet chips. A plain chord splits its
 *  modifier glyphs into separate keys ("⌘⇧B" → ⌘ ⇧ B); a RANGE ("⌃1…⌃9")
 *  stays ONE chip so it reads as a range, not as stray "1…" keys; " / "
 *  alternatives get a plain "/" separator between them. */
export function shortcutChips(keys: string): ShortcutChip[] {
  const out: ShortcutChip[] = [];
  keys.split(' / ').forEach((alt, i) => {
    if (i > 0) out.push({ kind: 'sep', text: '/' });
    if (alt.includes('…')) {
      out.push({ kind: 'key', text: alt });
      return;
    }
    for (const t of alt.match(/⌘|⌃|⌥|⇧|[^⌘⌃⌥⇧]+/g) ?? [alt]) out.push({ kind: 'key', text: t });
  });
  return out;
}
