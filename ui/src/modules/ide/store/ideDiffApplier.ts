// Pure, zero-DOM helpers and shared types for IDE Agent context assembly,
// multi-harness Threads resolution, fenced code proposal extraction
// (`otto-fake-claude` safe), range splicing, and sub-2ms line diffing.

import { escapeHtml, unescapeHtml } from '../../../lib/html.ts';

export type IdeContextKind =
  | 'selection'
  | 'file'
  | 'symbol'
  | 'diagnostic'
  | 'test_failure'
  | 'debug_frame'
  | 'run_output';

export interface IdeContextChip {
  id: string; key: string; kind: IdeContextKind; label: string; sublabel?: string;
  path?: string; startLine?: number; endLine?: number; language?: string; snippet: string;
}
export interface IdeDiffLine {
  kind: 'same' | 'add' | 'del'; text: string; oldLine: number | null; newLine: number | null;
}
export interface IdeCodeProposal {
  id: string; messageId: string; path: string; relPath: string; language: string; code: string;
  range?: { from: number; to: number; startLine: number; endLine: number };
  diffLines: IdeDiffLine[]; addedCount: number; removedCount: number; applied: boolean;
}
export interface IdeInlinePromptState {
  open: boolean; path: string; relPath: string; language: string; from: number; to: number;
  startLine: number; endLine: number; selectedText: string; query: string;
  provider: string; model: string | null; effort: string | null;
  busy: boolean; error: string | null; proposal: IdeCodeProposal | null;
}
export interface ParsedPromptContextSummary {
  kind: IdeContextKind; label: string; path?: string; startLine?: number; endLine?: number; language?: string;
}
export interface ParsedFenceMeta {
  language: string; path: string | null; lineRange: { startLine: number; endLine: number } | null; isPlaceholder: boolean;
}
export interface ExtractProposalsOptions {
  messageId?: string; fallbackPath?: string | null;
  fallbackRange?: { from?: number; to?: number; startLine: number; endLine: number } | null;
  preferRange?: boolean; resolvePath?: (rawPath: string) => string; relativePath?: (fullPath: string) => string;
  getFileContent?: (fullPath: string) => string; appliedIds?: Readonly<Record<string, string>>;
}
export interface MatchIdeThreadInput {
  sessionId: string;
  activeSessionId?: string | null;
  workspaceId: string;
  targetWorkspaceId: string;
  cwd?: string | null;
  projectRoots?: readonly (string | null | undefined)[];
}

export const MAX_CONTEXT_CHIPS = 12;
export const MAX_CHIP_SNIPPET_CHARS = 8_000;
export const MAX_IDE_CONTEXT_PAYLOAD_CHARS = 28_000;
export const IDE_PROPOSAL_INSTRUCTION =
  'When proposing code changes for a file, wrap the updated file or selection in a fenced block like ```<lang> path=<relative-path>\n...\n```.';
export const IDE_RUNTIME_PROVIDERS = ['claude', 'codex', 'gemini', 'agy', 'grok', 'opencode', 'pi'] as const;

const NON_CODE_FENCE_LANGS = new Set(['sh', 'bash', 'zsh', 'shell', 'console', 'mermaid', 'diff', 'patch']);
const EXT_LANG_MAP: Record<string, string> = {
  go: 'go', java: 'java', class: 'java', rs: 'rust',
  py: 'python', pyi: 'python', pyw: 'python',
  ts: 'typescript', tsx: 'tsx', mts: 'typescript', cts: 'typescript',
  js: 'javascript', mjs: 'javascript', cjs: 'javascript', jsx: 'jsx',
  json: 'json', jsonc: 'json', jsonl: 'jsonl', ndjson: 'jsonl',
  csv: 'csv', tsv: 'tsv',
  sh: 'shell', bash: 'shell', zsh: 'shell', bats: 'shell', bashrc: 'shell', zshrc: 'shell',
  yaml: 'yaml', yml: 'yaml', md: 'markdown', sql: 'sql',
  html: 'html', css: 'css', svelte: 'svelte', toml: 'toml',
};

const sanitizeContextBody = (t: string): string => t.replace(/<\/ide_context>/gi, '<\\/ide_context>');
const normPath = (p: string | null | undefined): string => {
  const clean = (p ?? '').trim().replace(/\/+$/, '').replace(/^\/private\/(var|tmp)(?=\/|$)/, '/$1');
  return clean === '~' ? '' : clean;
};

