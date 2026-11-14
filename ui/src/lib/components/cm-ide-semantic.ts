// Viewport-only (<1ms) CodeMirror 6 Semantic Classifier, Rainbow Brackets,
// Inlay Hints, CodeVision & Import Folding backed by `cm-ide-semantic-model.ts`.

import { foldEffect, foldService, foldedRanges, unfoldEffect } from '@codemirror/language';
import { setDiagnostics, type Action as LintAction, type Diagnostic } from '@codemirror/lint';
import { EditorState, StateEffect, StateField, type Extension, type Range } from '@codemirror/state';
import { Decoration, EditorView, ViewPlugin, WidgetType, type DecorationSet, type ViewUpdate } from '@codemirror/view';
import type { IdeDiagnostic } from '../api/types.ts';
import type { IdeEditorPreferences } from './cm-ide-theme.ts';
import {
  analyzeJavaDiagnostics,
  classifySemanticTokens,
  classifySemanticTokensWithModel,
  computeImportFoldRange,
  computeRainbowBrackets,
  detectSemanticLang,
  getCachedSemanticDoc,
  visibleBracketSpansFromIndex,
  type ClientDiagQuickFix,
  type ClientIdeDiagnostic,
  type JavaDiagPrefs,
  type RainbowBracketSpan,
  type SemanticLangKind,
  type SemanticRole,
  type SemanticTokenSpan,
} from './cm-ide-semantic-model.ts';

export {
  analyzeJavaDiagnostics,
  classifySemanticTokens,
  computeImportFoldRange,
  computeRainbowBrackets,
  detectSemanticLang,
  type ClientDiagQuickFix,
  type ClientIdeDiagnostic,
  type JavaDiagPrefs,
  type RainbowBracketSpan,
  type SemanticLangKind,
  type SemanticRole,
  type SemanticTokenSpan,
};

class InlayHintWidget extends WidgetType {
  label: string;
  constructor(label: string) {
    super();
    this.label = label;
  }
  eq(o: InlayHintWidget): boolean {
    return this.label === o.label;
  }
  toDOM(): HTMLElement {
    const el = document.createElement('span');
    el.className = 'cm-ide-inlay-hint';
    el.textContent = `${this.label}:`;
    return el;
  }
}

class CodeVisionWidget extends WidgetType {
  count: number;
  line: number;
  symbol: string;
  onUsages: ((line: number, symbol: string) => void) | undefined;

  constructor(
    count: number,
    line: number,
    symbol: string,
    onUsages?: (line: number, symbol: string) => void,
  ) {
    super();
    this.count = count;
    this.line = line;
    this.symbol = symbol;
    this.onUsages = onUsages;
  }

  eq(o: CodeVisionWidget): boolean {
    return this.count === o.count && this.line === o.line && this.symbol === o.symbol;
  }

  toDOM(): HTMLElement {
    const el = document.createElement('span');
    el.className = 'cm-ide-codevision';
    el.dataset.testid = 'ide-codevision-usages';
    el.textContent = `${this.count} ${this.count === 1 ? 'usage' : 'usages'}`;
    if (this.onUsages) {
      el.addEventListener('mousedown', (e) => {
        e.preventDefault();
        e.stopPropagation();
        this.onUsages?.(this.line, this.symbol);
      });
    }
    return el;
  }
}

