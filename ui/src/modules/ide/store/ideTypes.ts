// Canonical TypeScript types & pure helpers for the Otto Full IDE module.
// Re-exported from `../ideStore.svelte.ts` so all IDE components import from one place.

import { getToken, laneFetch } from '../../../lib/api/client';

export type IdeTargetKind = 'local' | 'ssh';
export type IdeKeymapPreset = 'jetbrains' | 'standard';
export type IdeBottomTab = 'debug' | 'run' | 'problems' | 'usages' | 'terminal';
export type IdeSearchMode = 'files' | 'text' | 'symbols' | 'all';
export type IdeCollabRole = 'viewer' | 'editor';

export interface IdeTargetSpec {
  kind: IdeTargetKind;
  workspace_id?: string | null;
  root: string;
  connection_id?: string | null;
  connection_name?: string | null;
}

export interface IdeFsEntry {
  name: string;
  path: string;
  kind: 'dir' | 'file' | 'symlink';
  size?: number;
  mtime?: number;
  git_status?: 'M' | 'A' | 'D' | '?' | string | null;
  language?: string | null;
}

export interface IdeFsTreeResp {
  root: string;
  path: string;
  parent?: string | null;
  target_os?: string;
  target_arch?: string;
  entries: IdeFsEntry[];
}

export interface IdeFileReadResp {
  path: string;
  content: string;
  language?: string | null;
  size: number;
  sha256: string;
  read_only: boolean;
  truncated: boolean;
  line_count: number;
}

export interface IdeOpenTab {
  path: string;
  name: string;
  content: string;
  savedContent: string;
  sha256: string;
  language: string;
  readOnly: boolean;
  truncated: boolean;
  loading: boolean;
  error: string | null;
  preview: boolean;
}

export interface IdeSearchItem {
  kind: 'file' | 'text' | 'symbol';
  path: string;
  line?: number;
  column?: number;
  preview: string;
  symbol_kind?: string | null;
  container?: string | null;
}

export interface IdeReplacePreviewItem {
  path: string;
  count: number;
  sample_before: string;
  sample_after: string;
}

export interface IdeReplaceResp {
  files_changed: number;
  replacements: number;
  previews: IdeReplacePreviewItem[];
}

export interface IdeQuickRunSpec {
  runner?: string;
  command?: string | null;
  args?: string[];
  cwd?: string | null;
  target_path?: string | null;
  test_filter?: string | null;
  test_name?: string | null;
  program?: string | null;
  debug_program?: string | null;
  debug_test_name?: string | null;
  language?: string | null;
}

export interface IdeGutterItem {
  line: number;
  kind: 'test' | 'suite' | 'main' | 'bench';
  label: string;
  language: string;
  run_spec: IdeQuickRunSpec;
}

export interface IdeSymbolNode {
  name: string;
  kind: string;
  detail?: string | null;
  line: number;
  end_line: number;
  children?: IdeSymbolNode[];
}

export interface IdeDiagnostic {
  path: string;
  line: number;
  column?: number;
  severity: 'error' | 'warning' | 'info' | 'hint';
  message: string;
  source?: string | null;
  code?: string | null;
}

export interface IdeReferenceLocation {
  path: string;
  line: number;
  column: number;
  preview: string;
}

export interface IdeRenameEdit {
  path: string;
  occurrences: number;
  preview_lines: { line: number; before: string; after: string }[];
}

export interface IdeCodeAction {
  id: string;
  title: string;
  kind?: string;
  edit_preview?: string;
  replacement_content?: string;
}

export interface IdeInstalledSdk {
  id: string;
  version: string;
  path: string;
  source: 'system' | 'otto' | string;
  active: boolean;
}

export interface IdeToolStatus {
  name: string;
  available: boolean;
  path?: string | null;
}

export interface IdeDownloadableSdk {
  version: string;
  label: string;
  lts: boolean;
  os: string;
  arch: string;
  url: string;
  size_hint?: string | null;
}

export interface IdeToolchainEcosystem {
  ecosystem: 'java' | 'go' | 'rust' | 'node' | 'python' | string;
  display_name: string;
  installed: IdeInstalledSdk[];
  lsp_status: IdeToolStatus;
  dap_status: IdeToolStatus;
  downloadable: IdeDownloadableSdk[];
}

export interface IdeInstallJob {
  id: string;
  ecosystem: string;
  component: string;
  version?: string | null;
  target_os: string;
  target_arch: string;
  status: 'running' | 'done' | 'error';
  progress_pct: number;
  log_lines: string[];
  installed_path?: string | null;
  error?: string | null;
}

