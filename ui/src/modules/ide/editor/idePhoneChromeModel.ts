// Which IDE chrome a phone shows, as one pure decision (unit-tested; the reactive
// wrapper is idePhoneChrome.svelte.ts).
//
// - Typing mode: the person is typing (focus in the code or the agent composer) and the
//   on-screen keyboard is up (the shell's measurement). The page header and its pane switcher
//   step aside, the crumbs row too; the file row stays only while it can be afforded —
//   on an iPhone SE with the keyboard up every row is a line of code.
// - Short stacked page (a phone on its side): the file row (active file ▾ + editor ⋯)
//   sits in the page header's free middle instead of costing its own row.

/** Below this visible height, typing mode drops the file row as well. */
export const TYPING_FILE_ROW_MIN_H = 300;

export interface PhoneChromeInput {
  stack: boolean;
  /** The IDE page's measured height. */
  pageH: number;
  /** The page is shorter than the stack's tall-page height (ideLayout STACK_MAX_H). */
  short: boolean;
  /** The page is as wide as the workbench threshold (ideLayout STACK_MAX_W): a phone on
   *  its side. A short narrow page (iPhone SE upright) has no room in its header. */
  wide: boolean;
  /** Focus is in the code editor or the agent composer. */
  typing: boolean;
  /** The shell's keyboard source of truth (lib/viewportFit.ts). */
  kbOpen: boolean;
  /** The visible area's height (0 = unknown). */
  visibleH: number;
}

export interface PhoneChrome {
  typingMode: boolean;
  /** Page header (title row + pane switcher). */
  header: boolean;
  /** The file row is drawn inside the page header. */
  fileRowInHeader: boolean;
  /** The file row is drawn above the code (its own row). */
  fileRow: boolean;
}

export function idePhoneChromeFor(i: PhoneChromeInput): PhoneChrome {
  if (!i.stack) return { typingMode: false, header: true, fileRowInHeader: false, fileRow: true };
  // Only a measured on-screen keyboard: focus alone (a hardware keyboard, a tap on the
  // code of a phone on its side) keeps the chrome — and the debugger's step bar.
  const typingMode = i.typing && i.kbOpen;
  if (typingMode) {
    const h = i.visibleH > 0 ? i.visibleH : i.pageH;
    return { typingMode, header: false, fileRowInHeader: false, fileRow: h >= TYPING_FILE_ROW_MIN_H };
  }
  const inHeader = i.short && i.wide;
  return { typingMode, header: true, fileRowInHeader: inHeader, fileRow: !inHeader };
}

/** Font-size bounds of the editor preference (cm-ide-prefs normalises to the same range). */
export const IDE_FONT_MIN = 10;
export const IDE_FONT_MAX = 24;

/** The editor font size after a Text size −/+ step (phone ⋯), inside the bounds. */
export function nextIdeFontSize(cur: number, delta: -1 | 1): number {
  const base = Number.isFinite(cur) ? Math.round(cur) : 13;
  return Math.max(IDE_FONT_MIN, Math.min(IDE_FONT_MAX, base + delta));
}
