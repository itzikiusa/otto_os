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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeBuildToolsModelResp {
    pub has_maven: bool,
    pub has_gradle: bool,
    pub profiles: Vec<IdeBuildToolProfile>,
    pub projects: Vec<IdeBuildToolProject>,
}
