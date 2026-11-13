// Pooled CodeMirror 6 LSP Client + GoLand Cmd-Hover Dive-In (Definition & Implementation),
// Signature Help, Auto-Imports (`additionalTextEdits`), and Direct JSON-RPC WS Fallback.

import {
  Decoration, EditorView, ViewPlugin, hoverTooltip,
  type DecorationSet, type ViewUpdate,
} from '@codemirror/view';
import {
  EditorState, Prec, StateEffect, StateField, type ChangeSpec, type Extension,
} from '@codemirror/state';
import type { Completion, CompletionContext, CompletionResult } from '@codemirror/autocomplete';
import { setDiagnostics, type Diagnostic } from '@codemirror/lint';
import type { IdeDiagnostic } from '../api/types';
import { ctxMenu } from '../contextmenu.svelte';

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

interface LspPosition {
  line: number;
  character: number;
}

interface LspRange {
  start: LspPosition;
  end: LspPosition;
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

function pathBasename(p: string): string {
  const clean = p.split('?')[0];
  const idx = Math.max(clean.lastIndexOf('/'), clean.lastIndexOf('\\'));
  return idx >= 0 ? clean.slice(idx + 1) : clean;
}

function posToLsp(doc: EditorState['doc'], pos: number): LspPosition {
  const clamped = Math.max(0, Math.min(doc.length, pos));
  const ln = doc.lineAt(clamped);
  return { line: ln.number - 1, character: clamped - ln.from };
}

function lspToPos(doc: EditorState['doc'], p: LspPosition): number {
  const ln = doc.line(Math.max(1, Math.min(doc.lines, p.line + 1)));
  return Math.min(ln.to, ln.from + Math.max(0, p.character));
}

function normalizeLocations(raw: unknown): Array<{ path: string; line: number; column: number }> {
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

// ── Pooled LanguageServerClient Registry (Refcounted + <=2 LRU Cap) ──────────

type MarimoMod = typeof import('@marimo-team/codemirror-languageserver');
const MAX_POOLED_LSP_CLIENTS = 2;
const IDLE_CLOSE_MS = 250;

interface PooledClientEntry {
  key: string;
  rootPath: string;
  client: InstanceType<MarimoMod['LanguageServerClient']>;
  transport: InstanceType<MarimoMod['WebSocketTransport']>;
  offTransportClose: (() => void) | null;
  evictFn: () => void;
  diagListeners: Map<string, Set<(diags: IdeDiagnostic[]) => void>>;
  refCount: number;
  closeTimer: ReturnType<typeof setTimeout> | null;
  disposed: boolean;
}

let marimoPromise: Promise<MarimoMod> | null = null;
let marimoCache: MarimoMod | null = null;
const clientPool = new Map<string, PooledClientEntry>();

function loadMarimo(): Promise<MarimoMod> {
  if (marimoCache) return Promise.resolve(marimoCache);
  return (marimoPromise ??= import('@marimo-team/codemirror-languageserver').then((m) => ((marimoCache = m), m)));
}

function disposePooledEntry(entry: PooledClientEntry): void {
  if (entry.disposed) return;
  entry.disposed = true;
  if (entry.closeTimer !== null) {
    clearTimeout(entry.closeTimer);
    entry.closeTimer = null;
  }
  if (clientPool.get(entry.key) === entry) clientPool.delete(entry.key);
  entry.diagListeners.clear();
  try { entry.offTransportClose?.(); } catch { /* ignore */ }
  entry.offTransportClose = null;
  try { (entry.client as unknown as { close?: () => void }).close?.(); } catch { /* ignore */ }
  try { entry.transport.close(); } catch { /* ignore */ }
  try { (entry.transport as unknown as { socket?: WebSocket }).socket?.close(); } catch { /* ignore */ }
}

function scheduleIdleDispose(entry: PooledClientEntry, delayMs = IDLE_CLOSE_MS): void {
  if (entry.disposed) return;
  if (entry.closeTimer !== null) clearTimeout(entry.closeTimer);
  entry.closeTimer = setTimeout(() => {
    entry.closeTimer = null;
    if (entry.refCount <= 0) disposePooledEntry(entry);
  }, delayMs);
}

/** Deterministically close pooled LSP WebSocket clients (e.g. on workspace switch or closing all tabs). */
export function disposePooledLspClients(opts?: { rootPath?: string; forceAll?: boolean }): void {
  for (const entry of [...clientPool.values()]) {
    if (opts?.rootPath && entry.rootPath !== opts.rootPath) continue;
    if (opts?.forceAll || entry.refCount <= 0) disposePooledEntry(entry);
  }
}

export interface AttachPooledLspOptions {
  wsUri: string;
  protocols?: string[];
  rootPath: string;
  connId?: string | null;
  filePath: string;
  languageId: string;
  onLocationJump: (uri: string, line: number, column: number) => void;
  onDiagnostics?: ((diags: IdeDiagnostic[]) => void) | null;
}

function acquirePooledClient(mod: MarimoMod, opts: AttachPooledLspOptions): PooledClientEntry {
  const key = `${opts.languageId}::${opts.connId ?? 'local'}::${opts.rootPath}`;
  const existing = clientPool.get(key);
  if (existing && !existing.disposed) {
    clientPool.delete(key);
    clientPool.set(key, existing);
    if (existing.closeTimer !== null) {
      clearTimeout(existing.closeTimer);
      existing.closeTimer = null;
    }
    return existing;
  }

  while (clientPool.size >= MAX_POOLED_LSP_CLIENTS) {
    let victim: PooledClientEntry | undefined;
    for (const candidate of clientPool.values()) {
      if (candidate.refCount <= 0) { victim = candidate; break; }
    }
    victim ??= clientPool.values().next().value;
    if (!victim) break;
    disposePooledEntry(victim);
  }

  const rootUri = `file://${opts.rootPath}`;
  const transport = new mod.WebSocketTransport(opts.wsUri, opts.protocols);
  const diagListeners = new Map<string, Set<(diags: IdeDiagnostic[]) => void>>();

  const client = new mod.LanguageServerClient({
    rootUri,
    workspaceFolders: [{ name: pathBasename(opts.rootPath) || 'workspace', uri: rootUri }],
    transport,
    capabilities: (defaults) => ({
      ...defaults,
      textDocument: {
        ...defaults?.textDocument,
        definition: { dynamicRegistration: true, linkSupport: true },
        typeDefinition: { dynamicRegistration: true, linkSupport: true },
        implementation: { dynamicRegistration: true, linkSupport: true },
        completion: {
          ...defaults?.textDocument?.completion,
          completionItem: {
            ...defaults?.textDocument?.completion?.completionItem,
            snippetSupport: true,
            additionalTextEditsSupport: true,
            resolveSupport: { properties: ['documentation', 'detail', 'additionalTextEdits'] },
          },
        },
      },
    }),
  });

  const entry: PooledClientEntry = {
    key,
    rootPath: opts.rootPath,
    client,
    transport,
    offTransportClose: null,
    evictFn: () => {},
    diagListeners,
    refCount: 0,
    closeTimer: null,
    disposed: false,
  };
  entry.evictFn = () => disposePooledEntry(entry);
  entry.offTransportClose = transport.onClose?.(() => entry.evictFn()) ?? null;
  void client.initializePromise.catch(() => entry.evictFn());

  client.onNotification((n) => {
    if (entry.disposed || n.method !== 'textDocument/publishDiagnostics' || !n.params?.uri) return;
    const subs = diagListeners.get(n.params.uri);
    if (!subs || subs.size === 0) return;
    const fPath = lspUriToPath(n.params.uri);
    const mapped: IdeDiagnostic[] = (n.params.diagnostics ?? []).map((d) => ({
      path: fPath,
      line: d.range.start.line + 1,
      column: d.range.start.character + 1,
      end_line: d.range.end.line + 1,
      end_column: d.range.end.character + 1,
      severity: d.severity === 1 ? 'error' : d.severity === 2 ? 'warning' : d.severity === 4 ? 'hint' : 'info',
      message: typeof d.message === 'string' ? d.message : d.message.value,
      source: d.source ?? null,
      code: d.code != null ? String(d.code) : null,
    }));
    for (const cb of subs) cb(mapped);
  });

  clientPool.set(key, entry);
  scheduleIdleDispose(entry, 600);
  return entry;
}

export async function buildPooledLspExtensions(opts: AttachPooledLspOptions): Promise<Extension[]> {
  const mod = await loadMarimo();
  const pooled = acquirePooledClient(mod, opts);
  const documentUri = opts.filePath.includes('://') ? opts.filePath : `file://${opts.filePath}`;

  const lifecycleCleanup = ViewPlugin.fromClass(class {
    private unsubDiag: (() => void) | null = null;
    constructor() {
      if (pooled.closeTimer !== null) {
        clearTimeout(pooled.closeTimer);
        pooled.closeTimer = null;
      }
      pooled.refCount += 1;
      if (opts.onDiagnostics) {
        const cb = opts.onDiagnostics;
        let set = pooled.diagListeners.get(documentUri);
        if (!set) { set = new Set(); pooled.diagListeners.set(documentUri, set); }
        set.add(cb);
        this.unsubDiag = () => {
          set?.delete(cb);
          if (set && set.size === 0) pooled.diagListeners.delete(documentUri);
        };
      }
    }
    destroy() {
      this.unsubDiag?.();
      this.unsubDiag = null;
      pooled.refCount = Math.max(0, pooled.refCount - 1);
      if (pooled.refCount <= 0) scheduleIdleDispose(pooled);
    }
  });

  const exts = mod.languageServerWithClient({
    client: pooled.client,
    documentUri,
    languageId: opts.languageId,
    signatureHelpEnabled: true,
    signatureActivateOnTyping: true,
    useSnippetOnCompletion: true,
    clientSideFiltering: true,
    renameEnabled: false,
    keyboardShortcuts: {
      goToDefinition: 'F13',
      rename: 'F13',
      codeActions: 'F13',
      signatureHelp: 'Mod-Shift-Space',
    },
    onGoToDefinition: (res) => opts.onLocationJump(res.uri, res.range.start.line + 1, res.range.start.character + 1),
    onShowLocation: (res) => opts.onLocationJump(res.uri, res.range.start.line + 1, res.range.start.character + 1),
  });

  return [lspTooltipTheme, lifecycleCleanup, ...exts];
}

/** Notify the active LSP plugin on `view` that the buffer was saved (`willSave` / `didSave`). */
export function notifyLspDidSave(view: EditorView | null): void {
  if (!view || !marimoCache) return;
  const plugin = marimoCache.getLanguageServerPlugin(view);
  if (plugin) void plugin.documentDidSave().catch(() => {});
}

// ── Programmatic Dive-In (Definition / TypeDefinition / Implementation) ──────

const IDENT_OR_PATH_RE = /[\w$./~#-]/;
const IDENT_RE = /[\w$]/;

export function tokenSpanAtPos(state: EditorState, pos: number): { from: number; to: number; word: string; isStringPath: boolean } | null {
  if (pos < 0 || pos > state.doc.length) return null;
  const ln = state.doc.lineAt(pos);
  const rel = pos - ln.from;
  const text = ln.text;
  if (!text) return null;

  // Check if cursor is inside a quoted string containing a relative path / $ref / module specifier.
  for (const quote of ['"', "'", '`']) {
    let qStart = text.lastIndexOf(quote, Math.max(0, rel - 1));
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

  // Also check unquoted shell `source ./foo.sh` / `. ./foo.sh` path tokens.
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

async function queryPluginLocations(
  view: EditorView,
  pos: number,
  mode: 'definition' | 'implementation',
): Promise<Array<{ path: string; line: number; column: number }>> {
  if (!marimoCache) return [];
  const plugin = marimoCache.getLanguageServerPlugin(view);
  if (!plugin || !plugin.client.ready) return [];
  const position = posToLsp(view.state.doc, pos);
  const params = { textDocument: { uri: plugin.documentUri }, position };
  const rpc = (plugin.client as unknown as { client?: { request: (m: string, p: unknown) => Promise<unknown> } }).client;

  try {
    if (mode === 'implementation') {
      if (rpc && plugin.client.hasCapability('textDocument/implementation')) {
        const res = await rpc.request('textDocument/implementation', params);
        const locs = normalizeLocations(res);
        if (locs.length > 0) return locs;
      }
    } else {
      if (plugin.client.hasCapability('textDocument/definition')) {
        const res = await plugin.client.textDocumentDefinition(params);
        const locs = normalizeLocations(res);
        if (locs.length > 0) return locs;
      }
      if (rpc && plugin.client.hasCapability('textDocument/typeDefinition')) {
        const res = await rpc.request('textDocument/typeDefinition', params);
        const locs = normalizeLocations(res);
        if (locs.length > 0) return locs;
      }
    }
  } catch {
    /* Fall through to backend HTTP / lexical dive-in */
  }
  return [];
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

  // If the user Cmd+Clicked a string path (`./foo.ts`, `source ./lib.sh`, `"$ref": "./a.json"`),
  // hand the string token straight to the host resolver first if LSP has no jump.
  const locs = await queryPluginLocations(view, pos, mode);
  if (locs.length === 1) {
    cb({
      path: locs[0].path,
      line: locs[0].line,
      column: locs[0].column,
      symbol: span?.word,
      originLine: ln.number,
      originColumn: column,
    });
    return;
  }
  if (locs.length > 1) {
    const items = locs.slice(0, 40).map((loc) => ({
      label: `${pathBasename(loc.path)}:${loc.line}`,
      hint: loc.path,
      icon: 'file' as const,
      action: () =>
        cb({
          path: loc.path,
          line: loc.line,
          column: loc.column,
          symbol: span?.word,
          originLine: ln.number,
          originColumn: column,
        }),
    }));
    if (clickEvent) {
      ctxMenu.show(clickEvent, items);
    } else {
      const coords = view.coordsAtPos(pos);
      if (coords) {
        ctxMenu.show({ clientX: coords.left, clientY: coords.bottom, preventDefault() {}, stopPropagation() {} } as MouseEvent, items);
      } else {
        cb({
          path: locs[0].path,
          line: locs[0].line,
          column: locs[0].column,
          symbol: span?.word,
          originLine: ln.number,
          originColumn: column,
        });
      }
    }
    return;
  }

  // Fallback to backend HTTP `/ide/lsp/definition` | `/ide/lsp/implementation` + local fast resolver.
  cb({ line: ln.number, column, symbol: span?.word, originLine: ln.number, originColumn: column });
}

// ── Cmd/Ctrl Hover Symbol Underline + Cmd+Click / Cmd+Alt+Click Extension ────

const setCmdHoverRange = StateEffect.define<{ from: number; to: number } | null>();
const cmdLinkMark = Decoration.mark({ class: 'cm-ide-cmd-link' });

const lspTooltipTheme = EditorView.baseTheme({
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

// ── Direct WebSocket JSON-RPC LSP Fallback (Zero Node Polyfills) ─────────────

export interface DirectLspOptions {
  wsUri: string;
  protocols?: string[];
  rootUri: string;
  documentUri: string;
  filePath: string;
  languageId: string;
  onLocationJump?: (uri: string, line: number, column: number) => void;
  onDiagnostics?: (diags: IdeDiagnostic[]) => void;
}

export function createDirectLspExtension(opts: DirectLspOptions): Extension {
  let socket: WebSocket | null = null;
  let nextId = 1;
  let version = 1;
  let ready = false;
  let closed = false;
  let liveView: EditorView | null = null;
  const pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();

  const send = (p: Record<string, unknown>) => {
    if (!closed && socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify(p));
  };
  const request = <T = unknown>(method: string, params: unknown): Promise<T> =>
    new Promise<T>((resolve, reject) => {
      if (!ready || socket?.readyState !== WebSocket.OPEN) return reject(new Error('LSP unavailable'));
      const id = nextId++;
      pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
      send({ jsonrpc: '2.0', id, method, params });
      setTimeout(() => { if (pending.delete(id)) reject(new Error('LSP timeout')); }, 6000);
    });

  const plugin = ViewPlugin.fromClass(class {
    constructor(view: EditorView) {
      liveView = view;
      try { socket = new WebSocket(opts.wsUri, opts.protocols); } catch { return; }
      socket.addEventListener('open', () => {
        if (closed || !socket) return;
        const initId = nextId++;
        pending.set(initId, {
          resolve: () => {
            ready = true;
            send({ jsonrpc: '2.0', method: 'initialized', params: {} });
            send({ jsonrpc: '2.0', method: 'textDocument/didOpen', params: { textDocument: { uri: opts.documentUri, languageId: opts.languageId, version, text: view.state.doc.toString() } } });
          },
          reject: () => {},
        });
        send({
          jsonrpc: '2.0', id: initId, method: 'initialize',
          params: {
            processId: null, rootUri: opts.rootUri, workspaceFolders: [{ uri: opts.rootUri, name: 'workspace' }],
            capabilities: { textDocument: { hover: { contentFormat: ['plaintext', 'markdown'] }, completion: { completionItem: { snippetSupport: false, additionalTextEditsSupport: true } }, definition: { linkSupport: true }, implementation: { linkSupport: true }, publishDiagnostics: { relatedInformation: true } } },
          },
        });
      });
      socket.addEventListener('message', (ev) => {
        if (closed || typeof ev.data !== 'string') return;
        let msg: Record<string, unknown>;
        try { msg = JSON.parse(ev.data) as Record<string, unknown>; } catch { return; }
        if (typeof msg.id === 'number' && !msg.method) {
          const p = pending.get(msg.id);
          if (p) { pending.delete(msg.id); if (msg.error) p.reject(new Error('LSP error')); else p.resolve(msg.result); }
          return;
        }
        if (typeof msg.id === 'number' && typeof msg.method === 'string') {
          send({ jsonrpc: '2.0', id: msg.id, result: null });
          return;
        }
        if (msg.method === 'textDocument/publishDiagnostics' && liveView) {
          const params = msg.params as { uri?: string; diagnostics?: Array<{ range: LspRange; severity?: number; message: string; source?: string; code?: string | number }> };
          if (params?.uri && params.uri !== opts.documentUri) return;
          const raw = params?.diagnostics ?? [];
          const doc = liveView.state.doc;
          const cmDiags: Diagnostic[] = raw.map((d) => ({
            from: lspToPos(doc, d.range.start),
            to: Math.max(lspToPos(doc, d.range.start), lspToPos(doc, d.range.end)),
            severity: d.severity === 1 ? 'error' : d.severity === 2 ? 'warning' : 'info',
            message: d.message, source: d.source,
          }));
          liveView.dispatch(setDiagnostics(liveView.state, cmDiags));
          opts.onDiagnostics?.(raw.map((d) => ({
            path: opts.filePath, line: d.range.start.line + 1, column: d.range.start.character + 1,
            end_line: d.range.end.line + 1, end_column: d.range.end.character + 1,
            severity: d.severity === 1 ? 'error' : d.severity === 2 ? 'warning' : d.severity === 4 ? 'hint' : 'info',
            message: d.message, source: d.source ?? null, code: d.code != null ? String(d.code) : null,
          })));
        }
      });
    }
    update(u: ViewUpdate) {
      if (u.docChanged && ready) {
        version++;
        send({ jsonrpc: '2.0', method: 'textDocument/didChange', params: { textDocument: { uri: opts.documentUri, version }, contentChanges: [{ text: u.state.doc.toString() }] } });
      }
    }
    destroy() {
      closed = true;
      if (ready && socket?.readyState === WebSocket.OPEN) {
        try { socket.send(JSON.stringify({ jsonrpc: '2.0', method: 'textDocument/didClose', params: { textDocument: { uri: opts.documentUri } } })); } catch { /* ignore */ }
      }
      for (const [, p] of pending) p.reject(new Error('LSP closed'));
      pending.clear();
      socket?.close();
      socket = null;
      liveView = null;
    }
  });

  const hover = hoverTooltip(async (view, pos) => {
    if (!ready) return null;
    try {
      const res = await request<{ contents?: string | { value?: string } | Array<string | { value?: string }> } | null>('textDocument/hover', {
        textDocument: { uri: opts.documentUri }, position: posToLsp(view.state.doc, pos),
      });
      if (!res?.contents) return null;
      const parts = Array.isArray(res.contents) ? res.contents : [res.contents];
      const text = parts.map((p) => (typeof p === 'string' ? p : (p?.value ?? ''))).filter(Boolean).join('\n\n').trim();
      if (!text) return null;
      return {
        pos,
        create() {
          const dom = document.createElement('div');
          dom.className = 'cm-lsp-hover-tooltip';
          dom.style.whiteSpace = 'pre-wrap';
          dom.textContent = text;
          return { dom };
        },
      };
    } catch { return null; }
  });

  type RawCompItem = { label: string; detail?: string; insertText?: string; additionalTextEdits?: Array<{ range: LspRange; newText: string }> };
  const completion = EditorState.languageData.of(() => [{
    autocomplete: async (ctx: CompletionContext): Promise<CompletionResult | null> => {
      if (!ready) return null;
      const word = ctx.matchBefore(/[\w$]+/);
      if (!word && !ctx.explicit) return null;
      try {
        const res = await request<RawCompItem[] | { items?: RawCompItem[] } | null>('textDocument/completion', {
          textDocument: { uri: opts.documentUri }, position: posToLsp(ctx.state.doc, ctx.pos),
        });
        const items = Array.isArray(res) ? res : (res?.items ?? []);
        if (!items.length) return null;
        const from = word ? word.from : ctx.pos;
        const options: Completion[] = items.slice(0, 120).map((it) => ({
          label: it.label,
          detail: it.detail,
          apply: (view, _c, f, t) => {
            const insert = it.insertText || it.label;
            const changes: ChangeSpec[] = [{ from: f, to: t, insert }];
            for (const ed of it.additionalTextEdits ?? []) {
              changes.push({ from: lspToPos(view.state.doc, ed.range.start), to: lspToPos(view.state.doc, ed.range.end), insert: ed.newText });
            }
            view.dispatch({ changes, userEvent: 'input.complete' });
          },
        }));
        return { from, options };
      } catch { return null; }
    },
  }]);

  return [lspTooltipTheme, plugin, hover, completion];
}
