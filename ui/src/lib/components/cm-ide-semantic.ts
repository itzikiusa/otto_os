// Viewport-only (<1ms) Semantic Classifier (8 languages), Rainbow Brackets, Inlay Hints, CodeVision & Import Folding.
import { foldEffect, foldService, foldedRanges, unfoldEffect } from '@codemirror/language';
import { setDiagnostics, type Action as LintAction, type Diagnostic } from '@codemirror/lint';
import { EditorState, StateEffect, StateField, type Extension, type Range } from '@codemirror/state';
import { Decoration, EditorView, ViewPlugin, WidgetType, type DecorationSet, type ViewUpdate } from '@codemirror/view';
import type { IdeDiagnostic } from '../api/types.ts';
import type { IdeEditorPreferences } from './cm-ide-theme.ts';

export type SemanticLangKind = 'go' | 'java' | 'ts' | 'rust' | 'python' | 'shell' | 'json' | 'csv' | 'other';
export type SemanticRole = 'fg' | 'keyword' | 'control' | 'func-decl' | 'func-call' | 'field' | 'type' | 'annotation' | 'param' | 'doc-comment' | 'doc-tag' | 'doc-param' | 'unresolved';
export interface SemanticTokenSpan { from: number; to: number; role: SemanticRole; text: string; }
export interface RainbowBracketSpan { from: number; to: number; level: 1 | 2 | 3 | 4 | 'unmatched'; active: boolean; }
export interface ClientDiagQuickFix { title: string; line: number; endLine: number; deleteLine?: boolean; replacement?: string; edits?: Array<{ path?: string; start_line: number; start_col: number; end_line: number; end_col: number; new_text: string }>; }
export interface ClientIdeDiagnostic { line: number; column: number; endLine: number; endColumn: number; severity: 'error' | 'warning' | 'info'; message: string; source: string; code?: string; quickFixes: ClientDiagQuickFix[]; }

