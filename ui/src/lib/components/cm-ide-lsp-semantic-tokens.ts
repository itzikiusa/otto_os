// LSP `textDocument/semanticTokens/full` legend, delta decoder, and CodeMirror 6
// decoration plugin extracted from `cm-ide-lsp-completion.ts`.

import { EditorState, StateEffect, StateField, type Extension, type Range } from '@codemirror/state';
import { Decoration, EditorView, ViewPlugin, type DecorationSet, type ViewUpdate } from '@codemirror/view';

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
