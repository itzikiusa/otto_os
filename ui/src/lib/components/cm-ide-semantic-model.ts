// Pure symbol model, Go/Java import analyzers, rainbow-bracket index, and
// per-`Text` cache (`WeakMap<Text, CachedSemanticDoc>`).
// Keying by immutable CodeMirror `Text` guarantees zero `doc.toString()` or
// full-document rescans on cursor movement or viewport scroll.

import type { Text } from '@codemirror/state';

export type SemanticLangKind =
  | 'go' | 'java' | 'ts' | 'rust' | 'python' | 'shell' | 'json' | 'csv' | 'other';

export type SemanticRole =
  | 'fg' | 'keyword' | 'control' | 'func-decl' | 'func-call' | 'field'
  | 'type' | 'annotation' | 'param' | 'doc-comment' | 'doc-tag' | 'doc-param' | 'unresolved' | 'unused';

export interface SemanticTokenSpan { from: number; to: number; role: SemanticRole; text: string }
export interface RainbowBracketSpan { from: number; to: number; level: 1 | 2 | 3 | 4 | 'unmatched'; active: boolean }

export interface ClientDiagQuickFix {
  title: string;
  line: number;
  endLine: number;
  deleteLine?: boolean;
  replacement?: string;
  edits?: Array<{ path?: string; start_line: number; start_col: number; end_line: number; end_col: number; new_text: string }>;
}

export interface ClientIdeDiagnostic {
  line: number;
  column: number;
  endLine: number;
  endColumn: number;
  severity: 'error' | 'warning' | 'info';
  message: string;
  source: string;
  code?: string;
  quickFixes: ClientDiagQuickFix[];
}

export interface JavaDiagPrefs {
  javaUnresolvedImports?: boolean;
  lombokSupport?: boolean;
  unusedImports?: boolean;
}

export interface FileSymbolModel {
  pkgVars: Set<string>;
  typeNames: Set<string>;
  importedPkgs: Set<string>;
  funcParams: Map<string, string[]>;
  funcDecls: Array<{ name: string; line: number; nameFrom: number; nameTo: number; lineEnd: number }>;
  paramScopes: Array<{ from: number; to: number; params: Set<string> }>;
  declStarts: Set<number>;
  unresolvedIdents: Set<string>;
  unusedRanges: Array<{ from: number; to: number }>;
}

interface BracketPair { open: number; close: number; level: 1 | 2 | 3 | 4 }

export interface CachedBracketIndex {
  pairs: BracketPair[];
  posToPair: Map<number, BracketPair>;
  stack: Array<{ ch: string; pos: number; level: 1 | 2 | 3 | 4 }>;
  unmatched: number[];
}

export interface CachedSemanticDoc {
  docText: string;
  modelKey: string;
  model: FileSymbolModel | null;
  identCounts: Map<string, number> | null;
  brackets: CachedBracketIndex | null;
}

const semanticDocCache = new WeakMap<Text, CachedSemanticDoc>();

export function detectSemanticLang(filePath: string, language?: string): SemanticLangKind {
  if (filePath.startsWith('jdt://')) return 'java';
  const clean = filePath.split('?')[0].toLowerCase();
  const ext = clean.slice(Math.max(0, clean.lastIndexOf('/'))).split('.').pop() ?? '';
  if (ext === 'go' || language === 'go') return 'go';
  if (ext === 'java' || ext === 'class' || language === 'java') return 'java';
  if (['ts', 'tsx', 'js', 'jsx', 'mjs', 'cjs', 'mts', 'cts'].includes(ext) || language === 'typescript' || language === 'javascript') {
    return 'ts';
  }
  if (ext === 'rs' || language === 'rust') return 'rust';
  if (ext === 'py' || ext === 'pyi' || language === 'python') return 'python';
  if (['sh', 'bash', 'zsh', 'bats'].includes(ext) || language === 'shell' || language === 'shellscript') return 'shell';
  if (['json', 'jsonc', 'jsonl', 'ndjson'].includes(ext) || language === 'json' || language === 'jsonl') return 'json';
  return ext === 'csv' || ext === 'tsv' || language === 'csv' || language === 'tsv' ? 'csv' : 'other';
}

export const CONTROL_KW = new Set(
  'if else for range return switch case default select defer go break continue fallthrough try catch finally throw throws while do match loop await async yield elif except raise with'.split(' '),
);

