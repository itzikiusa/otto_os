// Pure CodeMirror 6 IDE keymap, cursor/selection commands, and LSP language-ID
// normalization. Kept free of `.svelte` / `.svelte.ts` imports so unit tests
// can exercise keybindings and LSP server-lang normalization directly in Node.

import {
  addCursorAbove, addCursorBelow, copyLineDown, deleteLine,
  moveLineDown, moveLineUp, selectParentSyntax, simplifySelection,
  toggleBlockComment, toggleComment,
} from '@codemirror/commands';
import { gotoLine as cmGotoLine, openSearchPanel, searchPanelOpen, selectNextOccurrence, selectSelectionMatches } from '@codemirror/search';
import {
  EditorSelection, EditorState, Prec,
  type ChangeSpec, type Extension, type SelectionRange,
} from '@codemirror/state';
import { EditorView, keymap, type KeyBinding } from '@codemirror/view';
import type { IdeDiagnostic, IdeGutterItem } from '../api/types.ts';
import type { IdeEditorSelectionRange } from './cm-ide-git-and-agent.ts';
import { applyConcreteEditsToView, computeDeterministicQuickFixes } from './cm-ide-quickfix.ts';
import { applyClientQuickFix, getIdeQuickFixesAtLine } from './cm-ide-semantic.ts';

export const toggleLineComment = toggleComment;

const IDENT_CHAR_RE = /[\w$]/;

const EXT_TO_LSP_LANG: Record<string, string> = {
  js: 'javascript', jsx: 'javascriptreact', ts: 'typescript', tsx: 'typescriptreact',
  mjs: 'javascript', cjs: 'javascript', mts: 'typescript', cts: 'typescript',
  py: 'python', pyi: 'python', go: 'go', rs: 'rust',
  json: 'json', jsonc: 'json', jsonl: 'json', ndjson: 'json',
  html: 'html', htm: 'html', css: 'css', scss: 'scss',
  less: 'less', md: 'markdown', mdx: 'markdown',
  java: 'java', class: 'java', kt: 'kotlin', kts: 'kotlin',
  sh: 'shellscript', bash: 'shellscript', zsh: 'shellscript', bats: 'shellscript',
  yaml: 'yaml', yml: 'yaml', graphql: 'graphql', gql: 'graphql', proto: 'protobuf',
};

/**
 * Normalize an LSP document `languageId` (e.g. `shellscript`, `typescriptreact`,
 * `javascriptreact`) to the canonical backend server registry `lang` key used
 * by `GET /lsp/capabilities` and `GET /ws/lsp?lang=`.
 */
export function normalizeIdeLspServerLang(lang: string): string {
  switch (lang.trim()) {
    case 'sh':
    case 'bash':
    case 'zsh':
    case 'bats':
    case 'shellscript':
      return 'shell';
    case 'typescriptreact':
    case 'tsx':
      return 'typescript';
    case 'javascriptreact':
    case 'jsx':
      return 'javascript';
    case 'scss':
    case 'less':
      return 'css';
    case 'jsonc':
    case 'jsonl':
    case 'ndjson':
      return 'json';
    default:
      return lang.trim();
  }
}

export function lspLanguageIdFor(filePath: string, ideMode: boolean, language?: string): string | null {
  if (filePath.startsWith('jdt://')) return 'java';
  const clean = filePath.split('?')[0];
  const base = clean.slice(clean.lastIndexOf('/') + 1);
  const dot = base.lastIndexOf('.');
  const ext = dot < 0 ? '' : base.slice(dot + 1).toLowerCase();
  if (ext === 'csv' || ext === 'tsv') return null;
  if (EXT_TO_LSP_LANG[ext]) return EXT_TO_LSP_LANG[ext];
  if (!ideMode || !language || language === 'csv' || language === 'tsv' || language === 'plaintext') {
    return null;
  }
  if (language === 'shell') return 'shellscript';
  if (language === 'jsonl') return 'json';
  return language;
}