/** Build the unified Threads harness list (`claude`, `codex`, `agy`, `gemini`, `grok`, `opencode`, `pi`, + custom CLI harnesses), never hardcoding a 3-provider subset. */
export function resolveIdeHarnesses(
  registeredProviders: readonly string[],
  extraProviders: readonly (string | null | undefined)[] = [],
): string[] {
  const merged: string[] = [];
  const push = (raw: string | null | undefined): void => {
    const clean = (raw ?? '').trim();
    if (clean && clean !== 'shell' && !merged.includes(clean)) merged.push(clean);
  };
  for (let i = 0; i < IDE_RUNTIME_PROVIDERS.length; i++) {
    const base = IDE_RUNTIME_PROVIDERS[i];
    if (base === 'claude' || base === 'codex' || base === 'agy' || registeredProviders.includes(base)) push(base);
  }
  for (let i = 0; i < registeredProviders.length; i++) push(registeredProviders[i]);
  for (let i = 0; i < IDE_RUNTIME_PROVIDERS.length; i++) push(IDE_RUNTIME_PROVIDERS[i]);
  for (let i = 0; i < extraProviders.length; i++) push(extraProviders[i]);
  return merged;
}

/** Match any structured runtime or PTY thread belonging to the current IDE workspace or any attached project folder. */
export function matchesIdeProjectThread(input: MatchIdeThreadInput): boolean {
  if (input.activeSessionId && input.sessionId === input.activeSessionId) return true;
  const roots = (input.projectRoots ?? []).map(normPath).filter(Boolean);
  const cwd = normPath(input.cwd);
  if (roots.length > 0 && cwd) {
    for (let i = 0; i < roots.length; i++) {
      const r = roots[i];
      if (cwd === r || cwd.startsWith(`${r}/`) || r.startsWith(`${cwd}/`)) return true;
    }
    return false;
  }
  return Boolean(input.targetWorkspaceId) && input.workspaceId === input.targetWorkspaceId;
}

export function truncateSnippet(text: string, maxChars = MAX_CHIP_SNIPPET_CHARS): string {
  const cap = Math.max(0, maxChars);
  const norm = text.replace(/\r\n/g, '\n');
  if (norm.length <= cap) return norm;
  const suffix = '\n… [truncated]';
  return cap <= suffix.length ? norm.slice(0, cap) : `${norm.slice(0, cap - suffix.length)}${suffix}`;
}

export function detectProposalLanguage(path: string, explicitLang = ''): string {
  const clean = explicitLang.trim().toLowerCase();
  if (clean && !clean.includes('/') && !clean.includes('.')) return clean;
  if (/^(?:jdt:\/\/|jar:)/i.test(path.trim())) return 'java';
  const noQuery = path.split(/[?#]/)[0] ?? path;
  const base = noQuery.split(/[\\/]/).pop() ?? noQuery;
  const lowerBase = base.toLowerCase();
  if (lowerBase === '.bashrc' || lowerBase === '.zshrc' || lowerBase === '.bash_profile' || lowerBase === '.zprofile') return 'shell';
  const ext = base.includes('.') ? (base.split('.').pop()?.toLowerCase() ?? '') : '';
  return EXT_LANG_MAP[ext] ?? (clean || 'txt');
}

/** Strip `<ide_context>...</ide_context>` blocks FIRST so echoed context in `otto-fake-claude` never leaks as a proposal. */
export function stripIdeContextBlocks(text: string): string {
  if (!text) return '';
  return text.replace(/<ide_context\b[^>]*>[\s\S]*?<\/ide_context>[ \t]*\r?\n?/gi, '').replace(IDE_PROPOSAL_INSTRUCTION, '').replace(/^echo:[ \t]+(?=```)/i, 'echo:\n').replace(/\n{3,}/g, '\n\n').trim();
}

export function parseIdeContextChipsFromPrompt(text: string): ParsedPromptContextSummary[] {
  const out: ParsedPromptContextSummary[] = [];
  if (!text) return out;
  for (const block of text.matchAll(/<ide_context\b[^>]*>([\s\S]*?)<\/ide_context>/gi)) {
    for (const tag of block[1].matchAll(/<context\b([^>]*)>/gi)) {
      const attrs = tag[1];
      const read = (n: string): string | undefined => { const m = new RegExp(`\\b${n}="([^"]*)"`, 'i').exec(attrs); return m ? unescapeHtml(m[1]) : undefined; };
      const label = read('label') ?? '';
      if (!label) continue;
      const sLine = Number(read('start_line') ?? ''), eLine = Number(read('end_line') ?? '');
      out.push({ kind: (read('kind') ?? 'file') as IdeContextKind, label, path: read('path'), startLine: Number.isFinite(sLine) && sLine > 0 ? sLine : undefined, endLine: Number.isFinite(eLine) && eLine > 0 ? eLine : undefined, language: read('language') });
    }
  }
  return out;
}

