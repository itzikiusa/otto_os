// Viewport-only (<1ms) CodeMirror 6 Semantic Classifier, Rainbow Brackets,
// Inlay Hints, CodeVision & Import Folding backed by `cm-ide-semantic-model.ts`.

import { foldEffect, foldService, foldedRanges, unfoldEffect } from '@codemirror/language';
import { setDiagnostics, type Action as LintAction, type Diagnostic } from '@codemirror/lint';
import { EditorState, StateEffect, StateField, type Extension, type Range } from '@codemirror/state';
import { Decoration, EditorView, ViewPlugin, type DecorationSet, type ViewUpdate } from '@codemirror/view';
import type { IdeDiagnostic } from '../api/types.ts';
import { hasLiveLspDiagnosticsForFile } from './cm-ide-drafts.ts';
import { extractGoBuildTagsFromSource, rewriteGoBuildTagDiagnostic } from './cm-ide-lsp-completion.ts';
import type { IdeEditorPreferences } from './cm-ide-theme.ts';
import {
  analyzeGoImports,
  analyzeJavaDiagnostics,
  buildSymbolModel,
  computeRainbowBrackets,
  CONTROL_KW,
  detectSemanticLang,
  GENERAL_KW,
  getCachedSemanticDoc,
  GO_BUILTIN_TYPES,
  inferGoImportPkgNames,
  splitTopLevelCommas,
  visibleBracketSpansFromIndex,
  type ClientDiagQuickFix,
  type ClientIdeDiagnostic,
  type FileSymbolModel,
  type JavaDiagPrefs,
  type RainbowBracketSpan,
  type SemanticLangKind,
  type SemanticRole,
  type SemanticTokenSpan,
} from './cm-ide-semantic-model.ts';

export {
  analyzeGoImports,
  analyzeJavaDiagnostics,
  computeRainbowBrackets,
  detectSemanticLang,
  inferGoImportPkgNames,
  type ClientDiagQuickFix,
  type ClientIdeDiagnostic,
  type JavaDiagPrefs,
  type RainbowBracketSpan,
  type SemanticLangKind,
  type SemanticRole,
  type SemanticTokenSpan,
};

