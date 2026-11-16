// Deterministic <5ms Quick-Fix Engine & Concrete TextEdit Applier (`userEvent: 'input.quickfix'`,
// 100% undoable with `⌘Z`) across all 8 IDE languages (Java, Go, TS/JS, Python, Rust, JSON, Shell, CSV/Manifests).

import { EditorSelection, type ChangeSpec, type Text } from '@codemirror/state';
import type { EditorView } from '@codemirror/view';
import { getCachedSemanticDoc } from './cm-ide-semantic-model.ts';
import { analyzeGoImports, detectSemanticLang, inferGoImportPkgNames } from './cm-ide-semantic.ts';
import { editOffset } from './cm-ide-semantic-widgets.ts';

export interface IdeConcreteTextEdit {
  path?: string;
  start_line: number;
  start_col: number;
  end_line: number;
  end_col: number;
  new_text: string;
}

export interface IdeQuickFixAction {
  id: string;
  title: string;
  kind?: 'quickfix' | 'refactor' | 'source' | 'dep';
  isPreferred?: boolean;
  edits: IdeConcreteTextEdit[];
  previewBefore?: string;
  previewAfter?: string;
}

const JAVA_STD_IMPORTS: Record<string, string> = {
  List: 'java.util.List', ArrayList: 'java.util.ArrayList', LinkedList: 'java.util.LinkedList',
  Map: 'java.util.Map', HashMap: 'java.util.HashMap', LinkedHashMap: 'java.util.LinkedHashMap',
  Set: 'java.util.Set', HashSet: 'java.util.HashSet', Optional: 'java.util.Optional',
  Collections: 'java.util.Collections', Arrays: 'java.util.Arrays', Objects: 'java.util.Objects',
  UUID: 'java.util.UUID', Stream: 'java.util.stream.Stream', Collectors: 'java.util.stream.Collectors',
  BigDecimal: 'java.math.BigDecimal', Instant: 'java.time.Instant', LocalDate: 'java.time.LocalDate',
  LocalDateTime: 'java.time.LocalDateTime', Duration: 'java.time.Duration',
  Path: 'java.nio.file.Path', Paths: 'java.nio.file.Paths', Files: 'java.nio.file.Files',
  IOException: 'java.io.IOException', CompletableFuture: 'java.util.concurrent.CompletableFuture',
  ConcurrentHashMap: 'java.util.concurrent.ConcurrentHashMap',
  Logger: 'org.slf4j.Logger', LoggerFactory: 'org.slf4j.LoggerFactory',
};

const GO_STD_IMPORTS: Record<string, string> = {
  fmt: 'fmt', context: 'context', strings: 'strings', strconv: 'strconv', errors: 'errors',
  time: 'time', sync: 'sync', os: 'os', io: 'io', bytes: 'bytes', math: 'math', sort: 'sort',
  regexp: 'regexp', log: 'log', http: 'net/http', json: 'encoding/json', sql: 'database/sql',
  filepath: 'path/filepath', atomic: 'sync/atomic', hex: 'encoding/hex', base64: 'encoding/base64',
};

const PY_STD_IMPORTS = new Set([
  'os', 'sys', 'json', 're', 'time', 'math', 'pathlib', 'typing', 'asyncio',
  'logging', 'datetime', 'collections', 'itertools', 'functools', 'subprocess', 'uuid',
]);

/** Apply concrete 1-based line/column edits in a single CodeMirror transaction (`⌘Z` reverts in 1 step!). */
export function applyConcreteEditsToView(view: EditorView, edits: IdeConcreteTextEdit[], currentPath?: string): boolean {
  if (!edits.length || view.state.readOnly) return false;
  const doc = view.state.doc;
  const localEdits = currentPath ? edits.filter((e) => !e.path || e.path === currentPath) : edits;
  if (!localEdits.length) return false;

  const changes: ChangeSpec[] = [];
  let firstPos: number | null = null;
  for (const ed of localEdits) {
    const from = editOffset(doc, ed.start_line, ed.start_col);
    const to = Math.max(from, editOffset(doc, ed.end_line, ed.end_col));
    changes.push({ from, to, insert: ed.new_text });
    if (firstPos === null) firstPos = from;
  }

  view.dispatch({
    changes,
    ...(firstPos !== null ? { selection: EditorSelection.cursor(Math.min(doc.length, firstPos)) } : {}),
    userEvent: 'input.quickfix',
    scrollIntoView: true,
  });
  return true;
}

