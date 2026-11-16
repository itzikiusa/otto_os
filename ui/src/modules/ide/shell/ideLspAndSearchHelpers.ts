// Extracted LSP, Workspace Search/Replace, Debug Evaluation, and Collab/Plugin
// API delegates for `IdeStore` so `ideStore.svelte.ts` stays focused and under 800 lines.

import { api } from '../../../lib/api/client';
import { readIdePreferences } from '../../../lib/components/cm-ide-prefs';
import { loadErrorText } from '../../../lib/loadError';
import { plural } from '../../../lib/plural';
import { toasts } from '../../../lib/toast.svelte';
import { toastError } from '../../../lib/toastError';
import { findContainingRoot } from './ideProjectHelpers';
import {
  type IdeBufferHost,
  type IdeBufferPerfController,
  reloadTabsFromDisk,
} from '../store/ideBufferPerf';
import {
  collabGuestPost,
  detectTextEol,
  normalizeEditorLf,
  serializeTabDiskContent,
  type IdeCodeAction,
  type IdeCollabRole,
  type IdeCollabRoom,
  type IdeDebugSessionSnapshot,
  type IdeFileReadResp,
  type IdePluginInfo,
  type IdeQuickFix,
  type IdeReferenceLocation,
  type IdeRenameEdit,
  type IdeReplaceResp,
  type IdeReplEntry,
  type IdeSearchItem,
  type IdeSearchMode,
  type IdeTextEdit,
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

async function flushDirtyOpenTabsForWorkspaceOp(
  host: IdeLspSearchHost,
  bufPerf?: IdeBufferPerfController,
  persistAllDirty = false,
): Promise<void> {
  if (!bufPerf) return;
  bufPerf.flushAll(host);
  if (!host.canEditCollab || host.collabIsGuest) return;
  if (!persistAllDirty && !readIdePreferences().autoSave) return;
  const dirtyTabs = host.tabs.filter(
    (t) => !t.readOnly && !t.truncated && bufPerf.isTabDirty(t) && !bufPerf.conflictedPaths.has(t.path),
  );
  for (let i = 0; i < dirtyTabs.length; i++) {
    await host.saveFile(dirtyTabs[i].path, false, { silent: true });
  }
}

export async function runFindReferences(
  host: IdeLspSearchHost,
  path: string,
  line: number,
  column: number,
  symbol?: string,
  bufPerf?: IdeBufferPerfController,
): Promise<void> {
  host.usagesLoading = true;
  host.openBottomTab('usages');
  try {
    await flushDirtyOpenTabsForWorkspaceOp(host, bufPerf, false);
    const resp = await api.post<{ symbol: string; locations: IdeReferenceLocation[] }>('/ide/lsp/references', {
      target: host.targetPayload,
      project_folders: host.targetPayload.project_folders,
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

export async function runPreviewRename(
  host: IdeLspSearchHost,
  newName?: string,
  bufPerf?: IdeBufferPerfController,
): Promise<void> {
  const next = (newName ?? host.renameState.newName).trim();
  host.renameState.newName = next;
  if (!next) return;
  host.renameState.loading = true;
  host.renameState.error = null;
  try {
    await flushDirtyOpenTabsForWorkspaceOp(host, bufPerf, false);
    const content = bufPerf ? bufPerf.flush(host, host.renameState.path) : host.getBuffer(host.renameState.path);
    const resp = await api.post<{ symbol: string; new_name: string; edits: IdeRenameEdit[] }>('/ide/lsp/rename', {
      target: host.targetPayload,
      project_folders: host.targetPayload.project_folders,
      path: host.renameState.path,
      line: host.renameState.line,
      column: host.renameState.column,
      symbol: host.renameState.symbol || undefined,
      content: content || undefined,
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
    const prefs = readIdePreferences();
    await flushDirtyOpenTabsForWorkspaceOp(host, bufPerf, true);
    const content = bufPerf.flush(host, host.renameState.path);
    const resp = await api.post<{ applied: boolean; edits: IdeRenameEdit[] }>('/ide/lsp/rename', {
      target: host.targetPayload,
      project_folders: host.targetPayload.project_folders,
      path: host.renameState.path,
      line: host.renameState.line,
      column: host.renameState.column,
      symbol: host.renameState.symbol || undefined,
      content: content || undefined,
      new_name: next,
      apply: true,
      local_history_enabled: prefs.localHistoryEnabled,
      local_history_retention_days: prefs.localHistoryRetentionDays,
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
  opts: { organizeImports?: boolean } = {},
): Promise<void> {
  const tab = host.activeTab;
  if (!tab || tab.readOnly || tab.truncated) return;
  const targetPath = tab.path;
  const content = bufPerf.flush(host, targetPath);
  try {
    const resp = await api.post<{ formatted: string; changed: boolean; formatter: string }>('/ide/lsp/format', {
      target: host.targetPayload,
      path: targetPath,
      content,
      language: tab.language,
      organize_imports: Boolean(opts.organizeImports),
    });
    if (resp.changed && typeof resp.formatted === 'string') {
      if (bufPerf.getBuffer(host, targetPath) !== content || !host.tabs.some((t) => t.path === targetPath)) {
        return;
      }
      await host.applyAgentEdit(targetPath, normalizeEditorLf(resp.formatted));
      toasts.info(opts.organizeImports ? 'Optimized imports' : `Formatted with ${resp.formatter || 'formatter'}`);
    }
  } catch (e) {
    toastError(opts.organizeImports ? 'Couldn’t optimize imports' : 'Couldn’t format file', e);
  }
}

const GO_STDLIB_BY_IDENT: Record<string, string> = {
  fmt: 'fmt', context: 'context', strings: 'strings', errors: 'errors', time: 'time',
  http: 'net/http', json: 'encoding/json', os: 'os', io: 'io', sync: 'sync', strconv: 'strconv',
  filepath: 'path/filepath', regexp: 'regexp', bytes: 'bytes', sort: 'sort', math: 'math',
};

const JAVA_UTIL_BY_IDENT: Record<string, string> = {
  List: 'java.util.List', ArrayList: 'java.util.ArrayList', Map: 'java.util.Map',
  HashMap: 'java.util.HashMap', Set: 'java.util.Set', HashSet: 'java.util.HashSet',
  Optional: 'java.util.Optional', Objects: 'java.util.Objects', UUID: 'java.util.UUID',
  Collectors: 'java.util.stream.Collectors',
};

function lineStartsOf(src: string): number[] {
  const starts = [0];
  for (let i = 0; i < src.length; i++) if (src.charCodeAt(i) === 10) starts.push(i + 1);
  return starts;
}

export function textEditToOffsets(src: string, edit: IdeTextEdit): { from: number; to: number } {
  const starts = lineStartsOf(src);
  const toOff = (ln: number, col: number): number => {
    if (Math.floor(ln) > starts.length) return src.length; // past EOF: append (no trailing newline case)
    const idx = Math.max(0, Math.floor(ln) - 1);
    const lineStart = starts[idx];
    const limit = idx + 1 < starts.length ? starts[idx + 1] : src.length;
    return Math.min(limit, Math.max(lineStart, lineStart + Math.max(0, Math.floor(col) - 1)));
  };
  const from = toOff(edit.start_line, edit.start_col);
  const to = Math.max(from, toOff(edit.end_line, edit.end_col));
  return { from, to };
}

function applyTextEditsToString(src: string, edits: IdeTextEdit[]): string {
  const resolved = edits
    .map((e) => ({ ...textEditToOffsets(src, e), newText: e.new_text }))
    .sort((a, b) => b.from - a.from || b.to - a.to);
  let out = src;
  for (let i = 0; i < resolved.length; i++) {
    const r = resolved[i];
    out = out.slice(0, r.from) + r.newText + out.slice(r.to);
  }
  return out;
}

function buildFallbackQuickFixes(path: string, content: string, line: number, diagMsg = ''): IdeCodeAction[] {
  if (!content) return [];
  const lines = content.split('\n');
  const idx = Math.max(0, Math.min(lines.length - 1, line - 1));
  const curLine = lines[idx] ?? '';
  const actions: IdeCodeAction[] = [];

  if (/^\s*(?:import|use)\b/.test(curLine) || (path.endsWith('.go') && /^\s*(?:[A-Za-z_]\w*\s+)?"[^"]+"\s*$/.test(curLine))) {
    const endLine = idx + 1 < lines.length ? line + 1 : line;
    const endCol = idx + 1 < lines.length ? 1 : curLine.length + 1;
    actions.push({
      id: 'remove-import-line',
      title: 'Remove unused / unresolved import',
      kind: 'quickfix',
      is_preferred: true,
      edits: [{ path, start_line: line, start_col: 1, end_line: endLine, end_col: endCol, new_text: '' }],
    });
  }

  if (path.endsWith('.go')) {
    const undefMatch = diagMsg.match(/(?:undefined|undeclared name):\s*([A-Za-z_]\w*)/i) ?? curLine.match(/\b(fmt|context|strings|errors|time|http|json|os|io|sync|strconv)\./);
    const ident = undefMatch?.[1] ?? '';
    const pkg = GO_STDLIB_BY_IDENT[ident];
    if (pkg && !content.includes(`"${pkg}"`)) {
      const pkgLineIdx = lines.findIndex((l) => /^\s*package\s+\w+/.test(l));
      const insertLine = pkgLineIdx >= 0 ? pkgLineIdx + 2 : 1;
      actions.push({
        id: `go-import-${pkg}`,
        title: `Import "${pkg}"`,
        kind: 'quickfix',
        is_preferred: true,
        edits: [{ path, start_line: insertLine, start_col: 1, end_line: insertLine, end_col: 1, new_text: `\nimport "${pkg}"\n` }],
      });
    }
  } else if (path.endsWith('.java')) {
    const symMatch = diagMsg.match(/(?:cannot find symbol|unresolved).*?\b([A-Z]\w*)/i) ?? curLine.match(/\b(List|ArrayList|Map|HashMap|Set|HashSet|Optional|Objects|UUID|Collectors)\b/);
    const sym = symMatch?.[1] ?? '';
    const fqn = JAVA_UTIL_BY_IDENT[sym];
    if (fqn && !content.includes(`import ${fqn};`)) {
      const pkgIdx = lines.findIndex((l) => /^\s*package\s+[\w.]+;/.test(l));
      const insertLine = pkgIdx >= 0 ? pkgIdx + 2 : 1;
      actions.push({
        id: `java-import-${fqn}`,
        title: `Import '${fqn}'`,
        kind: 'quickfix',
        is_preferred: true,
        edits: [{ path, start_line: insertLine, start_col: 1, end_line: insertLine, end_col: 1, new_text: `import ${fqn};\n` }],
      });
    }
  } else if (path.endsWith('.json') && /,\s*$/.test(curLine)) {
    const commaIdx = curLine.lastIndexOf(',');
    actions.push({
      id: 'json-remove-trailing-comma',
      title: 'Remove trailing comma',
      kind: 'quickfix',
      is_preferred: true,
      edits: [{ path, start_line: line, start_col: commaIdx + 1, end_line: line, end_col: commaIdx + 2, new_text: '' }],
    });
  }
  return actions;
}

export function toIdeCodeAction(
  path: string,
  item: IdeCodeAction | IdeQuickFix,
  idx = 0,
  content = '',
): IdeCodeAction {
  if ('id' in item) return item;
  let edits = item.edits;
  if (!edits || edits.length === 0) {
    const lines = content ? content.split('\n') : null;
    const endIdx = Math.max(0, item.end_line - 1);
    if (item.delete_line) {
      const hasNext = lines ? item.end_line < lines.length : true;
      edits = hasNext
        ? [{ path, start_line: item.line, start_col: 1, end_line: item.end_line + 1, end_col: 1, new_text: '' }]
        : [{ path, start_line: item.line, start_col: 1, end_line: item.end_line, end_col: (lines?.[endIdx]?.length ?? 0) + 1, new_text: '' }];
    } else {
      const endCol = lines && endIdx < lines.length ? lines[endIdx].length + 1 : 1;
      const endLine = lines && endIdx < lines.length ? item.end_line : item.end_line + 1;
      const text = lines && endIdx < lines.length ? item.replacement : `${item.replacement}\n`;
      edits = [{ path, start_line: item.line, start_col: 1, end_line: endLine, end_col: endCol, new_text: text }];
    }
  }
  return {
    id: `qf-${item.line}-${idx}`,
    title: item.title,
    kind: 'quickfix',
    is_preferred: item.is_preferred ?? idx === 0,
    edits,
  };
}

export async function runFetchCodeActions(
  host: IdeLspSearchHost,
  bufPerf: IdeBufferPerfController,
  path: string,
  line: number,
  column: number,
): Promise<IdeCodeAction[]> {
  const resolved = host.resolvePath(path);
  let content = bufPerf.flush(host, resolved);
  if (!content && !host.tabs.some((t) => t.path === resolved)) {
    try {
      const containing = host.target.kind === 'local' && host.projectRoots
        ? findContainingRoot(resolved, host.projectRoots)
        : null;
      const q = containing && !containing.isPrimary && host.target.kind === 'local'
        ? host.targetQuery({ root: containing.path, path: resolved })
        : host.targetQuery({ path: resolved });
      const read = await api.get<IdeFileReadResp>(`/ide/fs/file?${q}`);
      content = read.content ?? '';
    } catch { /* ignore */ }
  }
  const diagOnLine = host.problems.find((d) => host.resolvePath(d.path) === resolved && d.line === line);
  const diagActions = (diagOnLine?.quick_fixes ?? []).map((qf, idx) => toIdeCodeAction(resolved, qf, idx, content));
  try {
    const resp = await api.post<{ actions: IdeCodeAction[] }>('/ide/lsp/code-actions', {
      target: host.targetPayload,
      project_folders: host.targetPayload.project_folders,
      path: resolved,
      line,
      column,
      content,
    });
    const serverActions = resp.actions ?? [];
    if (serverActions.some((a) => (a.edits && a.edits.length > 0) || typeof a.replacement_content === 'string')) {
      return serverActions;
    }
    const fallbacks = buildFallbackQuickFixes(resolved, content, line, diagOnLine?.message);
    return [...diagActions, ...fallbacks, ...serverActions];
  } catch {
    return [...diagActions, ...buildFallbackQuickFixes(resolved, content, line, diagOnLine?.message)];
  }
}

export async function runApplyQuickFixEdits(
  host: IdeLspSearchHost,
  bufPerf: IdeBufferPerfController,
  edits: IdeTextEdit[],
  title = 'Applied quick fix',
): Promise<boolean> {
  if (!edits || edits.length === 0) return false;
  const byFile = new Map<string, IdeTextEdit[]>();
  for (let i = 0; i < edits.length; i++) {
    const p = host.resolvePath(edits[i].path);
    const arr = byFile.get(p) ?? [];
    arr.push({ ...edits[i], path: p });
    byFile.set(p, arr);
  }
  let anyApplied = false;
  for (const [filePath, fileEdits] of byFile.entries()) {
    const openTab = host.tabs.find((t) => t.path === filePath);
    if (openTab) {
      const current = bufPerf.flush(host, filePath);
      if (fileEdits.length === 1 && filePath === host.activePath) {
        const span = textEditToOffsets(current, fileEdits[0]);
        await host.applyAgentEdit(filePath, fileEdits[0].new_text, span);
      } else {
        const nextDoc = applyTextEditsToString(current, fileEdits);
        await host.applyAgentEdit(filePath, nextDoc);
      }
      const editedLines = new Set(fileEdits.map((e) => e.start_line));
      host.problems = host.problems.filter((d) => !(host.resolvePath(d.path) === filePath && editedLines.has(d.line)));
      anyApplied = true;
    } else {
      try {
        const prefs = readIdePreferences();
        const containing = host.target.kind === 'local' && host.projectRoots
          ? findContainingRoot(filePath, host.projectRoots)
          : null;
        const readQuery = containing && !containing.isPrimary && host.target.kind === 'local'
          ? host.targetQuery({ root: containing.path, path: filePath })
          : host.targetQuery({ path: filePath });
        const fileTarget = containing && !containing.isPrimary && host.target.kind === 'local'
          ? { ...host.targetPayload, root: containing.path }
          : host.targetPayload;
        const read = await api.get<IdeFileReadResp>(`/ide/fs/file?${readQuery}`);
        if (read.truncated || read.read_only) continue;
        const rawPrev = read.content ?? '';
        const eol = detectTextEol(rawPrev);
        const prevLf = normalizeEditorLf(rawPrev);
        if (prefs.localHistoryEnabled) {
          void api.post('/ide/history/snapshot', { target: fileTarget, path: filePath, content: rawPrev, label: 'Before Quick Fix' }).catch(() => {});
        }
        const nextLf = applyTextEditsToString(prevLf, fileEdits);
        const nextDisk = serializeTabDiskContent({ eol }, nextLf);
        await api.put('/ide/fs/file', {
          target: fileTarget,
          path: filePath,
          content: nextDisk,
          expected_sha256: read.sha256 || undefined,
          create_dirs: false,
          local_history_enabled: prefs.localHistoryEnabled,
          local_history_retention_days: prefs.localHistoryRetentionDays,
        });
        const editedLines = new Set(fileEdits.map((e) => e.start_line));
        host.problems = host.problems.filter((d) => !(host.resolvePath(d.path) === filePath && editedLines.has(d.line)));
        anyApplied = true;
      } catch (e) {
        toastError('Couldn’t apply quick fix to file', e);
      }
    }
  }
  if (anyApplied) toasts.info(title);
  return anyApplied;
}

export async function runApplyCodeAction(
  host: IdeLspSearchHost,
  bufPerf: IdeBufferPerfController,
  path: string,
  rawAction: IdeCodeAction | IdeQuickFix,
): Promise<boolean> {
  const resolved = host.resolvePath(path);
  const action = 'id' in rawAction ? rawAction : toIdeCodeAction(resolved, rawAction, 0, bufPerf.flush(host, resolved));
  if (action.edits && action.edits.length > 0) {
    return runApplyQuickFixEdits(host, bufPerf, action.edits, action.title);
  }
  if (typeof action.replacement_content === 'string') {
    const ok = await host.applyAgentEdit(resolved, action.replacement_content);
    if (ok) toasts.info(action.title);
    return ok;
  }
  return false;
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
  const effectiveQuery = mode === 'text' ? query : query.trim();
  if (!host.target.root || effectiveQuery.length === 0) {
    host.searchResults = [];
    host.searchTruncated = false;
    return;
  }
  host.searchLoading = true;
  host.searchError = null;
  try {
    const resp = await api.long.post<{ items: IdeSearchItem[]; truncated: boolean }>('/ide/search', {
      target: host.targetPayload,
      query: effectiveQuery,
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
  if (!host.target.root || query.length === 0) return null;
  host.replaceLoading = true;
  try {
    await flushDirtyOpenTabsForWorkspaceOp(host, bufPerf, !dryRun);
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
): Promise<boolean> {
  bufPerf.flush(host, path);
  if (!host.collabRoom || !host.collabPeerId || !host.collabPeerSecret || !host.canEditCollab) return false;
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
    return true;
  } catch (e) {
    if (save) toastError('Couldn’t sync edit to room', e);
    return false;
  }
}

export async function runSetCollabPeerRole(
  host: IdeLspSearchHost,
  peerId: string,
  role: IdeCollabRole,
): Promise<boolean> {
  if (!host.collabRoom) return false;
  try {
    host.collabRoom = (
      await api.post<{ room: IdeCollabRoom }>(
        `/ide/collab/rooms/${encodeURIComponent(host.collabRoom.id)}/role`,
        { peer_id: peerId, role },
      )
    ).room;
    return true;
  } catch (e) {
    toastError('Couldn’t update teammate role', e);
    return false;
  }
}

export async function runTogglePlugin(host: IdeLspSearchHost, slug: string, enabled: boolean): Promise<boolean> {
  try {
    const resp = await api.post<{ plugin: IdePluginInfo }>(
      `/ide/plugins/${encodeURIComponent(slug)}/toggle`,
      { enabled },
    );
    host.plugins = host.plugins.map((p) => (p.slug === slug ? resp.plugin : p));
    return true;
  } catch (e) {
    toastError('Couldn’t toggle IDE plugin', e);
    return false;
  }
}

export async function runRemovePlugin(host: IdeLspSearchHost, slug: string): Promise<boolean> {
  try {
    await api.del(`/ide/plugins/${encodeURIComponent(slug)}`);
    host.plugins = host.plugins.filter((p) => p.slug !== slug);
    await host.loadPlugins();
    return true;
  } catch (e) {
    toastError('Couldn’t remove IDE plugin', e);
    return false;
  }
}
