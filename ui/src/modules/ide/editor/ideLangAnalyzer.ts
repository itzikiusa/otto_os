// Zero-cache, <1ms lexical analyzer for all 8 first-class IDE languages/formats:
// JS/TS, Go, Java, Rust, Python, Shell, JSON/JSONC/JSONL, and CSV/TSV.
// Powers instant Cmd+F12 File Structure, Prev/Next Function, JSONPath breadcrumbs,
// JSON Pretty/Minify/Sort transforms, and instant local Shell/JSON/$ref dive-in.

import { detectCsvDelimiter, parseCsvLine } from '../../../lib/components/cm-ide-csv';
import { loadErrorText } from '../../../lib/loadError';
import type { IdeSymbolNode } from '../store/ideTypes';

export type IdeOutlineNode = IdeSymbolNode & { column?: number };

export type IdeDocKind =
  | 'go' | 'ts' | 'java' | 'rust' | 'python' | 'shell' | 'json' | 'jsonl' | 'csv' | 'other';

export function detectIdeDocKind(filePath: string, language?: string): IdeDocKind {
  if (filePath.startsWith('jdt://')) return 'java';
  const clean = filePath.split('?')[0].toLowerCase();
  const base = clean.slice(clean.lastIndexOf('/') + 1);
  const dot = base.lastIndexOf('.');
  const ext = dot >= 0 ? base.slice(dot + 1) : '';
  if (ext === 'csv' || ext === 'tsv' || language === 'csv' || language === 'tsv') return 'csv';
  if (ext === 'jsonl' || ext === 'ndjson' || language === 'jsonl') return 'jsonl';
  if (ext === 'json' || ext === 'jsonc' || language === 'json') return 'json';
  if (ext === 'go' || language === 'go') return 'go';
  if (ext === 'java' || ext === 'class' || language === 'java') return 'java';
  if (ext === 'rs' || language === 'rust') return 'rust';
  if (ext === 'py' || ext === 'pyi' || language === 'python') return 'python';
  if (
    ext === 'sh' || ext === 'bash' || ext === 'zsh' || ext === 'bats' ||
    base === '.bashrc' || base === '.zshrc' || language === 'shell' || language === 'shellscript'
  ) return 'shell';
  if (
    ext === 'ts' || ext === 'tsx' || ext === 'js' || ext === 'jsx' ||
    ext === 'mjs' || ext === 'cjs' || ext === 'mts' || ext === 'cts' ||
    language === 'typescript' || language === 'javascript'
  ) return 'ts';
  return 'other';
}

const MAX_ANALYZE_CHARS = 512 * 1024;
const MAX_OUTLINE_ITEMS = 400;

function assignEndLines(nodes: IdeOutlineNode[], totalLines: number): IdeOutlineNode[] {
  for (let i = 0; i < nodes.length; i++) {
    const cur = nodes[i];
    const nextLine = i + 1 < nodes.length ? Math.max(cur.line, nodes[i + 1].line - 1) : totalLines;
    if (cur.end_line <= cur.line) cur.end_line = nextLine;
    if (cur.children && cur.children.length > 0) assignEndLines(cur.children as IdeOutlineNode[], cur.end_line);
  }
  return nodes;
}

