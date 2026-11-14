// Pure semantic classifier, symbol model, rainbow-bracket index, Java import
// diagnostics, and per-`Text` cache (`WeakMap<Text, CachedSemanticDoc>`).
// Keying by immutable CodeMirror `Text` guarantees zero `doc.toString()` or
// full-document rescans on cursor movement or viewport scroll (`PERF-2`).

import type { Text } from '@codemirror/state';

export type SemanticLangKind =
  | 'go' | 'java' | 'ts' | 'rust' | 'python' | 'shell' | 'json' | 'csv' | 'other';

export type SemanticRole =
  | 'fg' | 'keyword' | 'control' | 'func-decl' | 'func-call' | 'field'
  | 'type' | 'annotation' | 'param' | 'doc-comment' | 'doc-tag' | 'doc-param' | 'unresolved';

export interface SemanticTokenSpan {
  from: number;
  to: number;
  role: SemanticRole;
  text: string;
}

export interface RainbowBracketSpan {
  from: number;
  to: number;
  level: 1 | 2 | 3 | 4 | 'unmatched';
  active: boolean;
}

export interface ClientDiagQuickFix {
  title: string;
  line: number;
  endLine: number;
  deleteLine?: boolean;
  replacement?: string;
  edits?: Array<{
    path?: string;
    start_line: number;
    start_col: number;
    end_line: number;
    end_col: number;
    new_text: string;
  }>;
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
  funcParams: Map<string, string[]>;
  funcDecls: Array<{ name: string; line: number; nameFrom: number; nameTo: number; lineEnd: number }>;
  paramScopes: Array<{ from: number; to: number; params: Set<string> }>;
  declStarts: Set<number>;
  unresolvedIdents: Set<string>;
}

interface BracketPair {
  open: number;
  close: number;
  level: 1 | 2 | 3 | 4;
}

interface CachedBracketIndex {
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
  if (
    ['ts', 'tsx', 'js', 'jsx', 'mjs', 'cjs', 'mts', 'cts'].includes(ext) ||
    language === 'typescript' ||
    language === 'javascript'
  ) {
    return 'ts';
  }
  if (ext === 'rs' || language === 'rust') return 'rust';
  if (ext === 'py' || ext === 'pyi' || language === 'python') return 'python';
  if (['sh', 'bash', 'zsh', 'bats'].includes(ext) || language === 'shell' || language === 'shellscript') {
    return 'shell';
  }
  if (['json', 'jsonc', 'jsonl', 'ndjson'].includes(ext) || language === 'json' || language === 'jsonl') {
    return 'json';
  }
  return ext === 'csv' || ext === 'tsv' || language === 'csv' || language === 'tsv' ? 'csv' : 'other';
}

const CONTROL_KW = new Set(
  'if else for range return switch case default select defer go break continue fallthrough try catch finally throw throws while do match loop await async yield elif except raise with'.split(' '),
);