export function detectSemanticLang(filePath: string, language?: string): SemanticLangKind {
  if (filePath.startsWith('jdt://')) return 'java';
  const clean = filePath.split('?')[0].toLowerCase(), ext = clean.slice(Math.max(0, clean.lastIndexOf('/'))).split('.').pop() ?? '';
  if (ext === 'go' || language === 'go') return 'go';
  if (ext === 'java' || ext === 'class' || language === 'java') return 'java';
  if (['ts', 'tsx', 'js', 'jsx', 'mjs', 'cjs', 'mts', 'cts'].includes(ext) || language === 'typescript' || language === 'javascript') return 'ts';
  if (ext === 'rs' || language === 'rust') return 'rust';
  if (ext === 'py' || ext === 'pyi' || language === 'python') return 'python';
  if (['sh', 'bash', 'zsh', 'bats'].includes(ext) || language === 'shell' || language === 'shellscript') return 'shell';
  if (['json', 'jsonc', 'jsonl', 'ndjson'].includes(ext) || language === 'json' || language === 'jsonl') return 'json';
  return ext === 'csv' || ext === 'tsv' || language === 'csv' || language === 'tsv' ? 'csv' : 'other';
}
const CONTROL_KW = new Set('if else for range return switch case default select defer go break continue fallthrough try catch finally throw throws while do match loop await async yield elif except raise with'.split(' '));
const GENERAL_KW = new Set(('package import func var const type struct interface map chan string int int8 int16 int32 int64 uint uint8 uint16 uint32 uint64 uintptr float32 float64 bool byte rune error any comparable true false nil iota public private protected static final abstract synchronized volatile transient native sealed non-sealed permits class enum record extends implements new instanceof void boolean short long double float char null this super export from let function typeof keyof readonly as in of undefined fn mut pub impl trait mod use where crate self Self unsafe dyn move ref def None True False and or not is lambda pass global nonlocal fi then done esac').split(' '));
const BOGUS_JAVA_PREFIXES = ['com.nonexistent.', 'org.nonexistent.', 'net.nonexistent.', 'invalid.', 'bogus.', 'fake.', 'unknown.', 'does.not.exist.', 'nonexistent.'];
interface FileSymbolModel {
  pkgVars: Set<string>; typeNames: Set<string>; funcParams: Map<string, string[]>;
  funcDecls: Array<{ name: string; line: number; nameFrom: number; nameTo: number; lineEnd: number }>;
  paramScopes: Array<{ from: number; to: number; params: Set<string> }>; unresolvedIdents: Set<string>;
}
function splitParamNames(rawParams: string, lang: SemanticLangKind): string[] {
  const names: string[] = [];
  for (const part of rawParams.split(',')) {
    const clean = part.replace(/\/\*[\s\S]*?\*\//g, '').replace(/=[^,]+$/, '').trim();
    if (!clean || clean === 'self' || clean === '&self' || clean === '&mut self' || clean === 'this') continue;
    const m = lang === 'go' ? /^([A-Za-z_]\w*)(?:\s+(?:\.\.\.)?[*[\]\w.]+)?$/.exec(clean) : lang === 'java' ? /(?:final\s+|@\w+(?:\([^)]*\))?\s+)*[\w<>[\],.?]+\s*(?:\.\.\.)?\s+([A-Za-z_$][\w$]*)$/.exec(clean) : /^(?:pub\s+|private\s+|readonly\s+|mut\s+|\*\*{0,2})?([A-Za-z_$][\w$]*)\s*(?:\??\s*:|$)/.exec(clean);
    if (m && !GENERAL_KW.has(m[1])) names.push(m[1]);
  }
  return names;
}
export interface JavaDiagPrefs { javaUnresolvedImports?: boolean; lombokSupport?: boolean; unusedImports?: boolean; }
function buildSymbolModel(docText: string, lang: SemanticLangKind, extraUnresolved?: ReadonlySet<string>, jPrefs?: JavaDiagPrefs): FileSymbolModel {
  const sample = docText.length > 65536 ? docText.slice(0, 65536) : docText;
  const pkgVars = new Set<string>(), typeNames = new Set<string>(), funcParams = new Map<string, string[]>();
  const funcDecls: FileSymbolModel['funcDecls'] = [], paramScopes: FileSymbolModel['paramScopes'] = [], unresolvedIdents = new Set<string>(extraUnresolved);
  if (lang === 'java') {
    for (const id of analyzeJavaDiagnostics(sample, jPrefs).unresolvedIdents) unresolvedIdents.add(id);
    if ((jPrefs?.lombokSupport ?? true) && /@(?:Slf4j|Log4j2|Log|CommonsLog|JBossLog|Flogger)\b/.test(sample)) pkgVars.add('log');
  }
  const lines = sample.split('\n');
  let offset = 0, inGoVarBlock = false;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i], trimmed = line.trim(), lineStart = offset, lineEnd = offset + line.length;
    offset = lineEnd + 1;
    if (lang === 'go') {
      if (/^(?:var|const)\s*\(\s*$/.test(trimmed)) { inGoVarBlock = true; continue; }
      if (inGoVarBlock) {
        if (trimmed.startsWith(')')) { inGoVarBlock = false; continue; }
        const mv = /^([A-Za-z_]\w*(?:\s*,\s*[A-Za-z_]\w*)*)\b/.exec(trimmed);
        if (mv) for (const v of mv[1].split(',')) pkgVars.add(v.trim());
        continue;
      }
      const mTopVar = /^(?:var|const)\s+([A-Za-z_]\w*)\b/.exec(trimmed);
      if (mTopVar && line[0] !== '\t' && line[0] !== ' ') pkgVars.add(mTopVar[1]);
      const mType = /^type\s+([A-Za-z_]\w*)\b/.exec(trimmed);
      if (mType) typeNames.add(mType[1]);
      const mFn = /^func\s+(?:\(\s*([A-Za-z_]\w*)?\s*\*?[A-Za-z_][\w.]*\s*\)\s*)?([A-Za-z_]\w*)\s*\(([^)]*)\)/.exec(trimmed);
      if (mFn) {
        const fnName = mFn[2], nFrom = lineStart + Math.max(0, line.indexOf(`${fnName}(`) >= 0 ? line.indexOf(`${fnName}(`) : line.indexOf(fnName));
        funcDecls.push({ name: fnName, line: i + 1, nameFrom: nFrom, nameTo: nFrom + fnName.length, lineEnd });
        const pList = splitParamNames(mFn[3], 'go');
        if (pList.length) { funcParams.set(fnName, pList); paramScopes.push({ from: lineStart, to: sample.length, params: new Set(pList) }); }
      }
    } else if (lang === 'java') {
      const mType = /^(?:public|protected|private|abstract|final|static|sealed|\s)*(?:class|interface|enum|record)\s+([A-Za-z_$][\w$]*)/.exec(trimmed);
      if (mType) { typeNames.add(mType[1]); continue; }
      const mField = /^(?:private|protected|public)\s+(?:static\s+|final\s+|volatile\s+|transient\s+)*(?:[\w.$?[\]]+(?:<[^>{};]+>)?(?:\[\])*)\s+([A-Za-z_$][\w$]*)\s*[;=]/.exec(trimmed);
      if (mField && !trimmed.includes('(')) { pkgVars.add(mField[1]); continue; }
      const mMeth = /^(?:public|protected|private|static|final|synchronized|default|abstract|\s)+(?:<[^>{};]+>\s+)?(?:[\w.$?[\]]+(?:<[^>{};]+>)?(?:\[\])*)\s+([A-Za-z_$][\w$]*)\s*\(([^)]*)\)/.exec(trimmed);
      if (mMeth && !CONTROL_KW.has(mMeth[1])) {
        const fnName = mMeth[1], nFrom = lineStart + Math.max(0, line.indexOf(`${fnName}(`) >= 0 ? line.indexOf(`${fnName}(`) : line.indexOf(fnName));
        funcDecls.push({ name: fnName, line: i + 1, nameFrom: nFrom, nameTo: nFrom + fnName.length, lineEnd });
        const pList = splitParamNames(mMeth[2], 'java');
        if (pList.length) { funcParams.set(fnName, pList); paramScopes.push({ from: lineStart, to: sample.length, params: new Set(pList) }); }
      }
    } else if (lang === 'ts' || lang === 'rust' || lang === 'python' || lang === 'shell') {
      const re = lang === 'rust' ? /^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+|unsafe\s+|const\s+)*fn\s+([A-Za-z_]\w*)\s*(?:<[^>]*>)?\s*\(([^)]*)\)/
        : lang === 'python' ? /^(?:async\s+)?def\s+([A-Za-z_]\w*)\s*\(([^)]*)\)/
        : lang === 'shell' ? /^(?:function\s+([A-Za-z_][\w.-]*)|([A-Za-z_][\w.-]*)\s*\(\s*\))/
        : /^(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s+([A-Za-z_$][\w$]*)\s*(?:<[^>]*>)?\s*\(([^)]*)\)/;
      const m = re.exec(trimmed), fnName = m ? (m[1] || m[2]) : '', nIdx = fnName ? line.indexOf(fnName) : -1;
      if (m && fnName && nIdx >= 0) {
        funcDecls.push({ name: fnName, line: i + 1, nameFrom: lineStart + nIdx, nameTo: lineStart + nIdx + fnName.length, lineEnd });
        const pList = m[2] && lang !== 'shell' ? splitParamNames(m[2], lang) : [];
        if (pList.length) { funcParams.set(fnName, pList); paramScopes.push({ from: lineStart, to: sample.length, params: new Set(pList) }); }
      }
    }
  }
  for (let i = 0; i + 1 < paramScopes.length; i++) {
    const nextStart = funcDecls[i + 1]?.nameFrom;
    if (nextStart && nextStart > paramScopes[i].from) paramScopes[i].to = nextStart - 1;
  }
  return { pkgVars, typeNames, funcParams, funcDecls, paramScopes, unresolvedIdents };
}

