// IntelliJ / GoLand Refactoring, Inline CodeVision (`R✎ Rename usages` /
// `R✎ Implement interface`), Column Selection Mode (`⇧⌘8`), String Manipulation,
// Generate (`⌘N`), and Cascading Editor Context Menu (`Refactor ▸`, `Folding ▸`, …).

import {
  Decoration, EditorView, ViewPlugin, WidgetType, keymap, rectangularSelection,
  type DecorationSet, type ViewUpdate,
} from '@codemirror/view';
import {
  EditorSelection, EditorState, Prec, StateEffect, StateField,
  type Extension, type Range,
} from '@codemirror/state';
import { foldAll, foldCode, unfoldAll, unfoldCode } from '@codemirror/language';
import type { IdeGutterItem } from '../api/types.ts';
import { copyWithToast } from '../clipboard.ts';
import { ctxMenu, type MenuItem } from '../contextmenu.svelte.ts';
import {
  selectionOrCurrentLine, setSavedContentEffect,
} from './cm-ide-git-and-agent.ts';
import { buildIdeVcsContextRows } from './cm-ide-history.ts';
import { wordAtCursor, type IdeEditorCallbacks } from './cm-ide-keymap.ts';
import {
  computeExtractInterfaceEdit, computeExtractMethodEdit, computeGenerateCodeSnippet,
  computeInlineVariableEdit, computeIntroduceTypeEdit, computeIntroduceVariableEdit,
  detectInlineRefactorHints, MAX_CODEVISION_LINES, transformStringManipulation,
  type IdeInlineRefactorHint, type StringManipKind,
} from './cm-ide-refactor-core.ts';

export {
  computeExtractInterfaceEdit, computeExtractMethodEdit, computeGenerateCodeSnippet,
  computeInlineVariableEdit, computeIntroduceTypeEdit, computeIntroduceVariableEdit,
  detectInlineRefactorHints, transformStringManipulation,
  type IdeInlineRefactorHint, type StringManipKind,
};

export const setDiskSavedContentEffect = StateEffect.define<string | null>();
export const toggleColumnSelectionEffect = StateEffect.define<boolean | undefined>();

class CodeVisionRefactorWidget extends WidgetType {
  readonly lineHints: IdeInlineRefactorHint[];
  readonly indentCols: number;
  readonly getCallbacks: () => IdeEditorCallbacks;

  constructor(lineHints: IdeInlineRefactorHint[], indentCols: number, getCallbacks: () => IdeEditorCallbacks) {
    super();
    this.lineHints = lineHints;
    this.indentCols = indentCols;
    this.getCallbacks = getCallbacks;
  }

  eq(other: CodeVisionRefactorWidget): boolean {
    if (this.indentCols !== other.indentCols || this.lineHints.length !== other.lineHints.length) return false;
    return this.lineHints.every((h, idx) => {
      const o = other.lineHints[idx];
      if (h.kind !== o.kind || h.line !== o.line || h.column !== o.column) return false;
      return h.kind === 'rename-usages' && o.kind === 'rename-usages'
        ? h.oldName === o.oldName && h.newName === o.newName
        : h.kind === 'implement-interface' && o.kind === 'implement-interface' && h.symbol === o.symbol;
    });
  }

  toDOM(): HTMLElement {
    const row = document.createElement('div');
    row.className = 'cm-ide-codevision-row';
    row.style.paddingInlineStart = `${Math.min(48, Math.max(0, this.indentCols))}ch`;
    for (const h of this.lineHints) {
      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = `cm-ide-codevision-pill is-${h.kind}`;
      btn.dataset.testid = `ide-codevision-${h.kind}`;
      const badge = document.createElement('span');
      badge.className = 'cm-ide-cv-badge';
      badge.setAttribute('aria-hidden', 'true');
      badge.textContent = 'R✎';
      btn.appendChild(badge);
      const text = document.createElement('span');
      if (h.kind === 'rename-usages') {
        text.textContent = 'Rename usages';
        btn.title = `Propagate rename '${h.oldName}' → '${h.newName}' across workspace (⇧F6)`;
      } else {
        text.textContent = 'Implement interface';
        btn.title = `Show actions & interface stubs for '${h.symbol}' (⌥⏎)`;
      }
      btn.appendChild(text);
      btn.addEventListener('mousedown', (e) => { e.preventDefault(); e.stopPropagation(); });
      btn.addEventListener('click', (e) => {
        e.preventDefault();
        e.stopPropagation();
        const cbs = this.getCallbacks();
        if (h.kind === 'rename-usages') {
          cbs.onRenameSymbol?.({ line: h.line, column: h.column, symbol: h.oldName, newName: h.newName });
        } else {
          const r = btn.getBoundingClientRect();
          cbs.onQuickFix?.({ line: h.line, column: h.column, symbol: h.symbol, clientX: r.left, clientY: r.bottom + 4 });
        }
      });
      row.appendChild(btn);
    }
    return row;
  }
}

