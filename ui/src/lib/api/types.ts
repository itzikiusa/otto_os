// TS mirror of crates/otto-core (domain.rs + api.rs). Keep in lockstep — the
// Rust side is the source of truth (docs/contracts/api.md).
//
// Barrel: the contract mirror is split by area under ./types/. Import from
// `lib/api/types` as before; every name resolves through these re-exports.

export * from './types/activity';
export * from './types/agent-runtime';
export * from './types/api-client';
export * from './types/auth';
export * from './types/automation';
export * from './types/aws';
export * from './types/brokers';
export * from './types/browser';
export * from './types/connections';
export * from './types/core';
export * from './types/database';
export * from './types/design';
export * from './types/design-assist';
export * from './types/events';
export * from './types/git';
export * from './types/git-merge';
export * from './types/goal-loops';
export * from './types/insights-share';
export * from './types/integrations';
export * from './types/issue-update';
export * from './types/k8s';
export * from './types/k8s-monitor';
export * from './types/mcp';
export * from './types/memory';
export * from './types/notifications';
export * from './types/orchestrator';
export * from './types/personal-agents';
export type { ProductStopTurnReq, ProductStopTurnResp, ProductTurnIdentity } from './types/product';
export * from './types/review';
export * from './types/sessions';
export * from './types/skills-context';
export * from './types/skills-eval';
export * from './types/telemetry';
export * from './types/transcript';
export * from './types/ui-control';
export * from './types/vault';
export * from './types/workbench';
export * from './types/workflows';
export * from './types/ws-term';

export type * from './room-types';
export type * from './room-recap-types';
export type * from './db-multirun-types';
export type * from './maintenance-types';
// Product publication and paged transcript contracts (full set: modules/product/types).
export type {
  ReviewedContent, PublishAsRfcReq, PublishAsStoryReq, ProductTranscriptSummary,
  ProductTranscriptPage, ProductTranscriptSearchPage,
} from './types/product';
export type { ProductPublicationPreview } from './types/product-studio';
export type {
  TailnetDevice, PhoneServing, PhoneAccessStatus, PairingCreated, PairingStatus, PairingView,
  PhoneDevice, PairClaimResult, PairPollResult,
} from './types/phone';

// ── Full IDE (`/api/v1/ide/*` & `/api/v1/ide/collab/guest/*`) ────────────────

export type IdeTargetKind = 'local' | 'ssh';

export interface IdeTargetSpec {
  kind: IdeTargetKind;
  workspace_id?: string | null;
  root: string;
  connection_id?: string | null;
  connection_name?: string | null;
}

export type IdeFsEntryKind = 'dir' | 'file' | 'symlink';

