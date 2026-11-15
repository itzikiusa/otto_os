// TS mirror of crates/otto-ide/src/types/{lsp_projects,history,deps_types,runtime}.rs
// (docs/contracts/api.md § IDE): LSP intelligence, Quick-Doc hover, concrete
// Quick-Fix edits, Local History, Named Multi-Repo Projects, `<15ms` `@` path
// completion, Dependency & Vulnerability Inspector, Build Tools, Collab & Plugins.

import type { IdeDebugSessionSnapshot, IdeGutterItemKind, IdeTargetSpec } from './ide';

// ── LSP Extended Operations, Diagnostics, Hover & Concrete Quick Fixes ───────

export interface IdeTextEdit {
  path: string;
  start_line: number;
  start_col: number;
  end_line: number;
  end_col: number;
  new_text: string;
}

export interface IdeQuickFix {
  title: string;
  line: number;
  end_line: number;
  replacement: string;
  delete_line: boolean;
  is_preferred?: boolean;
  edits?: IdeTextEdit[];
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

export interface IdeRenameReq {
  target: IdeTargetSpec;
  path: string;
  line: number;
  character: number;
  new_name: string;
  apply?: boolean;
  local_history_enabled?: boolean;
  local_history_retention_days?: number;
}

export interface IdeRenameResp {
  symbol: string;
  new_name: string;
  applied: boolean;
  edits: IdeRenameFileEdit[];
}

export interface IdeFormatReq {
  target: IdeTargetSpec;
  path: string;
  content: string;
  language?: string | null;
  organize_imports?: boolean;
}

export interface IdeFormatResp {
  formatted: string;
  changed: boolean;
  formatter: string;
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

export interface IdeCodeActionsResp {
  actions: IdeCodeAction[];
}

export interface IdeDiagnostic {
  path?: string;
  line: number;
  col?: number;
  column?: number;
  end_line?: number;
  end_col?: number;
  end_column?: number;
  severity: 'error' | 'warning' | 'info' | 'hint';
  message: string;
  source?: string | null;
  code?: string | null;
  quick_fixes?: IdeQuickFix[];
}

export interface IdeDiagnosticsReq {
  target?: IdeTargetSpec;
  path: string;
  content?: string | null;
  language?: string | null;
  project_folders?: string[];
}

export interface IdeDiagnosticsResp {
  diagnostics: IdeDiagnostic[];
}

export interface IdeHoverReq {
  target?: IdeTargetSpec;
  path: string;
  content?: string | null;
  line: number;
  character?: number;
  project_folders?: string[];
}

export interface IdeHoverDefinedIn {
  path: string;
  line: number;
}

export interface IdeHoverResp {
  found: boolean;
  symbol?: string | null;
  kind?: string | null;
  package_name?: string | null;
  signature?: string | null;
  summary?: string | null;
  full_doc?: string | null;
  defined_in?: IdeHoverDefinedIn | null;
}

// ── Local History & Deleted-File Recovery (`/ide/history/*`) ─────────────────

export interface IdeLocalHistoryEntry {
  id: string;
  root: string;
  path: string;
  rel_path: string;
  event_kind: string;
  label: string;
  timestamp_ms: number;
  size_bytes: number;
  sha256: string;
}

export interface IdeHistoryListResp {
  entries: IdeLocalHistoryEntry[];
}

export interface IdeHistoryEntryResp {
  entry: IdeLocalHistoryEntry;
  content: string;
  current_content?: string | null;
  current_exists: boolean;
}

export interface IdeHistoryRestoreReq {
  root?: string;
  target?: IdeTargetSpec;
  id: string;
}

export interface IdeHistoryRestoreResp {
  ok: boolean;
  path: string;
  content: string;
  recreated: boolean;
}

export interface IdeHistorySnapshotReq {
  root?: string;
  target?: IdeTargetSpec;
  path: string;
  content?: string;
  label?: string | null;
}

export interface IdeHistorySnapshotResp {
  entry: IdeLocalHistoryEntry;
}

// ── Named Multi-Repo Projects & `@` Path Completion ──────────────────────────

export interface IdeNamedProject {
  id: string;
  name: string;
  primary_root: string;
  folders: string[];
  pinned: boolean;
  updated_at?: string;
}

export interface IdeProjectsResp {
  projects: IdeNamedProject[];
}

export interface IdeUpsertProjectReq {
  id?: string | null;
  name: string;
  primary_root?: string;
  folders?: string[];
  pinned?: boolean;
}

export interface IdeUpsertProjectResp {
  project: IdeNamedProject;
}

export interface IdeScannedRepo {
  name: string;
  path: string;
  kind: string;
  has_go_mod: boolean;
  has_pom_or_gradle: boolean;
  has_git: boolean;
}

export interface IdeScanReposReq {
  parent_path: string;
}

export interface IdeScanReposResp {
  parent_path: string;
  repos: IdeScannedRepo[];
}

export interface IdeBulkFoldersReq {
  target: IdeTargetSpec;
  folder_paths: string[];
}

export interface IdeBulkFoldersResp {
  additional_roots: string[];
}

export interface IdeFsCompleteReq {
  query: string;
  cwd?: string | null;
  roots?: string[];
  target?: IdeTargetSpec;
  aliases?: Record<string, string>;
  limit?: number;
}

export interface IdeFsCompleteItem {
  path: string;
  display: string;
  insert_token: string;
  name: string;
  kind: string;
  is_dir: boolean;
}

export interface IdeFsCompleteResp {
  items: IdeFsCompleteItem[];
  base_dir: string;
  elapsed_ms: number;
}

// ── Dependency Intelligence & Build Tools Model ──────────────────────────────

export interface IdeDepManagedIn {
  path: string;
  line: number;
}

export interface IdeDepVulnerability {
  id: string;
  cve?: string | null;
  severity: string;
  summary: string;
  fixed_version?: string | null;
}

export interface IdeDepItem {
  name: string;
  current_version: string;
  latest_version?: string | null;
  latest_patch?: string | null;
  indirect: boolean;
  deprecated?: string | null;
  line: number;
  name_col: number;
  name_end_col: number;
  version_line: number;
  version_col: number;
  version_end_col: number;
  block_start_line: number;
  block_end_line: number;
  property_name?: string | null;
  managed_in?: IdeDepManagedIn | null;
  vulnerabilities: IdeDepVulnerability[];
}

export interface IdeDepsInspectReq {
  target?: IdeTargetSpec;
  path: string;
  content?: string | null;
}

export interface IdeDepsInspectResp {
  ecosystem: string;
  items: IdeDepItem[];
}

export interface IdeDepsActionReq {
  target?: IdeTargetSpec;
  path: string;
  action: string;
  module?: string | null;
  version?: string | null;
}

export interface IdeDepsActionResp {
  ok: boolean;
  updated_content?: string | null;
  output: string;
}

export interface IdeBuildToolsModelReq {
  roots?: string[];
  target?: IdeTargetSpec;
}

export interface IdeBuildToolProfile {
  id: string;
  kind: 'maven' | 'gradle' | 'spring' | string;
  source_project: string;
  active_by_default: boolean;
}

export interface IdeBuildToolProject {
  name: string;
  group_id: string;
  artifact_id: string;
  version: string;
  tool: 'maven' | 'gradle' | string;
  project_coord: string;
  root_dir: string;
  location_display: string;
  manifest_path: string;
  lifecycle: string[];
  plugins: string[];
  dependencies: string[];
  repositories: string[];
  modules: IdeBuildToolProject[];
}

export interface IdeBuildToolsModelResp {
  has_maven: boolean;
  has_gradle: boolean;
  profiles: IdeBuildToolProfile[];
  projects: IdeBuildToolProject[];
}

// ── Collaborative IDE ("Share with Friends") & Runtime Plugins ───────────────

export type IdeCollabRole = 'viewer' | 'editor';
export interface IdeCollabSelection { anchor: number; head: number; }

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

export interface IdeCreateCollabRoomResp {
  room: IdeCollabRoom;
  invite_token: string;
  share_hash: string;
  host_peer_id: string;
  host_peer_secret: string;
}
export type IdeCollabCreateResp = IdeCreateCollabRoomResp;
export interface IdeCollabRoomsResp { rooms: IdeCollabRoom[]; }
export interface IdeCollabJoinResp { peer_id: string; peer_secret: string; color: string; room: IdeCollabRoom; }
export interface IdeCollabSharedBuffer { content: string; version: number; }
export interface IdeCollabStateResp { room: IdeCollabRoom; shared_buffers: Record<string, IdeCollabSharedBuffer>; active_debug?: IdeDebugSessionSnapshot | null; }

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
  sha256?: string | null;
  session?: IdeDebugSessionSnapshot | null;
  debug?: IdeDebugSessionSnapshot | null;
  room?: IdeCollabRoom;
}

export interface IdePluginLanguage {
  id: string;
  display_name: string;
  extensions: string[];
  line_comment?: string | null;
}

export interface IdePluginLspServer {
  language: string;
  command: string;
  args: string[];
  extensions: string[];
  install_hint?: string | null;
}

export interface IdePluginDapAdapter {
  language: string;
  adapter_name: string;
  command: string;
  args: string[];
  launch_template?: unknown;
}

export interface IdePluginGutterDetector {
  language: string;
  extensions: string[];
  pattern: string;
  kind: IdeGutterItemKind | string;
  runner: string;
  command_template: string;
  args_template: string[];
  label_prefix?: string | null;
}

export interface IdePluginKeymap {
  key: string;
  action: string;
  description?: string | null;
}

export interface IdePluginContributions {
  languages: IdePluginLanguage[];
  lsp_servers: IdePluginLspServer[];
  dap_adapters: IdePluginDapAdapter[];
  gutter_detectors: IdePluginGutterDetector[];
  keymaps: IdePluginKeymap[];
  snippets: { language: string; prefix: string; body: string; description?: string | null }[];
  panels: { id: string; title: string; icon?: string | null; command?: string | null }[];
}

export interface IdePluginInfo {
  slug: string;
  name: string;
  version: string;
  description: string;
  author?: string | null;
  enabled: boolean;
  source: string;
  install_path?: string;
  contributions?: IdePluginContributions;
  languages?: string[];
  capabilities?: string[];
}

export interface IdePluginCatalogEntry {
  slug: string;
  name: string;
  version: string;
  description: string;
  author?: string | null;
  tags: string[];
  installed: boolean;
  languages?: string[];
  source?: string;
}

export interface IdePluginsResp { plugins: IdePluginInfo[]; catalog: IdePluginCatalogEntry[]; }
export interface IdePluginResp { plugin: IdePluginInfo; }