function buildCodeVisionDecorations(
  doc: EditorState['doc'],
  diskBase: string | null,
  gitBase: string | null,
  language: string | undefined,
  getCallbacks: () => IdeEditorCallbacks,
): DecorationSet {
  if (doc.lines > MAX_CODEVISION_LINES) return Decoration.none;
  const docText = doc.toString();
  const baseline = diskBase != null && diskBase !== docText ? diskBase : gitBase;
  const hints = detectInlineRefactorHints(docText, baseline, language);
  if (!hints.length) return Decoration.none;
  const byLine = new Map<number, IdeInlineRefactorHint[]>();
  for (const h of hints) {
    const arr = byLine.get(h.line);
    if (arr) arr.push(h);
    else byLine.set(h.line, [h]);
  }
  const ranges: Range<Decoration>[] = [];
  for (const [ln, list] of [...byLine.entries()].sort((a, b) => a[0] - b[0])) {
    if (ln < 1 || ln > doc.lines) continue;
    const lineObj = doc.line(ln);
    let indentCols = 0;
    for (let i = 0; i < lineObj.text.length; i++) {
      const ch = lineObj.text[i];
      if (ch === ' ') indentCols += 1;
      else if (ch === '\t') indentCols += 4;
      else break;
    }
    ranges.push(
      Decoration.widget({
        widget: new CodeVisionRefactorWidget(list, indentCols, getCallbacks),
        block: true,
        side: -1,
      }).range(lineObj.from),
    );
  }
  return ranges.length ? Decoration.set(ranges, true) : Decoration.none;
}

const setCodeVisionDecosEffect = StateEffect.define<DecorationSet>();

export function createIdeRefactorCodeVisionExtension(opts: {
  diskSavedContent?: string | null;
  savedContent?: string | null;
  language?: string;
  getCallbacks: () => IdeEditorCallbacks;
}): Extension {
  const field = StateField.define<{ diskBase: string | null; gitBase: string | null; decos: DecorationSet }>({
    create(st) {
      const diskBase = opts.diskSavedContent ?? null;
      const gitBase = opts.savedContent ?? null;
      const decos = st.readOnly
        ? Decoration.none
        : buildCodeVisionDecorations(st.doc, diskBase, gitBase, opts.language, opts.getCallbacks);
      return { diskBase, gitBase, decos };
    },
    update(cur, tr) {
      let { diskBase, gitBase, decos } = cur;
      let baseChanged = false;
      for (const ef of tr.effects) {
        if (ef.is(setDiskSavedContentEffect) && ef.value !== diskBase) {
          diskBase = ef.value;
          baseChanged = true;
        } else if (ef.is(setSavedContentEffect) && ef.value !== gitBase) {
          gitBase = ef.value;
          baseChanged = true;
        } else if (ef.is(setCodeVisionDecosEffect)) {
          decos = ef.value;
        }
      }
      if (tr.state.readOnly) return decos === Decoration.none && !baseChanged ? cur : { diskBase, gitBase, decos: Decoration.none };
      if (baseChanged || tr.state.readOnly !== tr.startState.readOnly) {
        return {
          diskBase,
          gitBase,
          decos: buildCodeVisionDecorations(tr.state.doc, diskBase, gitBase, opts.language, opts.getCallbacks),
        };
      }
      if (tr.docChanged && decos !== Decoration.none) {
        decos = decos.map(tr.changes);
      }
      if (diskBase === cur.diskBase && gitBase === cur.gitBase && decos === cur.decos) return cur;
      return { diskBase, gitBase, decos };
    },
    provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
  });

  const debouncer = ViewPlugin.fromClass(class {
    private timer: ReturnType<typeof setTimeout> | null = null;
    update(u: ViewUpdate): void {
      if (!u.docChanged || u.state.readOnly) return;
      if (this.timer !== null) clearTimeout(this.timer);
      const target = u.view;
      this.timer = setTimeout(() => {
        this.timer = null;
        const st = target.state.field(field, false);
        if (!st || target.state.readOnly) return;
        const next = buildCodeVisionDecorations(target.state.doc, st.diskBase, st.gitBase, opts.language, opts.getCallbacks);
        target.dispatch({ effects: setCodeVisionDecosEffect.of(next) });
      }, 180);
    }
    destroy(): void {
      if (this.timer !== null) clearTimeout(this.timer);
      this.timer = null;
    }
  });

  return [field, debouncer];
}

