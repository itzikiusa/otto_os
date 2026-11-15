// GoLand-style Cmd/Ctrl-Hover symbol underline + Cmd+Click / Cmd+Alt+Click
// Dive-In (Definition, TypeDefinition, Implementation) for CodeMirror 6.

import { EditorState, Prec, StateEffect, StateField, type Extension } from '@codemirror/state';
import { Decoration, EditorView, type DecorationSet } from '@codemirror/view';
import { ctxMenu } from '../contextmenu.svelte';
import { posToLsp, type LspRange } from './cm-ide-lsp-completion.ts';

export interface IdeLspJumpTarget {
  path?: string;
  line: number;
  column: number;
  symbol?: string;
  originLine?: number;
  originColumn?: number;
}

export interface IdeDiveCallbacks {
  onDefinitionJump?: ((target: IdeLspJumpTarget) => void) | null;
  onImplementationJump?: ((target: IdeLspJumpTarget) => void) | null;
}

interface RawLspLocation {
  uri?: string;
  targetUri?: string;
  range?: LspRange;
  targetSelectionRange?: LspRange;
  targetRange?: LspRange;
}

export function lspUriToPath(uri: string): string {
  if (!uri.startsWith('file://')) return uri;
  try {
    return decodeURIComponent(uri.slice('file://'.length));
  } catch {
    return uri.slice('file://'.length);
  }
}

export function pathBasename(p: string): string {
  const clean = p.split('?')[0];
  const idx = Math.max(clean.lastIndexOf('/'), clean.lastIndexOf('\\'));
  return idx >= 0 ? clean.slice(idx + 1) : clean;
}

export function normalizeLocations(raw: unknown): Array<{ path: string; line: number; column: number }> {
  if (!raw) return [];
  const arr = Array.isArray(raw) ? raw : [raw];
  const out: Array<{ path: string; line: number; column: number }> = [];
  const seen = new Set<string>();
  for (const entry of arr as RawLspLocation[]) {
    if (!entry || typeof entry !== 'object') continue;
    const uri = entry.targetUri ?? entry.uri;
    const start = entry.targetSelectionRange?.start ?? entry.range?.start ?? entry.targetRange?.start;
    if (!uri || !start) continue;
    const path = lspUriToPath(uri);
    const line = start.line + 1;
    const column = start.character + 1;
    const key = `${path}:${line}:${column}`;
    if (!seen.has(key)) {
      seen.add(key);
      out.push({ path, line, column });
    }
  }
  return out;
}

