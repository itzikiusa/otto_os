// The IDE's keyboard shortcuts, ONE table per keymap preset. Everything that shows
// or handles an IDE key reads it: the window key handler (`ideShellCommands.ts`),
// the hints in ⋯ / ⌘K / quick-open (`ideActions.ts`), the Settings › Keymap table
// and the debugger toolbar's tooltips. A row lists only keys that really work:
// `unit/ideActionsShortcuts.test.ts` presses every chord of every row against its
// handler (window rows), the CodeMirror keymap (editor rows) or IdePage's double-⇧
// detector (page rows), and fails on a key that does nothing.
//
// Pure TS without imports, so Node runs it directly in unit tests.

/** Same union as `store/ideTypes.ts` `IdeKeymapPreset` (kept import-free here). */
export type IdeKeymapPreset = 'jetbrains' | 'standard';

export interface IdeChord {
  /** `KeyboardEvent.key` (any case) or a named key: 'p', ',', '`', 'F5', 'ArrowDown'. */
  key: string;
  /** ⌘ on a Mac, Ctrl elsewhere (exactly one of the two, as every IDE chord always matched). */
  mod?: boolean;
  /** A literal ⌃. With `meta` too: both held (⌃⇧⌘R). */
  ctrl?: boolean;
  /** A literal ⌘, only next to `ctrl`. */
  meta?: boolean;
  alt?: boolean;
  shift?: boolean;
  /** Two quick taps of ⇧ alone (IdePage's keyup detector). */
  double?: boolean;
}

/** Who handles a row: `window` = the handler map in `ideShellCommands.ts`,
 *  `editor` = `cm-ide-keymap.ts` (focus in the code), `page` = IdePage (double ⇧). */
export type IdeShortcutScope = 'window' | 'editor' | 'page';

export type IdeShortcutCategory = 'IDE' | 'Navigate' | 'Code' | 'Editor' | 'Tabs' | 'Windows' | 'Agent' | 'Run' | 'Debugger';

export interface IdeShortcutRow {
  id: string;
  title: string;
  category: IdeShortcutCategory;
  scope: IdeShortcutScope;
  jetbrains: readonly IdeChord[];
  standard: readonly IdeChord[];
}

const m = (key: string, extra: Omit<IdeChord, 'key' | 'mod'> = {}): IdeChord => ({ key, mod: true, ...extra });
const k = (key: string, extra: Omit<IdeChord, 'key'> = {}): IdeChord => ({ key, ...extra });
const both = (...chords: IdeChord[]) => ({ jetbrains: chords, standard: chords });