export function applyClientQuickFix(view: EditorView, fix: ClientDiagQuickFix): void {
  const doc = view.state.doc;
  if (fix.edits?.length) {
    const changes = fix.edits.map((ed) => {
      const s = doc.line(Math.max(1, Math.min(doc.lines, ed.start_line)));
      const e = doc.line(Math.max(1, Math.min(doc.lines, ed.end_line)));
      const from = Math.min(doc.length, s.from + Math.max(0, (ed.start_col || 1) - 1));
      const to =
        ed.end_line > doc.lines
          ? doc.length
          : Math.max(from, Math.min(doc.length, e.from + Math.max(0, (ed.end_col || 1) - 1)));
      return { from, to, insert: ed.new_text };
    });
    view.dispatch({ changes, userEvent: 'input.quickfix' });
    return;
  }
  const sLn = Math.max(1, Math.min(doc.lines, fix.line));
  const endLn = doc.line(Math.max(sLn, Math.min(doc.lines, fix.endLine || sLn)));
  view.dispatch({
    changes: {
      from: doc.line(sLn).from,
      to: fix.deleteLine ? Math.min(doc.length, endLn.to + 1) : endLn.to,
      insert: fix.deleteLine ? '' : (fix.replacement ?? ''),
    },
    userEvent: 'input.quickfix',
  });
}

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
        const unresChanged = u.transactions.some((t) =>
          t.effects.some((e) => e.is(setServerUnresolvedEffect)),
        );
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
            ? analyzeJavaDiagnostics(docText, prefs)
                .diagnostics.filter((d) => d.severity === 'error')
                .map((d) => d.line)
            : [],
        );
        const r = computeImportFoldRange(docText, lang, errLines);
        if (r && r.to <= view.state.doc.length) {
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
        const { docText } = getCachedSemanticDoc(startDoc, lang, undefined, prefs, false, false);
        const jRes = lang === 'java' ? analyzeJavaDiagnostics(docText, prefs) : null;
        const clientDiags: ClientIdeDiagnostic[] = jRes ? [...jRes.diagnostics] : [];
        const unresolved = new Set<string>(jRes ? jRes.unresolvedIdents : []);

        if (opts.root && (lang === 'java' || lang === 'go' || lang === 'python' || lang === 'shell' || lang === 'json')) {
          const ctrl = new AbortController();
          this.abort = ctrl;
          try {
            const { api } = await import('../api/client.ts');
            type RawDiag = {
              line: number;
              col?: number;
              column?: number;
              end_line?: number;
              end_col?: number;
              severity?: string;
              code?: string;
              source?: string;
              message: string;
              quick_fixes?: Array<{
                title: string;
                line: number;
                end_line?: number;
                replacement?: string;
                delete_line?: boolean;
                edits?: ClientDiagQuickFix['edits'];
              }>;
            };
            const resp = await api.post<{ diagnostics?: RawDiag[] }>(
              '/ide/lsp/diagnostics',
              {
                target: {
                  kind: opts.connectionId ? 'ssh' : 'local',
                  root: opts.root,
                  connection_id: opts.connectionId ?? null,
                  additional_roots: opts.projectFolders,
                },
                path: opts.filePath,
                content: docText,
                language: lang,
                project_folders: opts.projectFolders,
              },
              ctrl.signal,
            );
            for (const sd of resp.diagnostics ?? []) {
              if (prefs.unusedImports === false && (sd.code === 'unused-import' || /unused import/i.test(sd.message))) {
                continue;
              }
              if (
                lang === 'java' &&
                prefs.javaUnresolvedImports === false &&
                (sd.code === 'unresolved-import' || /unresolved import|package .* does not exist/i.test(sd.message))
              ) {
                continue;
              }
              if (
                lang === 'java' &&
                prefs.lombokSupport !== false &&
                /package lombok\b.*does not exist|cannot find symbol.*\bvariable log\b/i.test(sd.message)
              ) {
                continue;
              }
              const duplicate = clientDiags.some(
                (c) => c.line === sd.line && (c.message === sd.message || (Boolean(c.code) && c.code === sd.code)),
              );
              if (!duplicate) {
                clientDiags.push({
                  line: sd.line,
                  column: sd.col ?? sd.column ?? 1,
                  endLine: sd.end_line ?? sd.line,
                  endColumn: sd.end_col ?? 200,
                  severity: sd.severity === 'error' ? 'error' : sd.severity === 'warning' ? 'warning' : 'info',
                  message: sd.message,
                  source: sd.source ?? 'lsp',
                  code: sd.code,
                  quickFixes: (sd.quick_fixes ?? []).map((qf) => ({
                    title: qf.title,
                    line: qf.line,
                    endLine: qf.end_line ?? qf.line,
                    replacement: qf.replacement,
                    deleteLine: qf.delete_line,
                    edits: qf.edits,
                  })),
                });
              }
              if (
                sd.severity === 'error' &&
                (lang !== 'java' || prefs.javaUnresolvedImports !== false) &&
                sd.line >= 1 &&
                sd.line <= view.state.doc.lines
              ) {
                const mImp = /^\s*import\s+(?:static\s+)?[\w.$]*\.([A-Za-z_$][\w$]*)\s*;/.exec(
                  view.state.doc.line(sd.line).text,
                );
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
          const from = Math.min(lnObj.to, lnObj.from + Math.max(0, d.column - 1));
          const to = Math.max(from + 1, Math.min(endLnObj.to, endLnObj.from + Math.max(0, d.endColumn - 1)));
          if (d.quickFixes.length) {
            fixMap.set(d.line, [...(fixMap.get(d.line) ?? []), ...d.quickFixes]);
          }
          const actions: LintAction[] = d.quickFixes.map((qf) => ({
            name: qf.title,
            apply: (v) => applyClientQuickFix(v, qf),
          }));
          cmDiags.push({
            from: Math.min(from, doc.length),
            to: Math.min(to, doc.length),
            severity: d.severity,
            message: d.message,
            source: d.source,
            actions,
          });
          ideDiags.push({
            path: opts.filePath,
            line: d.line,
            column: d.column,
            end_line: d.endLine,
            end_column: d.endColumn,
            severity: d.severity,
            message: d.message,
            source: d.source,
            code: d.code ?? null,
            quick_fixes: d.quickFixes.length
              ? d.quickFixes.map((qf, idx) => ({
                  title: qf.title,
                  line: qf.line,
                  end_line: qf.endLine,
                  replacement: qf.replacement ?? '',
                  delete_line: Boolean(qf.deleteLine),
                  is_preferred: idx === 0,
                  edits: qf.edits?.length
                    ? qf.edits.map((e) => ({ ...e, path: e.path || opts.filePath }))
                    : [
                        {
                          path: opts.filePath,
                          start_line: qf.line,
                          start_col: 1,
                          end_line: qf.deleteLine ? Math.min(doc.lines + 1, qf.endLine + 1) : qf.endLine,
                          end_col:
                            qf.deleteLine && qf.endLine < doc.lines
                              ? 1
                              : doc.line(Math.min(doc.lines, qf.endLine)).length + 1,
                          new_text: qf.deleteLine ? '' : (qf.replacement ?? ''),
                        },
                      ],
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
          const prevEf = Array.isArray(trSpec.effects)
            ? trSpec.effects
            : trSpec.effects
              ? [trSpec.effects]
              : [];
          view.dispatch({
            ...trSpec,
            effects: [...prevEf, setLineQuickFixesEffect.of(fixMap), setServerUnresolvedEffect.of(unresolved)],
          });
          opts.onDiagnostics?.(ideDiags);
        } else {
          view.dispatch({
            effects: [setLineQuickFixesEffect.of(fixMap), setServerUnresolvedEffect.of(unresolved)],
          });
        }
      }

      compute(view: EditorView): DecorationSet {
        const prefs = opts.getPreferences();
        const vis = view.visibleRanges;
        const extraUnres = view.state.field(serverUnresolvedField, false);
        const wantBrackets = Boolean(prefs.rainbowBrackets && lang !== 'csv');
        const wantCv = Boolean(prefs.codeVision && lang !== 'csv' && lang !== 'json');
        const cached = getCachedSemanticDoc(
          view.state.doc,
          lang,
          extraUnres,
          prefs,
          wantBrackets,
          wantCv,
        );
        const { docText, model, identCounts, brackets } = cached;
        const items: Range<Decoration>[] = [];

        for (const tok of classifySemanticTokensWithModel(docText, lang, model, vis)) {
          if (tok.to > tok.from) {
            items.push(Decoration.mark({ class: `cm-syn-${tok.role}` }).range(tok.from, tok.to));
          }
        }

        if (wantBrackets && brackets) {
          const spans = visibleBracketSpansFromIndex(brackets, vis, view.state.selection.main.head);
          for (const rb of spans) {
            const cls =
              rb.level === 'unmatched'
                ? 'cm-syn-rb-unmatched'
                : `cm-syn-rb-${rb.level}${rb.active ? ' cm-syn-rb-active' : ''}`;
            items.push(Decoration.mark({ class: cls }).range(rb.from, rb.to));
          }
        }

        if ((prefs.inlayHints || prefs.codeVision) && docText.length <= 131072 && lang !== 'csv' && lang !== 'json') {
          if (prefs.inlayHints && model.funcParams.size) {
            let hintCount = 0;
            for (const { from, to } of vis) {
              const slice = docText.slice(from, to);
              const callRe = /\b([A-Za-z_$][\w$]*)\s*\(([^()\n]{1,120})\)/g;
              let m: RegExpExecArray | null;
              while ((m = callRe.exec(slice)) !== null && hintCount < 24) {
                const pNames = model.funcParams.get(m[1]);
                if (!pNames || model.declStarts.has(from + m.index)) continue;
                const argsStart = from + m.index + m[0].indexOf('(') + 1;
                let cursor = 0;
                for (const [aIdx, rawArg] of m[2].split(',').entries()) {
                  if (aIdx >= pNames.length) break;
                  const trimmedArg = rawArg.trim();
                  const pName = pNames[aIdx];
                  if (
                    trimmedArg &&
                    trimmedArg !== pName &&
                    !trimmedArg.startsWith(`${pName}:`) &&
                    !trimmedArg.startsWith(`${pName}=`)
                  ) {
                    const pos = argsStart + cursor + (rawArg.length - rawArg.trimStart().length);
                    items.push(Decoration.widget({ widget: new InlayHintWidget(pName), side: -1 }).range(pos));
                    hintCount++;
                  }
                  cursor += rawArg.length + 1;
                }
              }
            }
          }
          if (prefs.codeVision && identCounts) {
            let cvCount = 0;
            for (const decl of model.funcDecls) {
              if (cvCount >= 16 || !vis.some((r) => decl.lineEnd >= r.from && decl.lineEnd <= r.to)) continue;
              const usages = Math.max(0, (identCounts.get(decl.name) ?? 1) - 1);
              const widget = new CodeVisionWidget(
                usages,
                decl.line,
                decl.name,
                opts.onFindUsages ? (ln, sym) => opts.onFindUsages?.({ line: ln, column: 1, symbol: sym }) : undefined,
              );
              items.push(Decoration.widget({ widget, side: 1 }).range(decl.lineEnd));
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