export interface IdeFsEntry {
  name: string;
  path: string;
  kind: IdeFsEntryKind;
  size?: number;
  mtime?: number | string | null;
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

export interface IdeFileWriteReq {
  target: IdeTargetSpec;
  path: string;
  content: string;
  expected_sha256?: string | null;
  create_dirs?: boolean;
}

export interface IdeFileWriteResp {
  path: string;
  size: number;
  sha256: string;
}

export type IdeFsMutateOp = 'mkdir' | 'create_file' | 'rename' | 'delete' | 'duplicate';

export interface IdeFsMutateReq {
  target: IdeTargetSpec;
  op: IdeFsMutateOp;
  path: string;
  dest_path?: string | null;
}

export interface IdeFsMutateResp {
  ok: boolean;
  path: string;
}

export type IdeSearchMode = 'files' | 'text' | 'symbols' | 'all';
export type IdeSearchItemKind = 'file' | 'text' | 'symbol';

export interface IdeSearchReq {
  target: IdeTargetSpec;
  query: string;
  mode: IdeSearchMode;
  case_sensitive?: boolean;
  regex?: boolean;
  file_glob?: string | null;
  limit?: number;
}

export interface IdeSearchItem {
  kind: IdeSearchItemKind;
  path: string;
  line?: number | null;
  column?: number | null;
  preview: string;
  symbol_kind?: string | null;
  container?: string | null;
}

export interface IdeSearchResp {
  items: IdeSearchItem[];
  truncated: boolean;
}

export interface IdeReplaceReq {
  target: IdeTargetSpec;
  query: string;
  replacement: string;
  case_sensitive?: boolean;
  regex?: boolean;
  paths?: string[];
  dry_run?: boolean;
}

export interface IdeReplacePreview {
  path: string;
  count: number;
  sample_before: string;
  sample_after: string;
}

export interface IdeReplaceResp {
  files_changed: number;
  replacements: number;
  previews: IdeReplacePreview[];
}

export type IdeGutterItemKind = 'test' | 'suite' | 'main' | 'bench';
export type IdeRunnerKind =
  | 'go_test'
  | 'go_run'
  | 'java_junit'
  | 'java_main'
  | 'maven'
  | 'gradle'
  | 'cargo_test'
  | 'cargo_run'
  | 'node'
  | 'npm'
  | 'vitest'
  | 'pytest'
  | 'python'
  | 'custom';

export interface IdeQuickRunSpec {
  runner?: IdeRunnerKind | string;
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
  kind: IdeGutterItemKind;
  label: string;
  language: string;
  run_spec: IdeQuickRunSpec;
}

export interface IdeGutterResp {
  items: IdeGutterItem[];
}

export interface IdeSymbolNode {
  name: string;
  kind: string;
  detail?: string | null;
  line: number;
  end_line: number;
  children?: IdeSymbolNode[];
}

export interface IdeSymbolsResp {
  symbols: IdeSymbolNode[];
}

export interface IdeReferenceLocation {
  path: string;
  line: number;
  column: number;
  preview: string;
}

export interface IdeReferencesResp {
  symbol: string;
  locations: IdeReferenceLocation[];
}

export interface IdeRenameLinePreview {
  line: number;
  before: string;
  after: string;
}

export interface IdeRenameFileEdit {
  path: string;
  occurrences: number;
  preview_lines: IdeRenameLinePreview[];
}

export interface IdeRenameResp {
  symbol: string;
  new_name: string;
  applied: boolean;
  edits: IdeRenameFileEdit[];
}

export interface IdeFormatResp {
  formatted: string;
  changed: boolean;
  formatter: string;
}

export interface IdeDiagnostic {
  path: string;
  line: number;
  column?: number;
  end_line?: number;
  end_column?: number;
  severity: 'error' | 'warning' | 'info' | 'hint';
  message: string;
  source?: string | null;
  code?: string | null;
}

export interface IdeCodeAction {
  id: string;
  title: string;
  kind?: string;
  edit_preview?: string | null;
  replacement_content?: string | null;
}

export interface IdeCodeActionsResp {
  actions: IdeCodeAction[];
}

// ── Toolchains & OS/Arch SDK Downloader ──────────────────────────────────────

export type IdeEcosystem = 'java' | 'go' | 'rust' | 'node' | 'python';
export type IdeInstallComponent = 'sdk' | 'lsp' | 'dap' | 'all';

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
  ecosystem: IdeEcosystem | string;
  display_name: string;
  installed: IdeInstalledSdk[];
  lsp_status: IdeToolStatus;
  dap_status: IdeToolStatus;
  downloadable: IdeDownloadableSdk[];
}

export type IdeInstallJobStatus = 'running' | 'done' | 'error';

export interface IdeInstallJob {
  id: string;
  ecosystem: IdeEcosystem | string;
  component: IdeInstallComponent | string;
  version?: string | null;
  target_os: string;
  target_arch: string;
  status: IdeInstallJobStatus;
  progress_pct: number;
  log_lines: string[];
  installed_path?: string | null;
  error?: string | null;
}

export interface IdeToolchainsResp {
  target_os: string;
  target_arch: string;
  ecosystems: IdeToolchainEcosystem[];
  jobs: IdeInstallJob[];
}

export interface IdeToolchainInstallReq {
  target: IdeTargetSpec;
  ecosystem: IdeEcosystem | string;
  component: IdeInstallComponent | string;
  version?: string | null;
}

export interface IdeToolchainJobResp {
  job: IdeInstallJob;
}

// ── Environments, Run Configurations & Test Runner ───────────────────────────

