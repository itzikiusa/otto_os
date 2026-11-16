// Pooled CodeMirror 6 LSP Client + GoLand Cmd-Hover Dive-In (Definition & Implementation),
// Signature Help, Dot-Triggered Auto-Imports (`additionalTextEdits`), `semanticTokens/full`,
// and Direct JSON-RPC WS Fallback.

import { EditorView, ViewPlugin } from '@codemirror/view';
import type { Extension } from '@codemirror/state';
import type { IdeDiagnostic } from '../api/types';
import { pathToFileUri } from '../fileUri.ts';
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
  dedupeLspDiagnostics,
  dedupeLspJsonRpcMessage,
  DEFAULT_LSP_TOKEN_MODIFIERS,
  DEFAULT_LSP_TOKEN_TYPES,
  detectLspCompletionTrigger,
  extractGoBuildTagsFromSource,
  IDLE_CLOSE_MS,
  isPosInCommentOrString,
  LSP_CODE_LANGUAGES,
  lspDiagnosticDedupeKey,
  MAX_POOLED_LSP_CLIENTS,
  normalizeIdeLspLanguage,
  posToLsp,
  resolveIdeLspModuleRoot,
  rewriteGoBuildTagDiagnostic,
  shouldAttachIdeLsp,
  toAbsoluteIdeFilePath,
  type LspPos,
  type LspRange,
  type RawLspCompletionItem,
  type RawLspDiagnosticLike,
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
import { appLive } from '../live.ts';
import { cancelLspReconnects, isLspReconnecting, noteLspDaemonShutdown, noteLspSessionReady, noteLspSocketClosed, noteLspStatus } from './cm-ide-lsp-reconnect.ts';

export {
  buildLspCompletionChangeSpec,
  buildLspInitializationOptions,
  clearLiveLspDiagnostics,
  createCmdHoverDiveExtension,
  decodeLspSemanticTokens,
  dedupeLspDiagnostics,
  dedupeLspJsonRpcMessage,
  DEFAULT_LSP_TOKEN_MODIFIERS,
  DEFAULT_LSP_TOKEN_TYPES,
  detectLspCompletionTrigger,
  extractGoBuildTagsFromSource,
  hasLiveLspDiagnosticsForFile,
  IDLE_CLOSE_MS,
  isPosInCommentOrString,
  LSP_CODE_LANGUAGES,
  lspDiagnosticDedupeKey,
  lspUriToPath,
  markLiveLspDiagnostics,
  MAX_POOLED_LSP_CLIENTS,
  normalizeIdeLspLanguage,
  requestIdeDiveAtPos,
  resolveIdeLspModuleRoot,
  rewriteGoBuildTagDiagnostic,
  shouldAttachIdeLsp,
  toAbsoluteIdeFilePath,
  tokenSpanAtPos,
  type IdeDiveCallbacks,
  type IdeLspJumpTarget,
  type LspPos,
  type LspRange,
  type RawLspCompletionItem,
  type RawLspDiagnosticLike,
  type RawLspTextEdit,
};

// ── Pooled LanguageServerClient Registry (Refcounted + <=3 LRU Cap) ──────────

type MarimoMod = typeof import('@marimo-team/codemirror-languageserver');

interface PooledClientEntry {
  key: string;
  aliasKeys: Set<string>;
  rootPath: string;
  client: InstanceType<MarimoMod['LanguageServerClient']>;
  transport: InstanceType<MarimoMod['WebSocketTransport']>;
  offTransportClose: (() => void) | null;
  evictFn: () => void;
  diagListeners: Map<string, Set<(diags: IdeDiagnostic[]) => void>>;
  openViews: Map<string, EditorView>;
  refCount: number;
  closeTimer: ReturnType<typeof setTimeout> | null;
  disposed: boolean;
}

const MODULE_SCOPED_LSP_LANGS = new Set(['go', 'rust', 'java', 'python']);

