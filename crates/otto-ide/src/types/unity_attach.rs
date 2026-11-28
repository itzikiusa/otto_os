//! Unity Editor attach (`POST /ide/debug/unity/instances`).

use serde::{Deserialize, Serialize};

use super::IdeTargetSpec;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeUnityInstancesReq {
    pub target: IdeTargetSpec,
    /// The Unity project folder when it is not the target root (a workspace
    /// sub-root); it must lie inside the target.
    #[serde(default)]
    pub cwd: Option<String>,
}

/// A running Unity Editor that has the project open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeUnityEditor {
    pub pid: u32,
    /// The Editor version from `EditorInstance.json` (`6000.0.23f1`).
    pub version: String,
    /// The Editor's Mono soft-debugger port on loopback (`56000 + pid % 1000`).
    pub port: u16,
    pub project_root: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeUnityInstancesResp {
    /// Empty when no Editor has the project open (or the target is not a local Unity project).
    pub editors: Vec<IdeUnityEditor>,
}
