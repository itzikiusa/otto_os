//! Wire contracts for Dependency Intelligence & Vulnerability Inspector
//! (`POST /ide/deps/inspect` & `POST /ide/deps/action`).

use serde::{Deserialize, Serialize};

use super::IdeTargetSpec;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDepsInspectReq {
    #[serde(default)]
    pub target: Option<IdeTargetSpec>,
    pub path: String,
    #[serde(default)]
    pub content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDepManagedIn {
    pub path: String,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDepVulnerability {
    pub id: String,
    #[serde(default)]
    pub cve: Option<String>,
    pub severity: String,
    pub summary: String,
    #[serde(default)]
    pub fixed_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDepItem {
    pub name: String,
    pub current_version: String,
    #[serde(default)]
    pub latest_version: Option<String>,
    #[serde(default)]
    pub latest_patch: Option<String>,
    #[serde(default)]
    pub indirect: bool,
    #[serde(default)]
    pub deprecated: Option<String>,
    pub line: u32,
    pub name_col: u32,
    pub name_end_col: u32,
    pub version_line: u32,
    pub version_col: u32,
    pub version_end_col: u32,
    pub block_start_line: u32,
    pub block_end_line: u32,
    #[serde(default)]
    pub property_name: Option<String>,
    #[serde(default)]
    pub managed_in: Option<IdeDepManagedIn>,
    #[serde(default)]
    pub vulnerabilities: Vec<IdeDepVulnerability>,
    /// `current_version` is the floor of a range (`^1.0.0`, `>=1.0`), not an
    /// installed version: no vulnerability lookup is made for it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub range: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDepsInspectResp {
    pub ecosystem: String,
    pub items: Vec<IdeDepItem>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDepsActionReq {
    #[serde(default)]
    pub target: Option<IdeTargetSpec>,
    pub path: String,
    pub action: String,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDepsActionResp {
    pub ok: bool,
    pub updated_content: Option<String>,
    pub output: String,
}

// ---------------------------------------------------------------------------
// Auto-Detected Maven / Gradle / Spring Boot Build Tools Model (`POST /ide/build-tools/model`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeBuildToolsModelReq {
    #[serde(default)]
    pub roots: Vec<String>,
    #[serde(default)]
    pub target: Option<IdeTargetSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeBuildToolProfile {
    pub id: String,
    /// `"maven"` | `"gradle"` | `"spring"`.
    pub kind: String,
    pub source_project: String,
    #[serde(default)]
    pub active_by_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeBuildToolProject {
    pub name: String,
    pub group_id: String,
    pub artifact_id: String,
    pub version: String,
    /// `"maven"` | `"gradle"`.
    pub tool: String,
    pub project_coord: String,
    pub root_dir: String,
    pub location_display: String,
    pub manifest_path: String,
    #[serde(default)]
    pub lifecycle: Vec<String>,
    #[serde(default)]
    pub plugins: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub repositories: Vec<String>,
    #[serde(default)]
    pub modules: Vec<IdeBuildToolProject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeMobileProject {
    pub name: String,
    /// `"xcode"` | `"swiftpm"` | `"android"` | `"flutter"` | `"react_native"` | `"expo"` | `"capacitor"`.
    pub framework: String,
    /// Platforms supported by this project (e.g. `["ios", "android"]`).
    #[serde(default)]
    pub platforms: Vec<String>,
    pub root_dir: String,
    pub location_display: String,
    pub manifest_path: String,
    /// Bundle ID / Android `applicationId` if detected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    /// Xcode schemes, Android Gradle tasks, or framework actions.
    #[serde(default)]
    pub targets: Vec<String>,
    /// Suggested quick run tasks (`"installDebug"`, `"build"`, `"run-ios"`, etc.).
    #[serde(default)]
    pub tasks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeSpringBootApp {
    pub name: String,
    pub main_class: String,
    pub file_path: String,
    pub language: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeSpringBootInfo {
    pub detected: bool,
    #[serde(default)]
    pub apps: Vec<IdeSpringBootApp>,
    #[serde(default)]
    pub profiles: Vec<String>,
    #[serde(default)]
    pub config_files: Vec<String>,
    pub server_port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub management_port: Option<u16>,
    pub actuator_base_path: String,
    pub actuator_url: String,
    #[serde(default)]
    pub has_actuator: bool,
    #[serde(default)]
    pub has_devtools: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_root: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeSpringActuatorReq {
    #[serde(default)]
    pub target: Option<IdeTargetSpec>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub base_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeSpringHealthComponent {
    pub name: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeSpringEndpoint {
    pub method: String,
    pub path: String,
    pub handler: String,
    #[serde(default)]
    pub produces: Vec<String>,
    #[serde(default)]
    pub consumes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeSpringBean {
    pub name: String,
    pub bean_type: String,
    pub scope: String,
    pub context: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeSpringEnvProp {
    pub key: String,
    pub value: String,
    pub source: String,
    #[serde(default)]
    pub redacted: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeSpringActuatorResp {
    pub base_url: String,
    pub reachable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default)]
    pub active_profiles: Vec<String>,
    #[serde(default)]
    pub available_links: Vec<String>,
    #[serde(default)]
    pub health_components: Vec<IdeSpringHealthComponent>,
    #[serde(default)]
    pub endpoints: Vec<IdeSpringEndpoint>,
    #[serde(default)]
    pub beans: Vec<IdeSpringBean>,
    #[serde(default)]
    pub env_properties: Vec<IdeSpringEnvProp>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeSpringDevtoolsRestartReq {
    pub target: IdeTargetSpec,
    #[serde(default)]
    pub trigger_file: Option<String>,
    #[serde(default)]
    pub module_root: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeSpringDevtoolsRestartResp {
    pub ok: bool,
    pub trigger_path: String,
    pub requested_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restarted_at: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeBuildToolsModelResp {
    pub has_maven: bool,
    pub has_gradle: bool,
    #[serde(default)]
    pub has_xcode: bool,
    #[serde(default)]
    pub has_android: bool,
    #[serde(default)]
    pub has_flutter: bool,
    #[serde(default)]
    pub has_react_native: bool,
    #[serde(default)]
    pub has_capacitor: bool,
    pub profiles: Vec<IdeBuildToolProfile>,
    pub projects: Vec<IdeBuildToolProject>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mobile_projects: Vec<IdeMobileProject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spring_boot: Option<IdeSpringBootInfo>,
}