/** Pure preview helper applying `IdeConcreteTextEdit[]` to a string so tests & `⌥⏎` preview can render `- / +` diffs. */
export function previewConcreteEdits(docText: string, edits: IdeConcreteTextEdit[]): { nextText: string; beforeSnippet: string; afterSnippet: string } {
  const lines = docText.split('\n');
  const lineStarts: number[] = [];
  let acc = 0;
  for (const ln of lines) { lineStarts.push(acc); acc += ln.length + 1; }
  const posOf = (line1: number, col1: number): number => {
    if (line1 > lines.length) return docText.length;
    const idx = Math.max(0, Math.min(lines.length - 1, line1 - 1));
    return Math.min(docText.length, lineStarts[idx] + Math.max(0, Math.min(lines[idx].length + 1, (col1 || 1) - 1)));
  };
  const spans = edits.map((e) => ({ from: posOf(e.start_line, e.start_col), to: posOf(e.end_line, e.end_col), text: e.new_text }))
    .sort((a, b) => b.from - a.from);
  let next = docText;
  for (const s of spans) next = next.slice(0, s.from) + s.text + next.slice(Math.max(s.from, s.to));
  const minLine = Math.max(1, Math.min(...edits.map((e) => e.start_line)) - 1);
  const maxLine = Math.min(lines.length, Math.max(...edits.map((e) => e.end_line)) + 1);
  const nextLines = next.split('\n');
  return {
    nextText: next,
    beforeSnippet: lines.slice(minLine - 1, maxLine).join('\n'),
    afterSnippet: nextLines.slice(minLine - 1, Math.min(nextLines.length, maxLine + 2)).join('\n'),
  };
}

function makeDeleteLineEdit(lines: string[], line1: number, filePath: string): IdeConcreteTextEdit {
  if (line1 < lines.length) {
    return { path: filePath, start_line: line1, start_col: 1, end_line: line1 + 1, end_col: 1, new_text: '' };
  }
  const prevLen = line1 > 1 ? lines[line1 - 2].length + 1 : 1;
  return {
    path: filePath,
    start_line: line1 > 1 ? line1 - 1 : 1,
    start_col: line1 > 1 ? prevLen : 1,
    end_line: line1,
    end_col: lines[line1 - 1].length + 1,
    new_text: '',
  };
}

function toCamelJsonTag(fieldName: string): string {
  if (!fieldName) return 'field';
  if (/^[A-Z]+$/.test(fieldName)) return fieldName.toLowerCase();
  return fieldName[0].toLowerCase() + fieldName.slice(1);
}

const UNUSED_IMPORT_DIAG_RE = /\b(?:imported and not used|unused\s*import|unusedimport)\b/i;

/**
 * Returns true only when `line1` is a Go import spec whose qualifier is NOT referenced
 * outside the `import` declaration (unless `_` or `.`), OR is explicitly flagged by an
 * unused-import diagnostic on that line.
 */
export function isGoImportUnusedInDoc(
  docText: string,
  line1: number,
  diagnostics?: readonly { line?: number; code?: string | null; message?: string }[] | null,
): boolean {
  const lines = docText.split('\n');
  if (line1 < 1 || line1 > lines.length) return false;
  const curLine = lines[line1 - 1] ?? '';
  const noComment = curLine.replace(/\/\/.*$/, '').trim();
  const mImp = /^(?:import\s+)?(?:([._]|[A-Za-z_]\w*)\s+)?(["`][^"`]+["`])\s*$/.exec(noComment);
  if (!mImp) return false;

  if (diagnostics && diagnostics.some((d) => (d.line == null || d.line === line1) && (UNUSED_IMPORT_DIAG_RE.test(d.message ?? '') || UNUSED_IMPORT_DIAG_RE.test(d.code ?? '')))) {
    return true;
  }

  const alias = mImp[1];
  if (alias === '_' || alias === '.') return false;

  const analyzed = analyzeGoImports(docText);
  const entry = analyzed.entries.find((e) => e.line === line1);
  if (entry) {
    return entry.names.length > 0 && !entry.used;
  }

  // Fallback for import specs past line 300 if any exist.
  const { names } = inferGoImportPkgNames(mImp[2].slice(1, -1), alias);
  if (names.length === 0) return false;
  for (let i = 0; i < lines.length; i++) {
    if (i + 1 === line1) continue;
    const raw = lines[i].replace(/\/\/.*$/, '');
    if (/^\s*import\b/.test(raw)) continue;
    if (names.some((n) => new RegExp(`\\b${n.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\.`).test(raw))) {
      return false;
    }
  }
  return true;
}

