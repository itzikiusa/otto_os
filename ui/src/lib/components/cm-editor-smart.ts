// Smart auto-indentation, minimal external-format text diffs, and bounded
// 300-entry `sessionStorage` cursor/scroll position persistence for `CodeEditor`.

import { completionStatus } from '@codemirror/autocomplete';
import { indentWithTab } from '@codemirror/commands';
import { indentUnit } from '@codemirror/language';
import { EditorSelection, EditorState, Prec, type Extension } from '@codemirror/state';
import { EditorView, keymap } from '@codemirror/view';
import { IDE_CURSOR_POS_KEY, isSecretIdePath } from './cm-ide-drafts.ts';
import {
  clampSelectionToDoc,
  computeMinimalTextUpdate,
  extOf,
  langExtOf,
} from './cm-editor-helpers.ts';

export { clampSelectionToDoc, computeMinimalTextUpdate, extOf, langExtOf };

export function useTabsForLang(filePath: string, hint?: string): boolean {
  const clean = filePath.split('?')[0];
  const base = clean.slice(Math.max(clean.lastIndexOf('/'), clean.lastIndexOf('\\')) + 1).toLowerCase();
  const ext = extOf(filePath);
  const lang = (hint ?? '').toLowerCase();
  if (ext === 'go' || lang === 'go' || lang === 'golang') return true;
  if (base === 'makefile' || base === 'gnumakefile' || ext === 'mk' || lang === 'makefile') return true;
  return false;
}

export function resolveIndentUnit(filePath: string, language?: string, tabSize = 4): string {
  if (useTabsForLang(filePath, language)) return '\t';
  const width = Number.isFinite(tabSize) ? Math.max(1, Math.min(8, Math.round(tabSize))) : 4;
  return ' '.repeat(width);
}

export function computeSmartEnterEdit(
  docText: string,
  pos: number,
  unit: string,
  lang: string,
): { from: number; to: number; insert: string; cursorPos: number } {
  const p = Math.max(0, Math.min(docText.length, pos));
  const lineStart = docText.lastIndexOf('\n', Math.max(0, p - 1)) + 1;
  const nlAfter = docText.indexOf('\n', p);
  const lineEnd = nlAfter >= 0 ? nlAfter : docText.length;
  return computeSmartEnterOnLine(docText.slice(lineStart, lineEnd), p - lineStart, lineStart, unit, lang);
}

