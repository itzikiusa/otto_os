// Pooled CodeMirror 6 LSP Client + GoLand Cmd-Hover Dive-In (Definition & Implementation),
// Signature Help, Dot-Triggered Auto-Imports (`additionalTextEdits`), `semanticTokens/full`,
// and Direct JSON-RPC WS Fallback.

import { EditorView, ViewPlugin } from '@codemirror/view';
import type { Extension } from '@codemirror/state';
import type { IdeDiagnostic } from '../api/types';
import {
  clearLiveLspDiagnostics,
  hasLiveLspDiagnosticsForFile,
  isSecretIdePath,
  markLiveLspDiagnostics,
} from './cm-ide-drafts.ts';
import {
  buildLspCompletionChangeSpec,
  buildLspInitializationOptions,
  createIdeLspCompletionExtension,
  createLspSemanticTokensExtension,
  decodeLspSemanticTokens,
  DEFAULT_LSP_TOKEN_MODIFIERS,
  DEFAULT_LSP_TOKEN_TYPES,
  detectLspCompletionTrigger,
  IDLE_CLOSE_MS,
  isPosInCommentOrString,
  LSP_CODE_LANGUAGES,
  MAX_POOLED_LSP_CLIENTS,
  normalizeIdeLspLanguage,
  posToLsp,
  resolveIdeLspModuleRoot,
  shouldAttachIdeLsp,
  toAbsoluteIdeFilePath,
  type LspPos,
  type LspRange,
  type RawLspCompletionItem,
  type RawLspTextEdit,
} from './cm-ide-lsp-completion.ts';
import {
  createDirectLspExtension as createDirectLspExtensionCore,
  type DirectLspOptions,
} from './cm-ide-lsp-direct.ts';
import {
  createCmdHoverDiveExtension,
  lspTooltipTheme,
  lspUriToPath,
  normalizeLocations,
  pathBasename,
  registerDiveLocationLookup,
  requestIdeDiveAtPos,
  tokenSpanAtPos,
  type IdeDiveCallbacks,
  type IdeLspJumpTarget,
} from './cm-ide-lsp-dive.ts';

export {
  buildLspCompletionChangeSpec,
  buildLspInitializationOptions,
  clearLiveLspDiagnostics,
  createCmdHoverDiveExtension,
  decodeLspSemanticTokens,
  DEFAULT_LSP_TOKEN_MODIFIERS,
  DEFAULT_LSP_TOKEN_TYPES,
  detectLspCompletionTrigger,
  hasLiveLspDiagnosticsForFile,
  IDLE_CLOSE_MS,
  isPosInCommentOrString,
  LSP_CODE_LANGUAGES,
  lspUriToPath,
  markLiveLspDiagnostics,
  MAX_POOLED_LSP_CLIENTS,
  normalizeIdeLspLanguage,
  requestIdeDiveAtPos,
  resolveIdeLspModuleRoot,
  shouldAttachIdeLsp,
  toAbsoluteIdeFilePath,
  tokenSpanAtPos,
  type IdeDiveCallbacks,
  type IdeLspJumpTarget,
  type LspPos,
  type LspRange,
  type RawLspCompletionItem,
  type RawLspTextEdit,
};

// ── Pooled LanguageServerClient Registry (Refcounted + <=3 LRU Cap) ──────────

type MarimoMod = typeof import('@marimo-team/codemirror-languageserver');

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
const unavailableLangUntil = new Map<string, number>();

function loadMarimo(): Promise<MarimoMod> {
  if (marimoCache) return Promise.resolve(marimoCache);
  return (marimoPromise ??= import('@marimo-team/codemirror-languageserver').then((m) => ((marimoCache = m), m)));
}

function closeTransportSilently(transport: InstanceType<MarimoMod['WebSocketTransport']>): void {
  const internal = transport as unknown as { closed?: boolean; socket?: WebSocket | null };
  internal.closed = true;
  const ws = internal.socket;
  if (ws) {
    ws.onopen = null;
    ws.onclose = null;
    ws.onerror = null;
    ws.onmessage = null;
    try {
      if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING) {
        ws.close(1000, 'idle');
      }
    } catch {
      /* ignore */
    }
  }
  try {
    transport.close();
  } catch {
    /* ignore */
  }
}