export function buildIdeAgentPrompt(
  userPrompt: string, contexts: readonly IdeContextChip[] = [],
  opts: {
    workspaceRoot?: string; activePath?: string | null; planMode?: boolean;
    maxContextChars?: number; omitWhenEmpty?: boolean;
  } = {},
): string {
  const cleanUser = userPrompt.trim();
  if (opts.omitWhenEmpty && contexts.length === 0 && !opts.planMode) return cleanUser;
  const maxChars = Math.max(512, opts.maxContextChars ?? MAX_IDE_CONTEXT_PAYLOAD_CHARS);
  const openTag = '<ide_context>\n', closeTag = '\n</ide_context>';
  const planNote = opts.planMode ? '\nPlan mode: analyze, explain, and propose code blocks only; do not modify files on disk directly.' : '';
  const instructions = `<instructions>\n${IDE_PROPOSAL_INSTRUCTION}${planNote}\n</instructions>`;
  const wsHeader = opts.workspaceRoot || opts.activePath ? `<workspace${opts.workspaceRoot ? ` root="${escapeHtml(opts.workspaceRoot)}"` : ''}${opts.activePath ? ` active_file="${escapeHtml(opts.activePath)}"` : ''} />\n` : '';
  let remaining = Math.max(0, maxChars - (openTag.length + closeTag.length + wsHeader.length + instructions.length));
  const entries: string[] = [];
  for (let i = 0; i < Math.min(contexts.length, MAX_CONTEXT_CHIPS) && remaining > 64; i++) {
    const chip = contexts[i];
    const attrs = [`kind="${escapeHtml(chip.kind)}"`, `label="${escapeHtml(chip.label)}"`, chip.path ? `path="${escapeHtml(chip.path)}"` : '', chip.startLine ? `start_line="${chip.startLine}"` : '', chip.endLine ? `end_line="${chip.endLine}"` : '', chip.language ? `language="${escapeHtml(chip.language)}"` : ''].filter(Boolean).join(' ');
    const head = `<context ${attrs}>\n`, tail = '\n</context>\n', wrapCost = head.length + tail.length;
    if (remaining <= wrapCost + 16) break;
    const rawBody = sanitizeContextBody(truncateSnippet(chip.snippet, MAX_CHIP_SNIPPET_CHARS));
    const fitted = rawBody.length + wrapCost <= remaining ? rawBody : truncateSnippet(rawBody, remaining - wrapCost);
    const entry = `${head}${fitted}${tail}`;
    entries.push(entry);
    remaining -= entry.length;
  }
  const contextXml = `${openTag}${wsHeader}${entries.join('')}${instructions}${closeTag}`;
  return cleanUser ? `${contextXml}\n\n${cleanUser}` : contextXml;
}

export function offsetsForLineRange(content: string, startLine: number, endLine: number): { from: number; to: number; startLine: number; endLine: number; text: string } {
  if (!content) return { from: 0, to: 0, startLine: 1, endLine: 1, text: '' };
  const sTarget = Math.max(1, Math.min(startLine, endLine));
  const eTarget = Math.max(sTarget, endLine);
  let line = 1, from = 0, foundStart = sTarget === 1;
  for (let i = 0; i < content.length; i++) {
    if (content.charCodeAt(i) === 10) {
      if (line === eTarget) return { from, to: i, startLine: sTarget, endLine: line, text: content.slice(from, i) };
      if (++line === sTarget) { from = i + 1; foundStart = true; }
    }
  }
  if (!foundStart) from = content.length;
  return { from, to: content.length, startLine: Math.min(sTarget, line), endLine: line, text: content.slice(from) };
}

