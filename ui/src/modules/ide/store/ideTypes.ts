// Canonical TypeScript types & pure helpers for the Otto Full IDE module.
// Re-exported from `../ideStore.svelte.ts` so all IDE components import from one place.

import { getToken, laneFetch } from '../../../lib/api/client';
import type {
  IdeBridgedServiceSummary,
  IdeDebugLaunch,
  IdeEnvironmentResp,
  IdeHostMapping,
  IdeNetBridgeStatusResp,
  IdeNetBridgeTestReq,
  IdeNetworkBridgeConfig,
  IdeNetworkBridgeHostMapping,
  IdeShellEnvResp,
  IdeUpdateEnvironmentReq,
} from '../../../lib/api/types/ide';
import type {
  IdePluginCatalogEntry,
  IdePluginContributions,
  IdePluginDapAdapter,
  IdePluginGutterDetector,
  IdePluginInfo,
  IdePluginKeymap,
  IdePluginLanguage,
  IdePluginLspServer,
  IdeQuickFix,
} from '../../../lib/api/types/ide-intelligence';

export type {
  IdeBridgedServiceSummary,
  IdeEnvironmentResp,
  IdeHostMapping,
  IdeNetBridgeStatusResp,
  IdeNetBridgeTestReq,
  IdeNetworkBridgeConfig,
  IdeNetworkBridgeHostMapping,
  IdePluginCatalogEntry,
  IdePluginContributions,
  IdePluginDapAdapter,
  IdePluginGutterDetector,
  IdePluginInfo,
  IdePluginKeymap,
  IdePluginLanguage,
  IdePluginLspServer,
  IdeQuickFix,
  IdeShellEnvResp,
  IdeUpdateEnvironmentReq,
};
export type * from '../../../lib/api/types/ide-db-watch';

export type IdeTargetKind = 'local' | 'ssh';
export type IdeKeymapPreset = 'jetbrains' | 'standard';
export type IdeBottomTab = 'debug' | 'run' | 'git' | 'problems' | 'usages' | 'terminal' | 'dbwatch';
export type BottomTab = IdeBottomTab;
export type IdeSearchMode = 'files' | 'text' | 'symbols' | 'all';
export type IdeCollabRole = 'viewer' | 'editor';

export interface IdeTargetSpec {
  kind: IdeTargetKind; workspace_id?: string | null; root: string;
  connection_id?: string | null; connection_name?: string | null;
  additional_roots?: string[];
  project_folders?: string[];
}

export interface IdeNamedProject {
  id: string;
  name: string;
  primary_root: string;
  folders: string[];
  pinned: boolean;
  updated_at?: number | string;
}

export interface IdeProjectRoot {
  path: string;
  name: string;
  isPrimary: boolean;
}

export interface IdeNavLocation {
  path: string;
  line: number;
  col: number;
}

export interface IdeFsEntry {
  name: string; path: string; kind: 'dir' | 'file' | 'symlink';
  size?: number; mtime?: number; git_status?: 'M' | 'A' | 'D' | '?' | string | null; language?: string | null;
}

export interface IdeFsTreeResp {
  root: string; path: string; parent?: string | null;
  target_os?: string; target_arch?: string; entries: IdeFsEntry[];
}

export interface IdeFileReadResp {
  path: string; content: string; language?: string | null; size: number;
  sha256: string; binary?: boolean; read_only: boolean; truncated: boolean; line_count: number;
}

export interface IdeOpenTab {
  path: string; name: string; content: string; savedContent: string; dirty?: boolean;
  eol?: '\r\n' | '\n';
  /** Per-line terminators of the file as last read/written, kept only when it MIXES
   *  styles (CRLF + LF, lone CR): saving then keeps each untouched line's ending. */
  diskEols?: string[];
  sha256?: string; language: string; readOnly: boolean; truncated: boolean;
  loading: boolean; error: string | null; preview: boolean;
  pinned?: boolean; external?: boolean;
  lastAccessedAt?: number;
}

/** Dominant line ending (mirrors Rust `text_pos::detect_eol`): one stray CRLF must not turn an LF file CRLF on save. */
export function detectTextEol(text: string): '\r\n' | '\n' {
  const crlf = text.split('\r\n').length - 1;
  const lf = text.split('\n').length - 1;
  return crlf > 0 && crlf >= lf - crlf ? '\r\n' : '\n';
}

