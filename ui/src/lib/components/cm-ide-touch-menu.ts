// The IDE editor's context menu, shaped for touch. A phone has no ⌘B, no hover and no
// right-click (iOS shows its own Copy/Paste callout on a long-press in an editable area
// and never fires `contextmenu`), so the code's own verbs come first, as a short block,
// and no row carries a keyboard-shortcut hint a person on glass cannot press. The rest of
// the desktop menu follows unchanged (Cut/Copy/Paste, Agent, Refactor ›, Generate ›, …).
//
// Entry points: the phone tab row's ⋯ (`openIdeEditorMenu`, cm-ide-refactor.ts) and a
// long-press on Android (Chrome fires `contextmenu`); the desktop menu never changes.

import type { MenuItem } from '../contextmenu.svelte.ts';

export interface IdeTouchMenuVerbs {
  /** The identifier at the caret ('' = none: the symbol verbs are disabled). */
  symbol: string;
  readOnly: boolean;
  definition?: (() => void) | null;
  usages?: (() => void) | null;
  docs?: (() => void) | null;
  rename?: (() => void) | null;
  quickFix?: (() => void) | null;
  find?: (() => void) | null;
  replace?: (() => void) | null;
  goToLine?: (() => void) | null;
  /** Editor text size: −1 / +1 point (the IDE's font-size preference). */
  textSize?: ((delta: -1 | 1) => void) | null;
}

/** Labels the touch block replaces in the desktop menu (same verbs, said once). */
const HOISTED = new Set(['Show Context Actions', 'Find Usages', 'Rename…']);

/** Desktop rows minus their shortcut hints (recursively). */
export function withoutHints(items: readonly MenuItem[]): MenuItem[] {
  return items.map((it) => {
    const row: MenuItem = { ...it };
    delete row.hint;
    if (it.children) row.children = withoutHints(it.children);
    return row;
  });
}

/** The touch block: the code's verbs in the order a reader needs them. */
export function ideTouchVerbRows(v: IdeTouchMenuVerbs): MenuItem[] {
  const noSym = !v.symbol;
  const why = noSym ? 'Place the cursor on a name first' : undefined;
  const rows: MenuItem[] = [];
  const add = (label: string, icon: string, fn: (() => void) | null | undefined, opts: { needsSym?: boolean; writes?: boolean } = {}): void => {
    if (!fn) return;
    const disabled = Boolean((opts.needsSym && noSym) || (opts.writes && v.readOnly));
    rows.push({ label, icon, disabled, title: opts.needsSym ? why : undefined, action: fn });
  };
  add('Go to Definition', 'arrow', v.definition, { needsSym: true });
  add('Find Usages', 'search', v.usages, { needsSym: true });
  add('Show Documentation', 'book', v.docs, { needsSym: true });
  add('Rename…', 'edit', v.rename, { needsSym: true, writes: true });
  add('Quick Fix…', 'bulb', v.quickFix);
  if (rows.length) rows.push({ separator: true });
  const nav = rows.length;
  add('Find in File…', 'search', v.find);
  add('Replace in File…', 'swap', v.replace, { writes: true });
  add('Go to Line…', 'step', v.goToLine);
  if (v.textSize) {
    const ts = v.textSize;
    rows.push({ label: 'Text Size', icon: 'text', children: [
      { label: 'Larger', icon: 'plus', action: () => ts(1) },
      { label: 'Smaller', icon: 'minus', action: () => ts(-1) },
    ] });
  }
  if (rows.length > nav) rows.push({ separator: true });
  return rows;
}

/** The whole touch menu: the verb block, then the desktop rows without the hoisted
 *  duplicates or a leading separator, and without shortcut hints. */
export function touchShapeIdeMenu(desktop: readonly MenuItem[], v: IdeTouchMenuVerbs): MenuItem[] {
  const rest = withoutHints(desktop.filter((it) => !(it.label && HOISTED.has(it.label))));
  while (rest.length && rest[0].separator) rest.shift();
  // Collapse separators the hoisting left back to back.
  const tidy = rest.filter((it, i) => !(it.separator && rest[i - 1]?.separator));
  return [...ideTouchVerbRows(v), ...tidy];
}

/** True on a touch-first device (no hover, coarse pointer). */
export function isCoarsePointer(): boolean {
  return typeof matchMedia !== 'undefined' && matchMedia('(pointer: coarse)').matches;
}
