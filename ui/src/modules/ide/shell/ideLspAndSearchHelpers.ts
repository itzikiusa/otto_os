// Extracted LSP, Workspace Search/Replace, Debug Evaluation, and Collab/Plugin
// API delegates for `IdeStore` so `ideStore.svelte.ts` stays focused and under 800 lines.

import { api } from '../../../lib/api/client';
import { loadErrorText } from '../../../lib/loadError';
import { plural } from '../../../lib/plural';
import { toasts } from '../../../lib/toast.svelte';
import { toastError } from '../../../lib/toastError';
import {
  type IdeBufferHost,
  type IdeBufferPerfController,
  reloadTabsFromDisk,
} from '../store/ideBufferPerf';
import {
  collabGuestPost,
  type IdeCodeAction,
  type IdeCollabRole,
  type IdeCollabRoom,
  type IdeDebugSessionSnapshot,
  type IdePluginInfo,
  type IdeReferenceLocation,
  type IdeRenameEdit,
  type IdeReplaceResp,
  type IdeReplEntry,
  type IdeSearchItem,
  type IdeSearchMode,
  type IdeVariable,
} from '../store/ideTypes';

export interface IdeLspSearchHost extends IdeBufferHost {
  usages: IdeReferenceLocation[];
  usagesSymbol: string;
  usagesLoading: boolean;
  renameState: {
    open: boolean;
    path: string;
    line: number;
    column: number;
    symbol: string;
    newName: string;
    edits: IdeRenameEdit[];
    loading: boolean;
    error: string | null;
  };
  searchScopePath: string | null;
  searchResults: IdeSearchItem[];
  searchTruncated: boolean;
  searchLoading: boolean;
  searchError: string | null;
  replacePreview: IdeReplaceResp | null;
  replaceLoading: boolean;
  activeDebug: IdeDebugSessionSnapshot | null;
  expandedVariables: Record<number, IdeVariable[]>;
  replEntries: IdeReplEntry[];
  collabRoom: IdeCollabRoom | null;
  collabRooms: IdeCollabRoom[];
  collabPeerId: string | null;
  collabPeerSecret: string | null;
  collabGuestToken: string | null;
  sharedBuffers: Record<string, { content: string; version: number }>;
  plugins: IdePluginInfo[];
  getBuffer(path?: string): string;
  openBottomTab(tab: 'usages'): void;
  applyAgentEdit(path: string, nextContent: string, range?: { from: number; to: number }): Promise<boolean>;
  loadPlugins(): Promise<void>;
}

export async function runFindReferences(
  host: IdeLspSearchHost,
  path: string,
  line: number,
  column: number,
  symbol?: string,
): Promise<void> {
  host.usagesLoading = true;
  host.openBottomTab('usages');
  try {
    const resp = await api.post<{ symbol: string; locations: IdeReferenceLocation[] }>('/ide/lsp/references', {
      target: host.targetPayload,
      path,
      line,
      column,
      symbol,
      content: host.getBuffer(path),
    });
    host.usagesSymbol = resp.symbol || symbol || '';
    host.usages = resp.locations ?? [];
  } catch (e) {
    toastError('Couldn’t find usages', e);
  } finally {
    host.usagesLoading = false;
  }
}

export async function runPreviewRename(host: IdeLspSearchHost, newName?: string): Promise<void> {
  const next = (newName ?? host.renameState.newName).trim();
  host.renameState.newName = next;
  if (!next) return;
  host.renameState.loading = true;
  host.renameState.error = null;
  try {
    const resp = await api.post<{ symbol: string; new_name: string; edits: IdeRenameEdit[] }>('/ide/lsp/rename', {
      target: host.targetPayload,
      path: host.renameState.path,
      line: host.renameState.line,
      column: host.renameState.column,
      new_name: next,
      apply: false,
    });
    if (resp.symbol && !host.renameState.symbol) host.renameState.symbol = resp.symbol;
    host.renameState.edits = resp.edits ?? [];
  } catch (e) {
    host.renameState.error = loadErrorText(e);
  } finally {
    host.renameState.loading = false;
  }
}

export async function runApplyRename(
  host: IdeLspSearchHost,
  bufPerf: IdeBufferPerfController,
): Promise<boolean> {
  const next = host.renameState.newName.trim();
  if (!next) return false;
  host.renameState.loading = true;
  host.renameState.error = null;
  try {
    const resp = await api.post<{ applied: boolean; edits: IdeRenameEdit[] }>('/ide/lsp/rename', {
      target: host.targetPayload,
      path: host.renameState.path,
      line: host.renameState.line,
      column: host.renameState.column,
      new_name: next,
      apply: true,
    });
    const touched = new Set((resp.edits ?? []).map((e) => e.path));
    await reloadTabsFromDisk(host, bufPerf, touched);
    host.renameState.open = false;
    toasts.success(`Renamed to “${next}”`, `${plural(touched.size, 'file')} updated`);
    return true;
  } catch (e) {
    host.renameState.error = loadErrorText(e);
    return false;
  } finally {
    host.renameState.loading = false;
  }
}

