//! Which Prettier configuration format-on-save may honour.
//!
//! A Prettier config can be data (`.prettierrc` as JSON/YAML,
//! `.prettierrc.json|yaml|yml|json5|toml`, a `package.json` `"prettier"`
//! object) or code: `prettier.config.{js,cjs,mjs,ts}`, `.prettierrc.{js,…}`,
//! any config naming `plugins`, or a string naming a shared-config package
//! that Node resolves and loads. Code from a repo must not run on every save
//! (as the OS user, unsandboxed), so the IDE resolves the nearest config
//! itself, from the file up to the project root:
//! - none → Prettier defaults (`--no-config`; `.editorconfig` still applies);
//! - declarative → passed explicitly (`--config <file>`), for every caller;
//! - code → formatting is skipped with a message, unless a root caller's
//!   project environment sets `OTTO_IDE_TRUST_FORMATTER_CONFIG=1`.
//!
//! SSH targets keep Prettier's own discovery for root (it runs on the remote
//! host as the SSH user) and `--no-config` for everyone else.

use std::path::{Path, PathBuf};

use crate::target::ResolvedTarget;

/// First argument of the planned prettier candidate; the caller's root-ness
/// follows the `=` (the plan needs it for SSH targets, which carry none).
pub(super) const PRETTIER_PLAN_ROOT: &str = "--otto-prettier-plan=root";
pub(super) const PRETTIER_PLAN_USER: &str = "--otto-prettier-plan=user";
/// The candidate that answers "skipped" when prettier was refused.
pub(super) const PRETTIER_SKIP_CMD: &str = "otto:prettier-skipped";
/// What the UI shows as the formatter when formatting was skipped.
pub(super) const PRETTIER_SKIPPED: &str =
    "Formatting skipped: this project's Prettier config runs code. \
     Allow it in the project's environment settings (OTTO_IDE_TRUST_FORMATTER_CONFIG=1)";

/// Searched per directory in Prettier's own order (`package.json` first).
const CONFIG_FILES: &[&str] = &[
    ".prettierrc",
    ".prettierrc.json",
    ".prettierrc.yaml",
    ".prettierrc.yml",
    ".prettierrc.json5",
    ".prettierrc.toml",
    ".prettierrc.js",
    ".prettierrc.cjs",
    ".prettierrc.mjs",
    ".prettierrc.ts",
    ".prettierrc.cts",
    ".prettierrc.mts",
    "prettier.config.js",
    "prettier.config.cjs",
    "prettier.config.mjs",
    "prettier.config.ts",
    "prettier.config.cts",
    "prettier.config.mts",
];
const MAX_CONFIG_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PrettierConfig {
    None,
    Declarative(PathBuf),
    Executable(PathBuf),
}

/// What to do with a prettier candidate.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum PrettierPlan {
    Run(Vec<String>),
    /// Don't run prettier; the next candidate decides (skip message or biome).
    Refuse,
}

#[allow(clippy::disallowed_methods)] // a few stats + one small read, on the format request
fn read_small(p: &Path) -> Option<String> {
    let meta = std::fs::metadata(p).ok()?;
    if !meta.is_file() || meta.len() > MAX_CONFIG_BYTES {
        return None;
    }
    std::fs::read_to_string(p).ok()
}

/// A config file that is data and loads nothing.
fn classify_file(p: &Path) -> PrettierConfig {
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    if name != ".prettierrc" && matches!(ext, "js" | "cjs" | "mjs" | "ts" | "cts" | "mts") {
        return PrettierConfig::Executable(p.to_path_buf());
    }
    // Unreadable or oversized: can't prove it is data.
    let Some(text) = read_small(p) else {
        return PrettierConfig::Executable(p.to_path_buf());
    };
    let body = text.trim();
    // `plugins` anywhere (top level or inside `overrides`) loads modules.
    let names_plugins = body.contains("plugins");
    // A bare string (JSON `"@acme/prettier-config"` or YAML `@acme/…`) names
    // a shared-config package: Node resolves and runs it.
    let shared_ref = !name.ends_with(".toml")
        && !body.is_empty()
        && (body.starts_with('"')
            || body.starts_with('\'')
            || !(body.contains('{') || body.contains(':')));
    if names_plugins || shared_ref {
        PrettierConfig::Executable(p.to_path_buf())
    } else {
        PrettierConfig::Declarative(p.to_path_buf())
    }
}