const GENERAL_KW = new Set(
  (
    'package import func var const type struct interface map chan string int int8 int16 int32 int64 ' +
    'uint uint8 uint16 uint32 uint64 uintptr float32 float64 bool byte rune error any comparable ' +
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

function splitParamNames(rawParams: string, lang: SemanticLangKind): string[] {
  const names: string[] = [];
  for (const part of rawParams.split(',')) {
    const clean = part.replace(/\/\*[\s\S]*?\*\//g, '').replace(/=[^,]+$/, '').trim();
    if (!clean || clean === 'self' || clean === '&self' || clean === '&mut self' || clean === 'this') {
      continue;
    }
    let m: RegExpExecArray | null = null;
    if (lang === 'go') {
      m = /^([A-Za-z_]\w*)(?:\s+(?:\.\.\.)?[*[\]\w.]+)?$/.exec(clean);
    } else if (lang === 'java') {
      m = /(?:final\s+|@\w+(?:\([^)]*\))?\s+)*[\w<>[\],.?]+\s*(?:\.\.\.)?\s+([A-Za-z_$][\w$]*)$/.exec(clean);
    } else {
      m = /^(?:pub\s+|private\s+|readonly\s+|mut\s+|\*\*{0,2})?([A-Za-z_$][\w$]*)\s*(?:\??\s*:|$)/.exec(clean);
    }
    if (m && !GENERAL_KW.has(m[1])) names.push(m[1]);
  }
  return names;
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
  const funcParams = new Map<string, string[]>();
  const funcDecls: FileSymbolModel['funcDecls'] = [];
  const paramScopes: FileSymbolModel['paramScopes'] = [];
  const unresolvedIdents = new Set<string>(extraUnresolved);

  if (lang === 'java') {
    for (const id of analyzeJavaDiagnostics(sample, jPrefs).unresolvedIdents) {
      unresolvedIdents.add(id);
    }
    if ((jPrefs?.lombokSupport ?? true) && /@(?:Slf4j|Log4j2|Log|CommonsLog|JBossLog|Flogger)\b/.test(sample)) {
      pkgVars.add('log');
    }
  }

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
      const mFn = /^func\s+(?:\(\s*([A-Za-z_]\w*)?\s*\*?[A-Za-z_][\w.]*\s*\)\s*)?([A-Za-z_]\w*)\s*\(([^)]*)\)/.exec(trimmed);
      if (mFn) {
        const fnName = mFn[2];
        const callIdx = line.indexOf(`${fnName}(`);
        const nFrom = lineStart + Math.max(0, callIdx >= 0 ? callIdx : line.indexOf(fnName));
        funcDecls.push({ name: fnName, line: i + 1, nameFrom: nFrom, nameTo: nFrom + fnName.length, lineEnd });
        const pList = splitParamNames(mFn[3], 'go');
        if (pList.length) {
          funcParams.set(fnName, pList);
          paramScopes.push({ from: lineStart, to: sample.length, params: new Set(pList) });
        }
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
      const mMeth = /^(?:public|protected|private|static|final|synchronized|default|abstract|\s)+(?:<[^>{};]+>\s+)?(?:[\w.$?[\]]+(?:<[^>{};]+>)?(?:\[\])*)\s+([A-Za-z_$][\w$]*)\s*\(([^)]*)\)/.exec(trimmed);
      if (mMeth && !CONTROL_KW.has(mMeth[1])) {
        const fnName = mMeth[1];
        const callIdx = line.indexOf(`${fnName}(`);
        const nFrom = lineStart + Math.max(0, callIdx >= 0 ? callIdx : line.indexOf(fnName));
        funcDecls.push({ name: fnName, line: i + 1, nameFrom: nFrom, nameTo: nFrom + fnName.length, lineEnd });
        const pList = splitParamNames(mMeth[2], 'java');
        if (pList.length) {
          funcParams.set(fnName, pList);
          paramScopes.push({ from: lineStart, to: sample.length, params: new Set(pList) });
        }
      }
    } else if (lang === 'ts' || lang === 'rust' || lang === 'python' || lang === 'shell') {
      const re =
        lang === 'rust'
          ? /^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+|unsafe\s+|const\s+)*fn\s+([A-Za-z_]\w*)\s*(?:<[^>]*>)?\s*\(([^)]*)\)/
          : lang === 'python'
            ? /^(?:async\s+)?def\s+([A-Za-z_]\w*)\s*\(([^)]*)\)/
            : lang === 'shell'
              ? /^(?:function\s+([A-Za-z_][\w.-]*)|([A-Za-z_][\w.-]*)\s*\(\s*\))/
              : /^(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s+([A-Za-z_$][\w$]*)\s*(?:<[^>]*>)?\s*\(([^)]*)\)/;
      const m = re.exec(trimmed);
      const fnName = m ? (m[1] || m[2]) : '';
      const nIdx = fnName ? line.indexOf(fnName) : -1;
      if (m && fnName && nIdx >= 0) {
        funcDecls.push({
          name: fnName,
          line: i + 1,
          nameFrom: lineStart + nIdx,
          nameTo: lineStart + nIdx + fnName.length,
          lineEnd,
        });
        const pList = m[2] && lang !== 'shell' ? splitParamNames(m[2], lang) : [];
        if (pList.length) {
          funcParams.set(fnName, pList);
          paramScopes.push({ from: lineStart, to: sample.length, params: new Set(pList) });
        }
      }
    }
  }

  for (let i = 0; i + 1 < paramScopes.length; i++) {
    const nextStart = funcDecls[i + 1]?.nameFrom;
    if (nextStart && nextStart > paramScopes[i].from) paramScopes[i].to = nextStart - 1;
  }

  const declStarts = new Set(funcDecls.map((d) => d.nameFrom));
  return { pkgVars, typeNames, funcParams, funcDecls, paramScopes, declStarts, unresolvedIdents };
}