function disposePooledEntry(entry: PooledClientEntry): void {
  if (entry.disposed) return;
  entry.disposed = true;
  if (entry.closeTimer !== null) {
    clearTimeout(entry.closeTimer);
    entry.closeTimer = null;
  }
  if (clientPool.get(entry.key) === entry) clientPool.delete(entry.key);
  clearLiveLspDiagnostics(entry.rootPath);
  entry.diagListeners.clear();
  try {
    entry.offTransportClose?.();
  } catch {
    /* ignore */
  }
  entry.offTransportClose = null;
  try {
    (entry.client as unknown as { close?: () => void }).close?.();
  } catch {
    /* ignore */
  }
  closeTransportSilently(entry.transport);
}

function scheduleIdleDispose(entry: PooledClientEntry, delayMs = IDLE_CLOSE_MS): void {
  if (entry.disposed) return;
  if (entry.closeTimer !== null) clearTimeout(entry.closeTimer);
  entry.closeTimer = setTimeout(() => {
    entry.closeTimer = null;
    if (entry.refCount <= 0) disposePooledEntry(entry);
  }, delayMs);
}

/** True when a live pooled client already exists for `(lang, connId, resolvedRoot)`. */
export function hasWarmPooledLspClient(
  languageId: string,
  resolvedRoot: string,
  connId?: string | null,
): boolean {
  const serverLang = normalizeIdeLspLanguage(languageId) ?? languageId;
  const entry = clientPool.get(`${serverLang}::${connId ?? 'local'}::${resolvedRoot}`);
  return Boolean(entry && !entry.disposed);
}

/** Deterministically close pooled LSP WebSocket clients (e.g. on project switch or closing tabs). */
export function disposePooledLspClients(opts?: {
  rootPath?: string;
  forceAll?: boolean;
  exceptKey?: string;
}): void {
  for (const entry of [...clientPool.values()]) {
    if (opts?.exceptKey && entry.key === opts.exceptKey) continue;
    if (opts?.rootPath && entry.rootPath !== opts.rootPath) continue;
    if (opts?.forceAll || entry.refCount <= 0) disposePooledEntry(entry);
  }
}

export interface AttachPooledLspOptions {
  wsUri: string;
  protocols?: string[];
  rootPath: string;
  projectFolders?: readonly string[];
  connId?: string | null;
  filePath: string;
  languageId: string;
  onLocationJump: (uri: string, line: number, column: number) => void;
  onDiagnostics?: ((diags: IdeDiagnostic[]) => void) | null;
}