export function extractInstantSymbols(filePath: string, language: string | undefined, rawContent: string): IdeOutlineNode[] {
  const kind = detectIdeDocKind(filePath, language);
  const content = rawContent.length > MAX_ANALYZE_CHARS ? rawContent.slice(0, MAX_ANALYZE_CHARS) : rawContent;

  if (kind === 'csv') {
    const nl = content.indexOf('\n');
    const first = (nl >= 0 ? content.slice(0, nl) : content).replace(/\r$/, '');
    if (!first.trim()) return [];
    const delim = detectCsvDelimiter(content.slice(0, 4096), filePath);
    const cols = parseCsvLine(first, delim);
    return cols.slice(0, MAX_OUTLINE_ITEMS).map((c, idx) => ({
      name: `Col ${idx + 1}: ${c.value.trim() || `(column ${idx + 1})`}`,
      kind: 'field',
      line: 1,
      column: c.from + 1,
      end_line: 1,
      children: [],
    }));
  }

  if (kind === 'json') return extractJsonSymbols(content);
  if (kind === 'jsonl') return extractJsonlSymbols(content);

  const lines = content.split(/\r?\n/);
  const out: IdeOutlineNode[] = [];

  if (kind === 'python') return extractPythonSymbols(lines);

  for (let i = 0; i < lines.length && out.length < MAX_OUTLINE_ITEMS; i++) {
    const line = lines[i];
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith('//') || trimmed.startsWith('#') || trimmed.startsWith('/*') || trimmed.startsWith('*')) continue;
    const ln = i + 1;

    if (kind === 'go') {
      const mFn = /^func\s+(?:\(\s*\w*\s*\*?([\w.]+)\s*\)\s*)?(\w+)\s*\(/.exec(trimmed);
      if (mFn) {
        out.push({ name: mFn[1] ? `(${mFn[1]}).${mFn[2]}` : mFn[2], kind: mFn[1] ? 'method' : 'function', line: ln, column: 1, end_line: ln, children: [] });
        continue;
      }
      const mType = /^type\s+(\w+)\s+(struct|interface|\w+)/.exec(trimmed);
      if (mType) {
        out.push({ name: mType[1], kind: mType[2] === 'interface' ? 'interface' : 'struct', line: ln, column: 1, end_line: ln, children: [] });
        continue;
      }
      const mSub = /\bt\.Run\(\s*["'`]([^"'`]+)["'`]/.exec(trimmed);
      if (mSub) out.push({ name: `t.Run("${mSub[1]}")`, kind: 'test', line: ln, column: 1, end_line: ln, children: [] });
    } else if (kind === 'ts') {
      const mDecl = /^(?:export\s+)?(?:default\s+)?(?:abstract\s+)?(class|interface|type|enum|function)\s+(\w+)/.exec(trimmed);
      if (mDecl) {
        out.push({ name: mDecl[2], kind: mDecl[1], line: ln, column: 1, end_line: ln, children: [] });
        continue;
      }
      const mConstFn = /^(?:export\s+)?(?:const|let)\s+(\w+)\s*(?::[^=]+)?=\s*(?:async\s*)?(?:\([^)]*\)|\w+)\s*=>/.exec(trimmed);
      if (mConstFn) {
        out.push({ name: mConstFn[1], kind: 'function', line: ln, column: 1, end_line: ln, children: [] });
        continue;
      }
      const mTest = /^(?:describe|it|test)\(\s*["'`]([^"'`]+)["'`]/.exec(trimmed);
      if (mTest) {
        out.push({ name: mTest[1], kind: 'test', line: ln, column: 1, end_line: ln, children: [] });
        continue;
      }
      const mMethod = /^(?:public\s+|private\s+|protected\s+|static\s+|async\s+|readonly\s+)*(\w+)\s*\([^;]*\{\s*$/.exec(trimmed);
      if (mMethod && !['if', 'for', 'while', 'switch', 'catch', 'return', 'function'].includes(mMethod[1])) {
        out.push({ name: `${mMethod[1]}()`, kind: 'method', line: ln, column: 1, end_line: ln, children: [] });
      }
    } else if (kind === 'java') {
      const mType = /^(?:public\s+|protected\s+|private\s+|abstract\s+|final\s+|static\s+|sealed\s+)*(class|interface|enum|record|@interface)\s+(\w+)/.exec(trimmed);
      if (mType) {
        out.push({ name: mType[2], kind: mType[1], line: ln, column: 1, end_line: ln, children: [] });
        continue;
      }
      const mMeth = /^(?:public|protected|private|static|final|synchronized|default|\s)+[\w<>[\],.?]+\s+(\w+)\s*\([^;]*\)\s*(?:throws\s+[\w,\s]+)?\{?$/.exec(trimmed);
      if (mMeth && !['if', 'for', 'while', 'switch', 'catch'].includes(mMeth[1])) {
        out.push({ name: `${mMeth[1]}()`, kind: 'method', line: ln, column: 1, end_line: ln, children: [] });
      }
    } else if (kind === 'rust') {
      const mItem = /^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+|unsafe\s+|const\s+)*(fn|struct|enum|trait|mod|type)\s+(\w+)/.exec(trimmed);
      if (mItem) {
        out.push({ name: mItem[2], kind: mItem[1] === 'fn' ? 'function' : mItem[1], line: ln, column: 1, end_line: ln, children: [] });
        continue;
      }
      const mImpl = /^impl(?:<[^>]+>)?\s+([^{]+)\{?/.exec(trimmed);
      if (mImpl) {
        out.push({ name: `impl ${mImpl[1].trim()}`, kind: 'struct', line: ln, column: 1, end_line: ln, children: [] });
        continue;
      }
      const mMacro = /^macro_rules!\s+(\w+)/.exec(trimmed);
      if (mMacro) out.push({ name: `${mMacro[1]}!`, kind: 'function', line: ln, column: 1, end_line: ln, children: [] });
    } else if (kind === 'shell') {
      const mFn = /^(?:function\s+([\w.:-]+)(?:\s*\(\s*\))?|([\w.:-]+)\s*\(\s*\))\s*\{?/.exec(trimmed);
      if (mFn) {
        const fnName = mFn[1] || mFn[2];
        if (fnName && !['if', 'for', 'while', 'case'].includes(fnName)) {
          out.push({ name: `${fnName}()`, kind: 'function', line: ln, column: 1, end_line: ln, children: [] });
          continue;
        }
      }
      const mBats = /^@test\s+["']([^"']+)["']/.exec(trimmed);
      if (mBats) {
        out.push({ name: mBats[1], kind: 'test', line: ln, column: 1, end_line: ln, children: [] });
        continue;
      }
      const mExp = /^export\s+([A-Z_][A-Z0-9_]*)=/.exec(trimmed);
      if (mExp) out.push({ name: mExp[1], kind: 'variable', line: ln, column: 1, end_line: ln, children: [] });
    }
  }

  return assignEndLines(out, lines.length);
}

function extractPythonSymbols(lines: string[]): IdeOutlineNode[] {
  const roots: IdeOutlineNode[] = [];
  const stack: Array<{ indent: number; node: IdeOutlineNode }> = [];
  for (let i = 0; i < lines.length && roots.length < MAX_OUTLINE_ITEMS; i++) {
    const raw = lines[i];
    const m = /^(\s*)(?:async\s+)?(class|def)\s+(\w+)\s*[(,:]/.exec(raw);
    if (!m) continue;
    const indent = m[1].replace(/\t/g, '    ').length;
    const node: IdeOutlineNode = {
      name: m[2] === 'def' ? `${m[3]}()` : m[3],
      kind: m[2] === 'class' ? 'class' : indent > 0 ? 'method' : 'function',
      line: i + 1,
      column: indent + 1,
      end_line: i + 1,
      children: [],
    };
    while (stack.length > 0 && stack[stack.length - 1].indent >= indent) stack.pop();
    if (stack.length > 0) (stack[stack.length - 1].node.children ??= []).push(node);
    else roots.push(node);
    stack.push({ indent, node });
  }
  return assignEndLines(roots, lines.length);
}

function extractJsonSymbols(content: string): IdeOutlineNode[] {
  const lines = content.split(/\r?\n/);
  const roots: IdeOutlineNode[] = [];
  const stack: Array<{ indent: number; node: IdeOutlineNode }> = [];
  let count = 0;
  for (let i = 0; i < lines.length && count < MAX_OUTLINE_ITEMS; i++) {
    const raw = lines[i];
    const m = /^(\s*)"([^"\\]*(?:\\.[^"\\]*)*)"\s*:\s*(.*)$/.exec(raw);
    if (!m) continue;
    count++;
    const indent = m[1].length;
    const valHint = m[3].trim();
    const kind = valHint.startsWith('{') ? 'object' : valHint.startsWith('[') ? 'array' : 'property';
    const node: IdeOutlineNode = {
      name: m[2],
      kind,
      line: i + 1,
      column: indent + 1,
      end_line: i + 1,
      children: [],
    };
    while (stack.length > 0 && stack[stack.length - 1].indent >= indent) stack.pop();
    if (stack.length > 0 && stack.length <= 4) (stack[stack.length - 1].node.children ??= []).push(node);
    else if (stack.length === 0) roots.push(node);
    if (kind === 'object' || kind === 'array') stack.push({ indent, node });
  }
  return assignEndLines(roots, lines.length);
}

function extractJsonlSymbols(content: string): IdeOutlineNode[] {
  const lines = content.split(/\r?\n/);
  const out: IdeOutlineNode[] = [];
  let rowNum = 0;
  for (let i = 0; i < lines.length && out.length < 250; i++) {
    const t = lines[i].trim();
    if (!t) continue;
    rowNum++;
    const mKey = /"([\w-]+)"\s*:\s*("[^"]{0,28}"|\d+|true|false)/.exec(t);
    out.push({
      name: mKey ? `Row ${rowNum} (${mKey[1]}: ${mKey[2]})` : `Row ${rowNum}`,
      kind: 'object',
      line: i + 1,
      column: 1,
      end_line: i + 1,
      children: [],
    });
  }
  return out;
}

// ── Live JSONPath Breadcrumb Computation (`$.server.ports[0]`) ───────────────

export function computeJsonPathAt(content: string, targetLine: number, targetCol: number): string | null {
  if (!content || content.length > MAX_ANALYZE_CHARS) return null;
  let line = 1;
  let col = 1;
  let targetOffset = content.length;
  for (let i = 0; i < content.length; i++) {
    if (line === targetLine && col >= targetCol) { targetOffset = i; break; }
    if (line > targetLine) { targetOffset = i; break; }
    if (content[i] === '\n') { line++; col = 1; } else col++;
  }

  type Frame = { kind: 'obj'; key: string | null; expectingValue: boolean } | { kind: 'arr'; index: number };
  const stack: Frame[] = [];
  let inStr = false;
  let lastStr = '';
  let strBuf = '';

  for (let i = 0; i < targetOffset; i++) {
    const ch = content[i];
    if (inStr) {
      if (ch === '\\') { i++; continue; }
      if (ch === '"') { inStr = false; lastStr = strBuf; }
      else if (strBuf.length < 80) strBuf += ch;
      continue;
    }
    if (ch === '/' && content[i + 1] === '/') {
      while (i < targetOffset && content[i] !== '\n') i++;
      continue;
    }
    if (ch === '"') { inStr = true; strBuf = ''; continue; }
    const top = stack[stack.length - 1];
    if (ch === '{') {
      stack.push({ kind: 'obj', key: null, expectingValue: false });
    } else if (ch === '[') {
      stack.push({ kind: 'arr', index: 0 });
    } else if (ch === '}' || ch === ']') {
      stack.pop();
    } else if (ch === ':' && top?.kind === 'obj' && !top.expectingValue) {
      top.key = lastStr;
      top.expectingValue = true;
    } else if (ch === ',' && top) {
      if (top.kind === 'obj') { top.expectingValue = false; top.key = null; }
      else top.index += 1;
    }
  }

  if (stack.length === 0) return '$';
  let path = '$';
  for (const f of stack) {
    if (f.kind === 'arr') path += `[${f.index}]`;
    else if (f.key != null) path += /^[A-Za-z_$][\w$]*$/.test(f.key) ? `.${f.key}` : `["${f.key}"]`;
  }
  return path;
}

// ── JSON / JSONL Pretty-Print, Minify & Recursive Key Sort ───────────────────

function sortJsonDeep(val: unknown): unknown {
  if (Array.isArray(val)) return val.map(sortJsonDeep);
  if (val && typeof val === 'object') {
    const sorted: Record<string, unknown> = {};
    for (const k of Object.keys(val as Record<string, unknown>).sort((a, b) => a.localeCompare(b))) {
      sorted[k] = sortJsonDeep((val as Record<string, unknown>)[k]);
    }
    return sorted;
  }
  return val;
}

export function transformJsonText(
  content: string,
  mode: 'pretty' | 'minify' | 'sort',
  isJsonl = false,
): { ok: true; text: string } | { ok: false; error: string } {
  try {
    if (isJsonl) {
      const lines = content.split(/\r?\n/).filter((l) => l.trim().length > 0);
      const out = lines.map((l) => {
        const parsed = JSON.parse(l);
        const target = mode === 'sort' ? sortJsonDeep(parsed) : parsed;
        return mode === 'pretty' ? JSON.stringify(target, null, 2) : JSON.stringify(target);
      });
      return { ok: true, text: `${out.join('\n')}\n` };
    }
    const parsed = JSON.parse(content);
    const target = mode === 'sort' ? sortJsonDeep(parsed) : parsed;
    const formatted = mode === 'minify' ? JSON.stringify(target) : `${JSON.stringify(target, null, 2)}\n`;
    return { ok: true, text: formatted };
  } catch (e) {
    return { ok: false, error: loadErrorText(e) || 'Invalid JSON syntax' };
  }
}

// ── Fast Local Dive-In Resolver (Shell Functions/source, JSON $ref, Paths) ───

function joinNormalizedPath(baseDir: string, rel: string): string {
  if (rel.startsWith('/')) return rel;
  const parts = `${baseDir}/${rel}`.split('/');
  const stack: string[] = [];
  for (const p of parts) {
    if (!p || p === '.') continue;
    if (p === '..') stack.pop();
    else stack.push(p);
  }
  return `/${stack.join('/')}`;
}

export function resolveLocalFastDive(
  fromPath: string,
  rootPath: string,
  content: string,
  line: number,
  symbol?: string,
): { path: string; line: number; column: number } | null {
  const kind = detectIdeDocKind(fromPath);
  const lines = content.split(/\r?\n/);
  const curLineText = lines[Math.max(0, Math.min(lines.length - 1, line - 1))] ?? '';
  const dir = fromPath.slice(0, Math.max(0, fromPath.lastIndexOf('/'))) || rootPath;

  // 1. JSON `$ref` or file path string (`"./schema.json#/defs/Item"` or `"#/defs/Item"`).
  if (kind === 'json' || kind === 'jsonl') {
    const token = symbol?.trim() ?? '';
    if (token.startsWith('#/') || token.includes('.json') || token.startsWith('./') || token.startsWith('../')) {
      const [filePart, pointer] = token.split('#');
      const targetPath = filePart ? joinNormalizedPath(dir, filePart) : fromPath;
      let targetLine = 1;
      if (!filePart && pointer) {
        const lastKey = pointer.split('/').filter(Boolean).pop();
        if (lastKey) {
          const idx = lines.findIndex((l) => l.includes(`"${lastKey}"`) && l.includes(':'));
          if (idx >= 0) targetLine = idx + 1;
        }
      }
      return { path: targetPath, line: targetLine, column: 1 };
    }
  }

  // 2. Shell `source ./foo.sh` / `. ./foo.sh` or local shell function definition!
  if (kind === 'shell') {
    const mSrc = /(?:^|[;&|]\s*)(?:source|\.)\s+["']?([^\s"';&|]+)["']?/.exec(curLineText);
    if (mSrc && mSrc[1]) {
      const rawP = mSrc[1].replace(/^\$\{?PWD\}?\//, './');
      if (!rawP.startsWith('$')) {
        return { path: joinNormalizedPath(dir, rawP), line: 1, column: 1 };
      }
    }
    const cleanSym = (symbol ?? '').replace(/^\$/, '').trim();
    if (cleanSym) {
      const fnRe = new RegExp(`^\\s*(?:function\\s+${cleanSym}(?:\\s*\\(\\s*\\))?|${cleanSym}\\s*\\(\\s*\\))\\s*\\{?`);
      const fnIdx = lines.findIndex((l) => fnRe.test(l));
      if (fnIdx >= 0) return { path: fromPath, line: fnIdx + 1, column: 1 };
      const varRe = new RegExp(`^\\s*(?:export\\s+|local\\s+|readonly\\s+)?${cleanSym}=`);
      const varIdx = lines.findIndex((l) => varRe.test(l));
      if (varIdx >= 0) return { path: fromPath, line: varIdx + 1, column: 1 };
    }
  }

  // 3. Explicit relative file path token (`./foo.ts`, `../bar.py`, `./config.yaml`).
  if (symbol && (symbol.startsWith('./') || symbol.startsWith('../')) && /\.[a-z0-9]+$/i.test(symbol)) {
    return { path: joinNormalizedPath(dir, symbol), line: 1, column: 1 };
  }

  return null;
}