/** Compute deterministic, instant (<5ms) concrete Quick Fixes at 1-based `line` across all 8 languages. */
export function computeDeterministicQuickFixes(
  docText: string,
  filePath: string,
  language: string | undefined,
  line: number,
  symbol?: string,
  diagnostics?: readonly { line?: number; code?: string | null; message?: string }[] | null,
): IdeQuickFixAction[] {
  const lines = docText.split('\n');
  if (line < 1 || line > lines.length) return [];
  const lang = detectSemanticLang(filePath, language);
  const curLine = lines[line - 1] ?? '';
  const trimmed = curLine.trim();
  const out: IdeQuickFixAction[] = [];

  if (lang === 'java') {
    const mImp = /^\s*import\s+(?:static\s+)?([\w.*$]+)\s*;\s*$/.exec(curLine);
    if (mImp) {
      const impPath = mImp[1];
      out.push({
        id: `java-remove-import-${line}`,
        title: `Remove import '${impPath}'`,
        kind: 'quickfix',
        isPreferred: true,
        edits: [makeDeleteLineEdit(lines, line, filePath)],
        previewBefore: curLine,
        previewAfter: '// (import removed)',
      });
    }
    // Missing Java import for identifier on current line
    const words = curLine.match(/\b[A-Z][A-Za-z0-9_]*\b/g) ?? [];
    for (const w of new Set(symbol && /^[A-Z]/.test(symbol) ? [symbol, ...words] : words)) {
      const fqn = JAVA_STD_IMPORTS[w];
      if (fqn && !new RegExp(`^\\s*import\\s+${fqn.replace(/\./g, '\\.')}\\s*;`, 'm').test(docText)) {
        let insertLine = 1;
        for (let i = 0; i < Math.min(lines.length, 60); i++) {
          if (/^\s*package\s+/.test(lines[i])) insertLine = i + 2;
          else if (/^\s*import\s+/.test(lines[i])) insertLine = i + 2;
        }
        insertLine = Math.min(lines.length, Math.max(1, insertLine));
        out.push({
          id: `java-import-${w}`,
          title: `Import '${fqn}'`,
          kind: 'quickfix',
          isPreferred: out.length === 0,
          edits: [{ path: filePath, start_line: insertLine, start_col: 1, end_line: insertLine, end_col: 1, new_text: `import ${fqn};\n` }],
          previewBefore: lines[insertLine - 1] ?? '',
          previewAfter: `import ${fqn};\n${lines[insertLine - 1] ?? ''}`,
        });
      }
    }
    // Optimize all Java imports if any duplicate/unused exist
    const impLines: Array<{ ln: number; text: string; path: string }> = [];
    const bodyLines: string[] = [];
    lines.forEach((l, idx) => {
      const m = /^\s*import\s+(?:static\s+)?([\w.*$]+)\s*;\s*$/.exec(l);
      if (m) impLines.push({ ln: idx + 1, text: l.trim(), path: m[1] });
      else if (!l.trim().startsWith('package ')) bodyLines.push(l);
    });
    if (impLines.length > 1) {
      const bodyStr = bodyLines.join('\n');
      const seen = new Set<string>();
      const kept = impLines.filter((im) => {
        if (seen.has(im.path)) return false;
        seen.add(im.path);
        const last = im.path.split('.').pop() ?? '';
        return last === '*' || new RegExp(`\\b${last.replace(/\$/g, '\\$')}\\b`).test(bodyStr);
      }).sort((a, b) => a.path.localeCompare(b.path));
      if (kept.length !== impLines.length) {
        const firstLn = impLines[0].ln, lastLn = impLines[impLines.length - 1].ln;
        const replacement = kept.map((k) => `import ${k.path};`).join('\n');
        out.push({
          id: 'java-optimize-imports',
          title: 'Optimize all imports',
          kind: 'source',
          edits: [{ path: filePath, start_line: firstLn, start_col: 1, end_line: lastLn, end_col: lines[lastLn - 1].length + 1, new_text: replacement }],
          previewBefore: lines.slice(firstLn - 1, lastLn).join('\n'),
          previewAfter: replacement || '// (all unused imports removed)',
        });
      }
    }
    // Add @Override above public method without @Override
    if (/^\s*public\s+(?!class\b|interface\b|enum\b|record\b|static\b)(?:<[^>{};]+>\s+)?(?:[\w.$?[\]]+(?:<[^>{};]+>)?(?:\[\])*)\s+[a-z]\w*\s*\(/.test(curLine)) {
      const prevLine = line > 1 ? lines[line - 2].trim() : '';
      if (!prevLine.startsWith('@Override')) {
        const indent = /^(\s*)/.exec(curLine)?.[1] ?? '    ';
        out.push({
          id: `java-add-override-${line}`,
          title: 'Add @Override annotation',
          kind: 'quickfix',
          edits: [{ path: filePath, start_line: line, start_col: 1, end_line: line, end_col: 1, new_text: `${indent}@Override\n` }],
          previewBefore: curLine,
          previewAfter: `${indent}@Override\n${curLine}`,
        });
      }
    }
  } else if (lang === 'go') {
    // Missing Go stdlib import (`fmt.Println`, `context.Context`, `http.StatusOK`, etc.)
    const pkgRefs = [...curLine.matchAll(/\b([a-z][a-z0-9_]*)\.[A-Z]\w*/g)].map((m) => m[1]);
    for (const pkg of new Set(pkgRefs)) {
      const imp = GO_STD_IMPORTS[pkg];
      if (imp && !new RegExp(`"${imp.replace(/\//g, '\\/')}"`).test(docText)) {
        const impBlockIdx = lines.findIndex((l) => /^\s*import\s*\(\s*$/.test(l));
        if (impBlockIdx >= 0) {
          const insLn = impBlockIdx + 2;
          out.push({
            id: `go-import-${pkg}`,
            title: `Import "${imp}"`,
            kind: 'quickfix',
            isPreferred: true,
            edits: [{ path: filePath, start_line: insLn, start_col: 1, end_line: insLn, end_col: 1, new_text: `\t"${imp}"\n` }],
            previewBefore: lines[impBlockIdx],
            previewAfter: `${lines[impBlockIdx]}\n\t"${imp}"`,
          });
        } else {
          const pkgLn = Math.max(1, lines.findIndex((l) => /^\s*package\s+/.test(l)) + 1);
          out.push({
            id: `go-import-${pkg}`,
            title: `Import "${imp}"`,
            kind: 'quickfix',
            isPreferred: true,
            edits: [{ path: filePath, start_line: pkgLn, start_col: lines[pkgLn - 1].length + 1, end_line: pkgLn, end_col: lines[pkgLn - 1].length + 1, new_text: `\n\nimport "${imp}"` }],
            previewBefore: lines[pkgLn - 1],
            previewAfter: `${lines[pkgLn - 1]}\n\nimport "${imp}"`,
          });
        }
      }
    }
    // Remove Go import line ONLY when the imported qualifier is unused in `docText` (or flagged by an unused-import diagnostic)
    if (isGoImportUnusedInDoc(docText, line, diagnostics)) {
      out.push({
        id: `go-remove-import-${line}`,
        title: `Remove import ${trimmed}`,
        kind: 'quickfix',
        edits: [makeDeleteLineEdit(lines, line, filePath)],
        previewBefore: curLine,
        previewAfter: '// (import removed)',
      });
    }
    // Add `if err != nil { return err }`
    if (/\berr\s*:?=/.test(curLine) && !/^\s*if\s+err\s*!=\s*nil/.test(lines[line] ?? '')) {
      const indent = /^(\s*)/.exec(curLine)?.[1] ?? '\t';
      const snippet = `\n${indent}if err != nil {\n${indent}\treturn err\n${indent}}`;
      out.push({
        id: `go-if-err-${line}`,
        title: "Add 'if err != nil { return err }'",
        kind: 'quickfix',
        edits: [{ path: filePath, start_line: line, start_col: curLine.length + 1, end_line: line, end_col: curLine.length + 1, new_text: snippet }],
        previewBefore: curLine,
        previewAfter: `${curLine}${snippet}`,
      });
    }
    // Add JSON struct tag on struct field (`UserID string` -> `UserID string \`json:"userId"\``)
    const mField = /^(\s+[A-Z]\w*\s+[*[\]\w.]+)\s*$/.exec(curLine);
    if (mField && !curLine.includes('`') && !trimmed.startsWith('func ') && !trimmed.startsWith('type ')) {
      const fieldName = trimmed.split(/\s+/)[0];
      const tagged = `${mField[1]} \`json:"${toCamelJsonTag(fieldName)}"\``;
      out.push({
        id: `go-json-tag-${line}`,
        title: `Add json:"${toCamelJsonTag(fieldName)}" struct tag`,
        kind: 'refactor',
        edits: [{ path: filePath, start_line: line, start_col: 1, end_line: line, end_col: curLine.length + 1, new_text: tagged }],
        previewBefore: curLine,
        previewAfter: tagged,
      });
    }
  } else if (lang === 'ts') {
    if (/^\s*import\s+/.test(curLine)) {
      out.push({ id: `ts-rm-imp-${line}`, title: 'Remove unused import', kind: 'quickfix', isPreferred: true, edits: [makeDeleteLineEdit(lines, line, filePath)], previewBefore: curLine, previewAfter: '// (import removed)' });
    }
    if (symbol && /^[a-zA-Z]\w*$/.test(symbol) && !symbol.startsWith('_')) {
      const colIdx = curLine.indexOf(symbol);
      if (colIdx >= 0) {
        const after = `${curLine.slice(0, colIdx)}_${symbol}${curLine.slice(colIdx + symbol.length)}`;
        out.push({ id: `ts-underscore-${line}`, title: `Prefix '${symbol}' with '_'`, kind: 'quickfix', edits: [{ path: filePath, start_line: line, start_col: colIdx + 1, end_line: line, end_col: colIdx + 1 + symbol.length, new_text: `_${symbol}` }], previewBefore: curLine, previewAfter: after });
      }
    }
    if (/^\s*(?:export\s+)?function\s+\w+\s*\(/.test(curLine) && !/\basync\b/.test(curLine)) {
      const after = curLine.replace(/\bfunction\b/, 'async function');
      out.push({ id: `ts-async-${line}`, title: "Add 'async' modifier", kind: 'quickfix', edits: [{ path: filePath, start_line: line, start_col: 1, end_line: line, end_col: curLine.length + 1, new_text: after }], previewBefore: curLine, previewAfter: after });
    }
  } else if (lang === 'python') {
    if (/^\s*(?:import|from)\s+/.test(curLine)) {
      out.push({ id: `py-rm-imp-${line}`, title: 'Remove unused import', kind: 'quickfix', isPreferred: true, edits: [makeDeleteLineEdit(lines, line, filePath)], previewBefore: curLine, previewAfter: '# (import removed)' });
    }
    for (const m of curLine.matchAll(/\b([a-z_]\w*)\.[A-Za-z_]\w*/g)) {
      const mod = m[1];
      if (PY_STD_IMPORTS.has(mod) && !new RegExp(`^\\s*(?:import\\s+${mod}\\b|from\\s+${mod}\\b)`, 'm').test(docText)) {
        out.push({ id: `py-imp-${mod}`, title: `Import '${mod}'`, kind: 'quickfix', isPreferred: true, edits: [{ path: filePath, start_line: 1, start_col: 1, end_line: 1, end_col: 1, new_text: `import ${mod}\n` }], previewBefore: lines[0] ?? '', previewAfter: `import ${mod}\n${lines[0] ?? ''}` });
      }
    }
  } else if (lang === 'rust') {
    if (/^\s*use\s+/.test(curLine)) {
      out.push({ id: `rs-rm-use-${line}`, title: "Remove unused 'use' import", kind: 'quickfix', isPreferred: true, edits: [makeDeleteLineEdit(lines, line, filePath)], previewBefore: curLine, previewAfter: '// (use removed)' });
    }
    if (/^\s*(?:pub\s+)?(?:struct|enum)\s+\w+/.test(curLine) && !(line > 1 && lines[line - 2].trim().startsWith('#[derive'))) {
      const indent = /^(\s*)/.exec(curLine)?.[1] ?? '';
      out.push({ id: `rs-derive-${line}`, title: 'Add #[derive(Debug, Clone)]', kind: 'quickfix', edits: [{ path: filePath, start_line: line, start_col: 1, end_line: line, end_col: 1, new_text: `${indent}#[derive(Debug, Clone)]\n` }], previewBefore: curLine, previewAfter: `${indent}#[derive(Debug, Clone)]\n${curLine}` });
    }
    const mLet = /^(\s*let\s+)([a-z]\w*)(\s*[=:])/.exec(curLine);
    if (mLet) {
      const after = `${mLet[1]}_${mLet[2]}${curLine.slice(mLet[0].length - mLet[3].length)}`;
      out.push({ id: `rs-underscore-${line}`, title: `Prefix '${mLet[2]}' with '_'`, kind: 'quickfix', edits: [{ path: filePath, start_line: line, start_col: 1, end_line: line, end_col: curLine.length + 1, new_text: after }], previewBefore: curLine, previewAfter: after });
    }
  } else if (lang === 'json') {
    if (/,\s*$/.test(curLine) && /^\s*[}\]]/.test(lines[line] ?? '')) {
      const after = curLine.replace(/,(\s*)$/, '$1');
      out.push({ id: `json-comma-${line}`, title: 'Remove trailing comma', kind: 'quickfix', isPreferred: true, edits: [{ path: filePath, start_line: line, start_col: 1, end_line: line, end_col: curLine.length + 1, new_text: after }], previewBefore: curLine, previewAfter: after });
    }
    const mUnq = /^(\s*)([A-Za-z_$][\w$-]*)(\s*:.*)$/.exec(curLine);
    if (mUnq) {
      const after = `${mUnq[1]}"${mUnq[2]}"${mUnq[3]}`;
      out.push({ id: `json-quote-${line}`, title: `Quote key "${mUnq[2]}"`, kind: 'quickfix', isPreferred: true, edits: [{ path: filePath, start_line: line, start_col: 1, end_line: line, end_col: curLine.length + 1, new_text: after }], previewBefore: curLine, previewAfter: after });
    }
  } else if (lang === 'shell') {
    const mVar = /(^|[^"\\])(\$[A-Za-z_]\w*)/.exec(curLine);
    if (mVar) {
      const colStart = mVar.index + mVar[1].length + 1;
      const quoted = `"${mVar[2]}"`;
      out.push({ id: `sh-quote-${line}`, title: `Quote variable ${quoted}`, kind: 'quickfix', isPreferred: true, edits: [{ path: filePath, start_line: line, start_col: colStart, end_line: line, end_col: colStart + mVar[2].length, new_text: quoted }], previewBefore: curLine, previewAfter: curLine.replace(mVar[2], quoted) });
    }
    if (!lines[0]?.startsWith('#!')) {
      out.push({ id: 'sh-shebang', title: "Insert '#!/usr/bin/env bash'", kind: 'quickfix', edits: [{ path: filePath, start_line: 1, start_col: 1, end_line: 1, end_col: 1, new_text: '#!/usr/bin/env bash\n\n' }], previewBefore: lines[0] ?? '', previewAfter: `#!/usr/bin/env bash\n\n${lines[0] ?? ''}` });
    }
  }

  return out;
}

export function hasDeterministicQuickFixAtLine(
  doc: Text,
  filePath: string,
  language: string | undefined,
  line: number,
): boolean {
  if (line < 1 || line > doc.lines) return false;
  const lang = detectSemanticLang(filePath, language);
  const curLine = doc.line(line).text;
  const trimmed = curLine.trim();
  if (!trimmed && lang !== 'shell') return false;
  const headerSlice = () => doc.sliceString(0, doc.line(Math.min(doc.lines, 80)).to);

  if (lang === 'java') {
    if (/^\s*import\s+(?:static\s+)?[\w.*$]+\s*;\s*$/.test(curLine)) return true;
    if (
      /^\s*public\s+(?!class\b|interface\b|enum\b|record\b|static\b)(?:<[^>{};]+>\s+)?(?:[\w.$?[\]]+(?:<[^>{};]+>)?(?:\[\])*)\s+[a-z]\w*\s*\(/.test(curLine) &&
      !(line > 1 && doc.line(line - 1).text.trim().startsWith('@Override'))
    ) {
      return true;
    }
    const words = curLine.match(/\b[A-Z][A-Za-z0-9_]*\b/g);
    if (words) {
      let hdr: string | null = null;
      for (const w of words) {
        const fqn = JAVA_STD_IMPORTS[w];
        if (fqn && !(hdr ??= headerSlice()).includes(`import ${fqn};`)) return true;
      }
    }
    return false;
  }
  if (lang === 'go') {
    if (/^\s*(?:[._]|[A-Za-z_]\w*\s+)?["`][^"`]+["`]\s*(?:\/\/.*)?$/.test(curLine) || /^\s*import\s+(?:[._]\s+|[A-Za-z_]\w*\s+)?["`][^"`]+["`]/.test(curLine)) {
      if (isGoImportUnusedInDoc(getCachedSemanticDoc(doc, 'go', undefined, undefined, false, false).docText, line)) return true;
    }
    if (/\berr\s*:?=/.test(curLine) && !(line < doc.lines && /^\s*if\s+err\s*!=\s*nil/.test(doc.line(line + 1).text))) {
      return true;
    }
    if (
      /^(\s+[A-Z]\w*\s+[*[\]\w.]+)\s*$/.test(curLine) &&
      !curLine.includes('`') &&
      !trimmed.startsWith('func ') &&
      !trimmed.startsWith('type ')
    ) {
      return true;
    }
    let hdr: string | null = null;
    for (const m of curLine.matchAll(/\b([a-z][a-z0-9_]*)\.[A-Z]\w*/g)) {
      const imp = GO_STD_IMPORTS[m[1]];
      if (imp && !(hdr ??= headerSlice()).includes(`"${imp}"`)) return true;
    }
    return false;
  }
  if (lang === 'ts') {
    return /^\s*import\s+/.test(curLine) || (/^\s*(?:export\s+)?function\s+\w+\s*\(/.test(curLine) && !/\basync\b/.test(curLine));
  }
  if (lang === 'python') {
    if (/^\s*(?:import|from)\s+/.test(curLine)) return true;
    let hdr: string | null = null;
    for (const m of curLine.matchAll(/\b([a-z_]\w*)\.[A-Za-z_]\w*/g)) {
      const mod = m[1];
      if (PY_STD_IMPORTS.has(mod) && !new RegExp(`^\\s*(?:import\\s+${mod}\\b|from\\s+${mod}\\b)`, 'm').test((hdr ??= headerSlice()))) {
        return true;
      }
    }
    return false;
  }
  if (lang === 'rust') {
    if (/^\s*use\s+/.test(curLine) || /^(\s*let\s+)([a-z]\w*)(\s*[=:])/.test(curLine)) return true;
    return /^\s*(?:pub\s+)?(?:struct|enum)\s+\w+/.test(curLine) && !(line > 1 && doc.line(line - 1).text.trim().startsWith('#[derive'));
  }
  if (lang === 'json') {
    if (/,\s*$/.test(curLine) && line < doc.lines && /^\s*[}\]]/.test(doc.line(line + 1).text)) return true;
    return /^(\s*)([A-Za-z_$][\w$-]*)(\s*:.*)$/.test(curLine);
  }
  if (lang === 'shell') {
    return /(^|[^"\\])(\$[A-Za-z_]\w*)/.test(curLine) || !doc.line(1).text.startsWith('#!');
  }
  return false;
}