function isGoQualifiedTypeContext(docText: string, dotPos: number, afterWordPos: number): boolean {
  let p = dotPos - 1;
  while (p >= 0 && (docText[p] === ' ' || docText[p] === '\t')) p--;
  while (p >= 0 && /\w/.test(docText[p])) p--;
  const pkgStart = p + 1;
  while (p >= 0 && (docText[p] === ' ' || docText[p] === '\t')) p--;
  const beforeCh = p >= 0 ? docText[p] : '';

  if (beforeCh === '*' || beforeCh === ']') return true;
  if (beforeCh === '(' && p >= 1 && docText[p - 1] === '.') return true;

  if (docText[afterWordPos] === '{') {
    const twoBefore = p >= 1 ? docText.slice(p - 1, p + 1) : '';
    if (!['==', '!=', '<=', '>=', '&&', '||'].includes(twoBefore) && beforeCh !== '<' && beforeCh !== '>') {
      return true;
    }
  }

  const lineStart = docText.lastIndexOf('\n', pkgStart) + 1;
  const lineBeforePkg = docText.slice(lineStart, pkgStart);
  const mPrevWord = /(?:^|[\s(,])([A-Za-z_]\w*)\s+(?:\.\.\.)?(?:<-)?$/.exec(lineBeforePkg);
  if (mPrevWord) {
    const w = mPrevWord[1];
    if (w === 'type' || w === 'chan') return true;
    if (!CONTROL_KW.has(w) && !GENERAL_KW.has(w)) return true;
  }
  return /\)\s*(?:\(\s*(?:[^()]*,\s*)?)?$/.test(lineBeforePkg);
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
      if (pos >= model.paramScopes[i].from && pos <= model.paramScopes[i].to) return model.paramScopes[i].params;
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
          if (docText[end] === '\\' && ch !== '`') {
            end += 2;
            continue;
          }
          if (docText[end] === ch) {
            end++;
            break;
          }
          end++;
        }
        if (lang === 'json' && ch === '"') {
          let a = end;
          while (a < to && (docText[a] === ' ' || docText[a] === '\t')) a++;
          if (docText[a] === ':') out.push({ from: i, to: end, role: 'field', text: docText.slice(i, end) });
        } else if (lang === 'shell' && ch === '"') {
          const sub = docText.slice(i, end);
          const vr = /\$(?:[A-Za-z_]\w*|\{[^}"\n]+\})/g;
          for (let vm: RegExpExecArray | null; (vm = vr.exec(sub)) !== null; ) {
            out.push({ from: i + vm.index, to: i + vm.index + vm[0].length, role: 'param', text: vm[0] });
          }
        }
        i = end;
        continue;
      }
      if (ch === '/' && docText[i + 1] === '*') {
        const cl = docText.indexOf('*/', i + 2);
        const end = cl >= 0 ? Math.min(to, cl + 2) : to;
        if (docText[i + 2] === '*' && docText[i + 3] !== '/') classifyDocComment(docText.slice(i, end), i, out);
        i = end;
        continue;
      }
      if (ch === '/' && docText[i + 1] === '/') {
        const nl = docText.indexOf('\n', i + 2);
        const end = nl >= 0 ? Math.min(to, nl) : to;
        if (docText[i + 2] === '/' && lang === 'rust') classifyDocComment(docText.slice(i, end), i, out);
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
        else if (!isAfterDot && lang === 'go' && GO_BUILTIN_TYPES.has(word)) role = 'type';
        else if (!isAfterDot && CONTROL_KW.has(word)) role = 'control';
        else if (!isAfterDot && GENERAL_KW.has(word)) role = 'keyword';
        else if (model.declStarts.has(i)) role = 'func-decl';
        else if (isAfterDot) {
          if (isBeforeParen) role = 'func-call';
          else if (lang === 'go' && /^[A-Z]/.test(word) && isGoQualifiedTypeContext(docText, prev, next)) role = 'type';
          else role = 'field';
        } else if (isBeforeDot) {
          role = scopeP?.has(word) ? 'param'
            : model.pkgVars.has(word) ? 'field'
            : (lang === 'java' || lang === 'rust') && /^[A-Z][a-zA-Z0-9]*$/.test(word) ? 'type'
            : 'fg';
        } else if (isBeforeParen) {
          role = /\bnew\s+$/.test(docText.slice(Math.max(0, i - 8), i)) && /^[A-Z]/.test(word) ? 'type' : 'func-call';
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

export interface InlayParamHintItem {
  pos: number;
  label: string;
  argText: string;
}

export function computeInlayParameterHintsWithModel(
  docText: string,
  lang: SemanticLangKind,
  model: FileSymbolModel,
  visibleRanges: readonly { from: number; to: number }[],
  maxHints = 24,
): InlayParamHintItem[] {
  if (!model.funcParams.size && !model.methodDecls.length) return [];
  const docLen = docText.length;
  const out: InlayParamHintItem[] = [];
  for (const { from, to } of visibleRanges) {
    const slice = docText.slice(from, to);
    const callRe = /\b([A-Za-z_$][\w$]*)\s*\(([^()\n]{1,120})\)/g;
    for (let m: RegExpExecArray | null; (m = callRe.exec(slice)) !== null && out.length < maxHints; ) {
      const fnName = m[1];
      const absCallStart = from + m.index;
      if (model.declStarts.has(absCallStart)) continue;

      // Inspect whether this call is dot/path qualified (`expr.fn(...)` or `pkg::fn(...)`).
      let p = absCallStart - 1;
      while (p >= 0 && (docText[p] === ' ' || docText[p] === '\t')) p--;
      const isAfterDot = p >= 0 && docText[p] === '.' && (p === 0 || docText[p - 1] !== '.');
      const isAfterColon = p >= 1 && docText[p] === ':' && docText[p - 1] === ':';

      let pNames: string[] | undefined;
      let isVariadic = false;

      if (isAfterColon) {
        continue;
      } else if (isAfterDot) {
        let q = p - 1;
        while (q >= 0 && /[A-Za-z0-9_$.\]()]/.test(docText[q])) q--;
        const qual = docText.slice(q + 1, p).trim();
        // Chained calls like `g.pool.Submit(f)` MUST NEVER inherit `GoRoutineConcurrencyHelper.Submit(ctx, f)`.
        if (!qual || qual.includes('.') || qual.includes('(') || qual.includes('[')) continue;
        if (lang === 'go') {
          const enclosingMeth = model.methodDecls.find(
            (md) => md.name === fnName && absCallStart >= md.scopeFrom && absCallStart <= md.scopeTo && md.recvIdent === qual,
          ) ?? model.methodDecls.find(
            (md) =>
              md.name === fnName &&
              md.recvIdent === qual &&
              model.methodDecls.some((enc) => absCallStart >= enc.scopeFrom && absCallStart <= enc.scopeTo && enc.recvIdent === qual && enc.recvType === md.recvType),
          );
          if (!enclosingMeth) continue;
          pNames = enclosingMeth.params;
          isVariadic = enclosingMeth.variadic;
        } else if (qual === 'this' || qual === 'self') {
          pNames = model.funcParams.get(fnName);
          isVariadic = model.variadicFuncs.has(fnName);
        } else {
          continue;
        }
      } else {
        pNames = model.funcParams.get(fnName);
        isVariadic = model.variadicFuncs.has(fnName);
      }

      if (!pNames || pNames.length === 0) continue;
      const rawArgs = splitTopLevelCommas(m[2]);
      if (rawArgs.length === 0 || (rawArgs.length === 1 && !rawArgs[0].trim())) continue;
      if (!isVariadic && rawArgs.length > pNames.length) continue;
      if ((lang === 'go' || lang === 'java' || lang === 'rust') && (isVariadic ? rawArgs.length < pNames.length - 1 : rawArgs.length !== pNames.length)) {
        continue;
      }

      const argsStart = absCallStart + m[0].indexOf('(') + 1;
      let cursor = 0;
      for (let aIdx = 0; aIdx < rawArgs.length && out.length < maxHints; aIdx++) {
        if (aIdx >= pNames.length) break;
        const rawArg = rawArgs[aIdx];
        const trimmedArg = rawArg.trim();
        const pName = pNames[aIdx];
        if (trimmedArg && trimmedArg !== pName && !trimmedArg.startsWith(`${pName}:`) && !trimmedArg.startsWith(`${pName}=`)) {
          const pos = Math.max(0, Math.min(docLen, argsStart + cursor + (rawArg.length - rawArg.trimStart().length)));
          out.push({ pos, label: pName, argText: trimmedArg });
        }
        cursor += rawArg.length + 1;
      }
    }
  }
  return out;
}

export function computeInlayParameterHints(
  docText: string,
  lang: SemanticLangKind,
  visibleRanges?: readonly { from: number; to: number }[],
): InlayParamHintItem[] {
  if (!docText || lang === 'csv' || lang === 'json' || lang === 'other') return [];
  const model = buildSymbolModel(docText, lang);
  return computeInlayParameterHintsWithModel(docText, lang, model, visibleRanges ?? [{ from: 0, to: docText.length }]);
}

import { applyClientQuickFix, CodeVisionWidget, InlayHintWidget } from './cm-ide-semantic-widgets.ts';
export { applyClientQuickFix };

const setLineQuickFixesEffect = StateEffect.define<Map<number, ClientDiagQuickFix[]>>();
const setServerUnresolvedEffect = StateEffect.define<Set<string>>();

const lineQuickFixesField = StateField.define<Map<number, ClientDiagQuickFix[]>>({
  create: () => new Map(),
  update: (v, tr) => {
    for (const e of tr.effects) {
      if (e.is(setLineQuickFixesEffect)) return e.value;
    }
    return v;
  },
});

const serverUnresolvedField = StateField.define<Set<string>>({
  create: () => new Set(),
  update: (v, tr) => {
    for (const e of tr.effects) {
      if (e.is(setServerUnresolvedEffect)) return e.value;
    }
    return v;
  },
});

export function getIdeQuickFixesAtLine(state: EditorState, line: number): ClientDiagQuickFix[] {
  return state.field(lineQuickFixesField, false)?.get(line) ?? [];
}

export interface CreateIdeSemanticOptions {
  filePath: string;
  language?: string;
  root?: string;
  connectionId?: string | null;
  projectFolders?: string[];
  getPreferences: () => IdeEditorPreferences;
  onDiagnostics?: ((diags: IdeDiagnostic[]) => void) | null;
  onFindUsages?: ((ctx: { line: number; column: number; symbol?: string }) => void) | null;
}

const AUTHORITATIVE_LSP_SOURCES = new Set(['gopls', 'jdtls', 'rust-analyzer', 'pyright', 'typescript', 'tsserver']);

export function createIdeSemanticExtensions(opts: CreateIdeSemanticOptions): Extension {
  const lang = detectSemanticLang(opts.filePath, opts.language);

  const importFoldExt = foldService.of((state, lineStart) => {
    const ln = state.doc.lineAt(lineStart).number;
    if (ln > 40) return null;
    const { docText } = getCachedSemanticDoc(state.doc, lang, undefined, undefined, false, false);
    const r = computeImportFoldRange(docText, lang);
    return r && r.startLine === ln ? { from: r.from, to: r.to } : null;
  });

  const vpPlugin = ViewPlugin.fromClass(
    class {
      decorations: DecorationSet = Decoration.none;
      private timer: ReturnType<typeof setTimeout> | null = null;
      private abort: AbortController | null = null;
      private didAutoFold = false;
      private hadSemanticDiags = false;

      constructor(view: EditorView) {
        this.decorations = this.compute(view);
        queueMicrotask(() => {
          if (view.dom.isConnected) {
            this.maybeAutoFoldImports(view);
            this.scheduleDiagnostics(view, 10);
          }
        });
      }

      update(u: ViewUpdate) {
        const unresChanged = u.transactions.some((t) => t.effects.some((e) => e.is(setServerUnresolvedEffect)));
        if (u.docChanged || u.viewportChanged || u.selectionSet || unresChanged) {
          this.decorations = this.compute(u.view);
        }
        if (u.docChanged) this.scheduleDiagnostics(u.view, 250);
      }

      destroy() {
        if (this.timer) clearTimeout(this.timer);
        this.abort?.abort();
      }

      maybeAutoFoldImports(view: EditorView) {
        const prefs = opts.getPreferences();
        if (this.didAutoFold || !prefs.autoFoldImports) return;
        this.didAutoFold = true;
        const { docText } = getCachedSemanticDoc(view.state.doc, lang, undefined, prefs, false, false);
        const errLines = new Set<number>(
          lang === 'java'
            ? analyzeJavaDiagnostics(docText, prefs).diagnostics.filter((d) => d.severity === 'error').map((d) => d.line)
            : [],
        );
        const r = computeImportFoldRange(docText, lang, errLines);
        if (r && r.from >= 0 && r.to <= view.state.doc.length) {
          view.dispatch({ effects: foldEffect.of({ from: r.from, to: r.to }) });
        }
      }

      scheduleDiagnostics(view: EditorView, delayMs: number) {
        if (this.timer) clearTimeout(this.timer);
        this.abort?.abort();
        if (lang !== 'csv' && lang !== 'other' && opts.filePath) {
          this.timer = setTimeout(() => void this.runDiagnostics(view), delayMs);
        }
      }

      async runDiagnostics(view: EditorView) {
        if (!view.dom.isConnected) return;
        const prefs = opts.getPreferences();
        const startDoc = view.state.doc;
        const lspOwnsDiags = hasLiveLspDiagnosticsForFile(opts.root, opts.filePath);
        const { docText } = getCachedSemanticDoc(startDoc, lang, undefined, prefs, false, false);
        const jRes = lang === 'java' ? analyzeJavaDiagnostics(docText, prefs) : null;
        const goImp = lang === 'go' ? analyzeGoImports(docText, prefs) : null;
        const clientDiags: ClientIdeDiagnostic[] = [];
        if (jRes) {
          for (const d of jRes.diagnostics) {
            if (lspOwnsDiags && d.code === 'unused-import') continue;
            clientDiags.push(d);
          }
        }
        if (goImp && !lspOwnsDiags) clientDiags.push(...goImp.diagnostics);
        const unresolved = new Set<string>(jRes ? jRes.unresolvedIdents : []);

        if (!lspOwnsDiags && opts.root && (lang === 'java' || lang === 'go' || lang === 'python' || lang === 'shell' || lang === 'json')) {
          const ctrl = new AbortController();
          this.abort = ctrl;
          try {
            const { api } = await import('../api/client.ts');
            type RawFix = { title: string; line: number; end_line?: number; replacement?: string; delete_line?: boolean; edits?: ClientDiagQuickFix['edits'] };
            type RawDiag = { line: number; col?: number; column?: number; end_line?: number; end_col?: number; severity?: string; code?: string; source?: string; message: string; quick_fixes?: RawFix[] };
            const resp = await api.post<{ diagnostics?: RawDiag[] }>(
              '/ide/lsp/diagnostics',
              {
                target: { kind: opts.connectionId ? 'ssh' : 'local', root: opts.root, connection_id: opts.connectionId ?? null, additional_roots: opts.projectFolders },
                path: opts.filePath,
                content: docText,
                language: lang,
                project_folders: opts.projectFolders,
              },
              ctrl.signal,
            );
            const liveAfter = hasLiveLspDiagnosticsForFile(opts.root, opts.filePath);
            for (const sd of resp.diagnostics ?? []) {
              const rw = lang === 'go' ? rewriteGoBuildTagDiagnostic(sd.message, sd.code, docText) : null;
              const effectiveMsg = rw?.isBuildTagDiag ? rw.message : sd.message;
              const effectiveCode = rw?.isBuildTagDiag ? 'otto_go_build_tag' : sd.code;
              const effectiveSeverity = rw?.isBuildTagDiag ? 'warning' : sd.severity;
              const isUnusedImp = effectiveCode === 'unused-import' || /unused import/i.test(effectiveMsg);
              if (isUnusedImp) {
                if (prefs.unusedImports === false) continue;
                if ((lspOwnsDiags || liveAfter) && !AUTHORITATIVE_LSP_SOURCES.has(sd.source ?? '')) continue;
                if (lang === 'go' && goImp) {
                  const impEntry = goImp.entries.find((e) => e.line === sd.line);
                  if (impEntry && (impEntry.used || !impEntry.certain)) continue;
                }
              }
              if (lang === 'java' && prefs.javaUnresolvedImports === false && (effectiveCode === 'unresolved-import' || /unresolved import|package .* does not exist/i.test(effectiveMsg))) {
                continue;
              }
              if (lang === 'java' && prefs.lombokSupport !== false && /package lombok\b.*does not exist|cannot find symbol.*\bvariable log\b/i.test(effectiveMsg)) {
                continue;
              }
              const duplicate = clientDiags.some((c) => c.line === sd.line && (c.message === effectiveMsg || (Boolean(c.code) && c.code === effectiveCode)));
              if (!duplicate) {
                clientDiags.push({
                  line: sd.line,
                  column: sd.col ?? sd.column ?? 1,
                  endLine: sd.end_line ?? sd.line,
                  endColumn: sd.end_col ?? 200,
                  severity: effectiveSeverity === 'error' ? 'error' : effectiveSeverity === 'warning' ? 'warning' : 'info',
                  message: effectiveMsg,
                  source: rw?.isBuildTagDiag && sd.source === 'go list' ? 'gopls (build tags)' : (sd.source ?? 'lsp'),
                  code: effectiveCode,
                  quickFixes: (sd.quick_fixes ?? []).map((qf) => ({
                    title: qf.title, line: qf.line, endLine: qf.end_line ?? qf.line,
                    replacement: qf.replacement, deleteLine: qf.delete_line, edits: qf.edits,
                  })),
                });
              }
              if (effectiveSeverity === 'error' && (lang !== 'java' || prefs.javaUnresolvedImports !== false) && sd.line >= 1 && sd.line <= startDoc.lines) {
                const mImp = /^\s*import\s+(?:static\s+)?[\w.$]*\.([A-Za-z_$][\w$]*)\s*;/.exec(startDoc.line(sd.line).text);
                if (mImp) unresolved.add(mImp[1]);
              }
            }
          } catch {
            // Fall back to instant client diagnostics.
          }
        }

        if (!view.dom.isConnected || view.state.doc !== startDoc) return;
        const doc = view.state.doc;
        const fixMap = new Map<number, ClientDiagQuickFix[]>();
        const errorLines = new Set<number>();
        const cmDiags: Diagnostic[] = [];
        const ideDiags: IdeDiagnostic[] = [];

        for (const d of clientDiags) {
          if (d.line < 1 || d.line > doc.lines) continue;
          if (d.severity === 'error') errorLines.add(d.line);
          const lnObj = doc.line(d.line);
          const endLnObj = doc.line(Math.max(d.line, Math.min(doc.lines, d.endLine)));
          const rawFrom = Math.min(lnObj.to, lnObj.from + Math.max(0, d.column - 1));
          const rawTo = Math.max(rawFrom + 1, Math.min(endLnObj.to, endLnObj.from + Math.max(0, d.endColumn - 1)));
          const clampedFrom = Math.max(0, Math.min(doc.length, rawFrom));
          const clampedTo = Math.max(clampedFrom, Math.min(doc.length, rawTo));
          if (d.quickFixes.length) fixMap.set(d.line, [...(fixMap.get(d.line) ?? []), ...d.quickFixes]);
          const actions: LintAction[] = d.quickFixes.map((qf) => ({ name: qf.title, apply: (v) => applyClientQuickFix(v, qf) }));
          if (d.code === 'otto_go_build_tag') {
            const fileTags = extractGoBuildTagsFromSource(docText);
            if (fileTags.length > 0) {
              actions.unshift({
                name: `Enable Go build tag "${fileTags.join(',')}"`,
                apply: (v) => {
                  v.dom.dispatchEvent(
                    new CustomEvent('otto:ide-enable-go-build-tag', {
                      bubbles: true,
                      detail: { tags: fileTags, filePath: opts.filePath },
                    }),
                  );
                },
              });
            }
          }
          cmDiags.push({ from: clampedFrom, to: clampedTo, severity: d.severity, message: d.message, source: d.source, actions });
          ideDiags.push({
            path: opts.filePath, line: d.line, column: d.column, end_line: d.endLine, end_column: d.endColumn,
            severity: d.severity, message: d.message, source: d.source, code: d.code ?? null,
            quick_fixes: d.quickFixes.length
              ? d.quickFixes.map((qf, idx) => ({
                  title: qf.title, line: qf.line, end_line: qf.endLine, replacement: qf.replacement ?? '',
                  delete_line: Boolean(qf.deleteLine), is_preferred: idx === 0,
                  edits: qf.edits?.length
                    ? qf.edits.map((e) => ({ ...e, path: e.path || opts.filePath }))
                    : [{
                        path: opts.filePath, start_line: qf.line, start_col: 1,
                        end_line: qf.deleteLine ? Math.min(doc.lines + 1, qf.endLine + 1) : qf.endLine,
                        end_col: qf.deleteLine && qf.endLine < doc.lines ? 1 : doc.line(Math.min(doc.lines, qf.endLine)).length + 1,
                        new_text: qf.deleteLine ? '' : (qf.replacement ?? ''),
                      }],
                }))
              : undefined,
          });
        }

        if (errorLines.size) {
          foldedRanges(view.state).between(0, Math.min(doc.length, 4096), (f, t) => {
            for (let ln = doc.lineAt(f).number; ln <= doc.lineAt(t).number; ln++) {
              if (errorLines.has(ln)) {
                view.dispatch({ effects: unfoldEffect.of({ from: f, to: t }) });
                break;
              }
            }
          });
        }

        if (ideDiags.length > 0 || this.hadSemanticDiags) {
          this.hadSemanticDiags = ideDiags.length > 0;
          const trSpec = setDiagnostics(view.state, cmDiags);
          const prevEf = Array.isArray(trSpec.effects) ? trSpec.effects : trSpec.effects ? [trSpec.effects] : [];
          view.dispatch({ ...trSpec, effects: [...prevEf, setLineQuickFixesEffect.of(fixMap), setServerUnresolvedEffect.of(unresolved)] });
          opts.onDiagnostics?.(ideDiags);
        } else {
          view.dispatch({ effects: [setLineQuickFixesEffect.of(fixMap), setServerUnresolvedEffect.of(unresolved)] });
        }
      }

      compute(view: EditorView): DecorationSet {
        const prefs = opts.getPreferences();
        const vis = view.visibleRanges;
        const docLen = view.state.doc.length;
        const extraUnres = view.state.field(serverUnresolvedField, false);
        const wantBrackets = Boolean(prefs.rainbowBrackets && lang !== 'csv');
        const wantCv = Boolean(prefs.codeVision && lang !== 'csv' && lang !== 'json');
        const { docText, model, identCounts, brackets } = getCachedSemanticDoc(view.state.doc, lang, extraUnres, prefs, wantBrackets, wantCv);
        const items: Range<Decoration>[] = [];

        for (const tok of classifySemanticTokensWithModel(docText, lang, model, vis)) {
          const f = Math.max(0, Math.min(docLen, tok.from));
          const t = Math.max(f, Math.min(docLen, tok.to));
          if (t > f) items.push(Decoration.mark({ class: `cm-syn-${tok.role}` }).range(f, t));
        }

        if (prefs.unusedImports !== false && !hasLiveLspDiagnosticsForFile(opts.root, opts.filePath)) {
          for (const ur of model.unusedRanges) {
            if (!vis.some((r) => ur.to > r.from && ur.from < r.to)) continue;
            const f = Math.max(0, Math.min(docLen, ur.from));
            const t = Math.max(f, Math.min(docLen, ur.to));
            if (t > f) items.push(Decoration.mark({ class: 'cm-syn-unused' }).range(f, t));
          }
        }

        if (wantBrackets && brackets) {
          for (const rb of visibleBracketSpansFromIndex(brackets, vis, view.state.selection.main.head)) {
            const f = Math.max(0, Math.min(docLen, rb.from));
            const t = Math.max(f, Math.min(docLen, rb.to));
            if (t <= f) continue;
            const cls = rb.level === 'unmatched' ? 'cm-syn-rb-unmatched' : `cm-syn-rb-${rb.level}${rb.active ? ' cm-syn-rb-active' : ''}`;
            items.push(Decoration.mark({ class: cls }).range(f, t));
          }
        }

        if ((prefs.inlayHints || prefs.codeVision) && docText.length <= 131072 && lang !== 'csv' && lang !== 'json') {
          if (prefs.inlayHints && (model.funcParams.size || model.methodDecls.length)) {
            for (const h of computeInlayParameterHintsWithModel(docText, lang, model, vis, 24)) {
              items.push(Decoration.widget({ widget: new InlayHintWidget(h.label), side: -1 }).range(h.pos));
            }
          }
          if (prefs.codeVision && identCounts) {
            let cvCount = 0;
            for (const decl of model.funcDecls) {
              if (cvCount >= 16 || !vis.some((r) => decl.lineEnd >= r.from && decl.lineEnd <= r.to)) continue;
              const usages = Math.max(0, (identCounts.get(decl.name) ?? 1) - 1);
              const widget = new CodeVisionWidget(usages, decl.line, decl.name, opts.onFindUsages ? (ln, sym) => opts.onFindUsages?.({ line: ln, column: 1, symbol: sym }) : undefined);
              items.push(Decoration.widget({ widget, side: 1 }).range(Math.max(0, Math.min(docLen, decl.lineEnd))));
              cvCount++;
            }
          }
        }
        return items.length ? Decoration.set(items, true) : Decoration.none;
      }
    },
    { decorations: (v) => v.decorations },
  );

  return [lineQuickFixesField, serverUnresolvedField, importFoldExt, vpPlugin];
}