/** Identity of a target for stale-response checks: a load that started under one key and
 *  finishes under another (the project switched meanwhile) must drop its result. */
export function ideTargetKey(t: Pick<IdeTargetSpec, 'kind' | 'root' | 'connection_id'>): string {
  return `${t.kind}:${t.connection_id ?? ''}:${t.root}`;
}

export function normalizeEditorLf(text: string): string {
  return text.indexOf('\r') >= 0 ? text.replace(/\r\n?/g, '\n') : text;
}

/** The line terminators of `raw`, one per line (the last line's is ''), or undefined when
 *  the file uses one style throughout (the common case: `eol` alone describes it). */
export function diskEolsOf(raw: string): string[] | undefined {
  if (raw.indexOf('\r') < 0) return undefined;
  const eols: string[] = [];
  const seen = new Set<string>();
  const re = /\r\n|\n|\r/g;
  for (let m = re.exec(raw); m; m = re.exec(raw)) { eols.push(m[0]); seen.add(m[0]); }
  if (seen.size < 2) return undefined;
  eols.push('');
  return eols;
}

/**
 * Editor text (LF) → the bytes to write. One-style files get that style. A file with mixed
 * endings keeps the ending of every line the edit did not touch, so a one-line edit stays
 * a one-line diff: lines in the unchanged prefix/suffix (against `savedContent`, which
 * `diskEols` describes) keep theirs, a same-size changed block keeps them line by line, and
 * only genuinely inserted lines take the dominant style.
 */
export function serializeTabDiskContent(
  tab: { eol?: IdeOpenTab['eol']; diskEols?: string[]; savedContent?: string },
  editorLfContent: string,
): string {
  const lf = normalizeEditorLf(editorLfContent);
  const def = tab.eol === '\r\n' ? '\r\n' : '\n';
  const eols = tab.diskEols;
  const oldLines = eols && typeof tab.savedContent === 'string' ? tab.savedContent.split('\n') : null;
  if (!eols || !oldLines || oldLines.length !== eols.length) return def === '\r\n' ? lf.replace(/\n/g, '\r\n') : lf;
  const newLines = lf.split('\n');
  let p = 0;
  while (p < oldLines.length && p < newLines.length && oldLines[p] === newLines[p]) p++;
  let so = oldLines.length;
  let sn = newLines.length;
  while (so > p && sn > p && oldLines[so - 1] === newLines[sn - 1]) { so--; sn--; }
  const sameSizeBlock = sn - p === so - p;
  let out = '';
  for (let i = 0; i < newLines.length; i++) {
    out += newLines[i];
    if (i === newLines.length - 1) break;
    const oldIdx = i < p ? i : i >= sn ? i - sn + so : sameSizeBlock ? i : -1;
    out += (oldIdx >= 0 ? eols[oldIdx] : '') || def;
  }
  return out;
}

/** Hex SHA-256 of `text` as UTF-8 (what `PUT /ide/fs/file` compares `expected_sha256`
 *  against), or undefined where WebCrypto is missing (a plain-http remote origin). */
export async function sha256HexOfText(text: string): Promise<string | undefined> {
  const subtle = globalThis.crypto?.subtle;
  const Enc = globalThis.TextEncoder;
  if (!subtle || !Enc) return undefined;
  try {
    const digest = await subtle.digest('SHA-256', new Enc().encode(text));
    return Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, '0')).join('');
  } catch {
    return undefined;
  }
}

export interface IdeSearchItem {
  kind: 'file' | 'text' | 'symbol'; path: string; line?: number; column?: number;
  preview: string; symbol_kind?: string | null; container?: string | null;
}

export interface IdeReplacePreviewItem { path: string; count: number; sample_before: string; sample_after: string }
export interface IdeReplaceResp { files_changed: number; replacements: number; previews: IdeReplacePreviewItem[] }

export interface IdeQuickRunSpec {
  runner?: string; command?: string | null; args?: string[]; cwd?: string | null;
  target_path?: string | null; test_filter?: string | null; test_name?: string | null;
  program?: string | null; debug_program?: string | null; debug_test_name?: string | null; language?: string | null;
}

export interface IdeGutterItem {
  line: number; kind: 'test' | 'suite' | 'main' | 'bench';
  label: string; language: string; run_spec: IdeQuickRunSpec;
}