export const GO_BUILTIN_TYPES = new Set(
  'string int int8 int16 int32 int64 uint uint8 uint16 uint32 uint64 uintptr float32 float64 complex64 complex128 bool byte rune error any comparable'.split(' '),
);

export const GENERAL_KW = new Set(
  (
    'package import func var const type struct interface map chan ' +
    'true false nil iota public private protected static final abstract synchronized volatile ' +
    'transient native sealed non-sealed permits class enum record extends implements new instanceof ' +
    'void boolean short long double float char null this super export from let function typeof keyof ' +
    'readonly as in of undefined fn mut pub impl trait mod use where crate self Self unsafe dyn move ' +
    'ref def None True False and or not is lambda pass global nonlocal fi then done esac'
  ).split(' '),
);

const BOGUS_JAVA_PREFIXES = [
  'com.nonexistent.', 'org.nonexistent.', 'net.nonexistent.',
  'invalid.', 'bogus.', 'fake.', 'unknown.', 'does.not.exist.', 'nonexistent.',
];

export function skipLiteralOrComment(src: string, idx: number, limit: number): number {
  const ch = src[idx];
  if (ch === '"' || ch === "'" || ch === '`') {
    let p = idx + 1;
    while (p < limit) {
      if (src[p] === '\\' && ch !== '`') {
        p += 2;
        continue;
      }
      if (src[p] === ch) return p;
      p++;
    }
    return limit;
  }
  if (ch === '/' && src[idx + 1] === '/') {
    const nl = src.indexOf('\n', idx + 2);
    return nl >= 0 && nl < limit ? nl : limit;
  }
  if (ch === '/' && src[idx + 1] === '*') {
    const cl = src.indexOf('*/', idx + 2);
    return cl >= 0 && cl + 1 < limit ? cl + 1 : limit;
  }
  if (ch === '#') {
    const nl = src.indexOf('\n', idx + 1);
    return nl >= 0 && nl < limit ? nl : limit;
  }
  return idx;
}

function splitTopLevelCommas(raw: string): string[] {
  const parts: string[] = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < raw.length; i++) {
    const jumped = skipLiteralOrComment(raw, i, raw.length);
    if (jumped > i) {
      i = jumped;
      continue;
    }
    const c = raw[i];
    if (c === '(' || c === '[' || c === '{' || c === '<') {
      depth++;
    } else if (c === ')' || c === ']' || c === '}' || c === '>') {
      depth = Math.max(0, depth - 1);
    } else if (c === ',' && depth === 0) {
      parts.push(raw.slice(start, i));
      start = i + 1;
    }
  }
  parts.push(raw.slice(start));
  return parts;
}