function applyStringManipulationInView(view: EditorView, kind: StringManipKind): boolean {
  if (view.state.readOnly) return false;
  const main = view.state.selection.main;
  let from = main.from;
  let to = main.to;
  if (from === to) {
    if (kind === 'sort_lines' || kind === 'unique_lines') {
      from = 0;
      to = view.state.doc.length;
    } else {
      const w = wordAtCursor(view.state);
      if (!w.word) return false;
      const ln = view.state.doc.line(w.line);
      from = ln.from + w.column - 1;
      to = from + w.word.length;
    }
  }
  const orig = view.state.sliceDoc(from, to);
  const next = transformStringManipulation(orig, kind);
  if (next === orig) return true;
  view.dispatch({
    changes: { from, to, insert: next },
    selection: EditorSelection.range(from, from + next.length),
    userEvent: 'input',
  });
  view.focus();
  return true;
}

export const columnSelectionModeField = StateField.define<boolean>({
  create: () => false,
  update(val, tr) {
    for (const ef of tr.effects) {
      if (ef.is(toggleColumnSelectionEffect)) return ef.value ?? !val;
    }
    return val;
  },
  provide: (f) =>
    EditorView.editorAttributes.from(f, (on): Record<string, string> =>
      on ? { class: 'cm-ide-col-select-mode' } : {},
    ),
});

export function isColumnSelectionMode(state: EditorState): boolean {
  return state.field(columnSelectionModeField, false) ?? false;
}

export function toggleColumnSelectionMode(view: EditorView): boolean {
  const next = !isColumnSelectionMode(view.state);
  view.dispatch({ effects: toggleColumnSelectionEffect.of(next) });
  return true;
}

function runIntroduceVariableInView(view: EditorView, language?: string, asConstant = false): boolean {
  if (view.state.readOnly) return false;
  const main = view.state.selection.main;
  const res = computeIntroduceVariableEdit(view.state.doc.toString(), main.from, main.to, language, asConstant);
  if (!res) return false;
  view.dispatch({
    changes: res.changes,
    selection: EditorSelection.range(res.selectFrom, res.selectTo),
    scrollIntoView: true,
    userEvent: 'input',
  });
  view.focus();
  return true;
}

function runInlineVariableInView(view: EditorView): boolean {
  if (view.state.readOnly) return false;
  const res = computeInlineVariableEdit(view.state.doc.toString(), view.state.selection.main.head);
  if (!res) return false;
  view.dispatch({ changes: res.changes, userEvent: 'input' });
  view.focus();
  return true;
}

function runExtractMethodInView(view: EditorView, language?: string): boolean {
  if (view.state.readOnly) return false;
  const main = view.state.selection.main;
  const res = computeExtractMethodEdit(view.state.doc.toString(), main.from, main.to, language);
  if (!res) return false;
  view.dispatch({ changes: res.changes, scrollIntoView: true, userEvent: 'input' });
  view.focus();
  return true;
}

function runExtractInterfaceInView(view: EditorView, language?: string): boolean {
  if (view.state.readOnly) return false;
  const res = computeExtractInterfaceEdit(view.state.doc.toString(), view.state.selection.main.head, language);
  if (!res) return false;
  view.dispatch({ changes: { from: res.insertPos, to: res.insertPos, insert: res.insertText }, scrollIntoView: true, userEvent: 'input' });
  view.focus();
  return true;
}

function runIntroduceTypeInView(view: EditorView, language?: string): boolean {
  if (view.state.readOnly) return false;
  const main = view.state.selection.main;
  const res = computeIntroduceTypeEdit(view.state.doc.toString(), main.from, main.to, language);
  if (!res) return false;
  view.dispatch({ changes: res.changes, scrollIntoView: true, userEvent: 'input' });
  view.focus();
  return true;
}