export const IDE_SHORTCUTS = [
  { id: 'settings', title: 'Settings & project structure', category: 'IDE', scope: 'window', ...both(m(',')) },
  { id: 'hardReload', title: 'Hard reload window', category: 'IDE', scope: 'window', ...both(k('r', { ctrl: true, meta: true, shift: true })) },
  { id: 'searchEverywhere', title: 'Search everywhere', category: 'Navigate', scope: 'page', ...both(k('Shift', { double: true })) },
  { id: 'goToFile', title: 'Go to file', category: 'Navigate', scope: 'window', jetbrains: [m('p'), m('o', { shift: true })], standard: [m('p')] },
  { id: 'goToSymbol', title: 'Go to symbol', category: 'Navigate', scope: 'window', jetbrains: [m('o')], standard: [m('o', { shift: true })] },
  { id: 'goToLine', title: 'Go to line : column', category: 'Navigate', scope: 'window', jetbrains: [m('g')], standard: [k('g', { ctrl: true })] },
  { id: 'recentFiles', title: 'Recent files', category: 'Navigate', scope: 'window', jetbrains: [m('e')], standard: [k('r', { ctrl: true })] },
  { id: 'findInPath', title: 'Find in path', category: 'Navigate', scope: 'window', ...both(m('f', { shift: true })) },
  { id: 'replaceInPath', title: 'Replace in path', category: 'Navigate', scope: 'window', jetbrains: [m('r', { shift: true })], standard: [m('h', { shift: true })] },
  { id: 'navigateBack', title: 'Navigate back', category: 'Navigate', scope: 'window', jetbrains: [m('[')], standard: [k('-', { ctrl: true })] },
  { id: 'navigateForward', title: 'Navigate forward', category: 'Navigate', scope: 'window', jetbrains: [m(']')], standard: [k('-', { ctrl: true, shift: true })] },
  { id: 'fileStructure', title: 'File structure', category: 'Navigate', scope: 'editor', ...both(m('F12')) },
  { id: 'goToDefinition', title: 'Go to definition', category: 'Code', scope: 'editor', jetbrains: [m('b')], standard: [k('F12')] },
  { id: 'findUsages', title: 'Find usages', category: 'Code', scope: 'window', jetbrains: [k('F7', { alt: true })], standard: [k('F12', { shift: true })] },
  { id: 'rename', title: 'Rename symbol', category: 'Code', scope: 'window', jetbrains: [k('F6', { shift: true })], standard: [k('F2')] },
  { id: 'quickFix', title: 'Quick fix / show intentions', category: 'Code', scope: 'editor', jetbrains: [k('Enter', { alt: true })], standard: [m('.')] },
  { id: 'reformat', title: 'Reformat code', category: 'Code', scope: 'window', jetbrains: [m('l', { alt: true })], standard: [k('f', { alt: true, shift: true })] },
  { id: 'optimizeImports', title: 'Optimize imports', category: 'Code', scope: 'window', jetbrains: [k('o', { ctrl: true, alt: true }), m('o', { alt: true })], standard: [k('o', { alt: true, shift: true })] },
  { id: 'toggleComment', title: 'Toggle line comment', category: 'Editor', scope: 'editor', ...both(m('/')) },
  { id: 'duplicateLine', title: 'Duplicate line or selection', category: 'Editor', scope: 'editor', jetbrains: [m('d')], standard: [m('d', { shift: true })] },
  { id: 'deleteLine', title: 'Delete line', category: 'Editor', scope: 'editor', jetbrains: [m('Backspace')], standard: [m('k', { shift: true })] },
  { id: 'splitSelection', title: 'Split selection into cursors', category: 'Editor', scope: 'editor', ...both(k('i', { alt: true, shift: true })) },
  { id: 'pasteHistory', title: 'Paste from clipboard history', category: 'Editor', scope: 'editor', ...both(m('v', { shift: true })) },
  { id: 'save', title: 'Save file', category: 'Tabs', scope: 'window', ...both(m('s')) },
  { id: 'saveAll', title: 'Save all files', category: 'Tabs', scope: 'window', ...both(m('s', { alt: true })) },
  { id: 'closeTab', title: 'Close editor tab', category: 'Tabs', scope: 'window', ...both(m('w')) },
  { id: 'reopenTab', title: 'Reopen closed tab', category: 'Tabs', scope: 'window', ...both(m('t', { shift: true })) },
  { id: 'toggleExplorer', title: 'Toggle project explorer', category: 'Windows', scope: 'window', jetbrains: [m('1')], standard: [m('b')] },
  { id: 'toggleDock', title: 'Toggle tool dock', category: 'Windows', scope: 'window', jetbrains: [], standard: [m('j')] },
  { id: 'runPanel', title: 'Run tool window', category: 'Windows', scope: 'window', ...both(m('4')) },
  { id: 'debugPanel', title: 'Debug tool window', category: 'Windows', scope: 'window', ...both(m('5')) },
  { id: 'problemsPanel', title: 'Problems tool window', category: 'Windows', scope: 'window', ...both(m('6')) },
  { id: 'gitPanel', title: 'Git tool window', category: 'Windows', scope: 'window', ...both(m('9')) },
  { id: 'terminal', title: 'Terminal tool window', category: 'Windows', scope: 'window', jetbrains: [k('F12', { alt: true })], standard: [k('`', { ctrl: true })] },
  { id: 'addSelectionToAgent', title: 'Add selection to agent', category: 'Agent', scope: 'window', ...both(m('l')) },
  { id: 'toggleAgent', title: 'Toggle AI agent', category: 'Agent', scope: 'window', ...both(m('l', { shift: true })) },
  { id: 'inlineEdit', title: 'Edit selection with AI', category: 'Agent', scope: 'window', ...both(m('i')) },
  { id: 'run', title: 'Run selected configuration', category: 'Run', scope: 'window', jetbrains: [k('r', { ctrl: true })], standard: [k('F5', { ctrl: true })] },
  { id: 'debug', title: 'Debug selected configuration', category: 'Run', scope: 'window', jetbrains: [k('d', { ctrl: true })], standard: [k('F5')] },
  { id: 'stop', title: 'Stop run or debug', category: 'Run', scope: 'window', jetbrains: [m('F2')], standard: [k('F5', { shift: true })] },
  { id: 'restartDebug', title: 'Restart debug session', category: 'Debugger', scope: 'window', jetbrains: [], standard: [m('F5', { shift: true })] },
  { id: 'toggleBreakpoint', title: 'Toggle line breakpoint', category: 'Debugger', scope: 'window', jetbrains: [m('F8')], standard: [k('F9')] },
  { id: 'resume', title: 'Resume program', category: 'Debugger', scope: 'window', jetbrains: [m('r', { alt: true }), k('F9')], standard: [k('F5')] },
  { id: 'stepOver', title: 'Step over', category: 'Debugger', scope: 'window', jetbrains: [k('F8'), k('ArrowDown', { ctrl: true, alt: true })], standard: [k('F10')] },
  { id: 'stepInto', title: 'Step into', category: 'Debugger', scope: 'window', jetbrains: [k('F7'), k('ArrowRight', { ctrl: true, alt: true })], standard: [k('F11')] },
  { id: 'stepOut', title: 'Step out', category: 'Debugger', scope: 'window', jetbrains: [k('F8', { shift: true }), k('ArrowUp', { ctrl: true, alt: true })], standard: [k('F11', { shift: true })] },
  { id: 'runToCursor', title: 'Run to cursor', category: 'Debugger', scope: 'window', jetbrains: [k('F9', { alt: true })], standard: [] },
] as const satisfies readonly IdeShortcutRow[];