export interface IdeEditorCallbacks {
  onToggleBreakpoint?: ((line: number) => void) | null;
  onEditBreakpoint?: ((line: number, event?: MouseEvent) => void) | null;
  onGutterAction?: ((item: IdeGutterItem, mode: 'run' | 'debug' | 'run_keep_tc') => void) | null;
  onDefinitionJump?: ((target: { path?: string; line: number; column: number; symbol?: string; originLine?: number; originColumn?: number }) => void) | null;
  onImplementationJump?: ((target: { path?: string; line: number; column: number; symbol?: string; originLine?: number; originColumn?: number }) => void) | null;
  onFindUsages?: ((ctx: { line: number; column: number; symbol?: string }) => void) | null;
  onFileStructure?: (() => void) | null;
  onJumpSymbol?: ((delta: -1 | 1) => void) | null;
  onNavigateBack?: (() => void) | null;
  onNavigateForward?: (() => void) | null;
  onQuickFix?: ((ctx: { line: number; column: number; symbol?: string; clientX?: number; clientY?: number }) => void) | null;
  onRenameSymbol?: ((ctx: { line: number; column: number; symbol: string; newName?: string }) => void) | null;
  onFormat?: (() => void) | null;
  onToggleBlame?: (() => void) | null;
  onDiagnostics?: ((diags: IdeDiagnostic[]) => void) | null;
  onAgentAction?: ((mode: 'chat' | 'inline', sel: IdeEditorSelectionRange) => void) | null;
  onPasteFromHistory?: (() => void) | null;
  onGoToLine?: (() => void) | null;
}

export type IdeDiveHandler = (
  view: EditorView,
  pos: number,
  mode: 'definition' | 'implementation',
  cbs: IdeEditorCallbacks,
) => unknown;

export function wordAtCursor(state: EditorState): { word: string; line: number; column: number } {
  const main = state.selection.main;
  const ln = state.doc.lineAt(main.head);
  const column = main.head - ln.from + 1;
  if (!main.empty) return { word: state.sliceDoc(main.from, main.to).trim(), line: ln.number, column };
  const text = ln.text;
  const rel = main.head - ln.from;
  let s = rel;
  let e = rel;
  while (s > 0 && IDENT_CHAR_RE.test(text[s - 1])) s--;
  while (e < text.length && IDENT_CHAR_RE.test(text[e])) e++;
  return { word: text.slice(s, e), line: ln.number, column };
}

export function selectAllOccurrences(view: EditorView): boolean {
  const { state } = view;
  if (state.selection.main.empty) {
    const main = state.selection.main;
    const ln = state.doc.lineAt(main.head);
    const rel = main.head - ln.from;
    let s = rel;
    let e = rel;
    while (s > 0 && IDENT_CHAR_RE.test(ln.text[s - 1])) s--;
    while (e < ln.text.length && IDENT_CHAR_RE.test(ln.text[e])) e++;
    if (e > s) view.dispatch({ selection: { anchor: ln.from + s, head: ln.from + e } });
  }
  return selectSelectionMatches(view);
}

export function splitSelectionIntoLines(view: EditorView): boolean {
  const { state } = view;
  const next: SelectionRange[] = [];
  let changed = false;
  for (const r of state.selection.ranges) {
    const startLn = state.doc.lineAt(r.from).number;
    let endLn = state.doc.lineAt(r.to).number;
    if (!r.empty && r.to > r.from && state.doc.lineAt(r.to).from === r.to) {
      endLn = state.doc.lineAt(r.to - 1).number;
    }
    if (endLn <= startLn) {
      next.push(r);
      continue;
    }
    changed = true;
    for (let ln = startLn; ln <= endLn; ln++) {
      const line = state.doc.line(ln);
      next.push(EditorSelection.range(Math.max(line.from, r.from), Math.min(line.to, r.to)));
    }
  }
  if (!changed || !next.length) return false;
  view.dispatch({ selection: EditorSelection.create(next) });
  return true;
}