function runGenerateInView(view: EditorView, kind: 'constructor' | 'getters' | 'test_stub', language?: string): boolean {
  if (view.state.readOnly) return false;
  const w = wordAtCursor(view.state).word || 'Feature';
  const res = computeGenerateCodeSnippet(view.state.doc.toString(), view.state.selection.main.head, kind, w, language);
  if (!res) return false;
  view.dispatch({ changes: { from: res.insertPos, to: res.insertPos, insert: res.insertText }, scrollIntoView: true, userEvent: 'input' });
  view.focus();
  return true;
}

function dispatchFileRefactorEvent(op: 'move' | 'copy' | 'safe-delete', filePath: string): void {
  if (typeof window === 'undefined' || !filePath) return;
  window.dispatchEvent(new CustomEvent('otto:ide-editor-file-refactor', { detail: { op, path: filePath } }));
}

function dispatchRepoTestAction(filePath: string, mode: 'run' | 'coverage' | 'debug'): void {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(new CustomEvent('otto:ide-repo-test-action', { detail: { path: filePath, mode } }));
}

function nearestGutterRunItem(runMap: Map<number, IdeGutterItem> | undefined, cursorLine: number): IdeGutterItem | null {
  if (!runMap || !runMap.size) return null;
  const exact = runMap.get(cursorLine);
  if (exact) return exact;
  let best: IdeGutterItem | null = null;
  for (const [ln, item] of runMap) {
    if (ln <= cursorLine && cursorLine - ln <= 60 && (!best || ln > best.line)) best = item;
  }
  return best ?? [...runMap.values()][0] ?? null;
}