export interface IdeEnvVar {
  key: string;
  value?: string;
  secret: boolean;
  has_secret?: boolean;
}

export interface IdeProjectEnvironment {
  java_home?: string | null;
  go_root?: string | null;
  rust_toolchain?: string | null;
  node_home?: string | null;
  python_interpreter?: string | null;
  dotenv_path?: string | null;
  extra_path?: string | null;
  env_vars: IdeEnvVar[];
}

export interface IdePythonVenv {
  path: string;
  interpreter_path?: string;
  version: string;
  kind: 'venv' | 'conda' | 'poetry' | 'uv' | string;
}

export interface IdeRunConfig {
  id: string;
  name: string;
  runner: string;
  command?: string | null;
  args: string[];
  cwd?: string | null;
  env_overrides: Record<string, string>;
  before_launch?: string | null;
  target_path?: string | null;
  test_filter?: string | null;
}

export interface IdeTestResult {
  name: string;
  suite?: string | null;
  status: 'passed' | 'failed' | 'skipped' | 'running';
  duration_ms?: number | null;
  file?: string | null;
  line?: number | null;
  message?: string | null;
}

export interface IdeRunState {
  id: string;
  title: string;
  runner: string;
  status: 'running' | 'passed' | 'failed' | 'stopped';
  exit_code?: number | null;
  started_at?: string;
  duration_ms?: number | null;
  output: string[];
  tests: IdeTestResult[];
}

export interface IdeBreakpoint {
  path: string;
  line: number;
  enabled: boolean;
  condition?: string | null;
  hit_condition?: string | null;
  log_message?: string | null;
}

export interface IdeThread {
  id: number;
  name: string;
}

export interface IdeStackFrame {
  id: number;
  name: string;
  path?: string | null;
  line: number;
  column?: number;
}

export interface IdeVariable {
  name: string;
  value: string;
  var_type?: string | null;
  variables_reference: number;
  indexed_variables?: number | null;
}

export interface IdeScope {
  name: string;
  variables_reference: number;
  variables: IdeVariable[];
  expensive?: boolean | null;
}

export interface IdeInlineValue {
  line: number;
  name: string;
  value: string;
}

export interface IdeDebugOutputEntry {
  category: 'stdout' | 'stderr' | 'console' | string;
  text: string;
}

export interface IdeDebugSessionSnapshot {
  id: string;
  title: string;
  language: string;
  adapter: string;
  state: 'starting' | 'running' | 'paused' | 'terminated' | 'error';
  stopped_reason?: string | null;
  active_thread_id?: number | null;
  active_frame_id?: number | null;
  paused_file?: string | null;
  paused_line?: number | null;
  threads: IdeThread[];
  stack_frames: IdeStackFrame[];
  scopes: IdeScope[];
  inline_values: IdeInlineValue[];
  breakpoints: IdeBreakpoint[];
  output: IdeDebugOutputEntry[];
  error?: string | null;
}

export interface IdeWatch {
  id: string;
  expression: string;
  value: string;
  var_type?: string | null;
  error?: string | null;
}

export interface IdeReplEntry {
  id: string;
  expression: string;
  result: string;
  var_type?: string | null;
  isError?: boolean;
}

export interface IdeCollabPeer {
  peer_id: string;
  display_name: string;
  color: string;
  role: IdeCollabRole;
  active_path?: string | null;
  active_line?: number | null;
  is_host?: boolean;
}

export interface IdeCollabRoom {
  id: string;
  title: string;
  target: IdeTargetSpec;
  default_role: IdeCollabRole;
  peers: IdeCollabPeer[];
  created_at?: string;
}

export interface IdePeerCursor {
  peer_id: string;
  name: string;
  color: string;
  path: string;
  active_line: number;
  selections: { anchor: number; head: number }[];
}

export interface IdePluginInfo {
  slug: string;
  name: string;
  version: string;
  description?: string | null;
  enabled: boolean;
  source: string;
  languages?: string[];
  capabilities?: string[];
}

export interface IdePluginCatalogEntry {
  slug: string;
  name: string;
  version: string;
  description: string;
  languages: string[];
  source: string;
  installed?: boolean;
}

export interface IdeEditorHandle {
  applyPrefixSuffix: (prefix: string, suffix: string) => void;
  splitSelectionToLines: () => void;
  goToLine: (line: number, col?: number) => void;
  focus: () => void;
}

export function ideBasename(p: string): string {
  const idx = Math.max(p.lastIndexOf('/'), p.lastIndexOf('\\'));
  return idx >= 0 ? p.slice(idx + 1) : p;
}