export interface IdeSymbolNode {
  name: string; kind: string; detail?: string | null;
  line: number; end_line: number; children?: IdeSymbolNode[];
}

export interface IdeTextEdit {
  path: string;
  start_line: number;
  start_col: number;
  end_line: number;
  end_col: number;
  new_text: string;
}

export interface IdeCodeAction {
  id: string;
  title: string;
  kind?: string;
  is_preferred?: boolean;
  edits?: IdeTextEdit[];
  command?: string | null;
  edit_preview?: string | null;
  replacement_content?: string | null;
}

export interface IdeDiagnostic {
  path: string; line: number; col?: number; column?: number; end_line?: number; end_col?: number; end_column?: number;
  severity: 'error' | 'warning' | 'info' | 'hint'; message: string; source?: string | null; code?: string | null;
  quick_fixes?: (IdeQuickFix | IdeCodeAction)[];
}

export interface IdeReferenceLocation { path: string; line: number; column: number; preview: string }
export interface IdeRenameEdit { path: string; occurrences: number; preview_lines: { line: number; before: string; after: string }[] }
export interface IdeInstalledSdk {
  id: string; version: string; path: string; source: 'system' | 'otto' | 'local' | string;
  active: boolean; valid?: boolean; broken?: boolean; label?: string;
}
export interface IdeDetectedSdk {
  ecosystem: 'java' | 'go' | 'rust' | 'node' | 'python' | 'shell' | string;
  name: string; label?: string; version: string; path: string; home_path?: string;
  bin_path?: string | null; source: string; arch?: string | null; valid?: boolean; broken?: boolean;
}
export interface IdeToolStatus { name: string; available: boolean; installed?: boolean; path?: string | null }
export interface IdeDownloadableSdk { version: string; label: string; lts: boolean; os: string; arch: string; url: string; size_hint?: string | null; sha256?: string | null }

export interface IdeToolchainEcosystem {
  ecosystem: 'java' | 'go' | 'rust' | 'node' | 'python' | string; display_name: string;
  installed: IdeInstalledSdk[]; detected?: IdeDetectedSdk[];
  lsp_status: IdeToolStatus; dap_status: IdeToolStatus;
  lsp?: IdeToolStatus | null; dap?: IdeToolStatus | null;
  downloadable: IdeDownloadableSdk[];
}

export interface IdeInstallJob {
  id: string; ecosystem: string; component: string; version?: string | null;
  target_os: string; target_arch: string; status: 'running' | 'done' | 'error';
  progress_pct: number; log_lines: string[]; installed_path?: string | null; error?: string | null;
}

export interface IdeEnvVar { key: string; value?: string; secret: boolean; has_secret?: boolean }

export interface IdeProjectEnvironment {
  java_home?: string | null; go_root?: string | null; rust_toolchain?: string | null;
  node_home?: string | null; python_interpreter?: string | null;
  dotenv_path?: string | null; extra_path?: string | null;
  inherit_shell_env?: boolean; additional_roots?: string[];
  module_sdks?: Record<string, string>;
  go_build_tags?: string[];
  module_go_build_tags?: Record<string, string[]>;
  network_bridge?: IdeNetworkBridgeConfig | null;
  env_vars: IdeEnvVar[];
}

export interface IdePythonVenv { path: string; interpreter_path?: string; name?: string; version: string; kind: 'venv' | 'conda' | 'poetry' | 'uv' | string }

export interface IdeBuildToolProfile {
  id: string;
  name?: string;
  kind?: 'maven' | 'gradle' | 'spring' | string;
  source?: string | null;
  source_project?: string | null;
  active_by_default?: boolean;
}

export interface IdeBuildToolRepo {
  id: string;
  url: string;
}

export interface IdeBuildToolModule {
  name: string;
  group_id?: string | null;
  artifact_id?: string | null;
  version?: string | null;
  project_coord?: string | null;
  tool: 'maven' | 'gradle';
  kind?: 'maven' | 'gradle';
  root_dir: string;
  root?: string;
  location_display?: string | null;
  manifest_path: string;
  lifecycle?: string[];
  plugins?: { name: string; goals?: string[] }[];
  dependencies?: string[];
  repositories?: IdeBuildToolRepo[];
  modules?: IdeBuildToolModule[];
  children?: IdeBuildToolModule[];
}

