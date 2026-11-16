// Pure & CodeMirror 6 helpers for IDE LSP gating, multi-repo module-root scoping,
// language-specific `initializationOptions`, dot-triggered completion with atomic
// `additionalTextEdits` auto-imports, and `textDocument/semanticTokens/full` decoding.

import { startCompletion, type Completion, type CompletionContext, type CompletionResult } from '@codemirror/autocomplete';
import { EditorSelection, EditorState, StateEffect, StateField, type ChangeSpec, type Extension, type Range } from '@codemirror/state';
import { Decoration, EditorView, ViewPlugin, type DecorationSet, type ViewUpdate } from '@codemirror/view';
import { isSecretIdePath } from './cm-ide-drafts.ts';

export const MAX_POOLED_LSP_CLIENTS = 3;
export const IDLE_CLOSE_MS = 180_000;

export interface LspPos {
  line: number;
  character: number;
}

export interface LspRange {
  start: LspPos;
  end: LspPos;
}

export interface RawLspTextEdit {
  range: LspRange;
  newText: string;
}

export interface RawLspCompletionItem {
  label: string;
  kind?: number;
  detail?: string;
  sortText?: string;
  filterText?: string;
  insertText?: string;
  insertTextFormat?: number;
  textEdit?: { range?: LspRange; insert?: LspRange; replace?: LspRange; newText: string };
  additionalTextEdits?: RawLspTextEdit[];
  data?: unknown;
}

export const LSP_CODE_LANGUAGES = new Set([
  'go',
  'rust',
  'java',
  'typescript',
  'javascript',
  'python',
]);

const NON_CODE_EXTENSIONS = new Set([
  'md', 'markdown', 'mdx', 'json', 'jsonc', 'jsonl', 'ndjson', 'yaml', 'yml',
  'toml', 'sql', 'csv', 'tsv', 'html', 'htm', 'css', 'scss', 'less', 'xml',
  'svg', 'txt', 'text', 'log', 'lock', 'env', 'ini', 'conf', 'cfg', 'properties',
  'dockerfile', 'sh', 'bash', 'zsh', 'fish', 'gitignore', 'dockerignore', 'proto',
]);

export function posToLsp(doc: EditorState['doc'], pos: number): LspPos {
  const clamped = Math.max(0, Math.min(doc.length, pos));
  const ln = doc.lineAt(clamped);
  return { line: ln.number - 1, character: clamped - ln.from };
}

export function lspToPos(doc: EditorState['doc'], p: LspPos): number {
  if (doc.lines === 0) return 0;
  const ln = doc.line(Math.max(1, Math.min(doc.lines, p.line + 1)));
  return Math.max(0, Math.min(doc.length, Math.min(ln.to, ln.from + Math.max(0, p.character))));
}

export function normalizeIdeLspLanguage(langOrExt?: string | null): string | null {
  const raw = (langOrExt ?? '').trim().replace(/^\.+/, '').toLowerCase();
  if (!raw) return null;
  if (raw === 'go' || raw === 'golang') return 'go';
  if (raw === 'rs' || raw === 'rust') return 'rust';
  if (raw === 'java') return 'java';
  if (raw === 'ts' || raw === 'tsx' || raw === 'mts' || raw === 'cts' || raw === 'typescript' || raw === 'typescriptreact') {
    return 'typescript';
  }
  if (raw === 'js' || raw === 'jsx' || raw === 'mjs' || raw === 'cjs' || raw === 'javascript' || raw === 'javascriptreact') {
    return 'javascript';
  }
  if (raw === 'py' || raw === 'pyi' || raw === 'python') return 'python';
  return null;
}

