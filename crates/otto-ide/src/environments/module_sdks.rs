//! Per-module SDK override persistence, longest-prefix matching, and automatic
//! ecosystem inference (`GOROOT`, `JAVA_HOME`, `VIRTUAL_ENV`, `RUSTUP_TOOLCHAIN`,
//! `PATH`) for multi-repo workspaces (`IdeProjectEnvironment.module_sdks`).

use std::collections::BTreeMap;
use std::path::Path;

use otto_core::Result;
use otto_state::DbPool;

use super::db_store::{load_stored_project_env, push_unique, StoredEnvVar};
use crate::types::{
    IdeProjectEnvironment, IdeRunConfig, IdeRunnerKind, IdeTargetKind, IdeTargetSpec,
};

pub(crate) fn sanitize_module_sdks(raw: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (k, v) in raw {
        let clean_k = k.trim().trim_end_matches('/');
        let clean_v = v.trim();
        if !clean_k.is_empty() && !clean_v.is_empty() {
            out.insert(clean_k.to_string(), clean_v.to_string());
        }
    }
    out
}

pub(crate) fn sanitize_module_go_build_tags(
    raw: &BTreeMap<String, Vec<String>>,
) -> Result<BTreeMap<String, Vec<String>>> {
    let mut out = BTreeMap::new();
    for (k, raw_tags) in raw {
        let clean_k = k.trim().trim_end_matches('/');
        if clean_k.is_empty() {
            continue;
        }
        let validated = crate::go_build_tags::validate_go_build_tags(raw_tags)?;
        // Preserve empty `[]` module overrides: `Some(vec![])` explicitly clears
        // inherited workspace Go build tags for that child/sibling module.
        out.insert(clean_k.to_string(), validated);
    }
    Ok(out)
}

#[allow(clippy::disallowed_methods)] // fast O(1) stat canonicalization on 1..N module paths
pub(crate) fn match_longest_module_go_build_tags(
    module_tags: &BTreeMap<String, Vec<String>>,
    candidate_paths: &[String],
    local: bool,
) -> Option<(String, Vec<String>)> {
    if module_tags.is_empty() {
        return None;
    }
    let mut best: Option<(usize, String, Vec<String>)> = None;
    for raw_cand in candidate_paths {
        let cand = raw_cand.trim().trim_end_matches('/');
        if cand.is_empty() {
            continue;
        }
        let cand_canon = if local {
            std::fs::canonicalize(cand).ok().or_else(|| {
                Path::new(cand)
                    .parent()
                    .and_then(|p| std::fs::canonicalize(p).ok())
            })
        } else {
            None
        };

        for (mod_root, tags) in module_tags {
            let mr = mod_root.trim().trim_end_matches('/');
            if mr.is_empty() {
                continue;
            }
            let mut matched = cand == mr || cand.starts_with(&format!("{mr}/"));
            if !matched && local {
                if let (Some(cc), Ok(cmr)) = (&cand_canon, std::fs::canonicalize(mr)) {
                    matched = cc == &cmr || cc.starts_with(&cmr);
                }
            }
            if matched
                && best
                    .as_ref()
                    .is_none_or(|(prev_len, _, _)| mr.len() > *prev_len)
            {
                best = Some((mr.len(), mr.to_string(), tags.clone()));
            }
        }
    }
    best.map(|(_, mr, tags)| (mr, tags))
}

#[allow(clippy::disallowed_methods)]
fn any_candidate_under_root(root: &str, candidate_paths: &[String], local: bool) -> bool {
    let r = root.trim().trim_end_matches('/');
    if r.is_empty() {
        return true;
    }
    let r_canon = if local {
        std::fs::canonicalize(r).ok()
    } else {
        None
    };
    for raw_cand in candidate_paths {
        let cand = raw_cand.trim().trim_end_matches('/');
        if cand.is_empty() {
            continue;
        }
        if cand == r || cand.starts_with(&format!("{r}/")) {
            return true;
        }
        if local {
            let cand_canon = std::fs::canonicalize(cand).ok().or_else(|| {
                Path::new(cand)
                    .parent()
                    .and_then(|p| std::fs::canonicalize(p).ok())
            });
            if let (Some(cc), Some(cr)) = (&cand_canon, &r_canon) {
                if cc == cr || cc.starts_with(cr) {
                    return true;
                }
            }
        }
    }
    false
}