export interface IdeRunConfig {
  id: string; name: string; runner: string; command?: string | null; args: string[];
  build_flags?: string[]; cwd?: string | null; env_overrides: Record<string, string>;
  env_file?: string | null; inherit_shell_env?: boolean; sdk_path?: string | null;
  toolchain_id?: string | null; compound_configs?: string[]; auto_detected?: boolean;
  before_launch?: string | null; target_path?: string | null; test_filter?: string | null;
  maven_profiles?: string[]; spring_profiles?: string[]; skip_tests?: boolean; offline?: boolean; coverage?: boolean;
}

export interface IdeTestResult {
  name: string; suite?: string | null; status: 'passed' | 'failed' | 'skipped' | 'running';
  duration_ms?: number | null; file?: string | null; line?: number | null; message?: string | null;
}

export interface IdeRunState {
  id: string; title: string; runner: string; status: 'running' | 'passed' | 'failed' | 'stopped';
  exit_code?: number | null; started_at?: string; duration_ms?: number | null; output_seq?: number; output: string[]; tests: IdeTestResult[];
  coverage_pct?: number | null; coverage_summary?: string | null;
}

export interface IdeBreakpoint {
  path: string; line: number; enabled: boolean;
  condition?: string | null; hit_condition?: string | null; log_message?: string | null;
  verified?: boolean; message?: string | null;
}

export interface IdeThread { id: number; name: string }
export interface IdeStackFrame { id: number; name: string; path?: string | null; line: number; column?: number }
export interface IdeVariable { name: string; value: string; var_type?: string | null; variables_reference: number; indexed_variables?: number | null }
export interface IdeScope { name: string; variables_reference: number; variables: IdeVariable[]; expensive?: boolean | null }
export interface IdeInlineValue { line: number; name: string; value: string }
export interface IdeDebugOutputEntry { category: 'stdout' | 'stderr' | 'console' | string; text: string; seq?: number }

export interface IdeDebugSessionSnapshot {
  id: string; revision?: number; title: string; language: string; adapter: string;
  config_id?: string | null; launch?: IdeDebugLaunch | null;
  state: 'starting' | 'running' | 'paused' | 'terminated' | 'error'; stopped_reason?: string | null;
  active_thread_id?: number | null; active_frame_id?: number | null;
  paused_file?: string | null; paused_line?: number | null;
  selected_frame_file?: string | null; selected_frame_line?: number | null;
  stopped_file?: string | null; stopped_line?: number | null;
  threads: IdeThread[]; stack_frames: IdeStackFrame[]; scopes: IdeScope[];
  inline_values: IdeInlineValue[]; breakpoints: IdeBreakpoint[]; output: IdeDebugOutputEntry[]; error?: string | null;
  error_kind?: 'adapter_missing' | 'toolchain_missing' | 'build_failed' | null; error_ecosystem?: string | null;
  target_root?: string | null;
}

export interface IdeWatch { id: string; expression: string; value: string; var_type?: string | null; error?: string | null }
/** `afterSeq`: last debug-output sequence number when it was evaluated (its console position);
 *  `variables_reference` > 0: the result has children (expandable like a variable). */
export interface IdeReplEntry { id: string; expression: string; result: string; var_type?: string | null; isError?: boolean; afterSeq?: number; variables_reference?: number }

export interface IdeCollabPeer {
  peer_id: string; display_name: string; color: string; role: IdeCollabRole;
  active_path?: string | null; active_line?: number | null; is_host?: boolean;
}

export interface IdeCollabRoom {
  id: string; title: string; target: IdeTargetSpec;
  default_role: IdeCollabRole; peers: IdeCollabPeer[]; created_at?: string;
}

export interface IdePeerCursor {
  peer_id: string; name: string; color: string; path: string;
  active_line: number; selections: { anchor: number; head: number }[];
}

export interface IdeEditorSelectionRange {
  from: number;
  to: number;
  startLine: number;
  endLine: number;
  text: string;
}

export interface IdeEditorHandle {
  applyPrefixSuffix: (prefix: string, suffix: string) => void;
  splitSelectionToLines: () => void;
  goToLine: (line: number, col?: number) => void;
  focus: () => void;
  getSelectionRange?: () => IdeEditorSelectionRange | null;
  replaceBufferContent?: (nextContent: string, range?: { from: number; to: number }) => boolean;
  applyAgentEdit?: (nextContent: string, range?: { from: number; to: number }) => boolean;
}