export function shouldAttachIdeLsp(
  filePath?: string | null,
  languageId?: string | null,
  rootPath?: string | null,
): boolean {
  if (rootPath !== undefined && (!rootPath || !rootPath.trim())) return false;
  if (!filePath || !filePath.trim()) return false;
  const clean = filePath.split('?')[0].replace(/^file:\/\//, '').trim();
  if (!clean || clean.startsWith('untitled:') || clean.startsWith('jdt://')) return false;
  if (isSecretIdePath(clean)) return false;

  const base = clean.slice(Math.max(clean.lastIndexOf('/'), clean.lastIndexOf('\\')) + 1).toLowerCase();
  if (base === 'dockerfile' || base.startsWith('dockerfile.') || base === 'makefile' || base === 'gnumakefile') {
    return false;
  }
  const dot = base.lastIndexOf('.');
  const ext = dot >= 0 ? base.slice(dot + 1) : '';
  if (ext && NON_CODE_EXTENSIONS.has(ext)) return false;

  const resolved = languageId ? normalizeIdeLspLanguage(languageId) : normalizeIdeLspLanguage(ext);
  return resolved !== null && LSP_CODE_LANGUAGES.has(resolved);
}

export function toAbsoluteIdeFilePath(rootPath: string, filePath: string): string {
  const cleanFile = filePath.split('?')[0].replace(/^file:\/\//, '').replace(/\\/g, '/').trim();
  if (!cleanFile) return '';
  if (cleanFile.startsWith('/') || /^[A-Za-z]:\//.test(cleanFile)) return cleanFile;
  const normRoot = rootPath.replace(/\\/g, '/').replace(/\/+$/, '');
  const rel = cleanFile.replace(/^\.\//, '').replace(/^\/+/, '');
  return normRoot ? `${normRoot}/${rel}` : `/${rel}`;
}

export function resolveIdeLspModuleRoot(
  rootPath: string,
  filePath: string,
  projectFolders?: readonly string[] | null,
): string {
  const normRoot = rootPath.replace(/\\/g, '/').replace(/\/+$/, '') || '/';
  if (!filePath || !filePath.trim()) return normRoot;
  const absFile = toAbsoluteIdeFilePath(normRoot, filePath);
  if (!absFile) return normRoot;

  let best = '';
  if (projectFolders && projectFolders.length > 0) {
    for (let i = 0; i < projectFolders.length; i++) {
      const rawFolder = (projectFolders[i] ?? '').replace(/\\/g, '/').replace(/\/+$/, '').trim();
      if (!rawFolder) continue;
      const absFolder = rawFolder.startsWith('/') || /^[A-Za-z]:\//.test(rawFolder)
        ? rawFolder
        : `${normRoot}/${rawFolder.replace(/^\.\//, '')}`;
      if (absFile === absFolder || absFile.startsWith(`${absFolder}/`)) {
        if (absFolder.length > best.length) best = absFolder;
      }
    }
  }
  if (best) return best;
  if (normRoot !== '/' && absFile !== normRoot && !absFile.startsWith(`${normRoot}/`)) {
    const slash = absFile.lastIndexOf('/');
    return slash > 0 ? absFile.slice(0, slash) : '/';
  }
  return normRoot;
}

import { extractGoBuildTagsFromSource, GO_BUILD_TAG_DIAG_RE, rewriteGoBuildTagDiagnostic } from '../goBuildTags.ts';

export { extractGoBuildTagsFromSource, rewriteGoBuildTagDiagnostic };

export interface RawLspDiagnosticLike {
  range: LspRange;
  severity?: number;
  code?: string | number | { value?: string | number } | null;
  source?: string | null;
  message: string | { value?: string };
}

export function lspDiagnosticDedupeKey(d: RawLspDiagnosticLike): string {
  const codeVal =
    d.code != null && typeof d.code === 'object' && 'value' in d.code
      ? String(d.code.value ?? '')
      : String(d.code ?? '');
  const msgText =
    typeof d.message === 'string'
      ? d.message
      : d.message && typeof d.message === 'object' && 'value' in d.message
        ? String(d.message.value ?? '')
        : String(d.message ?? '');
  return `${d.range.start.line}:${d.range.start.character}-${d.range.end.line}:${d.range.end.character}|${d.severity ?? ''}|${codeVal}|${d.source ?? ''}|${msgText}`;
}

export function dedupeLspDiagnostics<T extends RawLspDiagnosticLike>(diagnostics: readonly T[] | undefined | null): T[] {
  if (!diagnostics || diagnostics.length <= 1) return diagnostics ? [...diagnostics] : [];
  const seen = new Set<string>();
  const out: T[] = [];
  for (let i = 0; i < diagnostics.length; i++) {
    const d = diagnostics[i];
    if (!d?.range?.start || !d.range.end) {
      out.push(d);
      continue;
    }
    const key = lspDiagnosticDedupeKey(d);
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(d);
  }
  return out;
}

export function dedupeLspJsonRpcMessage<T>(
  message: T,
  getDocTextForUri?: ((uri: string) => string | null | undefined) | null,
): T {
  if (!message || typeof message !== 'object') return message;
  const msg = message as {
    method?: unknown;
    params?: { uri?: string; diagnostics?: RawLspDiagnosticLike[] } | null;
  };
  if (msg.method !== 'textDocument/publishDiagnostics' || !Array.isArray(msg.params?.diagnostics)) {
    return message;
  }
  const raw = msg.params.diagnostics;
  const uri = typeof msg.params.uri === 'string' ? msg.params.uri : '';
  let docTextFetched = false;
  let cachedDocText: string | null | undefined = undefined;
  let rewrittenAny = false;

  const rewritten = raw.map((d) => {
    const rawMsg =
      typeof d.message === 'string'
        ? d.message
        : d.message && typeof d.message === 'object' && 'value' in d.message
          ? String(d.message.value ?? '')
          : String(d.message ?? '');
    const codeStr =
      d.code != null && typeof d.code === 'object' && 'value' in d.code
        ? String(d.code.value ?? '')
        : d.code != null
          ? String(d.code)
          : '';
    if (codeStr !== 'otto_go_build_tag' && !GO_BUILD_TAG_DIAG_RE.test(rawMsg)) return d;
    if (!docTextFetched) {
      docTextFetched = true;
      cachedDocText = uri && getDocTextForUri ? getDocTextForUri(uri) : null;
    }
    const rw = rewriteGoBuildTagDiagnostic(rawMsg, d.code, cachedDocText);
    if (!rw.isBuildTagDiag) return d;
    rewrittenAny = true;
    return {
      ...d,
      severity: d.severity === 1 ? 2 : (d.severity ?? 2),
      code: 'otto_go_build_tag',
      source: d.source === 'go list' ? 'gopls (build tags)' : (d.source ?? 'gopls'),
      message: rw.message,
    };
  });

  const deduped = dedupeLspDiagnostics(rewritten);
  if (!rewrittenAny && deduped.length === raw.length) return message;
  return {
    ...(message as Record<string, unknown>),
    params: {
      ...(msg.params as Record<string, unknown>),
      diagnostics: deduped,
    },
  } as unknown as T;
}

export function buildLspInitializationOptions(languageId: string): Record<string, unknown> | undefined {
  const lang = normalizeIdeLspLanguage(languageId) ?? languageId.toLowerCase();
  if (lang === 'go') {
    return {
      buildFlags: ['-mod=mod'],
      completeUnimported: true,
      usePlaceholders: false,
      deepCompletion: true,
      matcher: 'Fuzzy',
      staticcheck: false,
      directoryFilters: ['-**/vendor', '-**/node_modules', '-**/testdata', '-**/.git'],
    };
  }
  if (lang === 'rust') {
    return {
      cargo: { buildScripts: { enable: false } },
      checkOnSave: true,
      files: { excludeDirs: ['.git', 'node_modules', 'target'] },
    };
  }
  if (lang === 'typescript' || lang === 'javascript') {
    return {
      preferences: {
        includeCompletionsForModuleExports: true,
        includeCompletionsWithInsertText: true,
      },
    };
  }
  if (lang === 'python') {
    return {
      python: {
        analysis: {
          autoImportCompletions: true,
          indexing: true,
        },
      },
    };
  }
  return undefined;
}

export function isPosInCommentOrString(docText: string, pos: number): boolean {
  const target = Math.max(0, Math.min(docText.length, pos));
  const start = target > 262_144 ? Math.max(0, docText.lastIndexOf('\n', target - 16_384) + 1) : 0;
  let i = start;
  while (i < target) {
    const ch = docText[i];
    if (ch === '"' || ch === "'" || ch === '`') {
      const q = ch;
      i++;
      while (i < target) {
        if (docText[i] === '\\' && q !== '`') {
          i += 2;
          continue;
        }
        if (docText[i] === q) break;
        i++;
      }
      if (i >= target) return true;
      i++;
      continue;
    }
    if (ch === '/' && docText[i + 1] === '/') {
      const nl = docText.indexOf('\n', i + 2);
      if (nl < 0 || nl >= target) return true;
      i = nl + 1;
      continue;
    }
    if (ch === '/' && docText[i + 1] === '*') {
      const cl = docText.indexOf('*/', i + 2);
      if (cl < 0 || cl + 2 > target) return true;
      i = cl + 2;
      continue;
    }
    if (ch === '#') {
      const nl = docText.indexOf('\n', i + 1);
      if (nl < 0 || nl >= target) return true;
      i = nl + 1;
      continue;
    }
    i++;
  }
  return false;
}

export function detectLspCompletionTrigger(
  doc: EditorState['doc'],
  pos: number,
  explicit: boolean,
  languageId?: string,
): { from: number; context: { triggerKind: 1 | 2; triggerCharacter?: string } } | null {
  const p = Math.max(0, Math.min(doc.length, pos));
  const line = doc.lineAt(p);
  const before = line.text.slice(0, p - line.from);
  const mWord = /[\w$]+$/.exec(before);
  if (mWord) {
    return { from: p - mWord[0].length, context: { triggerKind: 1 } };
  }
  if (before.endsWith('.') && !before.endsWith('..')) {
    return { from: p, context: { triggerKind: 2, triggerCharacter: '.' } };
  }
  const lang = normalizeIdeLspLanguage(languageId) ?? languageId ?? '';
  if (before.endsWith('::') && (lang === 'rust' || !lang)) {
    return { from: p, context: { triggerKind: 2, triggerCharacter: ':' } };
  }
  if (before.endsWith('@') && (lang === 'java' || lang === 'typescript' || lang === 'javascript' || lang === 'python')) {
    return { from: p, context: { triggerKind: 2, triggerCharacter: '@' } };
  }
  if (explicit) {
    return { from: p, context: { triggerKind: 1 } };
  }
  return null;
}

function stripLspSnippetPlaceholders(raw: string): string {
  return raw
    .replace(/\$\{(\d+):([^}]*)\}/g, '$2')
    .replace(/\$\{\d+\}/g, '')
    .replace(/\$\d+/g, '');
}

function mapLspCompletionKind(kind?: number): string | undefined {
  switch (kind) {
    case 2: return 'method';
    case 3: return 'function';
    case 4: return 'function';
    case 5: return 'property';
    case 6: return 'variable';
    case 7: return 'class';
    case 8: return 'interface';
    case 9: return 'namespace';
    case 10: return 'property';
    case 13: return 'enum';
    case 14: return 'keyword';
    case 20: return 'enum';
    case 21: return 'constant';
    case 22: return 'type';
    default: return undefined;
  }
}

export function buildLspCompletionChangeSpec(
  doc: EditorState['doc'],
  primaryFrom: number,
  primaryTo: number,
  insertText: string,
  mainEditRange?: LspRange | null,
  additionalEdits?: readonly RawLspTextEdit[] | null,
): { changes: ChangeSpec[]; cursorPos: number } {
  const mainFrom = mainEditRange
    ? lspToPos(doc, mainEditRange.start)
    : Math.max(0, Math.min(doc.length, primaryFrom));
  const mainTo = mainEditRange
    ? Math.max(mainFrom, lspToPos(doc, mainEditRange.end))
    : Math.max(mainFrom, Math.min(doc.length, primaryTo));

  const mapped: Array<{ from: number; to: number; insert: string; isMain: boolean }> = [
    { from: mainFrom, to: mainTo, insert: insertText, isMain: true },
  ];

  let deltaBeforeMain = 0;
  if (additionalEdits && additionalEdits.length > 0) {
    for (let i = 0; i < additionalEdits.length; i++) {
      const ed = additionalEdits[i];
      const f = lspToPos(doc, ed.range.start);
      const t = Math.max(f, lspToPos(doc, ed.range.end));
      if (t > mainFrom && f < mainTo) continue;
      if (f === mainFrom && t === mainTo && ed.newText === insertText) continue;
      mapped.push({ from: f, to: t, insert: ed.newText, isMain: false });
      if (t <= mainFrom) {
        deltaBeforeMain += ed.newText.length - (t - f);
      }
    }
  }

  mapped.sort((a, b) => (a.from !== b.from ? a.from - b.from : a.to - b.to));
  const changes: ChangeSpec[] = mapped.map((m) => ({ from: m.from, to: m.to, insert: m.insert }));
  return {
    changes,
    cursorPos: Math.max(0, mainFrom + deltaBeforeMain + insertText.length),
  };
}

function isDocPosInCommentOrString(doc: EditorState['doc'], pos: number): boolean {
  const target = Math.max(0, Math.min(doc.length, pos));
  if (target === 0) return false;
  const windowStart = Math.max(0, target - 16_384);
  const alignedStart = windowStart > 0 ? doc.lineAt(windowStart).from : 0;
  return isPosInCommentOrString(doc.sliceString(alignedStart, target), target - alignedStart);
}

export function createIdeLspCompletionExtension(opts: {
  documentUri: string;
  languageId: string;
  isReady: () => boolean;
  requestCompletion: (params: {
    textDocument: { uri: string };
    position: LspPos;
    context: { triggerKind: 1 | 2; triggerCharacter?: string };
  }) => Promise<RawLspCompletionItem[] | { items?: RawLspCompletionItem[] } | null>;
  resolveItem?: (item: RawLspCompletionItem) => Promise<RawLspCompletionItem | null>;
}): Extension {
  const source = async (ctx: CompletionContext): Promise<CompletionResult | null> => {
    if (!opts.isReady()) return null;
    const trig = detectLspCompletionTrigger(ctx.state.doc, ctx.pos, ctx.explicit, opts.languageId);
    if (!trig) return null;
    if (!ctx.explicit && isDocPosInCommentOrString(ctx.state.doc, ctx.pos)) return null;

    try {
      const res = await opts.requestCompletion({
        textDocument: { uri: opts.documentUri },
        position: posToLsp(ctx.state.doc, ctx.pos),
        context: trig.context,
      });
      const items = Array.isArray(res) ? res : (res?.items ?? []);
      if (items.length === 0) return null;

      const options: Completion[] = items.slice(0, 150).map((it, idx) => {
        const rawInsert = it.textEdit?.newText ?? it.insertText ?? it.label;
        const cleanInsert = it.insertTextFormat === 2 ? stripLspSnippetPlaceholders(rawInsert) : rawInsert;
        const editRange = it.textEdit?.replace ?? it.textEdit?.range ?? it.textEdit?.insert ?? null;
        return {
          label: it.label,
          detail: it.detail,
          type: mapLspCompletionKind(it.kind),
          boost: it.sortText ? Math.max(-90, 50 - idx) : undefined,
          apply: (view, _comp, from, to) => {
            const runApply = (resolved: RawLspCompletionItem) => {
              if (!view.dom.isConnected) return;
              const finalRaw = resolved.textEdit?.newText ?? resolved.insertText ?? cleanInsert;
              const finalInsert = resolved.insertTextFormat === 2 ? stripLspSnippetPlaceholders(finalRaw) : finalRaw;
              const finalRange = resolved.textEdit?.replace ?? resolved.textEdit?.range ?? resolved.textEdit?.insert ?? editRange;
              const spec = buildLspCompletionChangeSpec(
                view.state.doc,
                from,
                to,
                finalInsert,
                finalRange,
                resolved.additionalTextEdits ?? it.additionalTextEdits,
              );
              view.dispatch({
                changes: spec.changes,
                selection: EditorSelection.cursor(spec.cursorPos),
                userEvent: 'input.complete',
              });
            };
            if (!it.additionalTextEdits?.length && it.data !== undefined && opts.resolveItem) {
              let settled = false;
              const fallbackTimer = setTimeout(() => {
                if (!settled) {
                  settled = true;
                  runApply(it);
                }
              }, 350);
              void opts.resolveItem(it).then((resolved) => {
                if (settled) return;
                settled = true;
                clearTimeout(fallbackTimer);
                runApply(resolved ?? it);
              }).catch(() => {
                if (settled) return;
                settled = true;
                clearTimeout(fallbackTimer);
                runApply(it);
              });
              return;
            }
            runApply(it);
          },
        };
      });
      return { from: trig.from, options, validFor: /^[\w$]*$/ };
    } catch {
      return null;
    }
  };

  const dotTriggerListener = EditorView.updateListener.of((u) => {
    if (!u.docChanged || !u.view.hasFocus || !opts.isReady()) return;
    let shouldOpen = false;
    for (const tr of u.transactions) {
      if (!tr.isUserEvent('input.type')) continue;
      tr.changes.iterChanges((_fa, _ta, _fb, tb, ins) => {
        if (ins.length !== 1) return;
        const ch = ins.toString();
        if (ch === '.') {
          shouldOpen = true;
        } else if (ch === ':' && tb >= 2 && u.state.sliceDoc(tb - 2, tb) === '::') {
          shouldOpen = true;
        } else if (ch === '@') {
          shouldOpen = true;
        }
      });
    }
    if (!shouldOpen) return;
    const head = u.state.selection.main.head;
    if (isDocPosInCommentOrString(u.state.doc, head)) return;
    const targetView = u.view;
    queueMicrotask(() => {
      if (targetView.dom.isConnected && targetView.hasFocus) startCompletion(targetView);
    });
  });

  return [EditorState.languageData.of(() => [{ autocomplete: source }]), dotTriggerListener];
}

export const DEFAULT_LSP_TOKEN_TYPES = [
  'namespace', 'type', 'class', 'enum', 'interface', 'struct', 'typeParameter',
  'parameter', 'variable', 'property', 'enumMember', 'event', 'function',
  'method', 'macro', 'keyword', 'modifier', 'comment', 'string', 'number',
  'regexp', 'operator', 'decorator',
] as const;

export const DEFAULT_LSP_TOKEN_MODIFIERS = [
  'declaration', 'definition', 'readonly', 'static', 'deprecated', 'abstract',
  'async', 'modification', 'documentation', 'defaultLibrary',
] as const;

function semanticClassForLspToken(tokenType: string, mods: Set<string>): string | null {
  switch (tokenType) {
    case 'keyword':
    case 'modifier':
      return 'cm-syn-keyword';
    case 'function':
    case 'method':
      return mods.has('declaration') || mods.has('definition') ? 'cm-syn-func-decl' : 'cm-syn-func-call';
    case 'type':
    case 'class':
    case 'struct':
    case 'interface':
    case 'enum':
    case 'typeParameter':
      return 'cm-syn-type';
    case 'property':
    case 'enumMember':
      return 'cm-syn-field';
    case 'parameter':
      return 'cm-syn-param';
    case 'decorator':
    case 'macro':
      return 'cm-syn-annotation';
    case 'namespace':
      return 'cm-syn-fg';
    default:
      return null;
  }
}

export function decodeLspSemanticTokens(
  doc: EditorState['doc'],
  data: readonly number[],
  tokenTypes: readonly string[] = DEFAULT_LSP_TOKEN_TYPES,
  tokenModifiers: readonly string[] = DEFAULT_LSP_TOKEN_MODIFIERS,
): Array<{ from: number; to: number; className: string }> {
  if (!data.length || doc.length === 0) return [];
  const out: Array<{ from: number; to: number; className: string }> = [];
  let line = 0;
  let startChar = 0;

  for (let i = 0; i + 4 < data.length; i += 5) {
    const deltaLine = data[i];
    const deltaStart = data[i + 1];
    const length = data[i + 2];
    const typeIdx = data[i + 3];
    const modBits = data[i + 4];
    if (deltaLine < 0 || deltaStart < 0 || length <= 0) continue;

    line += deltaLine;
    startChar = deltaLine === 0 ? startChar + deltaStart : deltaStart;
    if (line < 0 || line >= doc.lines) continue;

    const tokenType = tokenTypes[typeIdx] ?? '';
    if (!tokenType) continue;
    const mods = new Set<string>();
    for (let bit = 0; bit < tokenModifiers.length && bit < 31; bit++) {
      if ((modBits & (1 << bit)) !== 0) mods.add(tokenModifiers[bit]);
    }
    const className = semanticClassForLspToken(tokenType, mods);
    if (!className) continue;

    const lnObj = doc.line(line + 1);
    const from = Math.max(0, Math.min(doc.length, Math.min(lnObj.to, lnObj.from + startChar)));
    const to = Math.max(from, Math.min(doc.length, Math.min(lnObj.to, from + length)));
    if (to > from) out.push({ from, to, className });
  }
  return out;
}

const setLspSemanticDecos = StateEffect.define<DecorationSet>();

export function createLspSemanticTokensExtension(opts: {
  documentUri: string;
  fetchTokens: (uri: string) => Promise<{
    data?: number[];
    tokenTypes?: string[];
    tokenModifiers?: string[];
  } | null>;
}): Extension {
  const field = StateField.define<DecorationSet>({
    create: () => Decoration.none,
    update(decos, tr) {
      for (const ef of tr.effects) {
        if (ef.is(setLspSemanticDecos)) return ef.value;
      }
      return tr.docChanged ? decos.map(tr.changes) : decos;
    },
    provide: (f) => EditorView.decorations.from(f),
  });

  const plugin = ViewPlugin.fromClass(class {
    private timer: ReturnType<typeof setTimeout> | null = null;
    private destroyed = false;

    constructor(view: EditorView) {
      this.schedule(view, 140);
    }

    update(u: ViewUpdate) {
      if (u.docChanged) this.schedule(u.view, 220);
    }

    destroy() {
      this.destroyed = true;
      if (this.timer) clearTimeout(this.timer);
    }

    schedule(view: EditorView, delayMs: number) {
      if (this.timer) clearTimeout(this.timer);
      this.timer = setTimeout(() => void this.refresh(view), delayMs);
    }

    async refresh(view: EditorView) {
      if (this.destroyed || !view.dom.isConnected) return;
      const snapshotDoc = view.state.doc;
      try {
        const res = await opts.fetchTokens(opts.documentUri);
        if (this.destroyed || !view.dom.isConnected || view.state.doc !== snapshotDoc) return;
        if (!res?.data?.length) return;
        const decoded = decodeLspSemanticTokens(
          snapshotDoc,
          res.data,
          res.tokenTypes ?? DEFAULT_LSP_TOKEN_TYPES,
          res.tokenModifiers ?? DEFAULT_LSP_TOKEN_MODIFIERS,
        );
        const ranges: Range<Decoration>[] = decoded.map((s) =>
          Decoration.mark({ class: s.className }).range(s.from, s.to),
        );
        view.dispatch({
          effects: setLspSemanticDecos.of(ranges.length ? Decoration.set(ranges, true) : Decoration.none),
        });
      } catch {
        /* semanticTokens unsupported or cancelled */
      }
    }
  });

  return [field, plugin];
}