function classifyDocComment(text: string, baseFrom: number, out: SemanticTokenSpan[]): void {
  out.push({ from: baseFrom, to: baseFrom + text.length, role: 'doc-comment', text });
  const tagRe = /(@(?:param|return|returns|throws|exception|see|since|deprecated|author))\b(?:\s+([A-Za-z_$][\w$]*))?/g;
  for (let m: RegExpExecArray | null; (m = tagRe.exec(text)) !== null; ) {
    const tFrom = baseFrom + m.index;
    out.push({ from: tFrom, to: tFrom + m[1].length, role: 'doc-tag', text: m[1] });
    if (m[2] && m[1] === '@param') {
      const pIdx = m[0].lastIndexOf(m[2]);
      out.push({ from: tFrom + pIdx, to: tFrom + pIdx + m[2].length, role: 'doc-param', text: m[2] });
    }
  }
}

export function classifySemanticTokensWithModel(
  docText: string,
  lang: SemanticLangKind,
  model: FileSymbolModel,
  visibleRanges?: readonly { from: number; to: number }[],
): SemanticTokenSpan[] {
  if (!docText || lang === 'csv' || lang === 'other') return [];
  const ranges = visibleRanges ?? [{ from: 0, to: docText.length }];
  const out: SemanticTokenSpan[] = [];

  const paramsAt = (pos: number): Set<string> | null => {
    for (let i = model.paramScopes.length - 1; i >= 0; i--) {
      if (pos >= model.paramScopes[i].from && pos <= model.paramScopes[i].to) {
        return model.paramScopes[i].params;
      }
    }
    return null;
  };

  for (const { from: rawFrom, to: rawTo } of ranges) {
    const from = Math.max(0, rawFrom);
    const to = Math.min(docText.length, rawTo);
    let i = from;
    while (i < to) {
      const ch = docText[i];
      if (ch === '"' || ch === "'" || ch === '`') {
        let end = i + 1;
        while (end < to) {
          if (docText[end] === '\\' && ch !== '`') { end += 2; continue; }
          if (docText[end] === ch) { end++; break; }
          end++;
        }
        if (lang === 'json' && ch === '"') {
          let a = end;
          while (a < to && (docText[a] === ' ' || docText[a] === '\t')) a++;
          if (docText[a] === ':') out.push({ from: i, to: end, role: 'field', text: docText.slice(i, end) });
        } else if (lang === 'shell' && ch === '"') {
          const sub = docText.slice(i, end);
          const vr = /\$(?:[A-Za-z_]\w*|\{[^}"\n]+\})/g;
          let vm: RegExpExecArray | null;
          while ((vm = vr.exec(sub)) !== null) {
            out.push({ from: i + vm.index, to: i + vm.index + vm[0].length, role: 'param', text: vm[0] });
          }
        }
        i = end;
        continue;
      }
      if (ch === '/' && docText[i + 1] === '*') {
        const cl = docText.indexOf('*/', i + 2);
        const end = cl >= 0 ? Math.min(to, cl + 2) : to;
        if (docText[i + 2] === '*' && docText[i + 3] !== '/') {
          classifyDocComment(docText.slice(i, end), i, out);
        }
        i = end;
        continue;
      }
      if (ch === '/' && docText[i + 1] === '/') {
        const nl = docText.indexOf('\n', i + 2);
        const end = nl >= 0 ? Math.min(to, nl) : to;
        if (docText[i + 2] === '/' && lang === 'rust') {
          classifyDocComment(docText.slice(i, end), i, out);
        }
        i = end;
        continue;
      }
      if (ch === '#' && (lang === 'python' || lang === 'shell' || lang === 'rust')) {
        if (lang === 'rust' && (docText[i + 1] === '[' || (docText[i + 1] === '!' && docText[i + 2] === '['))) {
          const rb = docText.indexOf(']', i + 2);
          const end = rb >= 0 ? Math.min(to, rb + 1) : i + 2;
          out.push({ from: i, to: end, role: 'annotation', text: docText.slice(i, end) });
          i = end;
          continue;
        }
        const nl = docText.indexOf('\n', i + 1);
        i = nl >= 0 ? Math.min(to, nl) : to;
        continue;
      }
      if (ch === '@' && (lang === 'java' || lang === 'ts' || lang === 'python')) {
        const m = /^@[A-Za-z_$][\w$.]*/.exec(docText.slice(i, Math.min(to, i + 64)));
        if (m) {
          out.push({ from: i, to: i + m[0].length, role: 'annotation', text: m[0] });
          i += m[0].length;
          continue;
        }
      }
      if (ch === '$' && lang === 'shell') {
        const m = /^\$(?:\{[A-Za-z_]\w*\}|[A-Za-z_]\w*)/.exec(docText.slice(i, Math.min(to, i + 64)));
        if (m) {
          out.push({ from: i, to: i + m[0].length, role: 'field', text: m[0] });
          i += m[0].length;
          continue;
        }
      }
      if (/[A-Za-z_$]/.test(ch)) {
        let end = i + 1;
        while (end < to && /[\w$]/.test(docText[end])) end++;
        const word = docText.slice(i, end);
        let prev = i - 1;
        while (prev >= 0 && (docText[prev] === ' ' || docText[prev] === '\t')) prev--;
        const prevCh = prev >= 0 ? docText[prev] : '';
        const isAfterDot = prevCh === '.' || (prevCh === ':' && docText[prev - 1] === ':');
        let next = end;
        while (next < docText.length && (docText[next] === ' ' || docText[next] === '\t')) next++;
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
        else if (model.declStarts.has(i)) role = 'func-decl';
        else if (isAfterDot) {
          if (isBeforeParen) role = 'func-call';
          else if (lang === 'go' && /^[A-Z]/.test(word) && (docText[next] === '{' || (prev >= 2 && docText[prev - 2] === '*'))) {
            role = 'type';
          } else {
            role = 'field';
          }
        } else if (isBeforeDot) {
          role = scopeP?.has(word)
            ? 'param'
            : model.pkgVars.has(word)
              ? 'field'
              : (lang === 'java' || lang === 'rust') && /^[A-Z][a-zA-Z0-9]*$/.test(word)
                ? 'type'
                : 'fg';
        } else if (isBeforeParen) {
          role = /\bnew\s+$/.test(docText.slice(Math.max(0, i - 8), i)) && /^[A-Z]/.test(word)
            ? 'type'
            : 'func-call';
        } else if (scopeP?.has(word)) role = 'param';
        else if (model.pkgVars.has(word)) role = 'field';
        else if (model.typeNames.has(word)) role = 'type';
        else if (/^[A-Z][A-Z0-9_]{1,}$/.test(word)) role = 'field';
        else if (/^[A-Z][A-Za-z0-9_]*$/.test(word) && lang !== 'shell' && lang !== 'json') role = 'type';

        if (role) out.push({ from: i, to: end, role, text: word });
        i = end;
        continue;
      }
      i++;
    }
  }
  return out;
}

export function classifySemanticTokens(
  docText: string,
  lang: SemanticLangKind,
  visibleRanges?: readonly { from: number; to: number }[],
  extraUnresolved?: ReadonlySet<string>,
  jPrefs?: JavaDiagPrefs,
): SemanticTokenSpan[] {
  if (!docText || lang === 'csv' || lang === 'other') return [];
  const model = buildSymbolModel(docText, lang, extraUnresolved, jPrefs);
  return classifySemanticTokensWithModel(docText, lang, model, visibleRanges);
}

function buildBracketIndex(docText: string): CachedBracketIndex {
  const limit = Math.min(docText.length, 262144);
  const stack: Array<{ ch: string; pos: number; level: 1 | 2 | 3 | 4 }> = [];
  const pairs: BracketPair[] = [];
  const posToPair = new Map<number, BracketPair>();
  const unmatched: number[] = [];

  for (let i = 0; i < limit; i++) {
    const ch = docText[i];
    if (ch === '"' || ch === "'" || ch === '`') {
      const q = ch;
      i++;
      while (i < limit) {
        if (docText[i] === '\\' && q !== '`') { i += 2; continue; }
        if (docText[i] === q) break;
        i++;
      }
      continue;
    }
    if (ch === '/' && docText[i + 1] === '/') {
      const nl = docText.indexOf('\n', i + 2);
      i = nl >= 0 ? nl : limit;
      continue;
    }
    if (ch === '/' && docText[i + 1] === '*') {
      const cl = docText.indexOf('*/', i + 2);
      i = cl >= 0 ? cl + 1 : limit;
      continue;
    }
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

function visibleBracketSpansFromIndex(
  idx: CachedBracketIndex,
  visibleRanges: readonly { from: number; to: number }[],
  cursorPos?: number | null,
): RainbowBracketSpan[] {
  const inVis = (p: number): boolean => visibleRanges.some((r) => p >= r.from && p < r.to);
  const active =
    cursorPos == null
      ? null
      : (idx.posToPair.get(cursorPos) ?? idx.posToPair.get(cursorPos - 1) ?? null);
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
    entry = {
      docText: doc.toString(),
      modelKey: '',
      model: null,
      identCounts: null,
      brackets: null,
    };
    semanticDocCache.set(doc, entry);
  }
  const unresKey = extraUnresolved?.size ? [...extraUnresolved].sort().join(',') : '';
  const prefsKey =
    lang === 'java'
      ? `${jPrefs?.javaUnresolvedImports ?? true}:${jPrefs?.lombokSupport ?? true}:${jPrefs?.unusedImports ?? true}`
      : '';
  const wantKey = `${lang}|${prefsKey}|${unresKey}`;
  if (!entry.model || entry.modelKey !== wantKey) {
    entry.modelKey = wantKey;
    entry.model = buildSymbolModel(entry.docText, lang, extraUnresolved, jPrefs);
  }
  if (needBrackets && !entry.brackets) {
    entry.brackets = buildBracketIndex(entry.docText);
  }
  if (needIdentCounts && !entry.identCounts) {
    entry.identCounts = buildIdentCountIndex(entry.docText);
  }
  return {
    docText: entry.docText,
    model: entry.model,
    identCounts: entry.identCounts,
    brackets: entry.brackets,
  };
}

export { visibleBracketSpansFromIndex };

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
      if (
        segs.length < 2 ||
        BOGUS_JAVA_PREFIXES.some((p) => impPath.startsWith(p)) ||
        (!lombok && impPath.startsWith('lombok.'))
      ) {
        if (checkUnres) {
          if (lastSeg && lastSeg !== '*') unresolvedIdents.add(lastSeg);
          diagnostics.push({
            line: ln,
            column: col,
            endLine: ln,
            endColumn: endCol,
            severity: 'error',
            source: 'java-imports',
            code: 'unresolved-import',
            message: `Unresolved import '${impPath}'`,
            quickFixes: [
              { title: `Remove unresolved import '${impPath}'`, line: ln, endLine: ln, deleteLine: true },
            ],
          });
        }
        continue;
      }
      if (seenImports.has(impPath)) {
        diagnostics.push({
          line: ln,
          column: col,
          endLine: ln,
          endColumn: endCol,
          severity: 'warning',
          source: 'java-imports',
          code: 'duplicate-import',
          message: `Duplicate import '${impPath}'`,
          quickFixes: [
            { title: `Remove duplicate import '${impPath}'`, line: ln, endLine: ln, deleteLine: true },
          ],
        });
        continue;
      }
      seenImports.add(impPath);
      if (lastSeg && lastSeg !== '*') {
        importEntries.push({ line: ln, col, endCol, path: impPath, ident: lastSeg });
      }
      continue;
    }
    bodyChunks.push(raw);
  }

  if (checkUnused && importEntries.length > 0) {
    const bodyCounts = buildIdentCountIndex(bodyChunks.join('\n'));
    for (const imp of importEntries) {
      if (!bodyCounts.has(imp.ident)) {
        diagnostics.push({
          line: imp.line,
          column: imp.col,
          endLine: imp.line,
          endColumn: imp.endCol,
          severity: 'warning',
          source: 'java-imports',
          code: 'unused-import',
          message: `Unused import '${imp.path}'`,
          quickFixes: [
            { title: `Remove unused import '${imp.path}'`, line: imp.line, endLine: imp.line, deleteLine: true },
          ],
        });
      }
    }
  }
  return { diagnostics, unresolvedIdents };
}

