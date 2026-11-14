//! Shared wire types for DAP Debugger, Collaborative IDE ("Share with Friends"),
//! and Runtime IDE Plugins. Re-exported from [`crate::types`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{IdeGutterKind, IdeRunnerKind, IdeTargetSpec};

fn default_true() -> bool {
    true
}

fn default_gutter_kind_test() -> IdeGutterKind {
    IdeGutterKind::Test
}

fn default_runner_custom() -> IdeRunnerKind {
    IdeRunnerKind::Custom
}

// ---------------------------------------------------------------------------
// D. Full DAP Debugger Engine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeBreakpoint {
    pub path: String,
    pub line: u32,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_condition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl Default for IdeBreakpoint {
    fn default() -> Self {
        Self {
            path: String::new(),
            line: 1,
            enabled: true,
            condition: None,
            hit_condition: None,
            log_message: None,
            verified: None,
            message: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDebugLaunchSpec {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub build_flags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attach_port: Option<u16>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_shell_env: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk_path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub maven_profiles: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spring_profiles: Vec<String>,
    #[serde(default)]
    pub skip_tests: bool,
    #[serde(default)]
    pub offline: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeStartDebugReq {
    pub target: IdeTargetSpec,
    pub language: String,
    #[serde(default)]
    pub config_id: Option<String>,
    pub launch: IdeDebugLaunchSpec,
    #[serde(default)]
    pub breakpoints: Vec<IdeBreakpoint>,
    #[serde(
        default,
        alias = "projectFolders",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub project_folders: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub maven_profiles: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spring_profiles: Vec<String>,
    #[serde(default)]
    pub skip_tests: bool,
    #[serde(default)]
    pub offline: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdeDebugState {
    Starting,
    Running,
    Paused,
    Terminated,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeThreadInfo {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeStackFrame {
    pub id: i64,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeVariable {
    pub name: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub var_type: Option<String>,
    pub variables_reference: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indexed_variables: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeScope {
    pub name: String,
    pub variables_reference: i64,
    #[serde(default)]
    pub variables: Vec<IdeVariable>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeInlineValue {
    pub line: u32,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDebugOutputEntry {
    /// `"stdout"` | `"stderr"` | `"console"`.
    pub category: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDebugSessionSnapshot {
    pub id: String,
    pub title: String,
    pub language: String,
    pub adapter: String,
    pub state: IdeDebugState,
    pub stopped_reason: Option<String>,
    pub active_thread_id: Option<i64>,
    pub active_frame_id: Option<i64>,
    pub paused_file: Option<String>,
    pub paused_line: Option<u32>,
    pub threads: Vec<IdeThreadInfo>,
    pub stack_frames: Vec<IdeStackFrame>,
    pub scopes: Vec<IdeScope>,
    pub inline_values: Vec<IdeInlineValue>,
    pub breakpoints: Vec<IdeBreakpoint>,
    pub output: Vec<IdeDebugOutputEntry>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDebugSessionResp {
    pub session: IdeDebugSessionSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDebugSessionsResp {
    pub sessions: Vec<IdeDebugSessionSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeSetBreakpointsReq {
    pub breakpoints: Vec<IdeBreakpoint>,
    #[serde(default)]
    pub break_on_exceptions: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdeDebugAction {
    Continue,
    Pause,
    StepOver,
    StepInto,
    StepOut,
    RunToCursor,
    SelectFrame,
    Restart,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDebugCommandReq {
    pub action: IdeDebugAction,
    #[serde(default)]
    pub thread_id: Option<i64>,
    #[serde(default)]
    pub frame_id: Option<i64>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDebugVariablesReq {
    pub variables_reference: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDebugVariablesResp {
    pub variables: Vec<IdeVariable>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDebugEvaluateReq {
    pub expression: String,
    #[serde(default)]
    pub frame_id: Option<i64>,
    /// `"watch"` | `"repl"` | `"hover"`.
    #[serde(default)]
    pub context: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDebugEvaluateResp {
    pub result: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub var_type: Option<String>,
    pub variables_reference: i64,
}

// ---------------------------------------------------------------------------
// E. "Share with Friends" Collaborative IDE
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IdeCollabRole {
    #[default]
    Viewer,
    Editor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeSelectionRange {
    pub anchor: usize,
    pub head: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeCollabPeer {
    pub peer_id: String,
    pub display_name: String,
    pub color: String,
    pub role: IdeCollabRole,
    pub is_host: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_line: Option<u32>,
    #[serde(default)]
    pub selections: Vec<IdeSelectionRange>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeCollabRoom {
    pub id: String,
    pub title: String,
    pub target: IdeTargetSpec,
    pub host_user_id: String,
    pub default_role: IdeCollabRole,
    pub peers: Vec<IdeCollabPeer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_debug_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeCreateCollabRoomReq {
    pub target: IdeTargetSpec,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub default_role: IdeCollabRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeCreateCollabRoomResp {
    pub room: IdeCollabRoom,
    pub invite_token: String,
    pub share_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeCollabRoomsResp {
    pub rooms: Vec<IdeCollabRoom>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeCollabRoleReq {
    pub peer_id: String,
    pub role: IdeCollabRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeCollabRoomResp {
    pub room: IdeCollabRoom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeCollabJoinReq {
    pub display_name: String,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeCollabJoinResp {
    pub peer_id: String,
    pub peer_secret: String,
    pub color: String,
    pub room: IdeCollabRoom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeSharedBuffer {
    pub content: String,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeCollabStateResp {
    pub room: IdeCollabRoom,
    pub shared_buffers: BTreeMap<String, IdeSharedBuffer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_debug: Option<IdeDebugSessionSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeCollabCursorReq {
    pub peer_id: String,
    pub peer_secret: String,
    pub path: String,
    #[serde(default)]
    pub selections: Vec<IdeSelectionRange>,
    pub active_line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeCollabEditReq {
    pub peer_id: String,
    pub peer_secret: String,
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub base_version: Option<u64>,
    #[serde(default)]
    pub save: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeCollabEditResp {
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeCollabDebugActionReq {
    pub peer_id: String,
    pub peer_secret: String,
    pub debug_id: String,
    pub action: IdeDebugAction,
    #[serde(default)]
    pub thread_id: Option<i64>,
    #[serde(default)]
    pub frame_id: Option<i64>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub breakpoints: Option<Vec<IdeBreakpoint>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IdeCollabEvent {
    PeerJoined {
        peer: IdeCollabPeer,
        room: IdeCollabRoom,
    },
    PeerLeft {
        peer_id: String,
        room: IdeCollabRoom,
    },
    RoleChanged {
        peer_id: String,
        role: IdeCollabRole,
        room: IdeCollabRoom,
    },
    CursorMoved {
        peer_id: String,
        display_name: String,
        color: String,
        path: String,
        selections: Vec<IdeSelectionRange>,
        active_line: u32,
    },
    BufferUpdated {
        peer_id: String,
        path: String,
        content: String,
        version: u64,
        saved: bool,
    },
    DebugUpdated {
        session: IdeDebugSessionSnapshot,
    },
    RoomClosed {
        room_id: String,
    },
}

// ---------------------------------------------------------------------------
// F. Runtime IDE Plugins
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginLanguageSpec {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginLspServerSpec {
    pub language: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginDapAdapterSpec {
    pub language: String,
    pub adapter_name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_template: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginGutterDetector {
    pub language: String,
    #[serde(default)]
    pub extensions: Vec<String>,
    pub pattern: String,
    #[serde(default = "default_gutter_kind_test")]
    pub kind: IdeGutterKind,
    #[serde(default = "default_runner_custom")]
    pub runner: IdeRunnerKind,
    #[serde(default)]
    pub command_template: String,
    #[serde(default)]
    pub args_template: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_prefix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginKeymapBinding {
    pub key: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginSnippetSpec {
    pub language: String,
    pub prefix: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginPanelSpec {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdePluginContributions {
    #[serde(default)]
    pub languages: Vec<PluginLanguageSpec>,
    #[serde(default)]
    pub lsp_servers: Vec<PluginLspServerSpec>,
    #[serde(default)]
    pub dap_adapters: Vec<PluginDapAdapterSpec>,
    #[serde(default)]
    pub gutter_detectors: Vec<PluginGutterDetector>,
    #[serde(default)]
    pub keymaps: Vec<PluginKeymapBinding>,
    #[serde(default)]
    pub snippets: Vec<PluginSnippetSpec>,
    #[serde(default)]
    pub panels: Vec<PluginPanelSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdePluginManifest {
    pub slug: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default)]
    pub contributions: IdePluginContributions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdePluginInfo {
    pub slug: String,
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub enabled: bool,
    pub source: String,
    pub install_path: String,
    pub contributions: IdePluginContributions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdePluginCatalogEntry {
    pub slug: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub tags: Vec<String>,
    pub installed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdePluginsResp {
    pub plugins: Vec<IdePluginInfo>,
    pub catalog: Vec<IdePluginCatalogEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeInstallPluginReq {
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdePluginResp {
    pub plugin: IdePluginInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeTogglePluginReq {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeOkResp {
    pub ok: bool,
}