export type IdeShortcutId = (typeof IDE_SHORTCUTS)[number]['id'];

const BY_ID = new Map<string, IdeShortcutRow>(IDE_SHORTCUTS.map((r) => [r.id, r]));

export function ideShortcutRow(id: IdeShortcutId): IdeShortcutRow {
  return BY_ID.get(id) as IdeShortcutRow;
}

export function ideShortcutChords(id: IdeShortcutId, preset: IdeKeymapPreset): readonly IdeChord[] {
  const row = BY_ID.get(id);
  if (!row) return [];
  return preset === 'standard' ? row.standard : row.jetbrains;
}

const KEY_GLYPH: Record<string, string> = {
  ArrowUp: '↑', ArrowDown: '↓', ArrowLeft: '←', ArrowRight: '→',
  Enter: '⏎', Backspace: '⌫', Escape: '⎋', Tab: '⇥',
};

/** macOS order: ⌃ ⌥ ⇧ ⌘, then the key. */
export function ideChordLabel(c: IdeChord): string {
  if (c.double) return 'Double ⇧';
  const key = KEY_GLYPH[c.key] ?? (c.key.length === 1 ? c.key.toUpperCase() : c.key);
  return `${c.ctrl ? '⌃' : ''}${c.alt ? '⌥' : ''}${c.shift ? '⇧' : ''}${c.mod || c.meta ? '⌘' : ''}${key}`;
}

/** The hint for a command in this preset: its first chord, or undefined when the
 *  preset has none (a hint is never shown for a key that does nothing). */
export function ideShortcutLabel(id: IdeShortcutId, preset: IdeKeymapPreset): string | undefined {
  const first = ideShortcutChords(id, preset)[0];
  return first ? ideChordLabel(first) : undefined;
}

/** Every chord of a command in this preset, for the Settings table ('—' when none). */
export function ideShortcutLabels(id: IdeShortcutId, preset: IdeKeymapPreset): string {
  const chords = ideShortcutChords(id, preset);
  return chords.length ? chords.map(ideChordLabel).join(' / ') : '—';
}

const CODE_FOR: Record<string, string> = {
  ',': 'Comma', '.': 'Period', '/': 'Slash', '`': 'Backquote', '-': 'Minus',
  '[': 'BracketLeft', ']': 'BracketRight',
};

function codeFor(key: string): string | undefined {
  if (/^[a-z]$/i.test(key)) return `Key${key.toUpperCase()}`;
  if (/^\d$/.test(key)) return `Digit${key}`;
  return CODE_FOR[key];
}

type KeyLike = Pick<KeyboardEvent, 'key' | 'code' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey'>;

export function ideChordMatches(e: KeyLike, c: IdeChord): boolean {
  if (c.double) return false;
  if (Boolean(c.alt) !== e.altKey || Boolean(c.shift) !== e.shiftKey) return false;
  if (c.mod) {
    if (e.metaKey === e.ctrlKey) return false;
  } else if (Boolean(c.meta) !== e.metaKey || Boolean(c.ctrl) !== e.ctrlKey) {
    return false;
  }
  const code = codeFor(c.key);
  return (code !== undefined && e.code === code) || (e.key ?? '').toLowerCase() === c.key.toLowerCase();
}

/** The rows of `scope` whose chord in `preset` matches the event, in table order. */
export function matchIdeShortcuts(
  e: KeyLike,
  preset: IdeKeymapPreset,
  scope: IdeShortcutScope = 'window',
): { id: IdeShortcutId; chord: IdeChord }[] {
  const out: { id: IdeShortcutId; chord: IdeChord }[] = [];
  for (const row of IDE_SHORTCUTS as readonly IdeShortcutRow[]) {
    if (row.scope !== scope) continue;
    for (const chord of preset === 'standard' ? row.standard : row.jetbrains) {
      if (ideChordMatches(e, chord)) {
        out.push({ id: row.id as IdeShortcutId, chord });
        break;
      }
    }
  }
  return out;
}

/** Normalised preset: anything but 'standard' is JetBrains (the default). */
export function ideKeymapPresetOf(raw: unknown): IdeKeymapPreset {
  return raw === 'standard' ? 'standard' : 'jetbrains';
}