export function isReadOnlySdkOrCachePath(p: string): boolean {
  if (!p) return false;
  if (/^(?:jdt|jar):/i.test(p)) return true;
  if (p.includes('/pkg/mod/') || p.includes('/go/pkg/mod/')) return true;
  if (/\/toolchain@v[\w.-]+\/src\//.test(p) || /\/go(?:@[\w.-]+)?\/(?:[\w.-]+\/)?libexec\/src\//.test(p) || p.startsWith('/usr/local/go/src/')) return true;
  if (p.includes('/.m2/repository/') || p.includes('/.gradle/caches/')) return true;
  if (p.includes('/.cargo/registry/') || p.includes('/rustlib/src/')) return true;
  if (p.includes('/node_modules/') || p.includes('/site-packages/') || p.includes('/dist-packages/')) return true;
  if (p.includes('/.otto/toolchains/')) return true;
  return false;
}

export function ideBasename(p: string): string {
  const clean = /^(?:jdt|jar):/i.test(p) ? p.split('?')[0] : p;
  const bangIdx = clean.lastIndexOf('!/');
  const tail = bangIdx >= 0 ? clean.slice(bangIdx + 2) : clean;
  const trimmed = tail.replace(/\/+$/, '') || tail;
  const idx = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));
  return idx >= 0 ? trimmed.slice(idx + 1) : trimmed;
}

export function detectIdeLanguage(p: string): string {
  if (/^jdt:/i.test(p)) return 'java';
  const base = ideBasename(p);
  const lower = base.toLowerCase();
  if (lower === 'bashrc' || lower === '.bashrc' || lower === '.zshrc' || lower === '.zshenv') return 'shell';
  const ext = lower.includes('.') ? (lower.split('.').pop() ?? '') : '';
  const map: Record<string, string> = {
    go: 'go', java: 'java', class: 'java', rs: 'rust', py: 'python', pyi: 'python',
    ts: 'typescript', mts: 'typescript', cts: 'typescript', tsx: 'tsx',
    js: 'javascript', mjs: 'javascript', cjs: 'javascript', jsx: 'jsx',
    json: 'json', jsonc: 'json', jsonl: 'jsonl', csv: 'csv', tsv: 'tsv',
    yaml: 'yaml', yml: 'yaml', md: 'markdown', mdx: 'markdown', sql: 'sql',
    sh: 'shell', bash: 'shell', zsh: 'shell', html: 'html', css: 'css', scss: 'css', svelte: 'html',
  };
  return map[ext] ?? 'txt';
}

export function defaultRunnerForLanguage(lang: string): string {
  const map: Record<string, string> = {
    go: 'go_run', rust: 'cargo_run', java: 'java_main', python: 'python',
    javascript: 'node', typescript: 'npm', tsx: 'npm', jsx: 'node', shell: 'shell',
  };
  return map[lang] ?? 'custom';
}

export function buildNewFileTemplate(fullPath: string, rootPath: string, siblingEntries: IdeFsEntry[] = []): string {
  const name = ideBasename(fullPath);
  const lower = name.toLowerCase();
  const slashIdx = fullPath.lastIndexOf('/');
  const parentDir = slashIdx > 0 ? fullPath.slice(0, slashIdx) : rootPath;
  const cleanRoot = rootPath.replace(/\/+$/, '');
  if (lower.endsWith('.go')) {
    const isMainDir = parentDir === cleanRoot && siblingEntries.some((e) => e.name === 'main.go');
    const isCmdDir = /\/cmd\/[^/]+$/.test(parentDir) || lower === 'main.go';
    const rawPkg = (isMainDir || isCmdDir || parentDir === cleanRoot)
      ? 'main'
      : ideBasename(parentDir).toLowerCase().replace(/[^a-z0-9_]/g, '_').replace(/^[0-9]+/, 'pkg_$&') || 'main';
    if (lower.endsWith('_test.go')) return `package ${rawPkg}\n\nimport "testing"\n\n`;
    if (lower === 'main.go') return `package main\n\nfunc main() {\n}\n`;
    return `package ${rawPkg}\n\n`;
  }
  if (lower.endsWith('.java')) {
    const stem = name.slice(0, -5).replace(/[^A-Za-z0-9_$]/g, '') || 'Main';
    const m = parentDir.match(/\/src\/(?:main|test)\/java\/(.+)$/);
    const pkgLine = m?.[1] ? `package ${m[1].replace(/\//g, '.')};\n\n` : '';
    return `${pkgLine}public class ${stem} {\n}\n`;
  }
  if (lower.endsWith('.sh') || lower.endsWith('.bash')) return '#!/usr/bin/env bash\nset -euo pipefail\n\n';
  if (lower.endsWith('.zsh')) return '#!/usr/bin/env zsh\nset -euo pipefail\n\n';
  if (lower.endsWith('.json')) return '{\n  \n}\n';
  return '';
}