export interface IdeEnvVar {
  key: string;
  value?: string | null;
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
  interpreter_path?: string | null;
  version: string;
  kind: 'venv' | 'conda' | 'poetry' | 'uv' | string;
}

export interface IdeRunConfig {
  id: string;
  name: string;
  runner: IdeRunnerKind | string;
  command?: string | null;
  args: string[];
  cwd?: string | null;
  env_overrides: Record<string, string>;
  before_launch?: string | null;
  target_path?: string | null;
  test_filter?: string | null;
}

export interface IdeEnvironmentsResp {
  env: IdeProjectEnvironment;
  python_venvs: IdePythonVenv[];
  dotenv_files: string[];
  configs: IdeRunConfig[];
}

export interface IdePythonVenvResp {
  path: string;
  interpreter_path: string;
  version: string;
}

export type IdeTestStatus = 'passed' | 'failed' | 'skipped' | 'running';

export interface IdeTestResult {
  name: string;
  suite?: string | null;
  status: IdeTestStatus;
  duration_ms?: number | null;
  file?: string | null;
  line?: number | null;
  message?: string | null;
}

export type IdeRunStatus = 'running' | 'passed' | 'failed' | 'stopped';

export interface IdeRunState {
  id: string;
  title: string;
  runner: string;
  status: IdeRunStatus;
  exit_code?: number | null;
  started_at?: string | number;
  duration_ms?: number | null;
  output_seq?: number;
  output: string[];
  tests: IdeTestResult[];
}

export interface IdeRunResp {
  run: IdeRunState;
}

// ── DAP Debugger Engine ──────────────────────────────────────────────────────

export type IdeDebugLanguage = 'go' | 'java' | 'rust' | 'javascript' | 'typescript' | 'python' | string;

export interface IdeBreakpoint {
  path: string;
  line: number;
  enabled: boolean;
  condition?: string | null;
  hit_condition?: string | null;
  log_message?: string | null;
  verified?: boolean;
}

export interface IdeDebugLaunch {
  program: string;
  args?: string[];
  cwd?: string | null;
  test_name?: string | null;
  attach_port?: number | null;
  env?: Record<string, string>;
}

export type IdeDebugState = 'starting' | 'running' | 'paused' | 'terminated' | 'error';
export type IdeDebugStoppedReason = 'breakpoint' | 'step' | 'exception' | 'pause' | string;

export interface IdeDebugThread {
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
  expensive?: boolean | null;
  variables: IdeVariable[];
}

export interface IdeInlineValue {
  line: number;
  name: string;
  value: string;
}

export interface IdeDebugOutput {
  category: 'stdout' | 'stderr' | 'console' | string;
  text: string;
}

export interface IdeDebugSessionSnapshot {
  id: string;
  title: string;
  language: string;
  adapter: string;
  state: IdeDebugState;
  stopped_reason?: IdeDebugStoppedReason | null;
  active_thread_id?: number | null;
  active_frame_id?: number | null;
  paused_file?: string | null;
  paused_line?: number | null;
  threads: IdeDebugThread[];
  stack_frames: IdeStackFrame[];
  scopes: IdeScope[];
  inline_values: IdeInlineValue[];
  breakpoints: IdeBreakpoint[];
  output: IdeDebugOutput[];
  error?: string | null;
}

export type IdeDebugAction =
  | 'continue'
  | 'pause'
  | 'step_over'
  | 'step_into'
  | 'step_out'
  | 'run_to_cursor'
  | 'select_frame'
  | 'restart'
  | 'stop';

export interface IdeDebugSessionResp {
  session: IdeDebugSessionSnapshot;
}

export interface IdeDebugSessionsResp {
  sessions: IdeDebugSessionSnapshot[];
}

export interface IdeDebugVariablesResp {
  variables: IdeVariable[];
}

export interface IdeDebugEvaluateResp {
  result: string;
  var_type?: string | null;
  variables_reference: number;
}

// ── Collaborative IDE ("Share with Friends") ─────────────────────────────────

export type IdeCollabRole = 'viewer' | 'editor';

export interface IdeCollabSelection {
  anchor: number;
  head: number;
}