let marimoPromise: Promise<MarimoMod> | null = null;
let marimoCache: MarimoMod | null = null;
const clientPool = new Map<string, PooledClientEntry>();
const canonicalModuleRootByDir = new Map<string, string>();
const unavailableLangUntil = new Map<string, number>();

function parentDirOf(absPath: string): string {
  const clean = absPath.replace(/\\/g, '/').replace(/\/+$/, '');
  const idx = clean.lastIndexOf('/');
  return idx > 0 ? clean.slice(0, idx) : '/';
}

function recordCanonicalModuleRoot(
  serverLang: string,
  connKey: string,
  fileDir: string,
  canonRoot: string,
): void {
  const normCanon = canonRoot.replace(/\\/g, '/').replace(/\/+$/, '') || '/';
  let cur = fileDir.replace(/\\/g, '/').replace(/\/+$/, '') || '/';
  for (let depth = 0; depth < 32; depth++) {
    canonicalModuleRootByDir.set(`${serverLang}::${connKey}::${cur}`, normCanon);
    if (cur === normCanon || !cur.startsWith(`${normCanon}/`)) break;
    const next = parentDirOf(cur);
    if (next === cur) break;
    cur = next;
  }
}

function resolvePoolKeyForFile(
  serverLang: string,
  connId: string | null | undefined,
  resolvedRoot: string,
  filePath?: string | null,
): string {
  const connKey = connId ?? 'local';
  const normRoot = resolvedRoot.replace(/\\/g, '/').replace(/\/+$/, '') || '/';
  if (!filePath || !MODULE_SCOPED_LSP_LANGS.has(serverLang)) {
    return `${serverLang}::${connKey}::${normRoot}`;
  }
  const absFile = toAbsoluteIdeFilePath(normRoot, filePath);
  if (!absFile) return `${serverLang}::${connKey}::${normRoot}`;
  const fileDir = parentDirOf(absFile);
  let probe = fileDir;
  for (let depth = 0; depth < 32; depth++) {
    const knownCanon = canonicalModuleRootByDir.get(`${serverLang}::${connKey}::${probe}`);
    if (knownCanon) return `${serverLang}::${connKey}::${knownCanon}`;
    if (probe === normRoot || !probe.startsWith(`${normRoot}/`)) break;
    const next = parentDirOf(probe);
    if (next === probe) break;
    probe = next;
  }
  if (fileDir === normRoot) return `${serverLang}::${connKey}::${normRoot}`;
  return `${serverLang}::${connKey}::dir:${fileDir}`;
}

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
  for (const ak of entry.aliasKeys) {
    if (clientPool.get(ak) === entry) clientPool.delete(ak);
  }
  entry.aliasKeys.clear();
  clearLiveLspDiagnostics(entry.rootPath);
  entry.diagListeners.clear();
  entry.openViews.clear();
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

/** True when a live pooled client already exists for `(lang, connId, resolvedRoot, filePath)`. */
export function hasWarmPooledLspClient(
  languageId: string,
  resolvedRoot: string,
  connId?: string | null,
  filePath?: string | null,
): boolean {
  const serverLang = normalizeIdeLspLanguage(languageId) ?? languageId;
  const key = resolvePoolKeyForFile(serverLang, connId, resolvedRoot, filePath);
  const entry = clientPool.get(key) ?? clientPool.get(`${serverLang}::${connId ?? 'local'}::${resolvedRoot}`);
  return Boolean(entry && !entry.disposed);
}