function classifyDocComment(text: string, baseFrom: number, out: SemanticTokenSpan[]): void {
  out.push({ from: baseFrom, to: baseFrom + text.length, role: 'doc-comment', text });
  const tagRe = /(@(?:param|return|returns|throws|exception|see|since|deprecated|author))\b(?:\s+([A-Za-z_$][\w$]*))?/g;
  for (let m: RegExpExecArray | null; (m = tagRe.exec(text)) !== null; ) {
    const tFrom = baseFrom + m.index;
    out.push({ from: tFrom, to: tFrom + m[1].length, role: 'doc-tag', text: m[1] });
    if (m[2] && m[1] === '@param') { const pIdx = m[0].lastIndexOf(m[2]); out.push({ from: tFrom + pIdx, to: tFrom + pIdx + m[2].length, role: 'doc-param', text: m[2] }); }
  }
}

export function classifySemanticTokens(
  docText: string, lang: SemanticLangKind, visibleRanges?: readonly { from: number; to: number }[], extraUnresolved?: ReadonlySet<string>, jPrefs?: JavaDiagPrefs,
): SemanticTokenSpan[] {
  if (!docText || lang === 'csv' || lang === 'other') return [];
  const ranges = visibleRanges ?? [{ from: 0, to: docText.length }], model = buildSymbolModel(docText, lang, extraUnresolved, jPrefs);
  const declStarts = new Set(model.funcDecls.map((d) => d.nameFrom)), out: SemanticTokenSpan[] = [];
  const paramsAt = (pos: number): Set<string> | null => {
    for (let i = model.paramScopes.length - 1; i >= 0; i--) if (pos >= model.paramScopes[i].from && pos <= model.paramScopes[i].to) return model.paramScopes[i].params;
    return null;
  };

  for (const { from: rawFrom, to: rawTo } of ranges) {
    const from = Math.max(0, rawFrom), to = Math.min(docText.length, rawTo);
    let i = from;
    while (i < to) {
      const ch = docText[i];
      if (ch === '"' || ch === "'" || ch === '`') {
        let end = i + 1;
        while (end < to) { if (docText[end] === '\\' && ch !== '`') { end += 2; continue; } if (docText[end] === ch) { end++; break; } end++; }
        if (lang === 'json' && ch === '"') {
          let a = end; while (a < to && (docText[a] === ' ' || docText[a] === '\t')) a++;
          if (docText[a] === ':') out.push({ from: i, to: end, role: 'field', text: docText.slice(i, end) });
        } else if (lang === 'shell' && ch === '"') {
          const sub = docText.slice(i, end), vr = /\$(?:[A-Za-z_]\w*|\{[^}"\n]+\})/g;
          let vm: RegExpExecArray | null;
          while ((vm = vr.exec(sub)) !== null) out.push({ from: i + vm.index, to: i + vm.index + vm[0].length, role: 'param', text: vm[0] });
        }
        i = end; continue;
      }
      if (ch === '/' && docText[i + 1] === '*') {
        const cl = docText.indexOf('*/', i + 2), end = cl >= 0 ? Math.min(to, cl + 2) : to;
        if (docText[i + 2] === '*' && docText[i + 3] !== '/') classifyDocComment(docText.slice(i, end), i, out);
        i = end; continue;
      }
      if (ch === '/' && docText[i + 1] === '/') {
        const nl = docText.indexOf('\n', i + 2), end = nl >= 0 ? Math.min(to, nl) : to;
        if (docText[i + 2] === '/' && lang === 'rust') classifyDocComment(docText.slice(i, end), i, out);
        i = end; continue;
      }
      if (ch === '#' && (lang === 'python' || lang === 'shell' || lang === 'rust')) {
        if (lang === 'rust' && (docText[i + 1] === '[' || (docText[i + 1] === '!' && docText[i + 2] === '['))) {
          const rb = docText.indexOf(']', i + 2), end = rb >= 0 ? Math.min(to, rb + 1) : i + 2;
          out.push({ from: i, to: end, role: 'annotation', text: docText.slice(i, end) });
          i = end; continue;
        }
        const nl = docText.indexOf('\n', i + 1); i = nl >= 0 ? Math.min(to, nl) : to; continue;
      }
      if (ch === '@' && (lang === 'java' || lang === 'ts' || lang === 'python')) {
        const m = /^@[A-Za-z_$][\w$.]*/.exec(docText.slice(i, Math.min(to, i + 64)));
        if (m) { out.push({ from: i, to: i + m[0].length, role: 'annotation', text: m[0] }); i += m[0].length; continue; }
      }
      if (ch === '$' && lang === 'shell') {
        const m = /^\$(?:\{[A-Za-z_]\w*\}|[A-Za-z_]\w*)/.exec(docText.slice(i, Math.min(to, i + 64)));
        if (m) { out.push({ from: i, to: i + m[0].length, role: 'field', text: m[0] }); i += m[0].length; continue; }
      }
      if (/[A-Za-z_$]/.test(ch)) {
        let end = i + 1; while (end < to && /[\w$]/.test(docText[end])) end++;
        const word = docText.slice(i, end);
        let prev = i - 1; while (prev >= 0 && (docText[prev] === ' ' || docText[prev] === '\t')) prev--;
        const prevCh = prev >= 0 ? docText[prev] : '';
        const isAfterDot = prevCh === '.' || (prevCh === ':' && docText[prev - 1] === ':');
        let next = end; while (next < docText.length && (docText[next] === ' ' || docText[next] === '\t')) next++;
        let callPos = next;
        if (docText[callPos] === '<' && (lang === 'ts' || lang === 'java' || lang === 'rust')) {
          const gt = docText.indexOf('>', callPos + 1);
          if (gt > callPos && gt - callPos < 64 && docText[gt + 1] === '(') callPos = gt + 1;
        }
        const isBeforeParen = docText[callPos] === '(';
        const isBeforeDot = docText[next] === '.' || (docText[next] === ':' && docText[next + 1] === ':');
        const scopeP = paramsAt(i);
        let role: SemanticRole | null = null;
        if (model.unresolvedIdents.has(word)) role = 'unresolved';
        else if (!isAfterDot && CONTROL_KW.has(word)) role = 'control';
        else if (!isAfterDot && GENERAL_KW.has(word)) role = 'keyword';
        else if (declStarts.has(i)) role = 'func-decl';
        else if (isAfterDot) {
          if (isBeforeParen) role = 'func-call';
          else if (lang === 'go' && /^[A-Z]/.test(word) && (docText[next] === '{' || (prev >= 2 && docText[prev - 2] === '*'))) role = 'type';
          else role = 'field';
        } else if (isBeforeDot) {
          role = scopeP?.has(word) ? 'param' : model.pkgVars.has(word) ? 'field' : (lang === 'java' || lang === 'rust') && /^[A-Z][a-zA-Z0-9]*$/.test(word) ? 'type' : 'fg';
        } else if (isBeforeParen) {
          role = /\bnew\s+$/.test(docText.slice(Math.max(0, i - 8), i)) && /^[A-Z]/.test(word) ? 'type' : 'func-call';
        } else if (scopeP?.has(word)) role = 'param';
        else if (model.pkgVars.has(word)) role = 'field';
        else if (model.typeNames.has(word)) role = 'type';
        else if (/^[A-Z][A-Z0-9_]{1,}$/.test(word)) role = 'field';
        else if (/^[A-Z][A-Za-z0-9_]*$/.test(word) && lang !== 'shell' && lang !== 'json') role = 'type';
        if (role) out.push({ from: i, to: end, role, text: word });
        i = end; continue;
      }
      i++;
    }
  }
  return out;
}

export function computeRainbowBrackets(docText: string, visibleRanges?: readonly { from: number; to: number }[], cursorPos?: number | null): RainbowBracketSpan[] {
  const ranges = visibleRanges ?? [{ from: 0, to: docText.length }];
  const limit = Math.min(docText.length, ranges.reduce((m, r) => Math.max(m, r.to), 0), 262144);
  const inVis = (p: number) => ranges.some((r) => p >= r.from && p < r.to);
  const stack: Array<{ ch: string; pos: number; level: 1 | 2 | 3 | 4 }> = [];
  const pairs: Array<{ open: number; close: number; level: 1 | 2 | 3 | 4 }> = [];
  const unmatched: number[] = [];
  for (let i = 0; i < limit; i++) {
    const ch = docText[i];
    if (ch === '"' || ch === "'" || ch === '`') {
      const q = ch; i++;
      while (i < limit) { if (docText[i] === '\\' && q !== '`') { i += 2; continue; } if (docText[i] === q) break; i++; }
      continue;
    }
    if (ch === '/' && docText[i + 1] === '/') { const nl = docText.indexOf('\n', i + 2); i = nl >= 0 ? nl : limit; continue; }
    if (ch === '/' && docText[i + 1] === '*') { const cl = docText.indexOf('*/', i + 2); i = cl >= 0 ? cl + 1 : limit; continue; }
    if (ch === '(' || ch === '{' || ch === '[') stack.push({ ch, pos: i, level: ((stack.length % 4) + 1) as 1 | 2 | 3 | 4 });
    else if (ch === ')' || ch === '}' || ch === ']') {
      const want = ch === ')' ? '(' : ch === '}' ? '{' : '[';
      const top = stack[stack.length - 1];
      if (top && top.ch === want) { stack.pop(); pairs.push({ open: top.pos, close: i, level: top.level }); }
      else unmatched.push(i);
    }
  }
  const active = cursorPos == null ? null : pairs.find((p) => p.open === cursorPos || p.open === cursorPos - 1 || p.close === cursorPos || p.close === cursorPos - 1);
  const out: RainbowBracketSpan[] = [];
  for (const p of pairs) {
    if (inVis(p.open)) out.push({ from: p.open, to: p.open + 1, level: p.level, active: active === p });
    if (inVis(p.close)) out.push({ from: p.close, to: p.close + 1, level: p.level, active: active === p });
  }
  for (const s of stack) if (inVis(s.pos)) out.push({ from: s.pos, to: s.pos + 1, level: s.level, active: false });
  for (const u of unmatched) if (inVis(u)) out.push({ from: u, to: u + 1, level: 'unmatched', active: false });
  return out.sort((a, b) => a.from - b.from);
}

export function analyzeJavaDiagnostics(docText: string, prefs?: JavaDiagPrefs): { diagnostics: ClientIdeDiagnostic[]; unresolvedIdents: Set<string> } {
  const checkUnres = prefs?.javaUnresolvedImports !== false, checkUnused = prefs?.unusedImports !== false, lombok = prefs?.lombokSupport !== false;
  const lines = docText.split(/\r?\n/), diagnostics: ClientIdeDiagnostic[] = [], unresolvedIdents = new Set<string>();
  const seenImports = new Set<string>(), bodyChunks: string[] = [];
  const importEntries: Array<{ line: number; col: number; endCol: number; path: string; ident: string }> = [];
  for (let idx = 0; idx < lines.length; idx++) {
    const raw = lines[idx], trimmed = raw.trim(), ln = idx + 1;
    if (trimmed.startsWith('package ')) continue;
    const mImp = /^\s*import\s+(?:static\s+)?([\w.*$]+)\s*;\s*$/.exec(raw);
    if (mImp) {
      const impPath = mImp[1], col = raw.indexOf('import') + 1, endCol = raw.length + 1;
      const segs = impPath.split('.'), lastSeg = segs[segs.length - 1] ?? '';
      if (segs.length < 2 || BOGUS_JAVA_PREFIXES.some((p) => impPath.startsWith(p)) || (!lombok && impPath.startsWith('lombok.'))) {
        if (checkUnres) {
          if (lastSeg && lastSeg !== '*') unresolvedIdents.add(lastSeg);
          diagnostics.push({ line: ln, column: col, endLine: ln, endColumn: endCol, severity: 'error', source: 'java-imports', code: 'unresolved-import', message: `Unresolved import '${impPath}'`, quickFixes: [{ title: `Remove unresolved import '${impPath}'`, line: ln, endLine: ln, deleteLine: true }] });
        }
        continue;
      }
      if (seenImports.has(impPath)) {
        diagnostics.push({ line: ln, column: col, endLine: ln, endColumn: endCol, severity: 'warning', source: 'java-imports', code: 'duplicate-import', message: `Duplicate import '${impPath}'`, quickFixes: [{ title: `Remove duplicate import '${impPath}'`, line: ln, endLine: ln, deleteLine: true }] });
        continue;
      }
      seenImports.add(impPath);
      if (lastSeg && lastSeg !== '*') importEntries.push({ line: ln, col, endCol, path: impPath, ident: lastSeg });
      continue;
    }
    bodyChunks.push(raw);
  }
  if (checkUnused) {
    const bodyText = bodyChunks.join('\n');
    for (const imp of importEntries) {
      if (!new RegExp(`\\b${imp.ident.replace(/\$/g, '\\$')}\\b`).test(bodyText)) {
        diagnostics.push({ line: imp.line, column: imp.col, endLine: imp.line, endColumn: imp.endCol, severity: 'warning', source: 'java-imports', code: 'unused-import', message: `Unused import '${imp.path}'`, quickFixes: [{ title: `Remove unused import '${imp.path}'`, line: imp.line, endLine: imp.line, deleteLine: true }] });
      }
    }
  }
  return { diagnostics, unresolvedIdents };
}

export function computeImportFoldRange(docText: string, lang: SemanticLangKind, errorLines?: ReadonlySet<number>): { from: number; to: number; startLine: number; endLine: number } | null {
  const lines = docText.split('\n');
  let offset = 0, startLine = -1, endLine = -1, foldFrom = -1, foldTo = -1, inGoParens = false;
  for (let i = 0; i < Math.min(lines.length, 250); i++) {
    const line = lines[i], trimmed = line.trim(), ln = i + 1, lStart = offset, lEnd = offset + line.length;
    offset = lEnd + 1;
    if (lang === 'go') {
      if (startLine < 0 && /^import\s*\(/.test(trimmed)) { startLine = ln; inGoParens = true; foldFrom = lStart + line.indexOf('(') + 1; continue; }
      if (inGoParens) { if (trimmed.startsWith(')')) { endLine = ln; foldTo = lStart; break; } continue; }
    } else if (lang === 'java' || lang === 'ts' || lang === 'python' || lang === 'rust') {
      const isImp = lang === 'rust' ? /^use\s+/.test(trimmed) : /^(?:import|from)\s+/.test(trimmed);
      if (isImp) { if (startLine < 0) { startLine = ln; foldFrom = lEnd; } endLine = ln; foldTo = lEnd; }
      else if (startLine > 0 && trimmed !== '' && !trimmed.startsWith('//') && !trimmed.startsWith('#')) break;
    }
  }
  if (startLine < 0 || endLine <= startLine || foldTo <= foldFrom) return null;
  if (errorLines) for (let ln = startLine; ln <= endLine; ln++) if (errorLines.has(ln)) return null;
  return { from: foldFrom, to: foldTo, startLine, endLine };
}

class InlayHintWidget extends WidgetType {
  label: string;
  constructor(label: string) { super(); this.label = label; }
  eq(o: InlayHintWidget): boolean { return this.label === o.label; }
  toDOM(): HTMLElement { const el = document.createElement('span'); el.className = 'cm-ide-inlay-hint'; el.textContent = `${this.label}:`; return el; }
}

class CodeVisionWidget extends WidgetType {
  count: number; line: number; symbol: string; onUsages: ((line: number, symbol: string) => void) | undefined;
  constructor(count: number, line: number, symbol: string, onUsages?: (line: number, symbol: string) => void) { super(); this.count = count; this.line = line; this.symbol = symbol; this.onUsages = onUsages; }
  eq(o: CodeVisionWidget): boolean { return this.count === o.count && this.line === o.line && this.symbol === o.symbol; }
  toDOM(): HTMLElement {
    const el = document.createElement('span'); el.className = 'cm-ide-codevision'; el.dataset.testid = 'ide-codevision-usages';
    el.textContent = `${this.count} ${this.count === 1 ? 'usage' : 'usages'}`;
    if (this.onUsages) el.addEventListener('mousedown', (e) => { e.preventDefault(); e.stopPropagation(); this.onUsages?.(this.line, this.symbol); });
    return el;
  }
}

export function applyClientQuickFix(view: EditorView, fix: ClientDiagQuickFix): void {
  const doc = view.state.doc;
  if (fix.edits?.length) {
    view.dispatch({ changes: fix.edits.map((ed) => { const s = doc.line(Math.max(1, Math.min(doc.lines, ed.start_line))), e = doc.line(Math.max(1, Math.min(doc.lines, ed.end_line))), from = Math.min(doc.length, s.from + Math.max(0, (ed.start_col || 1) - 1)); return { from, to: ed.end_line > doc.lines ? doc.length : Math.max(from, Math.min(doc.length, e.from + Math.max(0, (ed.end_col || 1) - 1))), insert: ed.new_text }; }), userEvent: 'input.quickfix' });
    return;
  }
  const sLn = Math.max(1, Math.min(doc.lines, fix.line)), endLn = doc.line(Math.max(sLn, Math.min(doc.lines, fix.endLine || sLn)));
  view.dispatch({ changes: { from: doc.line(sLn).from, to: fix.deleteLine ? Math.min(doc.length, endLn.to + 1) : endLn.to, insert: fix.deleteLine ? '' : (fix.replacement ?? '') }, userEvent: 'input.quickfix' });
}

const setLineQuickFixesEffect = StateEffect.define<Map<number, ClientDiagQuickFix[]>>();
const setServerUnresolvedEffect = StateEffect.define<Set<string>>();
const lineQuickFixesField = StateField.define<Map<number, ClientDiagQuickFix[]>>({ create: () => new Map(), update: (v, tr) => { for (const e of tr.effects) if (e.is(setLineQuickFixesEffect)) return e.value; return v; } });
const serverUnresolvedField = StateField.define<Set<string>>({ create: () => new Set(), update: (v, tr) => { for (const e of tr.effects) if (e.is(setServerUnresolvedEffect)) return e.value; return v; } });
export function getIdeQuickFixesAtLine(state: EditorState, line: number): ClientDiagQuickFix[] { return state.field(lineQuickFixesField, false)?.get(line) ?? []; }

export interface CreateIdeSemanticOptions {
  filePath: string; language?: string; root?: string; connectionId?: string | null; projectFolders?: string[];
  getPreferences: () => IdeEditorPreferences;
  onDiagnostics?: ((diags: IdeDiagnostic[]) => void) | null;
  onFindUsages?: ((ctx: { line: number; column: number; symbol?: string }) => void) | null;
}

export function createIdeSemanticExtensions(opts: CreateIdeSemanticOptions): Extension {
  const lang = detectSemanticLang(opts.filePath, opts.language);
  const importFoldExt = foldService.of((state, lineStart) => {
    const ln = state.doc.lineAt(lineStart).number;
    const r = ln <= 40 ? computeImportFoldRange(state.doc.toString(), lang) : null;
    return r && r.startLine === ln ? { from: r.from, to: r.to } : null;
  });
  const vpPlugin = ViewPlugin.fromClass(class {
    decorations: DecorationSet = Decoration.none;
    private timer: ReturnType<typeof setTimeout> | null = null;
    private abort: AbortController | null = null;
    private didAutoFold = false;
    private hadSemanticDiags = false;
    constructor(view: EditorView) {
      this.decorations = this.compute(view);
      queueMicrotask(() => { if (view.dom.isConnected) { this.maybeAutoFoldImports(view); this.scheduleDiagnostics(view, 10); } });
    }
    update(u: ViewUpdate) {
      if (u.docChanged || u.viewportChanged || u.selectionSet || u.transactions.some((t) => t.effects.some((e) => e.is(setServerUnresolvedEffect)))) this.decorations = this.compute(u.view);
      if (u.docChanged) this.scheduleDiagnostics(u.view, 250);
    }
    destroy() { if (this.timer) clearTimeout(this.timer); this.abort?.abort(); }
    maybeAutoFoldImports(view: EditorView) {
      const prefs = opts.getPreferences();
      if (this.didAutoFold || !prefs.autoFoldImports) return;
      this.didAutoFold = true;
      const docText = view.state.doc.toString();
      const errLines = new Set<number>(lang === 'java' ? analyzeJavaDiagnostics(docText, prefs).diagnostics.filter((d) => d.severity === 'error').map((d) => d.line) : []);
      const r = computeImportFoldRange(docText, lang, errLines);
      if (r && r.to <= view.state.doc.length) view.dispatch({ effects: foldEffect.of({ from: r.from, to: r.to }) });
    }
    scheduleDiagnostics(view: EditorView, delayMs: number) {
      if (this.timer) clearTimeout(this.timer);
      this.abort?.abort();
      if (lang !== 'csv' && lang !== 'other' && opts.filePath) this.timer = setTimeout(() => void this.runDiagnostics(view), delayMs);
    }
    async runDiagnostics(view: EditorView) {
      if (!view.dom.isConnected) return;
      const prefs = opts.getPreferences(), docText = view.state.doc.toString(), jRes = lang === 'java' ? analyzeJavaDiagnostics(docText, prefs) : null;
      const clientDiags: ClientIdeDiagnostic[] = jRes ? [...jRes.diagnostics] : [];
      const unresolved = new Set<string>(jRes ? jRes.unresolvedIdents : []);
      if (opts.root && (lang === 'java' || lang === 'go' || lang === 'python' || lang === 'shell' || lang === 'json')) {
        const ctrl = new AbortController(); this.abort = ctrl;
        try {
          const { api } = await import('../api/client.ts');
          type RawDiag = { line: number; col?: number; column?: number; end_line?: number; end_col?: number; severity?: string; code?: string; source?: string; message: string; quick_fixes?: Array<{ title: string; line: number; end_line?: number; replacement?: string; delete_line?: boolean; edits?: ClientDiagQuickFix['edits'] }> };
          const resp = await api.post<{ diagnostics?: RawDiag[] }>('/ide/lsp/diagnostics', { target: { kind: opts.connectionId ? 'ssh' : 'local', root: opts.root, connection_id: opts.connectionId ?? null, additional_roots: opts.projectFolders }, path: opts.filePath, content: docText, language: lang, project_folders: opts.projectFolders }, ctrl.signal);
          for (const sd of resp.diagnostics ?? []) {
            if (prefs.unusedImports === false && (sd.code === 'unused-import' || /unused import/i.test(sd.message))) continue;
            if (lang === 'java' && prefs.javaUnresolvedImports === false && (sd.code === 'unresolved-import' || /unresolved import|package .* does not exist/i.test(sd.message))) continue;
            if (lang === 'java' && prefs.lombokSupport !== false && /package lombok\b.*does not exist|cannot find symbol.*\bvariable log\b/i.test(sd.message)) continue;
            if (!clientDiags.some((c) => c.line === sd.line && (c.message === sd.message || (Boolean(c.code) && c.code === sd.code)))) {
              clientDiags.push({ line: sd.line, column: sd.col ?? sd.column ?? 1, endLine: sd.end_line ?? sd.line, endColumn: sd.end_col ?? 200, severity: sd.severity === 'error' ? 'error' : sd.severity === 'warning' ? 'warning' : 'info', message: sd.message, source: sd.source ?? 'lsp', code: sd.code, quickFixes: (sd.quick_fixes ?? []).map((qf) => ({ title: qf.title, line: qf.line, endLine: qf.end_line ?? qf.line, replacement: qf.replacement, deleteLine: qf.delete_line, edits: qf.edits })) });
            }
            if (sd.severity === 'error' && (lang !== 'java' || prefs.javaUnresolvedImports !== false) && sd.line >= 1 && sd.line <= view.state.doc.lines) {
              const mImp = /^\s*import\s+(?:static\s+)?[\w.$]*\.([A-Za-z_$][\w$]*)\s*;/.exec(view.state.doc.line(sd.line).text);
              if (mImp) unresolved.add(mImp[1]);
            }
          }
        } catch { /* fallback to instant client diagnostics */ }
      }
      if (!view.dom.isConnected || view.state.doc.toString() !== docText) return;
      const doc = view.state.doc, fixMap = new Map<number, ClientDiagQuickFix[]>(), errorLines = new Set<number>(), cmDiags: Diagnostic[] = [], ideDiags: IdeDiagnostic[] = [];
      for (const d of clientDiags) {
        if (d.line < 1 || d.line > doc.lines) continue;
        if (d.severity === 'error') errorLines.add(d.line);
        const lnObj = doc.line(d.line), endLnObj = doc.line(Math.max(d.line, Math.min(doc.lines, d.endLine)));
        const from = Math.min(lnObj.to, lnObj.from + Math.max(0, d.column - 1)), to = Math.max(from + 1, Math.min(endLnObj.to, endLnObj.from + Math.max(0, d.endColumn - 1)));
        if (d.quickFixes.length) fixMap.set(d.line, [...(fixMap.get(d.line) ?? []), ...d.quickFixes]);
        const actions: LintAction[] = d.quickFixes.map((qf) => ({ name: qf.title, apply: (v) => applyClientQuickFix(v, qf) }));
        cmDiags.push({ from: Math.min(from, doc.length), to: Math.min(to, doc.length), severity: d.severity, message: d.message, source: d.source, actions });
        ideDiags.push({ path: opts.filePath, line: d.line, column: d.column, end_line: d.endLine, end_column: d.endColumn, severity: d.severity, message: d.message, source: d.source, code: d.code ?? null, quick_fixes: d.quickFixes.length ? d.quickFixes.map((qf, idx) => ({ id: `qf-${d.line}-${idx}`, title: qf.title, kind: 'quickfix', is_preferred: idx === 0, edits: qf.edits?.length ? qf.edits.map((e) => ({ ...e, path: e.path || opts.filePath })) : [{ path: opts.filePath, start_line: qf.line, start_col: 1, end_line: qf.deleteLine ? Math.min(doc.lines + 1, qf.endLine + 1) : qf.endLine, end_col: qf.deleteLine && qf.endLine < doc.lines ? 1 : doc.line(Math.min(doc.lines, qf.endLine)).length + 1, new_text: qf.deleteLine ? '' : (qf.replacement ?? '') }] })) : undefined });
      }
      if (errorLines.size) foldedRanges(view.state).between(0, Math.min(doc.length, 4096), (f, t) => { for (let ln = doc.lineAt(f).number; ln <= doc.lineAt(t).number; ln++) if (errorLines.has(ln)) { view.dispatch({ effects: unfoldEffect.of({ from: f, to: t }) }); break; } });
      if (ideDiags.length > 0 || this.hadSemanticDiags) {
        this.hadSemanticDiags = ideDiags.length > 0;
        const trSpec = setDiagnostics(view.state, cmDiags), prevEf = Array.isArray(trSpec.effects) ? trSpec.effects : trSpec.effects ? [trSpec.effects] : [];
        view.dispatch({ ...trSpec, effects: [...prevEf, setLineQuickFixesEffect.of(fixMap), setServerUnresolvedEffect.of(unresolved)] });
        opts.onDiagnostics?.(ideDiags);
      } else {
        view.dispatch({ effects: [setLineQuickFixesEffect.of(fixMap), setServerUnresolvedEffect.of(unresolved)] });
      }
    }
    compute(view: EditorView): DecorationSet {
      const prefs = opts.getPreferences(), docText = view.state.doc.toString(), vis = view.visibleRanges, items: Range<Decoration>[] = [], extraUnres = view.state.field(serverUnresolvedField, false);
      for (const tok of classifySemanticTokens(docText, lang, vis, extraUnres, prefs)) if (tok.to > tok.from) items.push(Decoration.mark({ class: `cm-syn-${tok.role}` }).range(tok.from, tok.to));
      if (prefs.rainbowBrackets && lang !== 'csv') for (const rb of computeRainbowBrackets(docText, vis, view.state.selection.main.head)) items.push(Decoration.mark({ class: rb.level === 'unmatched' ? 'cm-syn-rb-unmatched' : `cm-syn-rb-${rb.level}${rb.active ? ' cm-syn-rb-active' : ''}` }).range(rb.from, rb.to));
      if ((prefs.inlayHints || prefs.codeVision) && docText.length <= 131072 && lang !== 'csv' && lang !== 'json') {
        const model = buildSymbolModel(docText, lang, extraUnres, prefs);
        if (prefs.inlayHints && model.funcParams.size) {
          let hintCount = 0;
          for (const { from, to } of vis) {
            const slice = docText.slice(from, to), callRe = /\b([A-Za-z_$][\w$]*)\s*\(([^()\n]{1,120})\)/g;
            let m: RegExpExecArray | null;
            while ((m = callRe.exec(slice)) !== null && hintCount < 24) {
              const pNames = model.funcParams.get(m[1]);
              if (!pNames || model.funcDecls.some((d) => d.nameFrom === from + m!.index)) continue;
              const argsStart = from + m.index + m[0].indexOf('(') + 1;
              let cursor = 0;
              for (const [aIdx, rawArg] of m[2].split(',').entries()) {
                if (aIdx >= pNames.length) break;
                const trimmedArg = rawArg.trim(), pName = pNames[aIdx];
                if (trimmedArg && trimmedArg !== pName && !trimmedArg.startsWith(`${pName}:`) && !trimmedArg.startsWith(`${pName}=`)) { items.push(Decoration.widget({ widget: new InlayHintWidget(pName), side: -1 }).range(argsStart + cursor + (rawArg.length - rawArg.trimStart().length))); hintCount++; }
                cursor += rawArg.length + 1;
              }
            }
          }
        }
        if (prefs.codeVision) {
          let cvCount = 0;
          for (const decl of model.funcDecls) {
            if (cvCount >= 16 || !vis.some((r) => decl.lineEnd >= r.from && decl.lineEnd <= r.to)) continue;
            const usages = Math.max(0, (docText.match(new RegExp(`\\b${decl.name.replace(/\$/g, '\\$')}\\b`, 'g'))?.length ?? 1) - 1);
            items.push(Decoration.widget({ widget: new CodeVisionWidget(usages, decl.line, decl.name, opts.onFindUsages ? (ln, sym) => opts.onFindUsages?.({ line: ln, column: 1, symbol: sym }) : undefined), side: 1 }).range(decl.lineEnd));
            cvCount++;
          }
        }
      }
      return items.length ? Decoration.set(items, true) : Decoration.none;
    }
  }, { decorations: (v) => v.decorations });
  return [lineQuickFixesField, serverUnresolvedField, importFoldExt, vpPlugin];
}
