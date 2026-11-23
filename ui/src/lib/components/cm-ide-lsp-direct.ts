// Direct WebSocket JSON-RPC LSP Fallback (Zero Node Polyfills) for `cm-ide-lsp.ts`.

import { EditorView, ViewPlugin, type ViewUpdate } from '@codemirror/view';
import type { Extension } from '@codemirror/state';
import { setDiagnostics, type Diagnostic } from '@codemirror/lint';
import type { IdeDiagnostic } from '../api/types';
import { fileUriToPath } from '../fileUri.ts';
import { markLiveLspDiagnostics } from './cm-ide-drafts.ts';
import {
  buildLspInitializationOptions,
  createIdeLspCompletionExtension,
  createLspSemanticTokensExtension,
  dedupeLspDiagnostics,
  DEFAULT_LSP_TOKEN_MODIFIERS,
  DEFAULT_LSP_TOKEN_TYPES,
  lspToPos,
  rewriteGoBuildTagDiagnostic,
  shouldAttachIdeLsp,
  type LspRange,
  type RawLspCompletionItem,
} from './cm-ide-lsp-completion.ts';

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

function uriToFsPath(uri: string): string {
  return fileUriToPath(uri);
}

function baseName(p: string): string {
  const clean = p.split('?')[0];
  const idx = Math.max(clean.lastIndexOf('/'), clean.lastIndexOf('\\'));
  return idx >= 0 ? clean.slice(idx + 1) : clean;
}

