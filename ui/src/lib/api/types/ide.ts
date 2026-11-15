// TS mirror of crates/otto-ide/src/types* (docs/contracts/api.md § IDE).
// Core target, filesystem, search/replace, toolchains, environments, run/debug,
// collaboration, and runtime IDE plugin contracts.

import type { IdeCodeAction, IdeTextEdit } from './ide-intelligence';

export type IdeTargetKind = 'local' | 'ssh';

export interface IdeTargetSpec {
  kind: IdeTargetKind;
  workspace_id?: string | null;
  root: string;
  connection_id?: string | null;
  connection_name?: string | null;
  additional_roots?: string[];
  project_folders?: string[];
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
  allow_overwrite_large?: boolean;
  local_history_enabled?: boolean;
  local_history_retention_days?: number;
}

export interface IdeGitBaseResp {
  in_repo: boolean;
  repo_root?: string | null;
  rel_path?: string | null;
  head_exists: boolean;
  content?: string | null;
  binary: boolean;
  truncated: boolean;
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
  local_history_enabled?: boolean;
  local_history_retention_days?: number;
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
  whole_word?: boolean;
  regex?: boolean;
  file_glob?: string | null;
  scope_path?: string | null;
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
  whole_word?: boolean;
  regex?: boolean;
  file_glob?: string | null;
  scope_path?: string | null;
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
  | 'go_build'
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
  | 'shell'
  | 'compound'
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

// ── Toolchains, Detected SDKs & OS/Arch SDK Downloader ───────────────────────

export type IdeEcosystem = 'java' | 'go' | 'rust' | 'node' | 'python' | 'shell' | 'json';
export type IdeInstallComponent = 'sdk' | 'lsp' | 'dap' | 'all';

export interface IdeInstalledSdk {
  id: string;
  version: string;
  path: string;
  source: 'system' | 'otto' | 'local' | string;
  active: boolean;
}

export interface IdeDetectedSdk {
  ecosystem: IdeEcosystem | string;
  name: string;
  subtitle?: string;
  version: string;
  arch?: string;
  home_path?: string;
  bin_path?: string;
  path: string;
  source: string;
  valid?: boolean;
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
  sha256?: string | null;
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
  detected?: IdeDetectedSdk[];
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

export interface IdeToolchainLocalReq {
  target: IdeTargetSpec;
  ecosystem: IdeEcosystem | string;
  path: string;
  version?: string | null;
}

export interface IdeToolchainPythonVenvReq {
  target?: IdeTargetSpec;
  root?: string | null;
  path?: string | null;
  base_python?: string | null;
}

export interface IdeToolchainPythonVenvResp {
  ok: boolean;
  venv: IdePythonVenv;
  interpreter_path: string;
}

export interface IdeShellEnvResp {
  status?: string;
  shell: string;
  vars: Record<string, string>;
  env?: Record<string, string>;
  var_count?: number;
  loaded_at?: string;
  duration_ms?: number;
  error?: string | null;
}

// ── Environments, Run Configurations & Test Runner ───────────────────────────

export interface IdeEnvVar {
  key: string;
  value?: string | null;
  secret: boolean;
  has_secret?: boolean;
}

export interface IdeHostMapping {
  from_host: string;
  from_port?: number | null;
  to_host: string;
  to_port?: number | null;
}

export type IdeNetworkBridgeHostMapping = IdeHostMapping;

export interface IdeNetworkBridgeConfig {
  enabled: boolean;
  k8s_cluster_id?: string | null;
  k8s_namespace?: string | null;
  ssh_tunnel_connection_id?: string | null;
  inject_k8s_service_env: boolean;
  containerized: boolean;
  host_mappings?: IdeHostMapping[];
}

export interface IdeBridgedServiceSummary {
  name: string;
  namespace: string;
  cluster_ip?: string | null;
  ports: number[];
  hosts?: string[];
}

export interface IdeNetBridgeTestReq {
  target: IdeTargetSpec;
  bridge: IdeNetworkBridgeConfig;
}

export interface IdeNetBridgeStatusResp {
  ok: boolean;
  cluster_name?: string | null;
  namespace?: string | null;
  ssh_tunnel_ready: boolean;
  interposer_ready: boolean;
  services: IdeBridgedServiceSummary[];
  ingress_hosts: string[];
  latency_ms: number;
  message: string;
}

export interface IdeProjectEnvironment {
  java_home?: string | null;
  go_root?: string | null;
  rust_toolchain?: string | null;
  node_home?: string | null;
  python_interpreter?: string | null;
  dotenv_path?: string | null;
  extra_path?: string | null;
  inherit_shell_env?: boolean;
  additional_roots?: string[];
  module_sdks?: Record<string, string>;
  go_build_tags?: string[];
  module_go_build_tags?: Record<string, string[]>;
  network_bridge?: IdeNetworkBridgeConfig | null;
  env_vars: IdeEnvVar[];
}

export interface IdePythonVenv {
  path: string;
  interpreter_path?: string | null;
  version: string;
  kind: 'venv' | 'conda' | 'poetry' | 'uv' | string;
}

export interface IdeCreatePythonVenvReq {
  target?: IdeTargetSpec;
  root?: string | null;
  venv_dir?: string | null;
  python_version?: string | null;
}

export interface IdeRunConfig {
  id: string;
  name: string;
  runner: IdeRunnerKind | string;
  command?: string | null;
  args: string[];
  build_flags?: string[];
  cwd?: string | null;
  env_overrides: Record<string, string>;
  env_file?: string | null;
  inherit_shell_env?: boolean;
  sdk_path?: string | null;
  toolchain_id?: string | null;
  compound_configs?: string[];
  auto_detected?: boolean;
  before_launch?: string | null;
  target_path?: string | null;
  test_filter?: string | null;
  maven_profiles?: string[];
  spring_profiles?: string[];
  skip_tests?: boolean;
  offline?: boolean;
  coverage?: boolean;
}

export interface IdeEnvironmentsResp {
  env: IdeProjectEnvironment;
  python_venvs: IdePythonVenv[];
  dotenv_files: string[];
  configs: IdeRunConfig[];
  detected_configs?: IdeRunConfig[];
  effective_env?: Record<string, string>;
}

export interface IdeUpdateEnvironmentReq extends IdeProjectEnvironment {
  target: IdeTargetSpec;
  confirm_prod_targets?: boolean;
}

export interface IdeEnvironmentResp {
  env: IdeProjectEnvironment;
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
  coverage_pct?: number | null;
  coverage_summary?: string | null;
}

export interface IdeStartRunReq {
  target: IdeTargetSpec;
  config_id?: string | null;
  adhoc?: IdeRunConfig | null;
  project_folders?: string[];
  maven_profiles?: string[];
  spring_profiles?: string[];
  skip_tests?: boolean;
  offline?: boolean;
  coverage?: boolean;
  confirm_prod_targets?: boolean;
}

export interface IdeRunResp {
  run: IdeRunState;
}

// ── DAP Debugger Engine ──────────────────────────────────────────────────────

export type IdeDebugLanguage = 'go' | 'java' | 'rust' | 'javascript' | 'typescript' | 'python' | 'shell' | string;

export interface IdeBreakpoint {
  path: string;
  line: number;
  enabled: boolean;
  condition?: string | null;
  hit_condition?: string | null;
  log_message?: string | null;
  verified?: boolean;
  message?: string | null;
}

export interface IdeDebugLaunch {
  program: string;
  args?: string[];
  build_flags?: string[];
  cwd?: string | null;
  test_name?: string | null;
  attach_port?: number | null;
  env?: Record<string, string>;
  env_file?: string | null;
  inherit_shell_env?: boolean;
  sdk_path?: string | null;
  maven_profiles?: string[];
  spring_profiles?: string[];
  skip_tests?: boolean;
  offline?: boolean;
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
  revision?: number;
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

export interface IdeDebugSessionResp { session: IdeDebugSessionSnapshot; }
export interface IdeDebugSessionsResp { sessions: IdeDebugSessionSnapshot[]; }
export interface IdeDebugVariablesResp { variables: IdeVariable[]; }
export interface IdeDebugEvaluateResp { result: string; var_type?: string | null; variables_reference: number; }