export function applyLinePrefixSuffix(view: EditorView, prefix: string, suffix: string): boolean {
  if ((!prefix && !suffix) || view.state.readOnly) return false;
  const { doc, selection } = view.state;
  const touched = new Set<number>();
  for (const r of selection.ranges) {
    const startLn = doc.lineAt(r.from).number;
    const endLn = !r.empty && r.to > r.from && doc.lineAt(r.to).from === r.to
      ? doc.lineAt(r.to - 1).number
      : doc.lineAt(r.to).number;
    for (let ln = startLn; ln <= endLn; ln++) touched.add(ln);
  }
  const lines = [...touched].sort((a, b) => a - b);
  if (!lines.length) return false;
  const changes: ChangeSpec[] = [];
  const cursors: SelectionRange[] = [];
  const delta = prefix.length + suffix.length;
  lines.forEach((ln, idx) => {
    const line = doc.line(ln);
    if (prefix.length) changes.push({ from: line.from, to: line.from, insert: prefix });
    if (suffix.length) changes.push({ from: line.to, to: line.to, insert: suffix });
    cursors.push(EditorSelection.cursor(line.to + (idx + 1) * delta));
  });
  view.dispatch({
    changes,
    selection: EditorSelection.create(cursors, cursors.length - 1),
    userEvent: 'input',
    scrollIntoView: true,
  });
  return true;
}

export function duplicateLineOrSelection(view: EditorView): boolean {
  if (view.state.readOnly) return false;
  if (!view.state.selection.ranges.some((r) => !r.empty)) return copyLineDown(view);
  const tr = view.state.changeByRange((range) => {
    if (range.empty) {
      const ln = view.state.doc.lineAt(range.head);
      const ins = `\n${ln.text}`;
      return {
        changes: { from: ln.to, to: ln.to, insert: ins },
        range: EditorSelection.cursor(range.head + ins.length),
      };
    }
    const sel = view.state.sliceDoc(range.from, range.to);
    return {
      changes: { from: range.to, to: range.to, insert: sel },
      range: EditorSelection.range(range.to, range.to + sel.length),
    };
  });
  view.dispatch(view.state.update(tr, { userEvent: 'input', scrollIntoView: true }));
  return true;
}

export function startNewLineBelow(view: EditorView): boolean {
  if (view.state.readOnly) return false;
  const ln = view.state.doc.lineAt(view.state.selection.main.head);
  const indent = /^(\s*)/.exec(ln.text)?.[1] ?? '';
  const ins = `\n${indent}`;
  view.dispatch({
    changes: { from: ln.to, to: ln.to, insert: ins },
    selection: EditorSelection.cursor(ln.to + ins.length),
    userEvent: 'input',
    scrollIntoView: true,
  });
  return true;
}

export function smartExpandSelection(view: EditorView): boolean {
  const main = view.state.selection.main;
  const ln = view.state.doc.lineAt(main.head);
  if (main.empty) {
    const rel = main.head - ln.from;
    let s = rel;
    let e = rel;
    while (s > 0 && IDENT_CHAR_RE.test(ln.text[s - 1])) s--;
    while (e < ln.text.length && IDENT_CHAR_RE.test(ln.text[e])) e++;
    if (e > s) {
      view.dispatch({ selection: { anchor: ln.from + s, head: ln.from + e } });
      return true;
    }
  }
  if (selectParentSyntax(view)) return true;
  if (main.from > ln.from || main.to < ln.to) {
    view.dispatch({ selection: { anchor: ln.from, head: ln.to } });
    return true;
  }
  return false;
}

export function openReplaceInFile(view: EditorView): boolean {
  openSearchPanel(view);
  requestAnimationFrame(() => {
    const rep = view.dom.querySelector<HTMLInputElement>('.cm-search input[name="replace"]');
    rep?.focus();
    rep?.select();
  });
  return true;
}