export function createDirectLspExtension(
  opts: DirectLspOptions,
  baseTheme?: Extension,
): Extension {
  const rootPath = uriToFsPath(opts.rootUri);
  if (!shouldAttachIdeLsp(opts.filePath, opts.languageId, rootPath)) return [];

  let socket: WebSocket | null = null;
  let socketAbort: AbortController | null = null;
  let nextId = 1;
  let version = 1;
  let ready = false;
  let closed = false;
  let liveView: EditorView | null = null;
  let semLegend: { tokenTypes?: string[]; tokenModifiers?: string[] } | null = null;
  const pending = new Map<
    number,
    { resolve: (v: unknown) => void; reject: (e: Error) => void; timer?: ReturnType<typeof setTimeout> }
  >();

  const rejectAllPending = (reason: string) => {
    for (const [, p] of pending) {
      if (p.timer) clearTimeout(p.timer);
      p.reject(new Error(reason));
    }
    pending.clear();
  };

  const send = (p: Record<string, unknown>) => {
    if (!closed && socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify(p));
  };
  const request = <T = unknown>(method: string, params: unknown): Promise<T> =>
    new Promise<T>((resolve, reject) => {
      if (!ready || socket?.readyState !== WebSocket.OPEN) return reject(new Error('LSP unavailable'));
      const id = nextId++;
      const timer = setTimeout(() => {
        if (pending.delete(id)) reject(new Error('LSP timeout'));
      }, 6000);
      pending.set(id, { resolve: resolve as (v: unknown) => void, reject, timer });
      send({ jsonrpc: '2.0', id, method, params });
    });

  const plugin = ViewPlugin.fromClass(class {
    constructor(view: EditorView) {
      liveView = view;
      try {
        socket = new WebSocket(opts.wsUri, opts.protocols);
      } catch {
        return;
      }
      socketAbort = new AbortController();
      const { signal } = socketAbort;
      const onDisconnect = () => {
        ready = false;
        semLegend = null;
        rejectAllPending('LSP disconnected');
      };
      socket.addEventListener('close', onDisconnect, { signal });
      socket.addEventListener('error', onDisconnect, { signal });
      socket.addEventListener('open', () => {
        if (closed || !socket) return;
        const initId = nextId++;
        pending.set(initId, {
          resolve: (initRes) => {
            if (closed) return;
            ready = true;
            const caps = (initRes as {
              capabilities?: { semanticTokensProvider?: { legend?: { tokenTypes?: string[]; tokenModifiers?: string[] } } };
            } | null)?.capabilities;
            semLegend = caps?.semanticTokensProvider?.legend ?? null;
            send({ jsonrpc: '2.0', method: 'initialized', params: {} });
            send({
              jsonrpc: '2.0',
              method: 'textDocument/didOpen',
              params: {
                textDocument: {
                  uri: opts.documentUri,
                  languageId: opts.languageId,
                  version,
                  text: view.state.doc.toString(),
                },
              },
            });
          },
          reject: () => {},
        });
        send({
          jsonrpc: '2.0',
          id: initId,
          method: 'initialize',
          params: {
            processId: null,
            rootUri: opts.rootUri,
            workspaceFolders: [{ uri: opts.rootUri, name: baseName(rootPath) || 'workspace' }],
            initializationOptions: buildLspInitializationOptions(opts.languageId),
            capabilities: {
              textDocument: {
                hover: { contentFormat: ['plaintext', 'markdown'] },
                completion: {
                  contextSupport: true,
                  completionItem: { snippetSupport: true, additionalTextEditsSupport: true },
                },
                definition: { linkSupport: true },
                implementation: { linkSupport: true },
                publishDiagnostics: { relatedInformation: true },
                semanticTokens: {
                  requests: { full: true, range: false },
                  tokenTypes: [...DEFAULT_LSP_TOKEN_TYPES],
                  tokenModifiers: [...DEFAULT_LSP_TOKEN_MODIFIERS],
                  formats: ['relative'],
                },
              },
            },
          },
        });
      }, { signal });
      socket.addEventListener('message', (ev) => {
        if (closed || typeof ev.data !== 'string') return;
        let msg: Record<string, unknown>;
        try {
          msg = JSON.parse(ev.data) as Record<string, unknown>;
        } catch {
          return;
        }
        if (typeof msg.id === 'number' && !msg.method) {
          const p = pending.get(msg.id);
          if (p) {
            pending.delete(msg.id);
            if (p.timer) clearTimeout(p.timer);
            if (msg.error) p.reject(new Error('LSP error'));
            else p.resolve(msg.result);
          }
          return;
        }
        if (typeof msg.id === 'number' && typeof msg.method === 'string') {
          send({ jsonrpc: '2.0', id: msg.id, result: null });
          return;
        }
        if (msg.method === 'textDocument/publishDiagnostics' && liveView && liveView.dom.isConnected) {
          const params = msg.params as {
            uri?: string;
            diagnostics?: Array<{ range: LspRange; severity?: number; message: string; source?: string; code?: string | number }>;
          };
          if (params?.uri && params.uri !== opts.documentUri) return;
          markLiveLspDiagnostics(rootPath, opts.filePath);
          const raw = dedupeLspDiagnostics(params?.diagnostics ?? []);
          const doc = liveView.state.doc;
          const headerText = doc.sliceString(0, Math.min(doc.length, 4096));
          const cmDiags: Diagnostic[] = raw.map((d) => {
            const from = Math.max(0, Math.min(doc.length, lspToPos(doc, d.range.start)));
            const to = Math.max(from, Math.min(doc.length, lspToPos(doc, d.range.end)));
            const rw = rewriteGoBuildTagDiagnostic(d.message, d.code, headerText);
            return {
              from,
              to,
              severity: rw.isBuildTagDiag ? 'warning' : (d.severity === 1 ? 'error' : d.severity === 2 ? 'warning' : 'info'),
              message: rw.message,
              source: rw.isBuildTagDiag && d.source === 'go list' ? 'gopls (build tags)' : d.source,
              actions: rw.isBuildTagDiag && rw.tags.length > 0
                ? [{
                    name: `Enable Go build tag "${rw.tags.join(',')}"`,
                    apply: (v) => {
                      v.dom.dispatchEvent(
                        new CustomEvent('otto:ide-enable-go-build-tag', {
                          bubbles: true,
                          detail: { tags: rw.tags, filePath: opts.filePath },
                        }),
                      );
                    },
                  }]
                : undefined,
            };
          });
          liveView.dispatch(setDiagnostics(liveView.state, cmDiags));
          opts.onDiagnostics?.(
            raw.map((d) => {
              const rw = rewriteGoBuildTagDiagnostic(d.message, d.code, headerText);
              return {
                path: opts.filePath,
                line: d.range.start.line + 1,
                column: d.range.start.character + 1,
                end_line: d.range.end.line + 1,
                end_column: d.range.end.character + 1,
                severity: rw.isBuildTagDiag
                  ? 'warning'
                  : (d.severity === 1 ? 'error' : d.severity === 2 ? 'warning' : d.severity === 4 ? 'hint' : 'info'),
                message: rw.message,
                source: rw.isBuildTagDiag && d.source === 'go list' ? 'gopls (build tags)' : (d.source ?? null),
                code: rw.isBuildTagDiag ? 'otto_go_build_tag' : (d.code != null ? String(d.code) : null),
              };
            }),
          );
        }
      }, { signal });
    }
    private changeTimer: ReturnType<typeof setTimeout> | null = null;
    update(u: ViewUpdate) {
      if (u.docChanged && ready) {
        if (this.changeTimer) clearTimeout(this.changeTimer);
        const viewRef = u.view;
        this.changeTimer = setTimeout(() => {
          this.changeTimer = null;
          if (closed || !ready || !viewRef.dom.isConnected) return;
          version++;
          send({
            jsonrpc: '2.0',
            method: 'textDocument/didChange',
            params: {
              textDocument: { uri: opts.documentUri, version },
              contentChanges: [{ text: viewRef.state.doc.toString() }],
            },
          });
        }, 150);
      }
    }
    destroy() {
      closed = true;
      ready = false;
      semLegend = null;
      if (this.changeTimer) {
        clearTimeout(this.changeTimer);
        this.changeTimer = null;
      }
      if (socket?.readyState === WebSocket.OPEN) {
        try {
          socket.send(JSON.stringify({ jsonrpc: '2.0', method: 'textDocument/didClose', params: { textDocument: { uri: opts.documentUri } } }));
        } catch {
          /* ignore */
        }
      }
      rejectAllPending('LSP closed');
      socketAbort?.abort();
      socketAbort = null;
      if (socket) {
        try {
          socket.close(1000, 'idle');
        } catch {
          /* ignore */
        }
      }
      socket = null;
      liveView = null;
    }
  });

  const completion = createIdeLspCompletionExtension({
    documentUri: opts.documentUri,
    languageId: opts.languageId,
    isReady: () => ready && !closed,
    requestCompletion: (params) =>
      request<RawLspCompletionItem[] | { items?: RawLspCompletionItem[] } | null>('textDocument/completion', params),
    resolveItem: (item) => request<RawLspCompletionItem | null>('completionItem/resolve', item),
  });

  const semanticExt = createLspSemanticTokensExtension({
    documentUri: opts.documentUri,
    fetchTokens: async (uri) => {
      if (!ready || closed || !semLegend) return null;
      const res = await request<{ data?: number[] } | null>('textDocument/semanticTokens/full', {
        textDocument: { uri },
      });
      if (!res?.data?.length) return null;
      return { data: res.data, tokenTypes: semLegend.tokenTypes, tokenModifiers: semLegend.tokenModifiers };
    },
  });

  return baseTheme ? [baseTheme, plugin, completion, semanticExt] : [plugin, completion, semanticExt];
}
