// On-demand async helpers for `CodeEditor.svelte` (LSP WebSocket attachment,
// Send-to-Agent action, and fallback quick-fix edits). Loaded dynamically only
// when `lsp={true}`, quick-fix edits, or "Send to agent" are invoked, keeping
// non-LSP CodeEditor consumers (`panel:files`, etc.) lean.

import type { Compartment } from '@codemirror/state';
import type { EditorView } from '@codemirror/view';
import { api, baseUrl, getToken, WS_BEARER_SUBPROTOCOL } from '../api/client';
import type { IdeDiagnostic, LspCapabilities } from '../api/types';
import { ws } from '../stores/workspace.svelte';
import { toasts } from '../toast.svelte';
import { extOf, langExtOf } from './cm-editor-helpers';
import type * as CmIdeDepsMod from './cm-ide-deps';
import type * as CmIdeMod from './cm-ide-extensions';
import { isManifestPath } from './cm-ide-history';
import { lspLanguageIdFor, normalizeIdeLspServerLang } from './cm-ide-keymap';
import {
  resolveIdeLspModuleRoot,
  shouldAttachIdeLsp,
  toAbsoluteIdeFilePath,
} from './cm-ide-lsp-completion';

export { lspLanguageIdFor, normalizeIdeLspServerLang, toAbsoluteIdeFilePath };

function lspLangFor(filePath: string, ideMode: boolean, language?: string): string | null {
  return lspLanguageIdFor(filePath, ideMode, language);
}

export function uriToPath(uri: string): string {
  if (!uri.startsWith('file://')) return uri;
  try { return decodeURIComponent(uri.slice('file://'.length)); }
  catch { return uri.slice('file://'.length); }
}

function lspWsUrl(lang: string, rootPath: string, connId?: string | null): string {
  const base = new URL(baseUrl());
  const proto = base.protocol === 'https:' ? 'wss:' : 'ws:';
  const params = new URLSearchParams({ lang, root: rootPath });
  if (connId) params.set('connection_id', connId);
  return `${proto}//${base.host}/ws/lsp?${params.toString()}`;
}

let capabilitiesCache: LspCapabilities | null = null;
let capabilitiesFetching: Promise<LspCapabilities | null> | null = null;
let capabilitiesFetchedAt = 0;

async function getLspCapabilities(force = false): Promise<LspCapabilities | null> {
  if (force && Date.now() - capabilitiesFetchedAt > 15_000) {
    capabilitiesCache = null;
    capabilitiesFetching = null;
  }
  if (capabilitiesCache) return capabilitiesCache;
  if (capabilitiesFetching) return capabilitiesFetching;
  capabilitiesFetching = api.get<LspCapabilities>('/lsp/capabilities')
    .then((c) => { capabilitiesFetchedAt = Date.now(); capabilitiesCache = c; return c; })
    .catch(() => null);
  return capabilitiesFetching;
}

export async function attachCodeEditorLsp(opts: {
  editorView: EditorView; filePath: string; rootPath: string; connId?: string | null;
  projectFolders?: string[];
  ideMode: boolean; language?: string; lspCompartment: Compartment;
  isCurrent: () => boolean; ensureIdeMod: () => Promise<typeof CmIdeMod>;
  onLocationJump: (targetPath: string, line: number, column: number, absLivePath: string) => void;
  onDiagnostics?: ((diags: IdeDiagnostic[]) => void) | null;
}): Promise<void> {
  try {
    if (!shouldAttachIdeLsp(opts.filePath, opts.language, opts.rootPath)) return;
    const resolvedRoot = resolveIdeLspModuleRoot(opts.rootPath, opts.filePath, opts.projectFolders ?? []);
    const absFile = toAbsoluteIdeFilePath(opts.rootPath, opts.filePath) || opts.filePath;
    const lspLang = lspLangFor(absFile, opts.ideMode, opts.language);
    if (!lspLang || !resolvedRoot) return;
    const serverLang = normalizeIdeLspServerLang(lspLang);
    if (!opts.connId) {
      let caps = await getLspCapabilities(false);
      if (opts.ideMode && caps && !caps.servers.some((s) => s.lang === serverLang && s.available)) {
        caps = await getLspCapabilities(true);
      }
      if (!caps?.servers.some((s) => s.lang === serverLang && s.available)) return;
    }
    if (!opts.isCurrent()) return;

    const wsUri = lspWsUrl(serverLang, resolvedRoot, opts.connId);
    const token = getToken();
    const protocols = token ? [WS_BEARER_SUBPROTOCOL, token] : undefined;
    const rootUri = `file://${resolvedRoot}`;
    const documentUri = absFile.includes('://') ? absFile : `file://${absFile}`;
    const mod = await opts.ensureIdeMod();
    if (!opts.isCurrent()) return;
    if (!mod.hasWarmPooledLspClient(serverLang, resolvedRoot, opts.connId)) {
      await new Promise<void>((resolve) => setTimeout(resolve, 160));
      if (!opts.isCurrent()) return;
    }

    const jump = (uri: string, line: number, column: number) => opts.onLocationJump(uriToPath(uri), line, column, absFile);
    try {
      const lspExtensions = await mod.buildPooledLspExtensions({
        wsUri, protocols, rootPath: resolvedRoot, connId: opts.connId,
        filePath: absFile, languageId: lspLang,
        onLocationJump: jump, onDiagnostics: opts.onDiagnostics,
      });
      if (!opts.isCurrent()) { mod.disposePooledLspClients(); return; }
      opts.editorView.dispatch({ effects: opts.lspCompartment.reconfigure(lspExtensions) });
    } catch {
      if (!opts.isCurrent()) return;
      const fallbackExt = mod.createDirectLspExtension({
        wsUri, protocols, rootUri, documentUri, filePath: absFile, languageId: lspLang,
        onLocationJump: jump, onDiagnostics: opts.onDiagnostics ?? undefined,
      });
      opts.editorView.dispatch({ effects: opts.lspCompartment.reconfigure(fallbackExt) });
    }
  } catch { /* LSP failures are silent — editor still shows highlighted code */ }
}