export interface IdeCollabPeer {
  peer_id: string;
  display_name: string;
  color: string;
  role: IdeCollabRole;
  is_host?: boolean;
  active_path?: string | null;
  path?: string | null;
  active_line?: number | null;
  selections?: IdeCollabSelection[];
  connected_at?: string | number;
}

export interface IdePeerCursor {
  peer_id: string;
  name?: string;
  display_name?: string;
  color?: string;
  path?: string | null;
  active_line?: number | null;
  selections?: IdeCollabSelection[];
}

export interface IdeCollabRoom {
  id: string;
  title: string;
  target: IdeTargetSpec;
  default_role: IdeCollabRole;
  host_user_id?: string | null;
  created_at?: string | number;
  peers: IdeCollabPeer[];
  active_debug_id?: string | null;
}

export interface IdeCollabCreateResp {
  room: IdeCollabRoom;
  invite_token: string;
  share_hash: string;
}

export interface IdeCollabRoomsResp {
  rooms: IdeCollabRoom[];
}

export interface IdeCollabJoinResp {
  peer_id: string;
  peer_secret: string;
  color: string;
  room: IdeCollabRoom;
}

export interface IdeCollabSharedBuffer {
  content: string;
  version: number;
}

export interface IdeCollabStateResp {
  room: IdeCollabRoom;
  shared_buffers: Record<string, IdeCollabSharedBuffer>;
  active_debug?: IdeDebugSessionSnapshot | null;
}

export interface IdeCollabEvent {
  type:
    | 'peer_joined'
    | 'peer_left'
    | 'role_changed'
    | 'cursor_moved'
    | 'buffer_updated'
    | 'buffer_edited'
    | 'debug_updated'
    | 'room_closed'
    | string;
  room_id?: string;
  peer_id?: string;
  display_name?: string;
  color?: string;
  peer?: IdeCollabPeer;
  role?: IdeCollabRole;
  path?: string;
  selections?: IdeCollabSelection[];
  active_line?: number | null;
  content?: string;
  version?: number;
  saved?: boolean;
  session?: IdeDebugSessionSnapshot | null;
  debug?: IdeDebugSessionSnapshot | null;
  room?: IdeCollabRoom;
}

// ── Runtime IDE Plugins ──────────────────────────────────────────────────────

export interface IdePluginLanguage {
  id: string;
  name?: string;
  extensions: string[];
}

export interface IdePluginLspServer {
  id?: string;
  language: string;
  command: string;
  args?: string[];
}

export interface IdePluginDapAdapter {
  id?: string;
  language: string;
  command: string;
  args?: string[];
}

export interface IdePluginGutterDetector {
  language: string;
  pattern: string;
  kind: IdeGutterItemKind | string;
  label_template?: string;
  runner?: string;
}

export interface IdePluginKeymap {
  key: string;
  command: string;
  description?: string;
}

export interface IdePluginContributions {
  languages: IdePluginLanguage[];
  lsp_servers: IdePluginLspServer[];
  dap_adapters: IdePluginDapAdapter[];
  gutter_detectors: IdePluginGutterDetector[];
  keymaps: IdePluginKeymap[];
  snippets?: { prefix: string; body: string; description?: string | null }[];
  panels?: { id: string; title: string; command: string }[];
}

export interface IdePluginInfo {
  slug: string;
  name: string;
  version: string;
  description?: string | null;
  author?: string | null;
  enabled: boolean;
  source: string;
  install_path?: string | null;
  contributions?: IdePluginContributions;
  languages?: (IdePluginLanguage | string)[];
  capabilities?: string[];
  lsp_servers?: IdePluginLspServer[];
  dap_adapters?: IdePluginDapAdapter[];
  gutter_detectors?: IdePluginGutterDetector[];
  keymaps?: IdePluginKeymap[];
}

export interface IdePluginCatalogEntry {
  slug: string;
  name: string;
  version: string;
  description: string;
  tags?: string[];
  ecosystems?: string[];
  languages?: string[];
  source?: string;
  installed?: boolean;
}

export interface IdePluginsResp {
  plugins: IdePluginInfo[];
  catalog: IdePluginCatalogEntry[];
}

export interface IdePluginResp {
  plugin: IdePluginInfo;
}