function computeSmartEnterOnLine(
  lineText: string,
  relPos: number,
  lineStart: number,
  unit: string,
  lang: string,
): { from: number; to: number; insert: string; cursorPos: number } {
  const baseIndent = /^[ \t]*/.exec(lineText.slice(0, relPos))?.[0] ?? '';
  let trimRelFrom = relPos;
  while (trimRelFrom > baseIndent.length && (lineText[trimRelFrom - 1] === ' ' || lineText[trimRelFrom - 1] === '\t')) {
    trimRelFrom--;
  }
  let trimRelTo = relPos;
  while (trimRelTo < lineText.length && (lineText[trimRelTo] === ' ' || lineText[trimRelTo] === '\t')) {
    trimRelTo++;
  }
  const from = lineStart + trimRelFrom;
  const to = lineStart + trimRelTo;
  const prevCh = trimRelFrom > 0 ? lineText[trimRelFrom - 1] : '';
  const nextCh = trimRelTo < lineText.length ? lineText[trimRelTo] : '';

  if (
    (prevCh === '{' && nextCh === '}') ||
    (prevCh === '(' && nextCh === ')') ||
    (prevCh === '[' && nextCh === ']')
  ) {
    const inner = `\n${baseIndent}${unit}`;
    const insert = `${inner}\n${baseIndent}`;
    return { from, to, insert, cursorPos: from + inner.length };
  }

  const beforeCode = lineText.slice(0, trimRelFrom).replace(/(?:\/\/|#).*$/, '').trimEnd();
  const lastCodeCh = beforeCode[beforeCode.length - 1] ?? '';
  const normLang = lang.toLowerCase();
  const isColonLang = normLang === 'py' || normLang === 'pyi' || normLang === 'python' || normLang === 'yaml' || normLang === 'yml';
  const shouldIndent =
    lastCodeCh === '{' ||
    lastCodeCh === '(' ||
    lastCodeCh === '[' ||
    (isColonLang && lastCodeCh === ':') ||
    (lastCodeCh === ':' && /^(?:case\b|default\b)/.test(beforeCode.trimStart()));

  const insert = shouldIndent ? `\n${baseIndent}${unit}` : `\n${baseIndent}`;
  return { from, to, insert, cursorPos: from + insert.length };
}

export function createSmartIndentExtension(getUnit: () => string, getLang: () => string): Extension {
  const unit = getUnit();
  const tabWidth = unit === '\t' ? 4 : Math.max(1, unit.length);

  const enterBinding = Prec.high(
    keymap.of([
      {
        key: 'Enter',
        run: (view) => {
          if (view.state.readOnly || completionStatus(view.state) === 'active' || view.state.selection.ranges.length > 1) {
            return false;
          }
          const sel = view.state.selection.main;
          const curUnit = getUnit();
          if (!sel.empty) {
            const line = view.state.doc.lineAt(sel.from);
            const baseIndent = /^[ \t]*/.exec(line.text.slice(0, sel.from - line.from))?.[0] ?? '';
            const ins = `\n${baseIndent}`;
            view.dispatch({
              changes: { from: sel.from, to: sel.to, insert: ins },
              selection: EditorSelection.cursor(sel.from + ins.length),
              scrollIntoView: true,
              userEvent: 'input',
            });
            return true;
          }
          const line = view.state.doc.lineAt(sel.head);
          const edit = computeSmartEnterOnLine(line.text, sel.head - line.from, line.from, curUnit, getLang());
          view.dispatch({
            changes: { from: edit.from, to: edit.to, insert: edit.insert },
            selection: EditorSelection.cursor(edit.cursorPos),
            scrollIntoView: true,
            userEvent: 'input',
          });
          return true;
        },
      },
    ]),
  );

  const closeDedentHandler = EditorView.inputHandler.of((view, from, to, text) => {
    if (view.state.readOnly || from !== to || (text !== '}' && text !== ')' && text !== ']')) return false;
    const line = view.state.doc.lineAt(from);
    const before = line.text.slice(0, from - line.from);
    if (!before || !/^[ \t]+$/.test(before)) return false;
    const curUnit = getUnit();
    let dedented = before;
    if (before.endsWith(curUnit)) {
      dedented = before.slice(0, before.length - curUnit.length);
    } else if (before.endsWith('\t')) {
      dedented = before.slice(0, -1);
    } else {
      dedented = before.slice(0, Math.max(0, before.length - tabWidth));
    }
    if (dedented === before) return false;
    const ins = `${dedented}${text}`;
    view.dispatch({
      changes: { from: line.from, to: from, insert: ins },
      selection: EditorSelection.cursor(line.from + ins.length),
      userEvent: 'input.type',
    });
    return true;
  });

  return [
    EditorState.tabSize.of(tabWidth),
    indentUnit.of(unit),
    enterBinding,
    closeDedentHandler,
    keymap.of([indentWithTab]),
  ];
}

export const MAX_IDE_VIEW_POSITIONS = 300;
const keptViewPositions = new Map<string, { anchor: number; head: number; scrollTop: number }>();
let hydratedSessionPositions = false;
let cursorPersistTimer: ReturnType<typeof setTimeout> | null = null;

function viewPosKey(rootPath: string, filePath: string): string {
  const normRoot = (rootPath ?? '').replace(/\\/g, '/').replace(/\/+$/, '');
  const normFile = filePath.split('?')[0].replace(/\\/g, '/');
  return `${normRoot}::${normFile}`;
}

function defaultSessionStore(): Storage | null {
  try {
    return typeof sessionStorage !== 'undefined' ? sessionStorage : null;
  } catch {
    return null;
  }
}

function ensureSessionPositionsLoaded(storage?: Storage | null): void {
  if (hydratedSessionPositions && !storage) return;
  const store = storage ?? defaultSessionStore();
  if (!storage) hydratedSessionPositions = true;
  if (!store) return;
  try {
    const raw = store.getItem(IDE_CURSOR_POS_KEY);
    if (!raw) return;
    const parsed = JSON.parse(raw) as
      | Array<[string, [number, number, number]]>
      | Record<string, { anchor?: number; head?: number; scrollTop?: number }>;
    if (Array.isArray(parsed)) {
      for (const entry of parsed) {
        if (!Array.isArray(entry) || typeof entry[0] !== 'string' || !Array.isArray(entry[1])) continue;
        const [a, h, s] = entry[1];
        if (!keptViewPositions.has(entry[0])) {
          keptViewPositions.set(entry[0], {
            anchor: Math.max(0, Number(a) || 0),
            head: Math.max(0, Number(h) || 0),
            scrollTop: Math.max(0, Number(s) || 0),
          });
        }
      }
    } else if (parsed && typeof parsed === 'object') {
      for (const [k, v] of Object.entries(parsed)) {
        if (v && typeof v === 'object' && !keptViewPositions.has(k)) {
          keptViewPositions.set(k, {
            anchor: Math.max(0, Number(v.anchor) || 0),
            head: Math.max(0, Number(v.head) || 0),
            scrollTop: Math.max(0, Number(v.scrollTop) || 0),
          });
        }
      }
    }
  } catch {
    /* ignore malformed sessionStorage */
  }
}

function flushSessionPositions(storage?: Storage | null): void {
  const store = storage ?? defaultSessionStore();
  if (!store) return;
  try {
    const compact: Array<[string, [number, number, number]]> = [];
    for (const [k, v] of keptViewPositions) {
      compact.push([k, [v.anchor, v.head, Math.round(v.scrollTop)]]);
    }
    store.setItem(IDE_CURSOR_POS_KEY, JSON.stringify(compact));
  } catch {
    /* ignore quota errors */
  }
}

export function saveIdeViewPosition(
  rootPath: string,
  filePath: string,
  pos: { anchor: number; head: number; scrollTop: number },
  opts?: { immediate?: boolean; storage?: Storage | null },
): void {
  if (!filePath || filePath.startsWith('untitled:') || isSecretIdePath(filePath)) return;
  ensureSessionPositionsLoaded(opts?.storage);
  const key = viewPosKey(rootPath, filePath);
  keptViewPositions.delete(key);
  keptViewPositions.set(key, {
    anchor: Math.max(0, Math.trunc(pos.anchor || 0)),
    head: Math.max(0, Math.trunc(pos.head || 0)),
    scrollTop: Math.max(0, pos.scrollTop || 0),
  });
  while (keptViewPositions.size > MAX_IDE_VIEW_POSITIONS) {
    const oldest = keptViewPositions.keys().next().value;
    if (oldest === undefined) break;
    keptViewPositions.delete(oldest);
  }
  if (opts?.storage || opts?.immediate) {
    if (cursorPersistTimer !== null) {
      clearTimeout(cursorPersistTimer);
      cursorPersistTimer = null;
    }
    flushSessionPositions(opts?.storage);
    return;
  }
  if (cursorPersistTimer === null) {
    cursorPersistTimer = setTimeout(() => {
      cursorPersistTimer = null;
      flushSessionPositions();
    }, 180);
  }
}

export function getIdeViewPosition(
  rootPath: string,
  filePath: string,
  storage?: Storage | null,
): { anchor: number; head: number; scrollTop: number } | null {
  if (!filePath || filePath.startsWith('untitled:') || isSecretIdePath(filePath)) return null;
  ensureSessionPositionsLoaded(storage);
  const key = viewPosKey(rootPath, filePath);
  const hit = keptViewPositions.get(key);
  if (!hit) return null;
  keptViewPositions.delete(key);
  keptViewPositions.set(key, hit);
  return { ...hit };
}

export function clearIdeViewPositions(storage?: Storage | null): void {
  if (cursorPersistTimer !== null) {
    clearTimeout(cursorPersistTimer);
    cursorPersistTimer = null;
  }
  keptViewPositions.clear();
  const store = storage ?? defaultSessionStore();
  try {
    store?.removeItem(IDE_CURSOR_POS_KEY);
  } catch {
    /* ignore */
  }
}

export function attachScrollPositionTracker(
  view: EditorView,
  getCtx: () => { root: string; filePath: string },
): () => void {
  const onScroll = () => {
    const { root, filePath } = getCtx();
    if (!filePath) return;
    const sel = view.state.selection.main;
    saveIdeViewPosition(root, filePath, {
      anchor: sel.anchor,
      head: sel.head,
      scrollTop: view.scrollDOM.scrollTop,
    });
  };
  view.scrollDOM.addEventListener('scroll', onScroll, { passive: true });
  return () => view.scrollDOM.removeEventListener('scroll', onScroll);
}

if (typeof window !== 'undefined' && typeof window.addEventListener === 'function') {
  window.addEventListener('otto:ide-storage-cleared', () => {
    keptViewPositions.clear();
  });
}