export function detectIdeLanguage(p: string): string {
  const ext = ideBasename(p).split('.').pop()?.toLowerCase() ?? '';
  const map: Record<string, string> = {
    go: 'go', java: 'java', rs: 'rust', py: 'python', ts: 'typescript', tsx: 'tsx',
    js: 'javascript', jsx: 'jsx', json: 'json', yaml: 'yaml', yml: 'yaml', md: 'markdown',
    sql: 'sql', sh: 'shell', bash: 'shell', zsh: 'shell', html: 'html', css: 'css', svelte: 'html',
  };
  return map[ext] ?? 'txt';
}

export function defaultRunnerForLanguage(lang: string): string {
  const map: Record<string, string> = {
    go: 'go_run', rust: 'cargo_run', java: 'java_main', python: 'python',
    javascript: 'node', typescript: 'npm', tsx: 'npm', jsx: 'node',
  };
  return map[lang] ?? 'custom';
}

export async function streamIdeSse(
  path: string,
  headers: Record<string, string>,
  signal: AbortSignal,
  onData: (raw: string) => void,
): Promise<void> {
  const token = getToken();
  const reqHeaders: Record<string, string> = { Accept: 'text/event-stream', ...headers };
  if (token && !reqHeaders['Authorization']) reqHeaders['Authorization'] = `Bearer ${token}`;
  const resp = await laneFetch('long', path, { method: 'GET', headers: reqHeaders, signal });
  if (!resp.ok || !resp.body) return;
  const reader = resp.body.getReader();
  const decoder = new TextDecoder();
  let buf = '';
  while (!signal.aborted) {
    const { done, value } = await reader.read();
    if (done) break;
    buf += decoder.decode(value, { stream: true });
    let sep = buf.indexOf('\n\n');
    while (sep >= 0) {
      const chunk = buf.slice(0, sep);
      buf = buf.slice(sep + 2);
      const lines = chunk.split('\n').filter((l) => l.startsWith('data:')).map((l) => l.slice(5).trimStart());
      if (lines.length > 0) onData(lines.join('\n'));
      sep = buf.indexOf('\n\n');
    }
  }
}

export async function collabGuestPost<T>(path: string, guestToken: string | null, body: unknown): Promise<T> {
  const headers: Record<string, string> = { 'Content-Type': 'application/json' };
  const bearer = getToken() || guestToken;
  if (bearer) headers['Authorization'] = `Bearer ${bearer}`;
  const resp = await laneFetch('int', path, { method: 'POST', headers, body: JSON.stringify(body) });
  if (!resp.ok) throw new Error(await resp.text());
  return (await resp.json()) as T;
}

const ENTRY_FILE_NAMES = [
  'readme.md', 'main.go', 'main.rs', 'main.py', 'index.ts',
  'app.svelte', 'go.mod', 'cargo.toml', 'package.json',
];

export function pickInitialEntryFile(entries: IdeFsEntry[]): IdeFsEntry | null {
  const files = entries.filter((e) => e.kind === 'file' && !e.name.startsWith('.'));
  for (let i = 0; i < ENTRY_FILE_NAMES.length; i++) {
    const hit = files.find((f) => f.name.toLowerCase() === ENTRY_FILE_NAMES[i]);
    if (hit) return hit;
  }
  return files[0] ?? null;
}

export async function autoOpenInitialEntry(store: {
  collabIsGuest: boolean;
  tabs: IdeOpenTab[];
  hasClosedAllTabs: boolean;
  treeRootEntries: IdeFsEntry[];
  childrenByDir: Record<string, IdeFsEntry[]>;
  toggleDir(dirPath: string): Promise<void>;
  openFile(rawPath: string, opts?: { preview?: boolean }): Promise<void>;
}): Promise<void> {
  if (store.collabIsGuest || store.tabs.length > 0 || store.hasClosedAllTabs || store.treeRootEntries.length === 0) return;
  if (typeof window !== 'undefined' && /[?&](file|collab)=/.test(window.location.hash)) return;
  const rootFile = pickInitialEntryFile(store.treeRootEntries);
  if (rootFile) {
    await store.openFile(rootFile.path, { preview: false });
    return;
  }
  const firstDir = store.treeRootEntries.find((e) => e.kind === 'dir' && !e.name.startsWith('.'));
  if (!firstDir) return;
  await store.toggleDir(firstDir.path);
  if (store.tabs.length > 0 || store.hasClosedAllTabs) return;
  const childFile = pickInitialEntryFile(store.childrenByDir[firstDir.path] ?? []);
  if (childFile) await store.openFile(childFile.path, { preview: false });
}