/// `package.json` is a config only when it has a `"prettier"` key.
fn classify_package_json(p: &Path) -> Option<PrettierConfig> {
    let text = read_small(p)?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let cfg = value.get("prettier")?;
    let declarative = cfg.is_object() && !cfg.to_string().contains("plugins");
    Some(if declarative {
        PrettierConfig::Declarative(p.to_path_buf())
    } else {
        PrettierConfig::Executable(p.to_path_buf())
    })
}

/// The nearest Prettier config for `file`, searching up to `root` (never
/// above it: a config outside the project is not the project's).
pub(super) fn resolve_prettier_config(file: &Path, root: &Path) -> PrettierConfig {
    let mut dir = file.parent();
    while let Some(d) = dir.filter(|d| d.starts_with(root)) {
        if let Some(cfg) = classify_package_json(&d.join("package.json")) {
            return cfg;
        }
        if read_small(&d.join("package.yaml")).is_some_and(|t| t.contains("prettier")) {
            return PrettierConfig::Executable(d.join("package.yaml"));
        }
        if let Some(p) = CONFIG_FILES.iter().map(|n| d.join(n)).find(|p| p.is_file()) {
            return classify_file(&p);
        }
        if d == root {
            break;
        }
        dir = d.parent();
    }
    PrettierConfig::None
}

/// The argv for a planned prettier candidate (`args` = `[PRETTIER_PLAN_*,
/// --stdin-filepath, <path>]`). `trusted` = the project environment opted in.
pub(super) fn plan_prettier_args(
    resolved: &ResolvedTarget,
    args: &[String],
    trusted: bool,
) -> PrettierPlan {
    let caller_is_root = args.first().is_some_and(|a| a == PRETTIER_PLAN_ROOT);
    let rest = &args[1.min(args.len())..];
    let file = rest.last().map(String::as_str).unwrap_or("");
    let with = |head: &[&str]| {
        let mut v: Vec<String> = head.iter().map(|s| s.to_string()).collect();
        v.extend(rest.iter().cloned());
        PrettierPlan::Run(v)
    };
    match resolved {
        ResolvedTarget::Ssh { .. } if caller_is_root => with(&[]),
        ResolvedTarget::Ssh { .. } => with(&["--no-config"]),
        ResolvedTarget::Local {
            root, is_root_user, ..
        } => match resolve_prettier_config(Path::new(file), root) {
            PrettierConfig::None => with(&["--no-config"]),
            PrettierConfig::Declarative(p) => {
                let cfg = p.to_string_lossy().into_owned();
                with(&["--config", cfg.as_str()])
            }
            PrettierConfig::Executable(_) if *is_root_user && trusted => with(&[]),
            PrettierConfig::Executable(_) => PrettierPlan::Refuse,
        },
    }
}