export async function streamIdeSse(path: string, headers: Record<string, string>, signal: AbortSignal, onData: (raw: string) => void, onConnected?: () => void): Promise<void> {
  const isCollabGuest = path.startsWith('/ide/collab/guest/') || Boolean(headers['X-Ide-Collab-Token']);
  const token = isCollabGuest ? null : getToken();
  const reqHeaders: Record<string, string> = { Accept: 'text/event-stream', ...headers };
  if (token && !reqHeaders['Authorization']) reqHeaders['Authorization'] = `Bearer ${token}`;
  const resp = await laneFetch('long', path, { method: 'GET', headers: reqHeaders, signal });
  if (!resp.ok || !resp.body) return;
  onConnected?.();
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

/** A refused guest call: the HTTP status plus the daemon's own reason (its JSON `error`). */
export class IdeCollabHttpError extends Error {
  readonly status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

export function collabErrorText(raw: string): string {
  try {
    const parsed = JSON.parse(raw) as { error?: unknown; message?: unknown };
    const msg = typeof parsed.error === 'string' ? parsed.error : typeof parsed.message === 'string' ? parsed.message : '';
    return msg || raw;
  } catch {
    return raw;
  }
}

export async function collabGuestPost<T>(path: string, guestToken: string | null, body: unknown): Promise<T> {
  const headers: Record<string, string> = { 'Content-Type': 'application/json' };
  if (guestToken) headers['X-Ide-Collab-Token'] = guestToken;
  const resp = await laneFetch('int', path, { method: 'POST', headers, body: JSON.stringify(body) });
  if (!resp.ok) throw new IdeCollabHttpError(resp.status, collabErrorText(await resp.text()));
  return (await resp.json()) as T;
}

const ENTRY_FILE_NAMES = ['readme.md', 'main.go', 'main.rs', 'main.py', 'index.ts', 'app.svelte', 'go.mod', 'cargo.toml', 'package.json'];

export function pickInitialEntryFile(entries: IdeFsEntry[]): IdeFsEntry | null {
  const files = entries.filter((e) => e.kind === 'file' && !e.name.startsWith('.'));
  for (let i = 0; i < ENTRY_FILE_NAMES.length; i++) {
    const hit = files.find((f) => f.name.toLowerCase() === ENTRY_FILE_NAMES[i]);
    if (hit) return hit;
  }
  return files[0] ?? null;
}

export async function autoOpenInitialEntry(store: {
  collabIsGuest: boolean; tabs: IdeOpenTab[]; hasClosedAllTabs: boolean;
  treeRootEntries: IdeFsEntry[]; childrenByDir: Record<string, IdeFsEntry[]>;
  toggleDir(dirPath: string): Promise<void>; openFile(rawPath: string, opts?: { preview?: boolean }): Promise<void>;
}): Promise<void> {
  if (store.collabIsGuest || store.tabs.length > 0 || store.hasClosedAllTabs || store.treeRootEntries.length === 0) return;
  if (typeof window !== 'undefined' && /[?&](file|collab)=/.test(window.location.hash)) return;
  const rootFile = pickInitialEntryFile(store.treeRootEntries);
  if (rootFile) { await store.openFile(rootFile.path, { preview: false }); return; }
  const firstDir = store.treeRootEntries.find((e) => e.kind === 'dir' && !e.name.startsWith('.'));
  if (!firstDir) return;
  await store.toggleDir(firstDir.path);
  if (store.tabs.length > 0 || store.hasClosedAllTabs) return;
  const childFile = pickInitialEntryFile(store.childrenByDir[firstDir.path] ?? []);
  if (childFile) await store.openFile(childFile.path, { preview: false });
}