export function computeImportFoldRange(
  docText: string,
  lang: SemanticLangKind,
  errorLines?: ReadonlySet<number>,
): { from: number; to: number; startLine: number; endLine: number } | null {
  const lines = docText.split('\n');
  let offset = 0;
  let startLine = -1;
  let endLine = -1;
  let foldFrom = -1;
  let foldTo = -1;
  let inGoParens = false;

  for (let i = 0; i < Math.min(lines.length, 250); i++) {
    const line = lines[i];
    const trimmed = line.trim();
    const ln = i + 1;
    const lStart = offset;
    const lEnd = offset + line.length;
    offset = lEnd + 1;

    if (lang === 'go') {
      if (startLine < 0 && /^import\s*\(/.test(trimmed)) {
        startLine = ln;
        inGoParens = true;
        foldFrom = lStart + line.indexOf('(') + 1;
        continue;
      }
      if (inGoParens) {
        if (trimmed.startsWith(')')) {
          endLine = ln;
          foldTo = lStart;
          break;
        }
        continue;
      }
    } else if (lang === 'java' || lang === 'ts' || lang === 'python' || lang === 'rust') {
      const isImp = lang === 'rust' ? /^use\s+/.test(trimmed) : /^(?:import|from)\s+/.test(trimmed);
      if (isImp) {
        if (startLine < 0) {
          startLine = ln;
          foldFrom = lEnd;
        }
        endLine = ln;
        foldTo = lEnd;
      } else if (startLine > 0 && trimmed !== '' && !trimmed.startsWith('//') && !trimmed.startsWith('#')) {
        break;
      }
    }
  }
  if (startLine < 0 || endLine <= startLine || foldTo <= foldFrom) return null;
  if (errorLines) {
    for (let ln = startLine; ln <= endLine; ln++) {
      if (errorLines.has(ln)) return null;
    }
  }
  return { from: foldFrom, to: foldTo, startLine, endLine };
}