/// For [`PRETTIER_SKIP_CMD`]: true when prettier was refused because of a
/// code config (so "skipped" is the answer, not biome or the builtin).
pub(super) fn prettier_was_refused(resolved: &ResolvedTarget, file: &str, trusted: bool) -> bool {
    match resolved {
        ResolvedTarget::Local {
            root, is_root_user, ..
        } => {
            matches!(
                resolve_prettier_config(Path::new(file), root),
                PrettierConfig::Executable(_)
            ) && !(*is_root_user && trusted)
        }
        ResolvedTarget::Ssh { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(root: &Path, is_root_user: bool) -> ResolvedTarget {
        ResolvedTarget::Local {
            root: root.to_path_buf(),
            additional_roots: Vec::new(),
            readonly_roots: Vec::new(),
            is_root_user,
            actor_id: "u".into(),
        }
    }

    fn plan(root: &Path, file: &Path, is_root: bool, trusted: bool) -> PrettierPlan {
        let marker = if is_root {
            PRETTIER_PLAN_ROOT
        } else {
            PRETTIER_PLAN_USER
        };
        let args = vec![
            marker.to_string(),
            "--stdin-filepath".into(),
            file.to_string_lossy().into_owned(),
        ];
        plan_prettier_args(&local(root, is_root), &args, trusted)
    }

    fn project(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        for (name, body) in files {
            std::fs::write(root.join(name), body).unwrap();
        }
        let file = root.join("src/a.ts");
        (d, root, file)
    }

    #[test]
    fn a_declarative_config_is_passed_explicitly_for_every_caller() {
        for (name, body) in [
            (".prettierrc", "{ \"semi\": false }"),
            (".prettierrc", "semi: false\nsingleQuote: true\n"),
            (".prettierrc.toml", "semi = false\n"),
            (
                "package.json",
                "{\"name\":\"x\",\"prettier\":{\"semi\":false}}",
            ),
        ] {
            let (_d, root, file) = project(&[(name, body)]);
            for is_root in [true, false] {
                let PrettierPlan::Run(argv) = plan(&root, &file, is_root, false) else {
                    panic!("{name} refused");
                };
                let cfg = root.join(name).to_string_lossy().into_owned();
                assert_eq!(
                    argv,
                    vec![
                        "--config".to_string(),
                        cfg,
                        "--stdin-filepath".into(),
                        file.to_string_lossy().into_owned()
                    ],
                    "{name}"
                );
            }
        }
    }

    #[test]
    fn no_config_means_prettier_defaults() {
        let (_d, root, file) = project(&[("package.json", "{\"name\":\"x\"}")]);
        let PrettierPlan::Run(argv) = plan(&root, &file, false, false) else {
            panic!("refused");
        };
        assert_eq!(argv[0], "--no-config");
    }

    #[test]
    fn a_config_that_runs_code_is_skipped_unless_root_opted_in() {
        for (name, body) in [
            ("prettier.config.js", "module.exports = {}"),
            (".prettierrc.cjs", "module.exports = {}"),
            (".prettierrc", "{ \"plugins\": [\"./evil.js\"] }"),
            (
                ".prettierrc.json",
                "{ \"overrides\": [{\"files\":\"*\",\"options\":{\"plugins\":[\"x\"]}}] }",
            ),
            (".prettierrc", "\"@acme/prettier-config\""),
            (".prettierrc", "@acme/prettier-config\n"),
            ("package.json", "{\"prettier\":\"@acme/prettier-config\"}"),
        ] {
            let (_d, root, file) = project(&[(name, body)]);
            let f = file.to_string_lossy().into_owned();
            for is_root in [true, false] {
                assert_eq!(
                    plan(&root, &file, is_root, false),
                    PrettierPlan::Refuse,
                    "{name}"
                );
                assert!(prettier_was_refused(&local(&root, is_root), &f, false));
            }
            // The opt-in is root's: honoured for root, never for others.
            let PrettierPlan::Run(argv) = plan(&root, &file, true, true) else {
                panic!("{name}: root opt-in ignored");
            };
            assert_eq!(argv[0], "--stdin-filepath", "prettier's own discovery");
            assert_eq!(
                plan(&root, &file, false, true),
                PrettierPlan::Refuse,
                "{name}"
            );
            assert!(prettier_was_refused(&local(&root, false), &f, true));
            assert!(!prettier_was_refused(&local(&root, true), &f, true));
        }
        assert!(PRETTIER_SKIPPED.contains("OTTO_IDE_TRUST_FORMATTER_CONFIG=1"));
    }

    #[test]
    fn the_search_stops_at_the_project_root() {
        let outer = tempfile::tempdir().unwrap();
        let outer_root = outer.path().canonicalize().unwrap();
        std::fs::write(outer_root.join("prettier.config.js"), "module.exports={}").unwrap();
        let root = outer_root.join("repo");
        std::fs::create_dir_all(root.join("src")).unwrap();
        let file = root.join("src/a.ts");
        assert_eq!(resolve_prettier_config(&file, &root), PrettierConfig::None);
    }
}