export function createIdeRefactorExtensions(opts: {
  filePath: string;
  language?: string;
  diskSavedContent?: string | null;
  savedContent?: string | null;
  isManifest: boolean;
  getCallbacks: () => IdeEditorCallbacks;
  getRunMap?: (view: EditorView) => Map<number, IdeGutterItem> | undefined;
}): Extension {
  const fPath = opts.filePath;
  const colModeRectExt = rectangularSelection({
    eventFilter: (e) => e.button === 0 && Boolean((e.target as HTMLElement | null)?.closest('.cm-ide-col-select-mode')),
  });

  const refactorKeymap = Prec.high(keymap.of([
    { key: 'Shift-Mod-8', run: (v) => toggleColumnSelectionMode(v) },
    { key: 'Alt-Mod-v', run: (v) => runIntroduceVariableInView(v, opts.language, false) },
    { key: 'Alt-Mod-c', run: (v) => runIntroduceVariableInView(v, opts.language, true) },
    { key: 'Alt-Mod-m', run: (v) => runExtractMethodInView(v, opts.language) },
    { key: 'Alt-Mod-n', run: (v) => runInlineVariableInView(v) },
    { key: 'F6', run: () => { if (!fPath) return false; dispatchFileRefactorEvent('move', fPath); return true; } },
    { key: 'F5', run: () => { if (!fPath) return false; dispatchFileRefactorEvent('copy', fPath); return true; } },
  ]));

  const contextMenuExt = opts.isManifest ? [] : Prec.highest(EditorView.domEventHandlers({
    contextmenu(event, view) {
      if (event.target instanceof Element && event.target.closest('.cm-gutters')) return false;
      const cbs = opts.getCallbacks();
      const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
      const main = view.state.selection.main;
      if (pos != null && (main.empty || pos < main.from || pos > main.to)) {
        view.dispatch({ selection: { anchor: pos, head: pos } });
      }
      const sym = wordAtCursor(view.state);
      const ro = view.state.readOnly;
      const hasSel = !view.state.selection.main.empty;
      const baseName = fPath.split('/').pop() || fPath;

      const showMain = () => {
        const items: MenuItem[] = [];
        if (cbs.onQuickFix) {
          items.push({
            label: 'Show Context Actions',
            icon: 'bulb',
            hint: '⌥⏎',
            action: () => cbs.onQuickFix?.({ line: sym.line, column: sym.column, symbol: sym.word || undefined, clientX: event.clientX, clientY: event.clientY }),
          });
          items.push({ separator: true });
        }

        items.push(
          {
            label: 'Cut',
            icon: 'edit',
            hint: '⌘X',
            disabled: ro,
            action: () => {
              const r = selectionOrCurrentLine(view.state);
              void copyWithToast(r.text, 'Cut to clipboard');
              view.dispatch({ changes: { from: r.from, to: r.to, insert: '' }, userEvent: 'delete.cut' });
            },
          },
          {
            label: 'Copy',
            icon: 'copy',
            hint: '⌘C',
            action: () => {
              const r = selectionOrCurrentLine(view.state);
              void copyWithToast(r.text, 'Copied');
            },
          },
          {
            label: 'Paste',
            icon: 'file',
            hint: '⌘V',
            disabled: ro,
            action: () => {
              void navigator.clipboard?.readText().then((clip) => {
                if (clip != null) view.dispatch(view.state.replaceSelection(clip), { scrollIntoView: true, userEvent: 'input.paste' });
              }).catch(() => {});
            },
          },
          {
            label: 'Copy / Paste Special',
            icon: 'copy',
            children: [
              {
                label: 'Paste from History…',
                icon: 'clock',
                hint: '⇧⌘V',
                disabled: ro,
                action: () => {
                  if (cbs.onPasteFromHistory) cbs.onPasteFromHistory();
                  else if (typeof window !== 'undefined') window.dispatchEvent(new CustomEvent('otto:ide-paste-history'));
                },
              },
              { separator: true },
              {
                label: 'Copy Reference',
                icon: 'link',
                hint: '⌥⇧⌘C',
                action: () => void copyWithToast(`${baseName}:${sym.line}${sym.word ? `#${sym.word}` : ''}`, 'Copied reference'),
              },
              {
                label: 'Copy Path',
                icon: 'folder',
                hint: '⇧⌘C',
                disabled: !fPath,
                action: () => void copyWithToast(fPath, 'Copied file path'),
              },
              {
                label: 'Copy File Name',
                icon: 'file',
                disabled: !baseName,
                action: () => void copyWithToast(baseName, 'Copied file name'),
              },
            ],
          },
        );

        if (cbs.onAgentAction) {
          items.push(
            { label: 'Add Selection to Agent', icon: 'assistant', hint: '⌘L', action: () => cbs.onAgentAction?.('chat', selectionOrCurrentLine(view.state)) },
            { label: 'Inline Edit with Agent', icon: 'sparkle', hint: '⌘I', disabled: ro, action: () => cbs.onAgentAction?.('inline', selectionOrCurrentLine(view.state)) },
          );
        }
        items.push({
          label: 'Column Selection Mode',
          checked: isColumnSelectionMode(view.state),
          hint: '⇧⌘8',
          action: () => toggleColumnSelectionMode(view),
        });

        items.push(
          { separator: true },
          {
            label: 'String Manipulation',
            icon: 'text',
            disabled: ro,
            children: [
              { label: 'TO UPPER CASE', action: () => applyStringManipulationInView(view, 'upper') },
              { label: 'to lower case', action: () => applyStringManipulationInView(view, 'lower') },
              { label: 'camelCase', action: () => applyStringManipulationInView(view, 'camel') },
              { label: 'PascalCase', action: () => applyStringManipulationInView(view, 'pascal') },
              { label: 'snake_case', action: () => applyStringManipulationInView(view, 'snake') },
              { label: 'kebab-case', action: () => applyStringManipulationInView(view, 'kebab') },
              { label: 'SCREAMING_SNAKE_CASE', action: () => applyStringManipulationInView(view, 'screaming_snake') },
              { separator: true },
              { label: 'Sort Lines', action: () => applyStringManipulationInView(view, 'sort_lines') },
              { label: 'Unique Lines', action: () => applyStringManipulationInView(view, 'unique_lines') },
            ],
          },
          { separator: true },
          {
            label: 'Find in Files…',
            icon: 'search',
            hint: '⇧⌘F',
            action: () => {
              if (typeof window !== 'undefined') window.dispatchEvent(new CustomEvent('otto:ide-open-find-in-files', { detail: { path: fPath } }));
            },
          },
        );
        if (cbs.onFindUsages) {
          items.push({ label: 'Find Usages', icon: 'search', hint: '⌥F7', action: () => cbs.onFindUsages?.({ line: sym.line, column: sym.column, symbol: sym.word || undefined }) });
        }

        const goToChildren: MenuItem[] = [];
        if (cbs.onDefinitionJump) goToChildren.push({ label: 'Declaration or Usages', icon: 'arrow', hint: '⌘B', action: () => cbs.onDefinitionJump?.({ line: sym.line, column: sym.column, symbol: sym.word || undefined }) });
        if (cbs.onImplementationJump) goToChildren.push({ label: 'Implementation(s)', icon: 'layers', hint: '⌥⌘B', action: () => cbs.onImplementationJump?.({ line: sym.line, column: sym.column, symbol: sym.word || undefined }) });
        if (cbs.onFileStructure) goToChildren.push({ label: 'File Structure…', icon: 'book', hint: '⌘F12', action: () => cbs.onFileStructure?.() });
        if (cbs.onGoToLine) goToChildren.push({ label: 'Line:Column…', icon: 'step', action: () => cbs.onGoToLine?.() });
        if (goToChildren.length) items.push({ label: 'Go To', icon: 'arrow', children: goToChildren });

        items.push({
          label: 'Folding',
          icon: 'fold',
          children: [
            { label: 'Collapse', icon: 'fold', hint: '⌘-', action: () => { foldCode(view); } },
            { label: 'Expand', icon: 'unfold', hint: '⌘+', action: () => { unfoldCode(view); } },
            { separator: true },
            { label: 'Collapse All', icon: 'fold', hint: '⇧⌘-', action: () => { foldAll(view); } },
            { label: 'Expand All', icon: 'unfold', hint: '⇧⌘+', action: () => { unfoldAll(view); } },
          ],
        });

        items.push({ separator: true });
        if (cbs.onRenameSymbol) {
          items.push({
            label: 'Rename…',
            icon: 'edit',
            hint: '⇧F6',
            disabled: ro || !sym.word,
            title: !sym.word ? 'Place cursor on an identifier to rename' : undefined,
            action: () => cbs.onRenameSymbol?.({ line: sym.line, column: sym.column, symbol: sym.word }),
          });
        }
        items.push(
          {
            label: 'Refactor',
            icon: 'function',
            children: [
              {
                label: 'Rename…',
                icon: 'edit',
                hint: '⇧F6',
                disabled: ro || !sym.word,
                action: () => cbs.onRenameSymbol?.({ line: sym.line, column: sym.column, symbol: sym.word }),
              },
              { label: 'Introduce Variable…', icon: 'plus', hint: '⌥⌘V', disabled: ro, action: () => { runIntroduceVariableInView(view, opts.language, false); } },
              { label: 'Introduce Constant…', icon: 'plus', hint: '⌥⌘C', disabled: ro, action: () => { runIntroduceVariableInView(view, opts.language, true); } },
              {
                label: 'Extract Method…',
                icon: 'procedure',
                hint: '⌥⌘M',
                disabled: ro || !hasSel,
                title: !hasSel ? 'Select statements to extract into a helper function' : undefined,
                action: () => { runExtractMethodInView(view, opts.language); },
              },
              { label: 'Extract Interface…', icon: 'layers', disabled: ro, action: () => { runExtractInterfaceInView(view, opts.language); } },
              {
                label: 'Introduce Type…',
                icon: 'shapes',
                disabled: ro || !hasSel,
                title: !hasSel ? 'Select a type expression to alias' : undefined,
                action: () => { runIntroduceTypeInView(view, opts.language); },
              },
              { label: 'Inline…', icon: 'arrow', hint: '⌥⌘N', disabled: ro, action: () => { runInlineVariableInView(view); } },
              { separator: true },
              { label: 'Move…', icon: 'folder', hint: 'F6', disabled: ro || !fPath, action: () => dispatchFileRefactorEvent('move', fPath) },
              { label: 'Copy File…', icon: 'copy', hint: 'F5', disabled: !fPath, action: () => dispatchFileRefactorEvent('copy', fPath) },
              { label: 'Safe Delete…', icon: 'trash', hint: '⌘⌫', danger: true, disabled: ro || !fPath, action: () => dispatchFileRefactorEvent('safe-delete', fPath) },
            ],
          },
          {
            label: 'Generate…',
            icon: 'sparkle',
            hint: '⌘N',
            disabled: ro,
            children: [
              { label: 'Constructor / Factory', icon: 'function', action: () => { runGenerateInView(view, 'constructor', opts.language); } },
              { label: 'Getter Methods', icon: 'procedure', action: () => { runGenerateInView(view, 'getters', opts.language); } },
              {
                label: 'Implement Interface…',
                icon: 'layers',
                action: () => cbs.onQuickFix?.({ line: sym.line, column: sym.column, symbol: sym.word || undefined, clientX: event.clientX, clientY: event.clientY }),
              },
              { label: 'Test Function / Stub', icon: 'flask', action: () => { runGenerateInView(view, 'test_stub', opts.language); } },
            ],
          },
        );

        const runItem = nearestGutterRunItem(opts.getRunMap?.(view), sym.line);
        if (runItem && cbs.onGutterAction) {
          items.push(
            { separator: true },
            { label: `Run '${runItem.label}'`, icon: 'play', hint: '⌃⇧R', action: () => cbs.onGutterAction?.(runItem, 'run') },
            { label: `Debug '${runItem.label}'`, icon: 'bug', hint: '⌃⇧D', action: () => cbs.onGutterAction?.(runItem, 'debug') },
          );
        }
        if (fPath) {
          if (!runItem || !cbs.onGutterAction) items.push({ separator: true });
          items.push({
            label: 'More Run/Debug',
            icon: 'flask',
            children: [
              { label: 'Run All Tests in Repo', icon: 'play', action: () => dispatchRepoTestAction(fPath, 'run') },
              { label: 'Run All Tests with Coverage', icon: 'gauge', action: () => dispatchRepoTestAction(fPath, 'coverage') },
              { label: 'Debug All Tests in Repo', icon: 'bug', action: () => dispatchRepoTestAction(fPath, 'debug') },
            ],
          });
        }

        if (cbs.onFormat) {
          items.push(
            { separator: true },
            { label: 'Reformat Code', icon: 'format', hint: '⌥⌘L', disabled: ro, action: () => cbs.onFormat?.() },
          );
        }
        if (fPath) {
          const vcsRows = buildIdeVcsContextRows(event, showMain, {
            filePath: fPath,
            readOnly: ro,
            onToggleBlame: cbs.onToggleBlame,
            onPasteFromHistory: cbs.onPasteFromHistory,
          }).filter((row) => row.label !== 'Paste from History…');
          items.push(...vcsRows);
        }

        const syn = new MouseEvent('contextmenu', { clientX: event.clientX, clientY: event.clientY, bubbles: true });
        ctxMenu.show(syn, items);
      };

      event.preventDefault();
      showMain();
      return true;
    },
  }));

  const ideRefactorTheme = EditorView.baseTheme({
    '.cm-ide-col-select-mode .cm-content': { cursor: 'crosshair' },
    '.cm-ide-codevision-row': {
      display: 'flex',
      alignItems: 'center',
      gap: 'var(--sp-3)',
      paddingBlock: '1px',
      lineHeight: '1.25',
      userSelect: 'none',
    },
    '.cm-ide-codevision-pill': {
      appearance: 'none',
      border: '1px solid transparent',
      background: 'transparent',
      color: 'var(--text-dim)',
      fontFamily: 'var(--font-ui)',
      fontSize: 'var(--fs-xs)',
      fontWeight: '500',
      lineHeight: '1.25',
      paddingBlock: '1px',
      paddingInline: '6px',
      borderRadius: 'var(--radius-s)',
      cursor: 'pointer',
      display: 'inline-flex',
      alignItems: 'center',
      gap: '4px',
    },
    '.cm-ide-codevision-pill.is-rename-usages': {
      color: 'var(--accent-text)',
      background: 'var(--accent-soft)',
      borderColor: 'color-mix(in srgb, var(--accent-text) 28%, transparent)',
    },
    '.cm-ide-codevision-pill:hover, .cm-ide-codevision-pill:focus-visible': {
      color: 'var(--accent-text)',
      background: 'var(--accent-soft)',
      borderColor: 'color-mix(in srgb, var(--accent-text) 40%, transparent)',
      outline: 'none',
    },
    '.cm-ide-cv-badge': {
      fontFamily: 'var(--font-mono)',
      fontWeight: '700',
      color: 'var(--accent-text)',
    },
  });

  return [
    ideRefactorTheme,
    columnSelectionModeField,
    colModeRectExt,
    createIdeRefactorCodeVisionExtension({
      diskSavedContent: opts.diskSavedContent,
      savedContent: opts.savedContent,
      language: opts.language,
      getCallbacks: opts.getCallbacks,
    }),
    refactorKeymap,
    contextMenuExt,
  ];
}