export async function runFormatActiveFile(
  host: IdeLspSearchHost,
  bufPerf: IdeBufferPerfController,
): Promise<void> {
  const tab = host.activeTab;
  if (!tab || tab.readOnly) return;
  const content = bufPerf.flush(host, tab.path);
  try {
    const resp = await api.post<{ formatted: string; changed: boolean; formatter: string }>('/ide/lsp/format', {
      target: host.targetPayload,
      path: tab.path,
      content,
      language: tab.language,
    });
    if (resp.changed && typeof resp.formatted === 'string') {
      await host.applyAgentEdit(tab.path, resp.formatted);
      toasts.info(`Formatted with ${resp.formatter || 'formatter'}`);
    }
  } catch (e) {
    toastError('Couldn’t format file', e);
  }
}

export async function runFetchCodeActions(
  host: IdeLspSearchHost,
  bufPerf: IdeBufferPerfController,
  path: string,
  line: number,
  column: number,
): Promise<IdeCodeAction[]> {
  const content = bufPerf.flush(host, path);
  try {
    const resp = await api.post<{ actions: IdeCodeAction[] }>('/ide/lsp/code-actions', {
      target: host.targetPayload,
      path,
      line,
      column,
      content,
    });
    return resp.actions ?? [];
  } catch (e) {
    toastError('Couldn’t load code actions', e);
    return [];
  }
}

export async function runSearchWorkspace(
  host: IdeLspSearchHost,
  query: string,
  mode: IdeSearchMode = 'all',
  opts: {
    caseSensitive?: boolean;
    wholeWord?: boolean;
    regex?: boolean;
    fileGlob?: string;
    scopePath?: string | null;
    limit?: number;
  } = {},
): Promise<void> {
  if (!host.target.root || !query.trim()) {
    host.searchResults = [];
    host.searchTruncated = false;
    return;
  }
  host.searchLoading = true;
  host.searchError = null;
  try {
    const resp = await api.long.post<{ items: IdeSearchItem[]; truncated: boolean }>('/ide/search', {
      target: host.targetPayload,
      query: query.trim(),
      mode,
      case_sensitive: opts.caseSensitive ?? false,
      whole_word: opts.wholeWord ?? false,
      regex: opts.regex ?? false,
      file_glob: opts.fileGlob || undefined,
      scope_path: opts.scopePath ?? host.searchScopePath ?? undefined,
      limit: opts.limit ?? 80,
    });
    host.searchResults = resp.items ?? [];
    host.searchTruncated = Boolean(resp.truncated);
  } catch (e) {
    host.searchError = loadErrorText(e);
  } finally {
    host.searchLoading = false;
  }
}

export async function runReplaceInWorkspace(
  host: IdeLspSearchHost,
  bufPerf: IdeBufferPerfController,
  query: string,
  replacement: string,
  dryRun: boolean,
  opts: {
    caseSensitive?: boolean;
    wholeWord?: boolean;
    regex?: boolean;
    fileGlob?: string;
    scopePath?: string | null;
    paths?: string[];
  } = {},
): Promise<IdeReplaceResp | null> {
  if (!host.target.root || !query) return null;
  host.replaceLoading = true;
  try {
    const resp = await api.long.post<IdeReplaceResp>('/ide/replace', {
      target: host.targetPayload,
      query,
      replacement,
      case_sensitive: opts.caseSensitive ?? false,
      whole_word: opts.wholeWord ?? false,
      regex: opts.regex ?? false,
      file_glob: opts.fileGlob || undefined,
      scope_path: opts.scopePath ?? host.searchScopePath ?? undefined,
      paths: opts.paths,
      dry_run: dryRun,
    });
    host.replacePreview = resp;
    if (!dryRun) {
      await reloadTabsFromDisk(host, bufPerf, new Set(resp.previews.map((p) => p.path)));
      toasts.success(
        `Replaced ${plural(resp.replacements, 'occurrence')}`,
        `${plural(resp.files_changed, 'file')} updated`,
      );
    }
    return resp;
  } catch (e) {
    toastError('Couldn’t replace in project', e);
    return null;
  } finally {
    host.replaceLoading = false;
  }
}