function splitParamNames(rawParams: string, lang: SemanticLangKind): string[] {
  const stripped = rawParams
    .replace(/\/\*[\s\S]*?\*\//g, ' ')
    .replace(/\/\/[^\n]*/g, ' ')
    .replace(/#[^\n]*/g, ' ');
  const parts = splitTopLevelCommas(stripped);
  const names: string[] = [];
  for (let idx = 0; idx < parts.length; idx++) {
    const clean = parts[idx].replace(/=[^,]+$/, '').trim();
    if (!clean || clean === 'self' || clean === '&self' || clean === '&mut self' || clean === 'this') continue;
    if (lang === 'go') {
      const mTyped = /^([A-Za-z_]\w*(?:\s*,\s*[A-Za-z_]\w*)*)\s+(?:\.\.\.)?(?:<-chan\b|chan\b|func\b|map\[|struct\b|interface\b|[*[\]\w.])/.exec(clean);
      if (mTyped) {
        for (const id of mTyped[1].split(',')) {
          const t = id.trim();
          if (t && t !== '_' && !GENERAL_KW.has(t) && !GO_BUILTIN_TYPES.has(t)) names.push(t);
        }
        continue;
      }
      if (/^[A-Za-z_]\w*$/.test(clean) && clean !== '_' && !GO_BUILTIN_TYPES.has(clean) && !GENERAL_KW.has(clean)) {
        const hasLaterTyped = parts.slice(idx + 1).some((later) => /^[A-Za-z_]\w*\s+\S/.test(later.trim()));
        if (hasLaterTyped) names.push(clean);
      }
      continue;
    }
    const m = lang === 'java'
      ? /(?:final\s+|@\w+(?:\([^)]*\))?\s+)*[\w<>[\],.?]+\s*(?:\.\.\.)?\s+([A-Za-z_$][\w$]*)$/.exec(clean)
      : /^(?:pub\s+|private\s+|readonly\s+|mut\s+|\*\*{0,2})?([A-Za-z_$][\w$]*)\s*(?:\??\s*:|$)/.exec(clean);
    if (m && m[1] !== '_' && !GENERAL_KW.has(m[1])) names.push(m[1]);
  }
  return names;
}

function scanSignatureAndBodyScope(
  src: string,
  openParenPos: number,
  lang: SemanticLangKind,
): { paramsRaw: string; bodyEndPos: number } | null {
  const limit = Math.min(src.length, openParenPos + 65536);
  if (src[openParenPos] !== '(') return null;
  let pDepth = 1;
  let closeParenPos = -1;
  const sigLimit = Math.min(limit, openParenPos + 4096);
  for (let p = openParenPos + 1; p < sigLimit; p++) {
    const jumped = skipLiteralOrComment(src, p, sigLimit);
    if (jumped > p) {
      p = jumped;
      continue;
    }
    if (src[p] === '(') {
      pDepth++;
    } else if (src[p] === ')' && --pDepth === 0) {
      closeParenPos = p;
      break;
    }
  }
  if (closeParenPos < 0) return null;
  const paramsRaw = src.slice(openParenPos + 1, closeParenPos);

  if (lang === 'python') {
    const lineStart = src.lastIndexOf('\n', openParenPos) + 1;
    let baseIndent = 0;
    while (lineStart + baseIndent < src.length && (src[lineStart + baseIndent] === ' ' || src[lineStart + baseIndent] === '\t')) {
      baseIndent++;
    }
    const nl = src.indexOf('\n', closeParenPos);
    if (nl < 0) return { paramsRaw, bodyEndPos: src.length };
    for (let pos = nl + 1; pos < limit; ) {
      const nextNl = src.indexOf('\n', pos);
      const lnEnd = nextNl >= 0 ? nextNl : limit;
      const lnTxt = src.slice(pos, lnEnd);
      const trimmed = lnTxt.trim();
      if (trimmed && !trimmed.startsWith('#')) {
        let ind = 0;
        while (ind < lnTxt.length && (lnTxt[ind] === ' ' || lnTxt[ind] === '\t')) ind++;
        if (ind <= baseIndent) return { paramsRaw, bodyEndPos: Math.max(closeParenPos, pos - 1) };
      }
      if (nextNl < 0) break;
      pos = nextNl + 1;
    }
    return { paramsRaw, bodyEndPos: limit };
  }

  let bodyOpenPos = -1;
  const retLimit = Math.min(limit, closeParenPos + 1024);
  for (let p = closeParenPos + 1; p < retLimit; p++) {
    const jumped = skipLiteralOrComment(src, p, retLimit);
    if (jumped > p) {
      p = jumped;
      continue;
    }
    const c = src[p];
    if (c === ';') return { paramsRaw, bodyEndPos: closeParenPos };
    if (c === '(') {
      let rd = 1;
      for (p++; p < retLimit && rd > 0; p++) {
        const rj = skipLiteralOrComment(src, p, retLimit);
        if (rj > p) {
          p = rj;
          continue;
        }
        if (src[p] === '(') rd++;
        else if (src[p] === ')') rd--;
      }
      p--;
      continue;
    }
    if (c === '{') {
      const before = src.slice(Math.max(closeParenPos, p - 16), p).trimEnd();
      if (lang === 'go' && /(?:struct|interface)$/.test(before)) {
        let td = 1;
        for (p++; p < retLimit && td > 0; p++) {
          const tj = skipLiteralOrComment(src, p, retLimit);
          if (tj > p) {
            p = tj;
            continue;
          }
          if (src[p] === '{') td++;
          else if (src[p] === '}') td--;
        }
        p--;
        continue;
      }
      bodyOpenPos = p;
      break;
    }
  }
  if (bodyOpenPos < 0) return { paramsRaw, bodyEndPos: closeParenPos };

  let bDepth = 1;
  for (let p = bodyOpenPos + 1; p < limit; p++) {
    const jumped = skipLiteralOrComment(src, p, limit);
    if (jumped > p) {
      p = jumped;
      continue;
    }
    if (src[p] === '{') {
      bDepth++;
    } else if (src[p] === '}' && --bDepth === 0) {
      return { paramsRaw, bodyEndPos: p };
    }
  }
  return { paramsRaw, bodyEndPos: limit };
}

/** Infer Go package qualifier candidate(s) from an import path, handling `/v2+`, `.v3`, `go-foo`, and `foo-go`. */
export function inferGoImportPkgNames(importPath: string, alias?: string): { names: string[]; certain: boolean } {
  const cleanAlias = alias?.trim();
  if (cleanAlias === '_' || cleanAlias === '.') return { names: [], certain: true };
  if (cleanAlias && /^[A-Za-z_]\w*$/.test(cleanAlias)) return { names: [cleanAlias], certain: true };
  const raw = importPath.trim().replace(/^["`]|["`]$/g, '').trim();
  if (!raw || raw === 'C') return { names: [], certain: true };
  const segs = raw.split('/').filter(Boolean);
  if (!segs.length) return { names: [], certain: false };
  if (segs.length >= 2 && /^v\d+$/i.test(segs[segs.length - 1])) segs.pop();
  const last = (segs[segs.length - 1] ?? '').replace(/\.git$/i, '').replace(/\.v\d+(?:[-+.\w]*)?$/i, '');
  if (!last) return { names: [], certain: false };
  if (/^[A-Za-z_]\w*$/.test(last)) return { names: [last], certain: true };
  const goPrefix = /^go[-_.]([A-Za-z_]\w*)$/i.exec(last);
  if (goPrefix) return { names: [goPrefix[1], `go${goPrefix[1]}`], certain: true };
  const goSuffix = /^([A-Za-z_]\w*)[-_.](?:go)$/i.exec(last);
  if (goSuffix) return { names: [goSuffix[1], `${goSuffix[1]}go`], certain: true };
  const collapsed = last.replace(/[^A-Za-z0-9_]/g, '');
  const parts = last.split(/[-_.]+/).filter((s) => /^[A-Za-z_]\w*$/.test(s));
  const candidates = new Set<string>();
  if (collapsed && /^[A-Za-z_]\w*$/.test(collapsed)) candidates.add(collapsed);
  if (parts.length) candidates.add(parts[parts.length - 1]);
  return { names: [...candidates], certain: false };
}

export function analyzeGoImports(
  docText: string,
  prefs?: { unusedImports?: boolean },
): {
  diagnostics: ClientIdeDiagnostic[];
  importedPkgs: Set<string>;
  unusedRanges: Array<{ from: number; to: number }>;
  entries: Array<{ line: number; path: string; names: string[]; certain: boolean; used: boolean }>;
} {
  const importedPkgs = new Set<string>();
  const diagnostics: ClientIdeDiagnostic[] = [];
  const unusedRanges: Array<{ from: number; to: number }> = [];
  const entries: Array<{ line: number; path: string; names: string[]; certain: boolean; used: boolean }> = [];
  const lines = docText.split('\n');
  const rawImports: Array<{ line: number; from: number; to: number; col: number; endCol: number; alias?: string; path: string }> = [];
  let offset = 0;
  let inBlock = false;
  let maxImportEnd = 0;

  for (let i = 0; i < Math.min(lines.length, 300); i++) {
    const raw = lines[i];
    const trimmed = raw.replace(/\/\/.*$/, '').trim();
    const ln = i + 1;
    const lStart = offset;
    const lEnd = offset + raw.length;
    offset = lEnd + 1;
    if (!inBlock) {
      if (/^import\s*\(\s*$/.test(trimmed)) {
        inBlock = true;
        continue;
      }
      const mSingle = /^import\s+(?:([._]|[A-Za-z_]\w*)\s+)?(["`][^"`]+["`])/.exec(trimmed);
      if (mSingle) {
        const pIdx = raw.indexOf(mSingle[2]);
        const firstNonWs = Math.max(0, raw.search(/\S/));
        rawImports.push({ line: ln, from: lStart + firstNonWs, to: lEnd, col: Math.max(1, pIdx + 1), endCol: raw.length + 1, alias: mSingle[1], path: mSingle[2] });
        maxImportEnd = lEnd;
      }
      continue;
    }
    if (trimmed.startsWith(')')) {
      inBlock = false;
      maxImportEnd = lEnd;
      continue;
    }
    const mSpec = /^(?:([._]|[A-Za-z_]\w*)\s+)?(["`][^"`]+["`])/.exec(trimmed);
    if (mSpec) {
      const firstNonWs = Math.max(0, raw.search(/\S/));
      rawImports.push({ line: ln, from: lStart + firstNonWs, to: lEnd, col: firstNonWs + 1, endCol: raw.length + 1, alias: mSpec[1], path: mSpec[2] });
      maxImportEnd = lEnd;
    }
  }

  const usedQualifiers = new Set<string>();
  const bodyLimit = Math.min(docText.length, 262144);
  for (let p = maxImportEnd; p < bodyLimit; p++) {
    const jumped = skipLiteralOrComment(docText, p, bodyLimit);
    if (jumped > p) {
      p = jumped;
      continue;
    }
    if (/[A-Za-z_]/.test(docText[p])) {
      let e = p + 1;
      while (e < bodyLimit && /\w/.test(docText[e])) e++;
      let nx = e;
      while (nx < bodyLimit && (docText[nx] === ' ' || docText[nx] === '\t')) nx++;
      if (docText[nx] === '.') usedQualifiers.add(docText.slice(p, e));
      p = e - 1;
    }
  }

  const checkUnused = prefs?.unusedImports !== false;
  for (const imp of rawImports) {
    const cleanPath = imp.path.slice(1, -1);
    const { names, certain } = inferGoImportPkgNames(cleanPath, imp.alias);
    for (const n of names) importedPkgs.add(n);
    const used = names.length === 0 || names.some((n) => usedQualifiers.has(n));
    entries.push({ line: imp.line, path: cleanPath, names, certain, used });
    if (checkUnused && certain && names.length > 0 && !used) {
      unusedRanges.push({ from: imp.from, to: imp.to });
      diagnostics.push({
        line: imp.line, column: imp.col, endLine: imp.line, endColumn: imp.endCol,
        severity: 'warning', source: 'go-imports', code: 'unused-import',
        message: `Unused import "${cleanPath}"`,
        quickFixes: [{ title: `Remove unused import "${cleanPath}"`, line: imp.line, endLine: imp.line, deleteLine: true }],
      });
    }
  }
  return { diagnostics, importedPkgs, unusedRanges, entries };
}

export function buildSymbolModel(
  docText: string,
  lang: SemanticLangKind,
  extraUnresolved?: ReadonlySet<string>,
  jPrefs?: JavaDiagPrefs,
): FileSymbolModel {
  const sample = docText.length > 65536 ? docText.slice(0, 65536) : docText;
  const pkgVars = new Set<string>();
  const typeNames = new Set<string>();
  const importedPkgs = new Set<string>();
  const funcParams = new Map<string, string[]>();
  const funcDecls: FileSymbolModel['funcDecls'] = [];
  const paramScopes: FileSymbolModel['paramScopes'] = [];
  const unresolvedIdents = new Set<string>(extraUnresolved);
  const unusedRanges: Array<{ from: number; to: number }> = [];

  if (lang === 'java') {
    for (const id of analyzeJavaDiagnostics(sample, jPrefs).unresolvedIdents) unresolvedIdents.add(id);
    if ((jPrefs?.lombokSupport ?? true) && /@(?:Slf4j|Log4j2|Log|CommonsLog|JBossLog|Flogger)\b/.test(sample)) pkgVars.add('log');
  } else if (lang === 'go') {
    const goImp = analyzeGoImports(sample, jPrefs);
    for (const p of goImp.importedPkgs) importedPkgs.add(p);
    unusedRanges.push(...goImp.unusedRanges);
  }

  const registerFuncDecl = (fnName: string, lineIdx: number, line: string, lineStart: number, lineEnd: number, parenSearchFrom: number, extraParams?: string[]) => {
    const nIdx = line.indexOf(fnName, Math.max(0, parenSearchFrom - lineStart - fnName.length - 2));
    const nFrom = lineStart + Math.max(0, nIdx >= 0 ? nIdx : line.indexOf(fnName));
    funcDecls.push({ name: fnName, line: lineIdx + 1, nameFrom: nFrom, nameTo: nFrom + fnName.length, lineEnd });
    if (lang === 'shell') return;
    const openParenPos = sample.indexOf('(', parenSearchFrom);
    if (openParenPos < 0 || openParenPos > lineEnd + 256) return;
    const scoped = scanSignatureAndBodyScope(sample, openParenPos, lang);
    if (!scoped) return;
    const pList = splitParamNames(scoped.paramsRaw, lang);
    if (pList.length) funcParams.set(fnName, pList);
    const scopeParams = extraParams?.length ? [...extraParams, ...pList] : pList;
    if (scopeParams.length) {
      paramScopes.push({ from: openParenPos, to: scoped.bodyEndPos, params: new Set(scopeParams) });
    }
  };

  const lines = sample.split('\n');
  let offset = 0;
  let inGoVarBlock = false;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();
    const lineStart = offset;
    const lineEnd = offset + line.length;
    offset = lineEnd + 1;
    if (lang === 'go') {
      if (/^(?:var|const)\s*\(\s*$/.test(trimmed)) {
        inGoVarBlock = true;
        continue;
      }
      if (inGoVarBlock) {
        if (trimmed.startsWith(')')) {
          inGoVarBlock = false;
          continue;
        }
        const mv = /^([A-Za-z_]\w*(?:\s*,\s*[A-Za-z_]\w*)*)\b/.exec(trimmed);
        if (mv) {
          for (const v of mv[1].split(',')) pkgVars.add(v.trim());
        }
        continue;
      }
      const mTopVar = /^(?:var|const)\s+([A-Za-z_]\w*)\b/.exec(trimmed);
      if (mTopVar && line[0] !== '\t' && line[0] !== ' ') pkgVars.add(mTopVar[1]);
      const mType = /^type\s+([A-Za-z_]\w*)\b/.exec(trimmed);
      if (mType) typeNames.add(mType[1]);
      const mFn = /^func\s+(?:\(\s*(?:([A-Za-z_]\w*)\s+)?\*?[A-Za-z_][\w.[\]]*\s*\)\s*)?([A-Za-z_]\w*)\s*(?:\[[^\]]*\])?\s*\(/.exec(trimmed);
      if (mFn) {
        const recvName = mFn[1] && mFn[1] !== '_' ? [mFn[1]] : undefined;
        const fnName = mFn[2];
        const recvEnd = /^func\s*\(/.test(trimmed) ? line.indexOf(')') + 1 : 0;
        const fnIdx = line.indexOf(fnName, Math.max(0, recvEnd));
        const fnOffset = fnIdx >= 0 ? fnIdx : Math.max(0, line.lastIndexOf(fnName));
        registerFuncDecl(fnName, i, line, lineStart, lineEnd, lineStart + fnOffset + fnName.length, recvName);
      }
    } else if (lang === 'java') {
      const mType = /^(?:public|protected|private|abstract|final|static|sealed|\s)*(?:class|interface|enum|record)\s+([A-Za-z_$][\w$]*)/.exec(trimmed);
      if (mType) {
        typeNames.add(mType[1]);
        continue;
      }
      const mField = /^(?:private|protected|public)\s+(?:static\s+|final\s+|volatile\s+|transient\s+)*(?:[\w.$?[\]]+(?:<[^>{};]+>)?(?:\[\])*)\s+([A-Za-z_$][\w$]*)\s*[;=]/.exec(trimmed);
      if (mField && !trimmed.includes('(')) {
        pkgVars.add(mField[1]);
        continue;
      }
      const mMeth = /^(?:public|protected|private|static|final|synchronized|default|abstract|\s)+(?:<[^>{};]+>\s+)?(?:[\w.$?[\]]+(?:<[^>{};]+>)?(?:\[\])*)\s+([A-Za-z_$][\w$]*)\s*\(/.exec(trimmed);
      if (mMeth && !CONTROL_KW.has(mMeth[1])) {
        const fnName = mMeth[1];
        const callIdx = line.indexOf(`${fnName}(`);
        const fnOffset = callIdx >= 0 ? callIdx : Math.max(0, line.indexOf(fnName));
        registerFuncDecl(fnName, i, line, lineStart, lineEnd, lineStart + fnOffset + fnName.length);
      }
    } else if (lang === 'ts' || lang === 'rust' || lang === 'python' || lang === 'shell') {
      const re =
        lang === 'rust' ? /^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+|unsafe\s+|const\s+)*fn\s+([A-Za-z_]\w*)\s*(?:<[^>]*>)?\s*\(/
        : lang === 'python' ? /^(?:async\s+)?def\s+([A-Za-z_]\w*)\s*\(/
        : lang === 'shell' ? /^(?:function\s+([A-Za-z_][\w.-]*)|([A-Za-z_][\w.-]*)\s*\(\s*\))/
        : /^(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s+([A-Za-z_$][\w$]*)\s*(?:<[^>]*>)?\s*\(/;
      const m = re.exec(trimmed);
      const fnName = m ? (m[1] || m[2]) : '';
      const nIdx = fnName ? line.indexOf(fnName) : -1;
      if (m && fnName && nIdx >= 0) {
        registerFuncDecl(fnName, i, line, lineStart, lineEnd, lineStart + nIdx + fnName.length);
      }
    }
  }

  const declStarts = new Set(funcDecls.map((d) => d.nameFrom));
  return { pkgVars, typeNames, importedPkgs, funcParams, funcDecls, paramScopes, declStarts, unresolvedIdents, unusedRanges };
}

function buildBracketIndex(docText: string): CachedBracketIndex {
  const limit = Math.min(docText.length, 262144);
  const stack: Array<{ ch: string; pos: number; level: 1 | 2 | 3 | 4 }> = [];
  const pairs: BracketPair[] = [];
  const posToPair = new Map<number, BracketPair>();
  const unmatched: number[] = [];

  for (let i = 0; i < limit; i++) {
    const jumped = skipLiteralOrComment(docText, i, limit);
    if (jumped > i) {
      i = jumped;
      continue;
    }
    const ch = docText[i];
    if (ch === '(' || ch === '{' || ch === '[') {
      stack.push({ ch, pos: i, level: ((stack.length % 4) + 1) as 1 | 2 | 3 | 4 });
    } else if (ch === ')' || ch === '}' || ch === ']') {
      const want = ch === ')' ? '(' : ch === '}' ? '{' : '[';
      const top = stack[stack.length - 1];
      if (top && top.ch === want) {
        stack.pop();
        const pair: BracketPair = { open: top.pos, close: i, level: top.level };
        pairs.push(pair);
        posToPair.set(pair.open, pair);
        posToPair.set(pair.close, pair);
      } else {
        unmatched.push(i);
      }
    }
  }
  return { pairs, posToPair, stack, unmatched };
}

export function visibleBracketSpansFromIndex(
  idx: CachedBracketIndex,
  visibleRanges: readonly { from: number; to: number }[],
  cursorPos?: number | null,
): RainbowBracketSpan[] {
  const inVis = (p: number): boolean => visibleRanges.some((r) => p >= r.from && p < r.to);
  const active = cursorPos == null ? null : (idx.posToPair.get(cursorPos) ?? idx.posToPair.get(cursorPos - 1) ?? null);
  const out: RainbowBracketSpan[] = [];
  for (const p of idx.pairs) {
    if (inVis(p.open)) out.push({ from: p.open, to: p.open + 1, level: p.level, active: active === p });
    if (inVis(p.close)) out.push({ from: p.close, to: p.close + 1, level: p.level, active: active === p });
  }
  for (const s of idx.stack) {
    if (inVis(s.pos)) out.push({ from: s.pos, to: s.pos + 1, level: s.level, active: false });
  }
  for (const u of idx.unmatched) {
    if (inVis(u)) out.push({ from: u, to: u + 1, level: 'unmatched', active: false });
  }
  return out.sort((a, b) => a.from - b.from);
}

export function computeRainbowBrackets(
  docText: string,
  visibleRanges?: readonly { from: number; to: number }[],
  cursorPos?: number | null,
): RainbowBracketSpan[] {
  const ranges = visibleRanges ?? [{ from: 0, to: docText.length }];
  return visibleBracketSpansFromIndex(buildBracketIndex(docText), ranges, cursorPos);
}

function buildIdentCountIndex(docText: string): Map<string, number> {
  const counts = new Map<string, number>();
  const sample = docText.length > 131072 ? docText.slice(0, 131072) : docText;
  const re = /\b[A-Za-z_$][\w$]*\b/g;
  for (let m: RegExpExecArray | null; (m = re.exec(sample)) !== null; ) {
    const id = m[0];
    counts.set(id, (counts.get(id) ?? 0) + 1);
  }
  return counts;
}

export function getCachedSemanticDoc(
  doc: Text,
  lang: SemanticLangKind,
  extraUnresolved: ReadonlySet<string> | undefined,
  jPrefs: JavaDiagPrefs | undefined,
  needBrackets: boolean,
  needIdentCounts: boolean,
): {
  docText: string;
  model: FileSymbolModel;
  identCounts: Map<string, number> | null;
  brackets: CachedBracketIndex | null;
} {
  let entry = semanticDocCache.get(doc);
  if (!entry) {
    entry = { docText: doc.toString(), modelKey: '', model: null, identCounts: null, brackets: null };
    semanticDocCache.set(doc, entry);
  }
  const unresKey = extraUnresolved?.size ? [...extraUnresolved].sort().join(',') : '';
  const prefsKey =
    lang === 'java' || lang === 'go'
      ? `${jPrefs?.javaUnresolvedImports ?? true}:${jPrefs?.lombokSupport ?? true}:${jPrefs?.unusedImports ?? true}`
      : '';
  const wantKey = `${lang}|${prefsKey}|${unresKey}`;
  if (!entry.model || entry.modelKey !== wantKey) {
    entry.modelKey = wantKey;
    entry.model = buildSymbolModel(entry.docText, lang, extraUnresolved, jPrefs);
  }
  if (needBrackets && !entry.brackets) entry.brackets = buildBracketIndex(entry.docText);
  if (needIdentCounts && !entry.identCounts) entry.identCounts = buildIdentCountIndex(entry.docText);
  return { docText: entry.docText, model: entry.model, identCounts: entry.identCounts, brackets: entry.brackets };
}

export function analyzeJavaDiagnostics(
  docText: string,
  prefs?: JavaDiagPrefs,
): { diagnostics: ClientIdeDiagnostic[]; unresolvedIdents: Set<string> } {
  const checkUnres = prefs?.javaUnresolvedImports !== false;
  const checkUnused = prefs?.unusedImports !== false;
  const lombok = prefs?.lombokSupport !== false;
  const lines = docText.split(/\r?\n/);
  const diagnostics: ClientIdeDiagnostic[] = [];
  const unresolvedIdents = new Set<string>();
  const seenImports = new Set<string>();
  const bodyChunks: string[] = [];
  const importEntries: Array<{ line: number; col: number; endCol: number; path: string; ident: string }> = [];

  for (let idx = 0; idx < lines.length; idx++) {
    const raw = lines[idx];
    const trimmed = raw.trim();
    const ln = idx + 1;
    if (trimmed.startsWith('package ')) continue;
    const mImp = /^\s*import\s+(?:static\s+)?([\w.*$]+)\s*;\s*$/.exec(raw);
    if (mImp) {
      const impPath = mImp[1];
      const col = raw.indexOf('import') + 1;
      const endCol = raw.length + 1;
      const segs = impPath.split('.');
      const lastSeg = segs[segs.length - 1] ?? '';
      const isBogus =
        segs.length < 2 ||
        BOGUS_JAVA_PREFIXES.some((p) => impPath.startsWith(p)) ||
        (!lombok && impPath.startsWith('lombok.'));
      if (isBogus) {
        if (checkUnres) {
          if (lastSeg && lastSeg !== '*') unresolvedIdents.add(lastSeg);
          diagnostics.push({
            line: ln, column: col, endLine: ln, endColumn: endCol,
            severity: 'error', source: 'java-imports', code: 'unresolved-import',
            message: `Unresolved import '${impPath}'`,
            quickFixes: [{ title: `Remove unresolved import '${impPath}'`, line: ln, endLine: ln, deleteLine: true }],
          });
        }
        continue;
      }
      if (seenImports.has(impPath)) {
        diagnostics.push({
          line: ln, column: col, endLine: ln, endColumn: endCol,
          severity: 'warning', source: 'java-imports', code: 'duplicate-import',
          message: `Duplicate import '${impPath}'`,
          quickFixes: [{ title: `Remove duplicate import '${impPath}'`, line: ln, endLine: ln, deleteLine: true }],
        });
        continue;
      }
      seenImports.add(impPath);
      if (lastSeg && lastSeg !== '*') importEntries.push({ line: ln, col, endCol, path: impPath, ident: lastSeg });
      continue;
    }
    bodyChunks.push(raw);
  }

  if (checkUnused && importEntries.length > 0) {
    const bodyCounts = buildIdentCountIndex(bodyChunks.join('\n'));
    for (const imp of importEntries) {
      if (!bodyCounts.has(imp.ident)) {
        diagnostics.push({
          line: imp.line, column: imp.col, endLine: imp.line, endColumn: imp.endCol,
          severity: 'warning', source: 'java-imports', code: 'unused-import',
          message: `Unused import '${imp.path}'`,
          quickFixes: [{ title: `Remove unused import '${imp.path}'`, line: imp.line, endLine: imp.line, deleteLine: true }],
        });
      }
    }
  }
  return { diagnostics, unresolvedIdents };
}