export function lineRangeFromOffsets(content: string, anchor: number, head: number): { from: number; to: number; startLine: number; endLine: number; text: string } {
  const len = content.length;
  const lo = Math.max(0, Math.min(len, Math.min(anchor, head)));
  const hi = Math.max(0, Math.min(len, Math.max(anchor, head)));
  let startLine = 1;
  for (let i = 0; i < lo; i++) if (content.charCodeAt(i) === 10) startLine++;
  if (lo === hi) {
    const from = lo === 0 ? 0 : content.lastIndexOf('\n', Math.max(0, lo - 1)) + 1;
    const nextNl = content.indexOf('\n', lo);
    const to = nextNl === -1 ? len : nextNl;
    return { from, to, startLine, endLine: startLine, text: content.slice(from, to) };
  }
  let endLine = startLine;
  const scanEnd = hi > lo && content.charCodeAt(hi - 1) === 10 ? hi - 1 : hi;
  for (let i = lo; i < scanEnd; i++) if (content.charCodeAt(i) === 10) endLine++;
  return { from: lo, to: hi, startLine, endLine, text: content.slice(lo, hi) };
}

export function spliceCodeRange(content: string, replacement: string, range?: { from?: number; to?: number; startLine?: number; endLine?: number }): string {
  const normCode = replacement.replace(/\r\n/g, '\n');
  if (!range) return normCode;
  let from: number, to: number;
  if (typeof range.from === 'number' && typeof range.to === 'number' && range.from >= 0 && range.to >= range.from && range.to <= content.length) {
    from = range.from; to = range.to;
  } else if (typeof range.startLine === 'number' && range.startLine >= 1) {
    const resolved = offsetsForLineRange(content, range.startLine, range.endLine ?? range.startLine);
    from = resolved.from; to = resolved.to;
  } else return normCode;
  const hadTrailingNl = to > from && content.charCodeAt(to - 1) === 10;
  let insert = normCode;
  if (hadTrailingNl && !insert.endsWith('\n')) insert += '\n';
  else if (!hadTrailingNl && insert.endsWith('\n') && (to === content.length || content.charCodeAt(to) === 10)) insert = insert.slice(0, -1);
  return `${content.slice(0, from)}${insert}${content.slice(to)}`;
}