pub(crate) fn effective_go_build_tags_for_paths_in_project(
    primary_root: Option<&str>,
    proj: &IdeProjectEnvironment,
    candidate_paths: &[String],
    local: bool,
) -> Vec<String> {
    let mut out = Vec::new();
    if let Some((_, mod_tags)) =
        match_longest_module_go_build_tags(&proj.module_go_build_tags, candidate_paths, local)
    {
        for t in mod_tags {
            push_unique(&mut out, t);
        }
        return out;
    }
    if let Some(pr) = primary_root {
        if !any_candidate_under_root(pr, candidate_paths, local) {
            return out;
        }
    }
    for t in &proj.go_build_tags {
        push_unique(&mut out, t.clone());
    }
    out
}

pub(crate) fn effective_go_build_tags_for_paths(
    proj: &IdeProjectEnvironment,
    candidate_paths: &[String],
    local: bool,
) -> Vec<String> {
    effective_go_build_tags_for_paths_in_project(None, proj, candidate_paths, local)
}

/// Find the longest-prefix match in `module_sdks` for any of `candidate_paths`,
/// comparing both normalized lexical paths and (when `local`) canonicalized paths.
#[allow(clippy::disallowed_methods)] // fast O(1) stat canonicalization on 1..N module paths
pub(crate) fn match_longest_module_sdk(
    module_sdks: &BTreeMap<String, String>,
    candidate_paths: &[String],
    local: bool,
) -> Option<(String, String)> {
    if module_sdks.is_empty() {
        return None;
    }
    let mut best: Option<(usize, String, String)> = None;
    for raw_cand in candidate_paths {
        let cand = raw_cand.trim().trim_end_matches('/');
        if cand.is_empty() {
            continue;
        }
        let cand_canon = if local {
            std::fs::canonicalize(cand).ok().or_else(|| {
                Path::new(cand)
                    .parent()
                    .and_then(|p| std::fs::canonicalize(p).ok())
            })
        } else {
            None
        };

        for (mod_root, sdk_val) in module_sdks {
            let mr = mod_root.trim().trim_end_matches('/');
            if mr.is_empty() || sdk_val.trim().is_empty() {
                continue;
            }
            let mut matched = cand == mr || cand.starts_with(&format!("{mr}/"));
            if !matched && local {
                if let (Some(cc), Ok(cmr)) = (&cand_canon, std::fs::canonicalize(mr)) {
                    matched = cc == &cmr || cc.starts_with(&cmr);
                }
            }
            if matched
                && best
                    .as_ref()
                    .is_none_or(|(prev_len, _, _)| mr.len() > *prev_len)
            {
                best = Some((mr.len(), mr.to_string(), sdk_val.trim().to_string()));
            }
        }
    }
    best.map(|(_, mr, sdk)| (mr, sdk))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SdkKindHint {
    Go,
    Java,
    Python,
    Rust,
    Node,
    GenericPath,
}

fn infer_sdk_kind(
    sdk: &str,
    runner: Option<IdeRunnerKind>,
    lang_or_path_hint: Option<&str>,
    mod_root: &str,
) -> SdkKindHint {
    if let Some(r) = runner {
        match r {
            IdeRunnerKind::GoRun | IdeRunnerKind::GoBuild | IdeRunnerKind::GoTest => {
                return SdkKindHint::Go;
            }
            IdeRunnerKind::JavaMain
            | IdeRunnerKind::JavaJunit
            | IdeRunnerKind::Maven
            | IdeRunnerKind::Gradle => {
                return SdkKindHint::Java;
            }
            IdeRunnerKind::Python | IdeRunnerKind::Pytest => return SdkKindHint::Python,
            IdeRunnerKind::CargoRun | IdeRunnerKind::CargoTest => return SdkKindHint::Rust,
            IdeRunnerKind::Node | IdeRunnerKind::Npm | IdeRunnerKind::Vitest => {
                return SdkKindHint::Node;
            }
            IdeRunnerKind::Shell | IdeRunnerKind::Compound | IdeRunnerKind::Custom => {}
        }
    }
    if let Some(h) = lang_or_path_hint.map(str::trim) {
        let lower = h.to_ascii_lowercase();
        if lower == "go" || lower.ends_with(".go") {
            return SdkKindHint::Go;
        }
        if lower == "java" || lower.ends_with(".java") {
            return SdkKindHint::Java;
        }
        if lower == "python" || lower.ends_with(".py") {
            return SdkKindHint::Python;
        }
        if lower == "rust" || lower.ends_with(".rs") {
            return SdkKindHint::Rust;
        }
        if matches!(
            lower.as_str(),
            "javascript" | "typescript" | "js" | "ts" | "node"
        ) || lower.ends_with(".js")
            || lower.ends_with(".ts")
            || lower.ends_with(".mjs")
            || lower.ends_with(".cjs")
        {
            return SdkKindHint::Node;
        }
    }
    if !sdk.starts_with('/') {
        return SdkKindHint::Rust;
    }
    let sp = Path::new(sdk);
    let s_low = sdk.to_ascii_lowercase();
    if s_low.ends_with("/python")
        || s_low.ends_with("/python3")
        || s_low.contains("/python3.")
        || s_low.contains("venv")
        || s_low.contains("conda")
        || sp.join("pyvenv.cfg").is_file()
        || sp.join("bin/python").is_file()
    {
        return SdkKindHint::Python;
    }
    if sp.join("bin/go").is_file() || s_low.contains("/go/") || s_low.contains("go1.") {
        return SdkKindHint::Go;
    }
    if sp.join("bin/javac").is_file()
        || s_low.contains("jdk")
        || s_low.contains("javavirtualmachines")
        || s_low.contains("/java/")
    {
        return SdkKindHint::Java;
    }
    if sp.join("bin/rustc").is_file() || sp.join("bin/cargo").is_file() || s_low.contains("rust") {
        return SdkKindHint::Rust;
    }
    if sp.join("bin/node").is_file()
        || s_low.ends_with("/node")
        || s_low.contains("/node/")
        || s_low.contains("nvm")
    {
        return SdkKindHint::Node;
    }
    let mr = Path::new(mod_root);
    if mr.join("go.mod").is_file() {
        return SdkKindHint::Go;
    }
    if mr.join("pom.xml").is_file()
        || mr.join("build.gradle").is_file()
        || mr.join("build.gradle.kts").is_file()
    {
        return SdkKindHint::Java;
    }
    if mr.join("pyproject.toml").is_file() || mr.join("requirements.txt").is_file() {
        return SdkKindHint::Python;
    }
    if mr.join("Cargo.toml").is_file() {
        return SdkKindHint::Rust;
    }
    if mr.join("package.json").is_file() {
        return SdkKindHint::Node;
    }
    SdkKindHint::GenericPath
}

pub(crate) fn apply_module_sdk_to_project_env(
    proj: &mut IdeProjectEnvironment,
    mod_root: &str,
    sdk_path: &str,
    runner: Option<IdeRunnerKind>,
    lang_or_path_hint: Option<&str>,
) {
    let sdk = sdk_path.trim();
    if sdk.is_empty() {
        return;
    }
    let trimmed = sdk.trim_end_matches('/');
    match infer_sdk_kind(trimmed, runner, lang_or_path_hint, mod_root) {
        SdkKindHint::Go => {
            let root = trimmed
                .strip_suffix("/bin/go")
                .or_else(|| trimmed.strip_suffix("/bin"))
                .unwrap_or(trimmed);
            proj.go_root = Some(root.to_string());
        }
        SdkKindHint::Java => {
            let root = trimmed
                .strip_suffix("/bin/java")
                .or_else(|| trimmed.strip_suffix("/bin/javac"))
                .or_else(|| trimmed.strip_suffix("/bin"))
                .unwrap_or(trimmed);
            proj.java_home = Some(root.to_string());
        }
        SdkKindHint::Python => {
            if trimmed.ends_with("/python")
                || trimmed.ends_with("/python3")
                || trimmed.contains("/python3.")
                || Path::new(trimmed).is_file()
            {
                proj.python_interpreter = Some(trimmed.to_string());
            } else {
                proj.python_interpreter = Some(format!("{trimmed}/bin/python"));
            }
        }
        SdkKindHint::Rust => {
            let root = trimmed
                .strip_suffix("/bin/cargo")
                .or_else(|| trimmed.strip_suffix("/bin/rustc"))
                .or_else(|| trimmed.strip_suffix("/bin"))
                .unwrap_or(trimmed);
            proj.rust_toolchain = Some(root.to_string());
        }
        SdkKindHint::Node => {
            let root = trimmed
                .strip_suffix("/bin/node")
                .or_else(|| trimmed.strip_suffix("/bin"))
                .unwrap_or(trimmed);
            proj.node_home = Some(root.to_string());
        }
        SdkKindHint::GenericPath => {
            let bin_dir = if Path::new(trimmed).is_file() {
                Path::new(trimmed)
                    .parent()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|| trimmed.to_string())
            } else {
                format!("{trimmed}/bin")
            };
            proj.extra_path = Some(match proj.extra_path.take() {
                Some(prev) if !prev.is_empty() => format!("{bin_dir}:{prev}"),
                _ => bin_dir,
            });
        }
    }
}