export function buildIdeKeymapSpecs(
  preset: 'jetbrains' | 'standard',
  getCallbacks: () => IdeEditorCallbacks,
  filePath = '',
  language?: string,
  diveAtPos?: IdeDiveHandler,
): KeyBinding[] {
  const applyFirstFix = (v: EditorView): boolean => {
    const i = wordAtCursor(v.state);
    const lFixes = getIdeQuickFixesAtLine(v.state, i.line);
    if (lFixes.length) {
      applyClientQuickFix(v, lFixes[0]);
      return true;
    }
    const dFixes = computeDeterministicQuickFixes(
      v.state.doc.toString(),
      filePath,
      language,
      i.line,
      i.word || undefined,
    );
    // Only a preferred fix applies blind; "Prefix with _" & co. stay in the ⌥⏎ menu.
    const preferred = dFixes.find((f) => f.isPreferred);
    if (preferred) return applyConcreteEditsToView(v, preferred.edits, filePath);
    const cb = getCallbacks().onQuickFix;
    if (cb) {
      cb({ line: i.line, column: i.column, symbol: i.word || undefined });
      return true;
    }
    return false;
  };

  const runDive = (v: EditorView, mode: 'definition' | 'implementation'): boolean => {
    const cbs = getCallbacks();
    if (mode === 'definition' && !cbs.onDefinitionJump) return false;
    if (mode === 'implementation' && !cbs.onImplementationJump && !cbs.onDefinitionJump) return false;
    if (diveAtPos) {
      void diveAtPos(v, v.state.selection.main.head, mode, cbs);
      return true;
    }
    const i = wordAtCursor(v.state);
    const target = { line: i.line, column: i.column, symbol: i.word || undefined };
    if (mode === 'implementation' && cbs.onImplementationJump) cbs.onImplementationJump(target);
    else cbs.onDefinitionJump?.(target);
    return true;
  };

  const findUsages = (v: EditorView): boolean => {
    const cb = getCallbacks().onFindUsages;
    if (!cb) return false;
    const i = wordAtCursor(v.state);
    cb({ line: i.line, column: i.column, symbol: i.word || undefined });
    return true;
  };
  const renameSymbol = (v: EditorView): boolean => {
    const cb = getCallbacks().onRenameSymbol;
    if (!cb) return false;
    const i = wordAtCursor(v.state);
    cb({ line: i.line, column: i.column, symbol: i.word });
    return true;
  };
  const reformat = (): boolean => {
    const cb = getCallbacks().onFormat;
    if (!cb) return false;
    cb();
    return true;
  };

  const runPasteFromHistory = (): boolean => {
    const cb = getCallbacks().onPasteFromHistory;
    if (cb) { cb(); return true; }
    if (typeof window !== 'undefined') {
      window.dispatchEvent(new CustomEvent('otto:ide-paste-history'));
      return true;
    }
    return false;
  };

  const runQuickFix = (v: EditorView): boolean => {
    const cb = getCallbacks().onQuickFix;
    if (!cb) return applyFirstFix(v);
    const i = wordAtCursor(v.state);
    const c = v.coordsAtPos(v.state.selection.main.head);
    cb({ line: i.line, column: i.column, symbol: i.word || undefined, clientX: c?.left, clientY: c?.bottom });
    return true;
  };

  return [
    { key: 'Mod-Alt-ArrowUp', run: addCursorAbove },
    { key: 'Mod-Alt-ArrowDown', run: addCursorBelow },
    { key: 'Alt-Shift-ArrowUp', run: moveLineUp },
    { key: 'Alt-Shift-ArrowDown', run: moveLineDown },
    {
      key: 'Ctrl-ArrowUp',
      run: (v) => {
        const cb = getCallbacks().onJumpSymbol;
        if (cb) { cb(-1); return true; }
        return moveLineUp(v);
      },
    },
    {
      key: 'Ctrl-ArrowDown',
      run: (v) => {
        const cb = getCallbacks().onJumpSymbol;
        if (cb) { cb(1); return true; }
        return moveLineDown(v);
      },
    },
    { key: 'Alt-ArrowUp', run: smartExpandSelection },
    { key: 'Alt-ArrowDown', run: simplifySelection },
    { key: 'Shift-Enter', run: startNewLineBelow },
    { key: 'Ctrl-Mod-g', run: selectAllOccurrences },
    { key: 'Alt-Shift-i', run: splitSelectionIntoLines },
    { key: 'Alt-Shift-I', run: splitSelectionIntoLines },
    { key: 'Mod-/', run: toggleLineComment },
    { key: 'Mod-Alt-/', run: toggleBlockComment },
    { key: 'Mod-Shift-/', run: toggleBlockComment },
    { key: 'Mod-r', run: openReplaceInFile },
    { key: 'Mod-Shift-v', run: runPasteFromHistory },
    { key: 'Mod-Shift-V', run: runPasteFromHistory },
    { key: 'Cmd-Shift-v', run: runPasteFromHistory },
    { key: 'Cmd-Shift-V', run: runPasteFromHistory },
    { key: 'F12', run: (v) => runDive(v, 'definition') },
    { key: 'Mod-Alt-b', run: (v) => runDive(v, 'implementation') },
    { key: 'Alt-F7', run: findUsages },
    {
      key: 'Mod-F12',
      run: () => {
        const cb = getCallbacks().onFileStructure;
        if (!cb) return false;
        cb();
        return true;
      },
    },
    {
      key: 'Mod-Alt-ArrowLeft',
      run: () => {
        const cb = getCallbacks().onNavigateBack;
        if (!cb) return false;
        cb();
        return true;
      },
    },
    {
      key: 'Mod-Alt-ArrowRight',
      run: () => {
        const cb = getCallbacks().onNavigateForward;
        if (!cb) return false;
        cb();
        return true;
      },
    },
    { key: 'Alt-Enter', run: runQuickFix },
    { key: 'Alt-Shift-Enter', run: applyFirstFix },
    { key: 'Shift-F6', run: renameSymbol },
    { key: 'Mod-Alt-l', run: reformat },
    {
      key: 'Mod-F8',
      run: (v) => {
        const cb = getCallbacks().onToggleBreakpoint;
        if (!cb) return false;
        cb(v.state.doc.lineAt(v.state.selection.main.head).number);
        return true;
      },
    },
    // The keys each preset means differently. Shell/window keys of the same presets
    // live in `modules/ide/shell/ideShortcuts.ts` (Standard's ⌘B / ⌃G / ⌘G belong to
    // the window there: explorer, Go to Line, find next).
    ...(preset === 'jetbrains'
      ? [
          { key: 'Ctrl-g', run: selectNextOccurrence },
          {
            key: 'Mod-g',
            run: (v: EditorView) => {
              // Find panel open: leave ⌘G to the search keymap's "find next".
              if (searchPanelOpen(v.state)) return false;
              const cb = getCallbacks().onGoToLine;
              if (cb) { cb(); return true; }
              return cmGotoLine(v);
            },
          },
          { key: 'Mod-b', run: (v: EditorView) => runDive(v, 'definition') },
          {
            key: 'Mod-[',
            run: () => {
              const cb = getCallbacks().onNavigateBack;
              if (!cb) return false;
              cb();
              return true;
            },
          },
          {
            key: 'Mod-]',
            run: () => {
              const cb = getCallbacks().onNavigateForward;
              if (!cb) return false;
              cb();
              return true;
            },
          },
          { key: 'Mod-d', run: duplicateLineOrSelection },
          { key: 'Mod-Backspace', run: deleteLine },
        ]
      : [
          // The Standard preset's editor rows of shell/ideShortcuts.ts (ideActionsShortcuts.test.ts holds them in step).
          { key: 'Mod-d', run: selectNextOccurrence },
          { key: 'Shift-Mod-d', run: duplicateLineOrSelection },
          { key: 'Shift-Mod-k', run: deleteLine },
          { key: 'Mod-.', run: runQuickFix },
        ]),
  ];
}

export function buildIdeKeymap(
  preset: 'jetbrains' | 'standard',
  getCallbacks: () => IdeEditorCallbacks,
  filePath = '',
  language?: string,
  diveAtPos?: IdeDiveHandler,
): Extension {
  return Prec.high(keymap.of(buildIdeKeymapSpecs(preset, getCallbacks, filePath, language, diveAtPos)));
}