export function parseFenceInfo(info: string): ParsedFenceMeta {
  const trimmed = info.trim();
  if (!trimmed) return { language: '', path: null, lineRange: null, isPlaceholder: false };
  if (/<lang>|<relative-path>|<path>/i.test(trimmed)) return { language: '', path: null, lineRange: null, isPlaceholder: true };
  const kv = /\b(?:path|file|filepath|filename|title)\s*=\s*(?:"([^"]+)"|'([^']+)'|(\S+))/i.exec(trimmed);
  let rawPath: string | null = kv ? (kv[1] ?? kv[2] ?? kv[3] ?? null) : null;
  const tokens = trimmed.replace(/\b[A-Za-z_-]+\s*=\s*(?:"[^"]*"|'[^']*'|\S+)/g, ' ').trim().split(/\s+/).filter(Boolean);
  let language = '';
  for (let i = 0; i < tokens.length; i++) {
    const tok = tokens[i].replace(/^['"`]|['"`]$/g, '');
    const isPath = !tok.includes('<') && !tok.includes('>') && tok !== '...' &&
      (tok.includes('/') || tok.includes('\\') || /^@?[A-Za-z0-9_.-]+\.[A-Za-z0-9]{1,10}(?::L?\d+(?:-L?\d+)?)?$/.test(tok));
    if (!rawPath && isPath) rawPath = tok;
    else if (!language && /^[A-Za-z0-9+#_-]+$/.test(tok)) language = tok.toLowerCase();
  }
  if (!rawPath) return { language, path: null, lineRange: null, isPlaceholder: false };
  if (rawPath.includes('<') || rawPath.includes('>') || rawPath === '...') return { language, path: null, lineRange: null, isPlaceholder: true };
  const clean = rawPath.trim().replace(/^['"`]|['"`]$/g, '').replace(/^@/, '');
  const m = /^(.+?):L?(\d+)(?:-L?(\d+))?$/i.exec(clean);
  const path = m ? m[1] : clean;
  const lineRange = m ? { startLine: Math.min(Number(m[2]), Number(m[3] ?? m[2])), endLine: Math.max(Number(m[2]), Number(m[3] ?? m[2])) } : null;
  return { language: detectProposalLanguage(path, language), path, lineRange, isPlaceholder: false };
}

export function extractCodeProposals(
  assistantText: string, fallbackPath?: string | null,
  fallbackRange?: { from?: number; to?: number; startLine: number; endLine: number } | null, opts: ExtractProposalsOptions = {},
): IdeCodeProposal[] {
  const cleaned = stripIdeContextBlocks(assistantText);
  if (!cleaned) return [];
  const proposals: IdeCodeProposal[] = [];
  const fenceRe = /^(?:[ \t]*|echo:[ \t]*)```([^\r\n`]*)\r?\n([\s\S]*?)^[ \t]*```[ \t]*$/gm;
  const resolve = opts.resolvePath ?? ((p: string) => p);
  const toRel = opts.relativePath ?? ((p: string) => p);
  const msgId = opts.messageId ?? 'msg';
  let idx = 0;
  for (const match of cleaned.matchAll(fenceRe)) {
    const meta = parseFenceInfo(match[1] ?? '');
    if (meta.isPlaceholder) continue;
    const rawCode = (match[2] ?? '').replace(/\r\n/g, '\n').replace(/\n$/, '');
    if (!rawCode.trim() || rawCode.trim() === '...') continue;
    const explicitPath = meta.path;
    const candidatePath = explicitPath ?? fallbackPath ?? null;
    if (!candidatePath) continue;
    if (!explicitPath && NON_CODE_FENCE_LANGS.has(meta.language)) {
      const targetLang = detectProposalLanguage(candidatePath);
      if (targetLang !== 'shell' && targetLang !== meta.language) continue;
    }
    const fullPath = resolve(candidatePath);
    const relPath = toRel(fullPath);
    const fileContent = opts.getFileContent ? opts.getFileContent(fullPath) : '';
    const useRange = meta.lineRange ?? ((!explicitPath || opts.preferRange) && fallbackRange ? fallbackRange : null);
    let resolvedRange: { from: number; to: number; startLine: number; endLine: number } | undefined;
    let baseline = fileContent;
    if (useRange) {
      const slice = offsetsForLineRange(fileContent, useRange.startLine, useRange.endLine);
      const from = !meta.lineRange && 'from' in useRange && typeof useRange.from === 'number' ? useRange.from : slice.from;
      const to = !meta.lineRange && 'to' in useRange && typeof useRange.to === 'number' ? useRange.to : slice.to;
      const outerPrefix = fileContent.slice(0, from).trim();
      if (meta.lineRange || !(outerPrefix.length > 0 && rawCode.trimStart().startsWith(outerPrefix.split('\n')[0]))) {
        resolvedRange = { from, to, startLine: useRange.startLine, endLine: useRange.endLine };
        baseline = fileContent ? fileContent.slice(from, to) : '';
      }
    }
    const diffLines = computeLineDiff(baseline, rawCode);
    let addedCount = 0, removedCount = 0;
    for (let i = 0; i < diffLines.length; i++) {
      if (diffLines[i].kind === 'add') addedCount++;
      else if (diffLines[i].kind === 'del') removedCount++;
    }
    const id = `${msgId}:p${idx++}`;
    proposals.push({
      id, messageId: msgId, path: fullPath, relPath, language: detectProposalLanguage(relPath, meta.language),
      code: rawCode, ...(resolvedRange ? { range: resolvedRange } : {}), diffLines, addedCount, removedCount,
      applied: Boolean(opts.appliedIds && Object.prototype.hasOwnProperty.call(opts.appliedIds, id)),
    });
  }
  return proposals;
}

type RawDiffOp = { kind: 'same' | 'add' | 'del'; text: string };

function lcsOps(a: readonly string[], a0: number, a1: number, b: readonly string[], b0: number, b1: number, out: RawDiffOp[]): void {
  const m = a1 - a0, n = b1 - b0, cols = n + 1;
  const dp = new Int32Array((m + 1) * cols);
  for (let i = m - 1; i >= 0; i--) {
    const ai = a[a0 + i], row = i * cols, nextRow = (i + 1) * cols;
    for (let j = n - 1; j >= 0; j--) dp[row + j] = ai === b[b0 + j] ? dp[nextRow + j + 1] + 1 : Math.max(dp[nextRow + j], dp[row + j + 1]);
  }
  let i = 0, j = 0;
  while (i < m && j < n) {
    if (a[a0 + i] === b[b0 + j]) { out.push({ kind: 'same', text: a[a0 + i] }); i++; j++; }
    else if (dp[(i + 1) * cols + j] >= dp[i * cols + j + 1]) out.push({ kind: 'del', text: a[a0 + i++] });
    else out.push({ kind: 'add', text: b[b0 + j++] });
  }
  while (i < m) out.push({ kind: 'del', text: a[a0 + i++] });
  while (j < n) out.push({ kind: 'add', text: b[b0 + j++] });
}

function uniqueAnchors(a: readonly string[], a0: number, a1: number, b: readonly string[], b0: number, b1: number): [number, number][] {
  const posA = new Map<string, number>(), posB = new Map<string, number>();
  for (let i = a0; i < a1; i++) posA.set(a[i], posA.has(a[i]) ? -1 : i);
  for (let j = b0; j < b1; j++) if ((posA.get(b[j]) ?? -1) >= 0) posB.set(b[j], posB.has(b[j]) ? -1 : j);
  const pairs: [number, number][] = [];
  for (let i = a0; i < a1; i++) {
    const ia = posA.get(a[i]) ?? -1, ib = posB.get(a[i]) ?? -1;
    if (ia >= 0 && ib >= 0) pairs.push([ia, ib]);
  }
  if (pairs.length === 0) return [];
  const tails: number[] = [], prev = new Int32Array(pairs.length).fill(-1);
  for (let k = 0; k < pairs.length; k++) {
    let lo = 0, hi = tails.length;
    while (lo < hi) { const mid = (lo + hi) >> 1; if (pairs[tails[mid]][1] < pairs[k][1]) lo = mid + 1; else hi = mid; }
    if (lo > 0) prev[k] = tails[lo - 1];
    tails[lo] = k;
  }
  const lis: [number, number][] = [];
  for (let cur = tails[tails.length - 1]; cur >= 0; cur = prev[cur]) lis.push(pairs[cur]);
  return lis.reverse();
}

function diffRange(a: readonly string[], a0: number, a1: number, b: readonly string[], b0: number, b1: number, out: RawDiffOp[], allowAnchor: boolean): void {
  while (a0 < a1 && b0 < b1 && a[a0] === b[b0]) { out.push({ kind: 'same', text: a[a0++] }); b0++; }
  let suf = 0;
  while (a0 < a1 - suf && b0 < b1 - suf && a[a1 - 1 - suf] === b[b1 - 1 - suf]) suf++;
  const ae = a1 - suf, be = b1 - suf, m = ae - a0, n = be - b0;
  if (m === 0) for (let j = b0; j < be; j++) out.push({ kind: 'add', text: b[j] });
  else if (n === 0) for (let i = a0; i < ae; i++) out.push({ kind: 'del', text: a[i] });
  else if (m + n <= 600 && m * n <= 90_000) lcsOps(a, a0, ae, b, b0, be, out);
  else if (allowAnchor) {
    const anchors = uniqueAnchors(a, a0, ae, b, b0, be);
    let ca = a0, cb = b0;
    for (let k = 0; k < anchors.length; k++) {
      const [ia, ib] = anchors[k];
      if (ia > ca || ib > cb) diffRange(a, ca, ia, b, cb, ib, out, false);
      out.push({ kind: 'same', text: a[ia] });
      ca = ia + 1; cb = ib + 1;
    }
    if (ca < ae || cb < be) diffRange(a, ca, ae, b, cb, be, out, false);
  } else if (m * n <= 40_000) lcsOps(a, a0, ae, b, b0, be, out);
  else {
    for (let i = a0; i < ae; i++) out.push({ kind: 'del', text: a[i] });
    for (let j = b0; j < be; j++) out.push({ kind: 'add', text: b[j] });
  }
  for (let s = ae; s < a1; s++) out.push({ kind: 'same', text: a[s] });
}

export function computeLineDiff(before: string, after: string): IdeDiffLine[] {
  const split = (s: string): string[] => {
    if (!s) return [];
    const n = s.replace(/\r\n/g, '\n');
    const t = n.endsWith('\n') ? n.slice(0, -1) : n;
    return t.length === 0 ? [''] : t.split('\n');
  };
  const a = split(before), b = split(after);
  if (a.length === 0 && b.length === 0) return [];
  const raw: RawDiffOp[] = [];
  diffRange(a, 0, a.length, b, 0, b.length, raw, true);
  let oldNo = 1, newNo = 1;
  return raw.map((op) =>
    op.kind === 'same' ? { kind: 'same', text: op.text, oldLine: oldNo++, newLine: newNo++ }
    : op.kind === 'del' ? { kind: 'del', text: op.text, oldLine: oldNo++, newLine: null }
    : { kind: 'add', text: op.text, oldLine: null, newLine: newNo++ },
  );
}