function acquirePooledClient(mod: MarimoMod, opts: AttachPooledLspOptions, resolvedRoot: string): PooledClientEntry {
  const serverLang = normalizeIdeLspLanguage(opts.languageId) ?? opts.languageId;
  const key = `${serverLang}::${opts.connId ?? 'local'}::${resolvedRoot}`;
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
      if (candidate.refCount <= 0) {
        victim = candidate;
        break;
      }
    }
    victim ??= clientPool.values().next().value;
    if (!victim) break;
    disposePooledEntry(victim);
  }

  const rootUri = `file://${resolvedRoot}`;
  const langProbeKey = `${serverLang}::${opts.connId ?? 'local'}`;
  const transport = new mod.WebSocketTransport(opts.wsUri, opts.protocols);
  const diagListeners = new Map<string, Set<(diags: IdeDiagnostic[]) => void>>();

  let onHandshakeAborted: (() => void) | null = null;
  class PooledLanguageServerClient extends mod.LanguageServerClient {
    override async initialize(): Promise<void> {
      try {
        await super.initialize();
        unavailableLangUntil.delete(langProbeKey);
      } catch {
        const self = this as unknown as { isClosed?: boolean };
        self.isClosed = true;
        this.ready = false;
        onHandshakeAborted?.();
      }
    }

    override textDocumentDidOpen(
      params: Parameters<InstanceType<MarimoMod['LanguageServerClient']>['textDocumentDidOpen']>[0],
    ): Promise<boolean> {
      const self = this as unknown as { isClosed?: boolean };
      if (!this.ready || self.isClosed) return Promise.resolve(false);
      return super.textDocumentDidOpen(params).catch(() => false);
    }

    override notify(method: string, params: unknown): Promise<void> {
      const self = this as unknown as { isClosed?: boolean };
      if (self.isClosed) return Promise.resolve();
      return super.notify(method as never, params as never).catch(() => {});
    }

    override close(): void {
      // Detach only this browser WebSocket multiplexer client; never send LSP
      // `shutdown`/`exit` to the shared daemon LspPool server.
      const self = this as unknown as {
        isClosed?: boolean;
        notificationListeners?: Set<unknown>;
        serverRequestHandlers?: Map<unknown, unknown>;
        dynamicCapabilities?: Map<unknown, unknown>;
        documentOpenCounts?: Map<unknown, unknown>;
        client?: { close?: () => void };
      };
      if (self.isClosed) return;
      self.isClosed = true;
      this.ready = false;
      self.notificationListeners?.clear();
      self.serverRequestHandlers?.clear();
      self.dynamicCapabilities?.clear();
      self.documentOpenCounts?.clear();
      self.client?.close?.();
    }
  }

  const client = new PooledLanguageServerClient({
    rootUri,
    workspaceFolders: [{ name: pathBasename(resolvedRoot) || 'workspace', uri: rootUri }],
    transport,
    initializationOptions: buildLspInitializationOptions(serverLang),
    capabilities: (defaults) => ({
      ...defaults,
      textDocument: {
        ...defaults?.textDocument,
        synchronization: {
          dynamicRegistration: false,
          willSave: false,
          willSaveWaitUntil: false,
          didSave: true,
        },
        definition: { dynamicRegistration: true, linkSupport: true },
        typeDefinition: { dynamicRegistration: true, linkSupport: true },
        implementation: { dynamicRegistration: true, linkSupport: true },
        semanticTokens: {
          dynamicRegistration: false,
          requests: { full: true, range: false },
          tokenTypes: [...DEFAULT_LSP_TOKEN_TYPES],
          tokenModifiers: [...DEFAULT_LSP_TOKEN_MODIFIERS],
          formats: ['relative'],
        },
        completion: {
          ...defaults?.textDocument?.completion,
          contextSupport: true,
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
    rootPath: resolvedRoot,
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
  onHandshakeAborted = () => {
    if (!entry.disposed) {
      unavailableLangUntil.set(langProbeKey, Date.now() + 30_000);
    }
    entry.evictFn();
  };
  entry.offTransportClose = transport.onClose?.(() => {
    if (!entry.disposed && !client.ready) {
      unavailableLangUntil.set(langProbeKey, Date.now() + 30_000);
    }
    entry.evictFn();
  }) ?? null;

  client.onNotification((n) => {
    if (entry.disposed || n.method !== 'textDocument/publishDiagnostics' || !n.params?.uri) return;
    const fPath = lspUriToPath(n.params.uri);
    markLiveLspDiagnostics(resolvedRoot, fPath);
    if (opts.rootPath && opts.rootPath !== resolvedRoot) {
      markLiveLspDiagnostics(opts.rootPath, fPath);
    }
    const subs = diagListeners.get(n.params.uri);
    if (!subs || subs.size === 0) return;
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
  scheduleIdleDispose(entry, IDLE_CLOSE_MS);
  return entry;
}

type RpcRequester = { request: (method: string, params: unknown) => Promise<unknown> };

function getPooledRpc(client: PooledClientEntry['client']): RpcRequester | null {
  return (client as unknown as { client?: RpcRequester }).client ?? null;
}

export async function buildPooledLspExtensions(opts: AttachPooledLspOptions): Promise<Extension[]> {
  if (!shouldAttachIdeLsp(opts.filePath, opts.languageId, opts.rootPath)) return [];
  const serverLang = normalizeIdeLspLanguage(opts.languageId) ?? opts.languageId;
  const langProbeKey = `${serverLang}::${opts.connId ?? 'local'}`;
  const cooldownUntil = unavailableLangUntil.get(langProbeKey) ?? 0;
  if (cooldownUntil > Date.now()) return [];
  if (cooldownUntil > 0) unavailableLangUntil.delete(langProbeKey);
  const resolvedRoot = resolveIdeLspModuleRoot(opts.rootPath, opts.filePath, opts.projectFolders);
  const absFile = toAbsoluteIdeFilePath(resolvedRoot, opts.filePath);
  if (!absFile || isSecretIdePath(absFile)) return [];

  const mod = await loadMarimo();
  const pooled = acquirePooledClient(mod, opts, resolvedRoot);
  const documentUri = opts.filePath.includes('://') ? opts.filePath : `file://${absFile}`;

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
        if (!set) {
          set = new Set();
          pooled.diagListeners.set(documentUri, set);
        }
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
      if (pooled.refCount <= 0) scheduleIdleDispose(pooled, IDLE_CLOSE_MS);
    }
  });

  const exts = mod.languageServerWithClient({
    client: pooled.client,
    documentUri,
    languageId: opts.languageId,
    hoverEnabled: false,
    completionEnabled: false,
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

  const completionExt = createIdeLspCompletionExtension({
    documentUri,
    languageId: opts.languageId,
    isReady: () => !pooled.disposed && Boolean(pooled.client.ready),
    requestCompletion: async (params) => {
      const rpc = getPooledRpc(pooled.client);
      if (rpc) {
        return (await rpc.request('textDocument/completion', params)) as RawLspCompletionItem[] | { items?: RawLspCompletionItem[] } | null;
      }
      return (await pooled.client.textDocumentCompletion(params as never)) as unknown as { items?: RawLspCompletionItem[] } | null;
    },
    resolveItem: async (item) => {
      const rpc = getPooledRpc(pooled.client);
      if (!rpc) return item;
      return (await rpc.request('completionItem/resolve', item)) as RawLspCompletionItem | null;
    },
  });

  const semanticExt = createLspSemanticTokensExtension({
    documentUri,
    fetchTokens: async (uri) => {
      await pooled.client.initializePromise;
      if (pooled.disposed || !pooled.client.ready) return null;
      const caps = (pooled.client as unknown as {
        capabilities?: {
          semanticTokensProvider?: {
            full?: boolean | Record<string, unknown>;
            legend?: { tokenTypes?: string[]; tokenModifiers?: string[] };
          };
        };
      }).capabilities;
      const semProvider = caps?.semanticTokensProvider;
      if (!semProvider?.full && !semProvider?.legend) return null;
      const rpc = getPooledRpc(pooled.client);
      if (!rpc) return null;
      const res = (await rpc.request('textDocument/semanticTokens/full', {
        textDocument: { uri },
      })) as { data?: number[] } | null;
      if (!res?.data?.length) return null;
      return {
        data: res.data,
        tokenTypes: semProvider?.legend?.tokenTypes,
        tokenModifiers: semProvider?.legend?.tokenModifiers,
      };
    },
  });

  return [lspTooltipTheme, lifecycleCleanup, ...exts, completionExt, semanticExt];
}

/** Query raw hover markdown from an active pooled LSP client if available, for `cm-ide-hover.ts`. */
export async function queryPooledLspHoverMarkdown(view: EditorView, pos: number): Promise<string | null> {
  if (!marimoCache) return null;
  const plugin = marimoCache.getLanguageServerPlugin(view);
  if (!plugin || !plugin.client.ready || !plugin.client.hasCapability('textDocument/hover')) return null;
  try {
    const res = await plugin.client.textDocumentHover({
      textDocument: { uri: plugin.documentUri },
      position: posToLsp(view.state.doc, pos),
    });
    if (!res?.contents) return null;
    const parts = Array.isArray(res.contents) ? res.contents : [res.contents];
    const text = parts
      .map((p) => (typeof p === 'string' ? p : 'value' in p && typeof p.value === 'string' ? p.value : ''))
      .filter(Boolean)
      .join('\n\n')
      .trim();
    return text || null;
  } catch {
    return null;
  }
}

/** Notify the active LSP plugin on `view` that the buffer was saved (`willSave` / `didSave`). */
export function notifyLspDidSave(view: EditorView | null): void {
  if (!view || !marimoCache) return;
  const plugin = marimoCache.getLanguageServerPlugin(view);
  if (plugin) void plugin.documentDidSave().catch(() => {});
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
  const rpc = getPooledRpc(plugin.client);

  try {
    if (mode === 'implementation') {
      if (rpc && plugin.client.hasCapability('textDocument/implementation')) {
        const locs = normalizeLocations(await rpc.request('textDocument/implementation', params));
        if (locs.length > 0) return locs;
      }
    } else {
      if (plugin.client.hasCapability('textDocument/definition')) {
        const locs = normalizeLocations(await plugin.client.textDocumentDefinition(params));
        if (locs.length > 0) return locs;
      }
      if (rpc && plugin.client.hasCapability('textDocument/typeDefinition')) {
        const locs = normalizeLocations(await rpc.request('textDocument/typeDefinition', params));
        if (locs.length > 0) return locs;
      }
    }
  } catch {
    /* Fall through to backend HTTP / lexical dive-in */
  }
  return [];
}

registerDiveLocationLookup(queryPluginLocations);

export type { DirectLspOptions };

export function createDirectLspExtension(opts: DirectLspOptions): Extension {
  return createDirectLspExtensionCore(opts, lspTooltipTheme);
}