export async function sendSelectionToAgentSession(
  filePath: string, language: string | undefined, startLine: number, endLine: number, text: string,
): Promise<void> {
  const sessionId = ws.activeSessionId;
  if (!sessionId || ws.activeSession?.kind !== 'agent') {
    toasts.error('No agent session', 'Open an agent session first.');
    return;
  }
  const snippet = `Re: ${filePath}:${startLine}-${endLine}\n\n\`\`\`${langExtOf(filePath, language)}\n${text}\n\`\`\`\n\n`;
  try {
    await api.post(`/sessions/${sessionId}/input`, { text: snippet, submit: false });
    toasts.success('Sent to agent', `${filePath} lines ${startLine}-${endLine}`);
  } catch {
    toasts.error('Couldn’t send to the session', 'The text couldn’t be injected into the agent session.');
  }
}

let cachedDepsMod: typeof CmIdeDepsMod | null = null;
let cachedDepsModPromise: Promise<typeof CmIdeDepsMod> | null = null;

export function mountIdeManifestDeps(opts: {
  target: EditorView; forPath: string; ideMode: boolean; appliedPath: string;
  setAppliedPath: (p: string) => void; isCurrent: () => boolean;
  ideDepsCompartment: Compartment; root: string; connectionId?: string | null; projectFolders?: string[];
  onApplyContent: (next: string) => void;
  onOpenIntentionMenu: (line: number, dep: CmIdeDepsMod.IdeDepItem | null) => void;
  onJumpLocation: (loc: { path?: string; line: number; column: number }) => void;
  onAddAgentContext: () => void; onOpenLocalHistory: () => void; onOpenDepAnalyzer: () => void;
  onToggleBlame?: (() => void) | null;
  onDepsUpdated: (items: CmIdeDepsMod.IdeDepItem[]) => void;
}): void {
  if (!opts.ideMode || !isManifestPath(opts.forPath)) {
    if (opts.appliedPath) {
      opts.setAppliedPath('');
      opts.target.dispatch({ effects: opts.ideDepsCompartment.reconfigure([]) });
      opts.onDepsUpdated([]);
    }
    return;
  }
  if (opts.appliedPath === opts.forPath) return;
  opts.setAppliedPath(opts.forPath);
  const apply = (m: typeof CmIdeDepsMod) => {
    if (!opts.isCurrent()) return;
    const ext = m.createIdeDepsExtension({
      filePath: opts.forPath, root: opts.root, connectionId: opts.connectionId, projectFolders: opts.projectFolders,
      onApplyContent: opts.onApplyContent, onOpenIntentionMenu: opts.onOpenIntentionMenu,
      onJumpLocation: opts.onJumpLocation, onAddAgentContext: opts.onAddAgentContext,
      onOpenLocalHistory: opts.onOpenLocalHistory, onOpenDepAnalyzer: opts.onOpenDepAnalyzer,
      onToggleBlame: opts.onToggleBlame, onDepsUpdated: opts.onDepsUpdated,
    });
    opts.target.dispatch({ effects: opts.ideDepsCompartment.reconfigure(ext) });
  };
  if (cachedDepsMod) apply(cachedDepsMod);
  else void (cachedDepsModPromise ??= import('./cm-ide-deps').then((m) => ((cachedDepsMod = m), m))).then(apply, () => {});
}