const IDENT_OR_PATH_RE = /[\w$./~#-]/;
const IDENT_RE = /[\w$]/;

export function tokenSpanAtPos(
  state: EditorState,
  pos: number,
): { from: number; to: number; word: string; isStringPath: boolean } | null {
  if (pos < 0 || pos > state.doc.length) return null;
  const ln = state.doc.lineAt(pos);
  const rel = pos - ln.from;
  const text = ln.text;
  if (!text) return null;

  for (const quote of ['"', "'", '`']) {
    const qStart = text.lastIndexOf(quote, Math.max(0, rel - 1));
    if (qStart >= 0) {
      const qEnd = text.indexOf(quote, rel);
      if (qEnd > qStart) {
        const inner = text.slice(qStart + 1, qEnd).trim();
        if (inner.length > 0 && (inner.includes('/') || inner.includes('.') || inner.startsWith('#'))) {
          return { from: ln.from + qStart + 1, to: ln.from + qEnd, word: inner, isStringPath: true };
        }
      }
    }
  }

  if (/(?:^|[;&|]\s*)(?:source|\.)\s+/.test(text.slice(0, rel + 16))) {
    let ps = rel;
    let pe = rel;
    while (ps > 0 && IDENT_OR_PATH_RE.test(text[ps - 1])) ps--;
    while (pe < text.length && IDENT_OR_PATH_RE.test(text[pe])) pe++;
    const candidate = text.slice(ps, pe);
    if (candidate.includes('/') || candidate.endsWith('.sh') || candidate.endsWith('.bash') || candidate.endsWith('.zsh')) {
      return { from: ln.from + ps, to: ln.from + pe, word: candidate, isStringPath: true };
    }
  }

  let s = rel;
  let e = rel;
  while (s > 0 && IDENT_RE.test(text[s - 1])) s--;
  while (e < text.length && IDENT_RE.test(text[e])) e++;
  if (e <= s) return null;
  return { from: ln.from + s, to: ln.from + e, word: text.slice(s, e), isStringPath: false };
}

export type DiveLocationLookup = (
  view: EditorView,
  pos: number,
  mode: 'definition' | 'implementation',
) => Promise<Array<{ path: string; line: number; column: number }>>;

let diveLookupProvider: DiveLocationLookup | null = null;

export function registerDiveLocationLookup(fn: DiveLocationLookup): void {
  diveLookupProvider = fn;
}

export async function requestIdeDiveAtPos(
  view: EditorView,
  pos: number,
  mode: 'definition' | 'implementation',
  cbs: IdeDiveCallbacks,
  clickEvent?: MouseEvent,
): Promise<void> {
  const cb = mode === 'implementation' ? (cbs.onImplementationJump ?? cbs.onDefinitionJump) : cbs.onDefinitionJump;
  if (!cb) return;

  const ln = view.state.doc.lineAt(Math.max(0, Math.min(view.state.doc.length, pos)));
  const column = pos - ln.from + 1;
  const span = tokenSpanAtPos(view.state, pos);
  const locs = diveLookupProvider ? await diveLookupProvider(view, pos, mode) : [];

  if (locs.length === 1) {
    cb({ path: locs[0].path, line: locs[0].line, column: locs[0].column, symbol: span?.word, originLine: ln.number, originColumn: column });
    return;
  }
  if (locs.length > 1) {
    const items = locs.slice(0, 40).map((loc) => ({
      label: `${pathBasename(loc.path)}:${loc.line}`,
      hint: loc.path,
      icon: 'file' as const,
      action: () => cb({ path: loc.path, line: loc.line, column: loc.column, symbol: span?.word, originLine: ln.number, originColumn: column }),
    }));
    if (clickEvent) {
      ctxMenu.show(clickEvent, items);
    } else {
      const coords = view.coordsAtPos(pos);
      if (coords) {
        ctxMenu.show({ clientX: coords.left, clientY: coords.bottom, preventDefault() {}, stopPropagation() {} } as MouseEvent, items);
      } else {
        cb({ path: locs[0].path, line: locs[0].line, column: locs[0].column, symbol: span?.word, originLine: ln.number, originColumn: column });
      }
    }
    return;
  }
  cb({ line: ln.number, column, symbol: span?.word, originLine: ln.number, originColumn: column });
}

const setCmdHoverRange = StateEffect.define<{ from: number; to: number } | null>();
const cmdLinkMark = Decoration.mark({ class: 'cm-ide-cmd-link' });

export const lspTooltipTheme = EditorView.baseTheme({
  '.cm-ide-cmd-link': {
    color: 'var(--accent-text)',
    textDecoration: 'underline',
    textDecorationColor: 'var(--accent-text)',
    textUnderlineOffset: '2px',
    cursor: 'pointer',
  },
  '.cm-tooltip.cm-lsp-hover-tooltip, .cm-lsp-hover-tooltip': {
    maxWidth: 'min(540px, calc(100vw - 24px))',
    maxHeight: '260px',
    overflow: 'auto',
    padding: 'var(--sp-2) var(--sp-3)',
    backgroundColor: 'var(--surface-2)',
    color: 'var(--text)',
    border: '1px solid var(--border)',
    borderRadius: 'var(--radius-s)',
    fontFamily: 'var(--font-mono)',
    fontSize: 'var(--fs-xs)',
    lineHeight: '1.45',
  },
});

export function createCmdHoverDiveExtension(getCallbacks: () => IdeDiveCallbacks): Extension {
  const hoverField = StateField.define<{ range: { from: number; to: number } | null; decos: DecorationSet }>({
    create: () => ({ range: null, decos: Decoration.none }),
    update(cur, tr) {
      for (const ef of tr.effects) {
        if (ef.is(setCmdHoverRange)) {
          const r = ef.value;
          if (!r || r.to <= r.from || r.to > tr.state.doc.length) return { range: null, decos: Decoration.none };
          return { range: r, decos: Decoration.set([cmdLinkMark.range(r.from, r.to)]) };
        }
      }
      if (tr.docChanged && cur.range) return { range: null, decos: Decoration.none };
      return cur;
    },
    provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
  });

  let lastCoords: { x: number; y: number } | null = null;
  const updateHoverAt = (view: EditorView, active: boolean): void => {
    const cur = view.state.field(hoverField).range;
    if (!active || !lastCoords) {
      if (cur) view.dispatch({ effects: setCmdHoverRange.of(null) });
      return;
    }
    const pos = view.posAtCoords(lastCoords);
    const span = pos == null ? null : tokenSpanAtPos(view.state, pos);
    const next = span ? { from: span.from, to: span.to } : null;
    if ((cur?.from ?? -1) !== (next?.from ?? -1) || (cur?.to ?? -1) !== (next?.to ?? -1)) {
      view.dispatch({ effects: setCmdHoverRange.of(next) });
    }
  };

  const handlers = Prec.high(EditorView.domEventHandlers({
    mousemove(event, view) {
      lastCoords = { x: event.clientX, y: event.clientY };
      updateHoverAt(view, event.metaKey || event.ctrlKey);
      return false;
    },
    mouseleave(_event, view) {
      lastCoords = null;
      updateHoverAt(view, false);
      return false;
    },
    keydown(event, view) {
      if (event.key === 'Meta' || event.key === 'Control') updateHoverAt(view, true);
      return false;
    },
    keyup(event, view) {
      if (event.key === 'Meta' || event.key === 'Control') updateHoverAt(view, false);
      return false;
    },
    blur(_event, view) {
      updateHoverAt(view, false);
      return false;
    },
    mousedown(event, view) {
      if (event.button !== 0 || !(event.metaKey || event.ctrlKey) || event.shiftKey) return false;
      if (event.target instanceof Element && event.target.closest('.cm-gutters')) return false;
      const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
      if (pos == null) return false;
      const cbs = getCallbacks();
      const mode = event.altKey ? 'implementation' : 'definition';
      if (mode === 'definition' && !cbs.onDefinitionJump) return false;
      if (mode === 'implementation' && !cbs.onImplementationJump && !cbs.onDefinitionJump) return false;
      event.preventDefault();
      view.dispatch({ selection: { anchor: pos, head: pos } });
      void requestIdeDiveAtPos(view, pos, mode, cbs, event);
      return true;
    },
  }));

  return [lspTooltipTheme, hoverField, handlers];
}

// suppress unused EditorState import warning when only used via tokenSpanAtPos signature
export type _DiveEditorState = EditorState;