export async function runExpandVariables(host: IdeLspSearchHost, variablesRef: number): Promise<IdeVariable[]> {
  if (!host.activeDebug || variablesRef <= 0) return [];
  try {
    const resp = await api.post<{ variables: IdeVariable[] }>(
      `/ide/debug/sessions/${encodeURIComponent(host.activeDebug.id)}/variables`,
      { variables_reference: variablesRef },
    );
    const list = resp.variables ?? [];
    host.expandedVariables = { ...host.expandedVariables, [variablesRef]: list };
    return list;
  } catch (e) {
    toastError('Couldn’t inspect variables', e);
    return [];
  }
}

export async function runEvaluateDebug(
  host: IdeLspSearchHost,
  expression: string,
  context: 'watch' | 'repl' | 'hover' = 'repl',
  frameId?: number,
): Promise<string> {
  const expr = expression.trim();
  if (!expr || !host.activeDebug) return '';
  try {
    const resp = await api.post<{ result: string; var_type?: string; variables_reference: number }>(
      `/ide/debug/sessions/${encodeURIComponent(host.activeDebug.id)}/evaluate`,
      { expression: expr, frame_id: frameId ?? host.activeDebug.active_frame_id ?? undefined, context },
    );
    if (context === 'repl') {
      host.replEntries = [
        ...host.replEntries,
        { id: `${Date.now()}-${host.replEntries.length}`, expression: expr, result: resp.result, var_type: resp.var_type },
      ].slice(-100);
    }
    return resp.result;
  } catch (e) {
    const msg = loadErrorText(e);
    if (context === 'repl') {
      host.replEntries = [
        ...host.replEntries,
        { id: `${Date.now()}-${host.replEntries.length}`, expression: expr, result: msg, isError: true },
      ].slice(-100);
    }
    return msg;
  }
}

export async function runSendCursorUpdate(
  host: IdeLspSearchHost,
  path: string,
  selections: { anchor: number; head: number }[],
  activeLine: number,
): Promise<void> {
  if (!host.collabRoom || !host.collabPeerId || !host.collabPeerSecret) return;
  try {
    await collabGuestPost(
      `/ide/collab/guest/${encodeURIComponent(host.collabRoom.id)}/cursor`,
      host.collabGuestToken,
      { peer_id: host.collabPeerId, peer_secret: host.collabPeerSecret, path, selections, active_line: activeLine },
    );
  } catch {
    /* transient */
  }
}

export async function runSendCollabEdit(
  host: IdeLspSearchHost,
  bufPerf: IdeBufferPerfController,
  path: string,
  content: string,
  save = false,
): Promise<void> {
  bufPerf.flush(host, path);
  if (!host.collabRoom || !host.collabPeerId || !host.collabPeerSecret || !host.canEditCollab) return;
  try {
    const res = await collabGuestPost<{ version: number }>(
      `/ide/collab/guest/${encodeURIComponent(host.collabRoom.id)}/edit`,
      host.collabGuestToken,
      {
        peer_id: host.collabPeerId,
        peer_secret: host.collabPeerSecret,
        path,
        content,
        base_version: host.sharedBuffers[path]?.version,
        save,
      },
    );
    host.sharedBuffers = { ...host.sharedBuffers, [path]: { content, version: res.version } };
  } catch (e) {
    if (save) toastError('Couldn’t sync edit to room', e);
  }
}

export async function runSetCollabPeerRole(
  host: IdeLspSearchHost,
  peerId: string,
  role: IdeCollabRole,
): Promise<void> {
  if (!host.collabRoom) return;
  try {
    host.collabRoom = (
      await api.post<{ room: IdeCollabRoom }>(
        `/ide/collab/rooms/${encodeURIComponent(host.collabRoom.id)}/role`,
        { peer_id: peerId, role },
      )
    ).room;
  } catch (e) {
    toastError('Couldn’t update teammate role', e);
  }
}

export async function runTogglePlugin(host: IdeLspSearchHost, slug: string, enabled: boolean): Promise<void> {
  try {
    const resp = await api.post<{ plugin: IdePluginInfo }>(
      `/ide/plugins/${encodeURIComponent(slug)}/toggle`,
      { enabled },
    );
    host.plugins = host.plugins.map((p) => (p.slug === slug ? resp.plugin : p));
  } catch (e) {
    toastError('Couldn’t toggle IDE plugin', e);
  }
}

export async function runRemovePlugin(host: IdeLspSearchHost, slug: string): Promise<void> {
  try {
    await api.del(`/ide/plugins/${encodeURIComponent(slug)}`);
    host.plugins = host.plugins.filter((p) => p.slug !== slug);
    await host.loadPlugins();
    toasts.info('Removed IDE plugin');
  } catch (e) {
    toastError('Couldn’t remove IDE plugin', e);
  }
}