pub(super) async fn layer_child_and_module_sdk_for_run(
    pool: &DbPool,
    target: &IdeTargetSpec,
    cfg: Option<&IdeRunConfig>,
    proj: &mut IdeProjectEnvironment,
    stored_vars: &mut Vec<StoredEnvVar>,
) {
    let Some(c) = cfg else {
        return;
    };
    let resolved_cwd = c
        .cwd
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != ".")
        .map(|raw| {
            if raw.starts_with('/') {
                raw.trim_end_matches('/').to_string()
            } else {
                format!(
                    "{}/{}",
                    target.root.trim_end_matches('/'),
                    raw.trim_start_matches("./").trim_end_matches('/')
                )
            }
        });
    let base_for_tp = resolved_cwd.as_deref().unwrap_or(target.root.as_str());
    let resolved_tp = c
        .target_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|raw| {
            if raw.starts_with('/') {
                raw.trim_end_matches('/').to_string()
            } else {
                format!(
                    "{}/{}",
                    base_for_tp.trim_end_matches('/'),
                    raw.trim_start_matches("./").trim_end_matches('/')
                )
            }
        });

    let mut child_own_go_tags: Vec<String> = Vec::new();
    if target.kind == IdeTargetKind::Local {
        let child_dir = resolved_cwd.clone().or_else(|| {
            resolved_tp.as_deref().and_then(|tp| {
                let p = Path::new(tp);
                if p.is_dir() {
                    Some(tp.to_string())
                } else {
                    p.parent().map(|d| d.to_string_lossy().into_owned())
                }
            })
        });
        if let Some(mod_dir) = child_dir.filter(|d| !d.is_empty() && d != &target.root) {
            let mod_key = format!("local:{}", mod_dir.trim_end_matches('/'));
            if let Ok((mod_proj, mod_vars)) = load_stored_project_env(pool, &mod_key).await {
                if mod_proj.java_home.is_some() {
                    proj.java_home = mod_proj.java_home;
                }
                if mod_proj.go_root.is_some() {
                    proj.go_root = mod_proj.go_root;
                }
                if mod_proj.rust_toolchain.is_some() {
                    proj.rust_toolchain = mod_proj.rust_toolchain;
                }
                if mod_proj.node_home.is_some() {
                    proj.node_home = mod_proj.node_home;
                }
                if mod_proj.python_interpreter.is_some() {
                    proj.python_interpreter = mod_proj.python_interpreter;
                }
                if mod_proj.dotenv_path.is_some() {
                    proj.dotenv_path = mod_proj.dotenv_path;
                }
                child_own_go_tags = mod_proj.go_build_tags;
                for (mk, mtags) in mod_proj.module_go_build_tags {
                    proj.module_go_build_tags.entry(mk).or_insert(mtags);
                }
                stored_vars.extend(mod_vars);
            }
        }
    }

    let mut candidates = Vec::new();
    if let Some(tp) = &resolved_tp {
        candidates.push(tp.clone());
    }
    if let Some(cw) = &resolved_cwd {
        candidates.push(cw.clone());
    }
    if candidates.is_empty() {
        candidates.push(target.root.clone());
    }
    let has_mod_override = match_longest_module_go_build_tags(
        &proj.module_go_build_tags,
        &candidates,
        target.kind == IdeTargetKind::Local,
    )
    .is_some();
    let mut eff_tags = effective_go_build_tags_for_paths_in_project(
        Some(&target.root),
        proj,
        &candidates,
        target.kind == IdeTargetKind::Local,
    );
    if !has_mod_override {
        for tag in child_own_go_tags {
            push_unique(&mut eff_tags, tag);
        }
    }
    proj.go_build_tags = eff_tags;
    if let Some((mod_root, sdk_val)) = match_longest_module_sdk(
        &proj.module_sdks,
        &candidates,
        target.kind == IdeTargetKind::Local,
    ) {
        apply_module_sdk_to_project_env(
            proj,
            &mod_root,
            &sdk_val,
            Some(c.runner),
            resolved_tp.as_deref(),
        );
    }
}