/** Deterministically close pooled LSP WebSocket clients (e.g. on project switch, build-tag change, or closing tabs). */
export function disposePooledLspClients(opts?: {
  rootPath?: string;
  forceAll?: boolean;
  exceptKey?: string;
}): void {
  const normFilter = opts?.rootPath ? opts.rootPath.replace(/\\/g, '/').replace(/\/+$/, '') : '';
  // Everything torn down (project switch, IDE closed): no reattach is wanted any more.
  if (!normFilter && opts?.forceAll) cancelLspReconnects();
  if (normFilter && opts?.forceAll) {
    for (const [dirKey, canon] of [...canonicalModuleRootByDir.entries()]) {
      if (canon === normFilter || canon.startsWith(`${normFilter}/`) || dirKey.endsWith(`::${normFilter}`) || dirKey.includes(`::${normFilter}/`)) {
        canonicalModuleRootByDir.delete(dirKey);
      }
    }
  }
  const uniqueEntries = new Set<PooledClientEntry>(clientPool.values());
  for (const entry of uniqueEntries) {
    if (opts?.exceptKey && entry.key === opts.exceptKey) continue;
    if (normFilter) {
      const entryRoot = entry.rootPath.replace(/\\/g, '/').replace(/\/+$/, '');
      if (entryRoot !== normFilter && !entryRoot.startsWith(`${normFilter}/`)) continue;
    }
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
  const connKey = opts.connId ?? 'local';
  const absFile = toAbsoluteIdeFilePath(resolvedRoot, opts.filePath) || opts.filePath;
  const fileDir = absFile ? parentDirOf(absFile) : resolvedRoot;
  const key = resolvePoolKeyForFile(serverLang, opts.connId, resolvedRoot, absFile);
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

  const distinctPooled = () => new Set(clientPool.values());
  while (distinctPooled().size >= MAX_POOLED_LSP_CLIENTS) {
    let victim: PooledClientEntry | undefined;
    for (const candidate of distinctPooled()) {
      if (candidate.refCount <= 0) {
        victim = candidate;
        break;
      }
    }
    victim ??= distinctPooled().values().next().value;
    if (!victim) break;
    disposePooledEntry(victim);
  }

  const rootUri = pathToFileUri(resolvedRoot);
  const langProbeKey = `${serverLang}::${connKey}`;
  const openViews = new Map<string, EditorView>();
  const transport = new mod.WebSocketTransport(opts.wsUri, opts.protocols);
  const origOnMessage = transport.onMessage.bind(transport);
  transport.onMessage = (handler) =>
    origOnMessage((msg) =>
      handler(
        dedupeLspJsonRpcMessage(msg, (uri) => {
          const v = openViews.get(uri);
          return v ? v.state.sliceDoc(0, Math.min(v.state.doc.length, 4096)) : null;
        }),
      ),
    );
  const diagListeners = new Map<string, Set<(diags: IdeDiagnostic[]) => void>>();

  let onHandshakeAborted: (() => void) | null = null;
  class PooledLanguageServerClient extends mod.LanguageServerClient {
    override async initialize(): Promise<void> {
      try {
        await super.initialize();
        unavailableLangUntil.delete(langProbeKey);
        if (this.ready) { noteLspSessionReady(resolvedRoot); noteLspStatus(resolvedRoot, 'ready', serverLang); }
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
    aliasKeys: new Set<string>(),
    rootPath: resolvedRoot,
    client,
    transport,
    offTransportClose: null,
    evictFn: () => {},
    diagListeners,
    openViews,
    refCount: 0,
    closeTimer: null,
    disposed: false,
  };
  entry.evictFn = () => disposePooledEntry(entry);
  // A dropped socket (any close: plain, with a reason, or after an eviction notice) evicts
  // the entry; a working session with editors attached then reattaches with backoff, and
  // a failed reattach attempt continues that sequence instead of cooling the language off.
  // `daemonShutdown` (eviction reason `shutdown`): no backoff against a daemon going
  // down; reattach once the app's own event socket is connected again.
  const onSocketGone = (handshakeFailed: boolean, daemonShutdown = false): void => {
    if (entry.disposed) return;
    const hadSession = !handshakeFailed && Boolean(client.ready) && entry.refCount > 0;
    if (!hadSession && !daemonShutdown && !isLspReconnecting(entry.rootPath)) {
      unavailableLangUntil.set(langProbeKey, Date.now() + 30_000);
      // Closed before any session (no binary: 503, or refused): say so instead of going silent.
      if (handshakeFailed || !client.ready) noteLspStatus(entry.rootPath, 'unavailable', serverLang, 'no_session');
    }
    const root = entry.rootPath;
    entry.evictFn();
    if (daemonShutdown) noteLspDaemonShutdown(root, hadSession, (resume) => appLive.onConnection((up) => { if (up) resume(); }));
    else noteLspSocketClosed(root, hadSession);
  };
  onHandshakeAborted = () => onSocketGone(true);
  entry.offTransportClose = transport.onClose?.(() => onSocketGone(false)) ?? null;

  client.onNotification((n) => {
    if (entry.disposed) return;
    const rawNotif = n as unknown as { method?: string; params?: Record<string, unknown> };
    // `otto/lspEvicted {reason, lang, root}` (docs/contracts/ws.md): the daemon is stopping
    // this server and closes the socket next. Treat it as that close so editors reattach
    // (`server_cap`, `build_tags_changed`, any future reason) — except `shutdown`, which
    // waits for the daemon instead of retrying against it.
    if (typeof rawNotif.method === 'string' && /^otto\/.*evict/i.test(rawNotif.method)) {
      onSocketGone(false, rawNotif.params?.reason === 'shutdown');
      return;
    }
    if (rawNotif.method === 'otto/lspAttached' && typeof rawNotif.params?.root === 'string' && rawNotif.params.root) {
      const canonRoot = rawNotif.params.root.replace(/\\/g, '/').replace(/\/+$/, '') || '/';
      entry.rootPath = canonRoot;
      recordCanonicalModuleRoot(serverLang, connKey, fileDir, canonRoot);
      const canonKey = `${serverLang}::${connKey}::${canonRoot}`;
      if (canonKey !== entry.key) {
        entry.aliasKeys.add(canonKey);
        const curCanon = clientPool.get(canonKey);
        if (!curCanon || curCanon.disposed) clientPool.set(canonKey, entry);
      }
      return;
    }
    if (n.method !== 'textDocument/publishDiagnostics' || !n.params?.uri) return;
    const fPath = lspUriToPath(n.params.uri);
    markLiveLspDiagnostics(entry.rootPath, fPath);
    if (resolvedRoot !== entry.rootPath) markLiveLspDiagnostics(resolvedRoot, fPath);
    if (opts.rootPath && opts.rootPath !== resolvedRoot) {
      markLiveLspDiagnostics(opts.rootPath, fPath);
    }
    const subs = diagListeners.get(n.params.uri);
    if (!subs || subs.size === 0) return;
    const viewForDoc = openViews.get(n.params.uri);
    const headerText = viewForDoc ? viewForDoc.state.sliceDoc(0, Math.min(viewForDoc.state.doc.length, 4096)) : null;
    const mapped: IdeDiagnostic[] = (n.params.diagnostics ?? []).map((d) => {
      const rawMsg = typeof d.message === 'string' ? d.message : d.message.value;
      const rw = rewriteGoBuildTagDiagnostic(rawMsg, d.code, headerText);
      return {
        path: fPath,
        line: d.range.start.line + 1,
        column: d.range.start.character + 1,
        end_line: d.range.end.line + 1,
        end_column: d.range.end.character + 1,
        severity: rw.isBuildTagDiag
          ? 'warning'
          : d.severity === 1 ? 'error' : d.severity === 2 ? 'warning' : d.severity === 4 ? 'hint' : 'info',
        message: rw.message,
        source: rw.isBuildTagDiag && d.source === 'go list' ? 'gopls (build tags)' : (d.source ?? null),
        code: rw.isBuildTagDiag ? 'otto_go_build_tag' : (d.code != null ? String(d.code) : null),
      };
    });
    for (const cb of subs) cb(mapped);
  });

  clientPool.set(key, entry);
  scheduleIdleDispose(entry, IDLE_CLOSE_MS);
  noteLspStatus(resolvedRoot, 'starting', serverLang);
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
  if (cooldownUntil > Date.now() && !isLspReconnecting(opts.rootPath)) return [];
  if (cooldownUntil > 0) unavailableLangUntil.delete(langProbeKey);
  const resolvedRoot = resolveIdeLspModuleRoot(opts.rootPath, opts.filePath, opts.projectFolders);
  const absFile = toAbsoluteIdeFilePath(resolvedRoot, opts.filePath);
  if (!absFile || isSecretIdePath(absFile)) return [];

  const mod = await loadMarimo();
  const pooled = acquirePooledClient(mod, opts, resolvedRoot);
  const documentUri = opts.filePath.includes('://') ? opts.filePath : pathToFileUri(absFile);

  const lifecycleCleanup = ViewPlugin.fromClass(class {
    private unsubDiag: (() => void) | null = null;
    constructor(view: EditorView) {
      if (pooled.closeTimer !== null) {
        clearTimeout(pooled.closeTimer);
        pooled.closeTimer = null;
      }
      pooled.refCount += 1;
      pooled.openViews.set(documentUri, view);
      if (pooled.client.ready) {
        markLiveLspDiagnostics(pooled.rootPath, absFile);
        if (opts.rootPath && opts.rootPath !== pooled.rootPath) markLiveLspDiagnostics(opts.rootPath, absFile);
      } else {
        void pooled.client.initializePromise?.then(() => {
          if (!pooled.disposed && pooled.client.ready && pooled.openViews.get(documentUri) === view) {
            markLiveLspDiagnostics(pooled.rootPath, absFile);
            if (opts.rootPath && opts.rootPath !== pooled.rootPath) markLiveLspDiagnostics(opts.rootPath, absFile);
          }
        }).catch(() => {});
      }
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
      pooled.openViews.delete(documentUri);
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

export interface PooledLspHoverResult {
  markdown: string;
  definedIn: { path: string; line: number; column?: number } | null;
}

/** Query raw hover markdown + authoritative definition location from an active pooled LSP client. */
export async function queryPooledLspHover(view: EditorView, pos: number): Promise<PooledLspHoverResult | null> {
  if (!marimoCache) return null;
  const plugin = marimoCache.getLanguageServerPlugin(view);
  if (!plugin) return null;
  if (!plugin.client.ready) {
    try {
      await Promise.race([
        plugin.client.initializePromise,
        new Promise<void>((resolve) => setTimeout(resolve, 1200)),
      ]);
    } catch {
      return null;
    }
  }
  if (!plugin.client.ready) return null;
  try {
    const hoverPromise = plugin.client.hasCapability('textDocument/hover')
      ? plugin.client.textDocumentHover({
          textDocument: { uri: plugin.documentUri },
          position: posToLsp(view.state.doc, pos),
        }).catch(() => null)
      : Promise.resolve(null);
    const defPromise = queryPluginLocations(view, pos, 'definition').catch(() => []);
    const [res, locs] = await Promise.all([hoverPromise, defPromise]);
    const definedIn = locs.length > 0 ? { path: locs[0].path, line: locs[0].line, column: locs[0].column } : null;
    if (!res?.contents) {
      return definedIn ? { markdown: '', definedIn } : null;
    }
    const parts = Array.isArray(res.contents) ? res.contents : [res.contents];
    const text = parts
      .map((p) => (typeof p === 'string' ? p : 'value' in p && typeof p.value === 'string' ? p.value : ''))
      .filter(Boolean)
      .join('\n\n')
      .trim();
    if (!text && !definedIn) return null;
    return { markdown: text, definedIn };
  } catch {
    return null;
  }
}

/** Query raw hover markdown from an active pooled LSP client if available, for `cm-ide-hover.ts`. */
export async function queryPooledLspHoverMarkdown(view: EditorView, pos: number): Promise<string | null> {
  const res = await queryPooledLspHover(view, pos);
  return res?.markdown || null;
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
