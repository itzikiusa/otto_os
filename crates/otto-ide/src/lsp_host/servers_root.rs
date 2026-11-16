//! Lightweight format filtering and module-root resolution for LSP servers.
//!
//! Ensures language servers never spawn for lightweight data/document files
//! (`markdown`, `csv`, `sql`, `json`, `yaml`, `toml`) and never launch at an
//! umbrella directory containing dozens of independent repositories.

use std::path::{Path, PathBuf};

use super::servers::normalize_lsp_lang;

/// Returns `true` when `lang` is handled by Otto's built-in fast static analyzer
/// and must never spawn an external LSP child process.
pub fn is_lightweight_non_lsp_lang(lang: &str) -> bool {
    let lower = lang.trim().to_ascii_lowercase();
    let norm = normalize_lsp_lang(&lower);
    matches!(
        norm,
        "markdown"
            | "csv"
            | "tsv"
            | "sql"
            | "json"
            | "yaml"
            | "toml"
            | "xml"
            | "ini"
            | "env"
            | "dotenv"
            | "plaintext"
            | "text"
            | "txt"
            | "log"
            | "diff"
            | "dockerfile"
            | "proto"
            | "protobuf"
    )
}

fn manifest_names_for_lang(norm_lang: &str) -> &'static [&'static str] {
    match norm_lang {
        "go" => &["go.work", "go.mod"],
        "java" | "kotlin" => &[
            "pom.xml",
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
        ],
        "rust" => &["Cargo.toml"],
        "typescript" | "javascript" => &["tsconfig.json", "jsconfig.json", "package.json"],
        "python" => &[
            "pyproject.toml",
            "setup.py",
            "setup.cfg",
            "requirements.txt",
            "Pipfile",
        ],
        "protobuf" | "proto" => &["buf.work.yaml", "buf.yaml"],
        "graphql" => &[
            "graphql.config.json",
            "graphql.config.yml",
            "graphql.config.yaml",
            ".graphqlrc",
            ".graphqlrc.json",
            ".graphqlrc.yml",
            ".graphqlrc.yaml",
            "package.json",
        ],
        "makefile" => &["Makefile", "GNUmakefile", "makefile"],
        "dockerfile" => &["docker-compose.yml", "docker-compose.yaml", "compose.yaml"],
        _ => &[],
    }
}

fn dir_has_any_manifest(dir: &Path, names: &[&str], extra_markers: &[String]) -> bool {
    names
        .iter()
        .any(|m| dir.join(m).is_file() || dir.join(m).is_dir())
        || extra_markers
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty() && !s.contains('/') && !s.contains('\\') && *s != "..")
            .any(|m| dir.join(m).exists())
}

fn dir_looks_like_child_repo(dir: &Path) -> bool {
    if dir.join(".git").exists() {
        return true;
    }
    [
        "go.mod",
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
    ]
    .iter()
    .any(|m| dir.join(m).is_file())
}

/// Detect whether `dir` is a multi-repository parent umbrella folder (no own
/// `.git` or build manifest, with `>= 2` immediate child repository directories).
#[allow(clippy::disallowed_methods)]
pub fn is_multi_repo_umbrella_dir(dir: &Path) -> bool {
    if !dir.is_dir() || dir.join(".git").exists() {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let mut child_repos = 0usize;
    for entry in entries.flatten().take(96) {
        let p = entry.path();
        if p.is_dir() && dir_looks_like_child_repo(&p) {
            child_repos += 1;
            if child_repos >= 2 {
                return true;
            }
        }
    }
    false
}

#[allow(clippy::disallowed_methods)]
fn canonical_or_existing_ancestor(path: &Path) -> PathBuf {
    if let Ok(c) = std::fs::canonicalize(path) {
        return c;
    }
    let mut cur = path;
    let mut tail = Vec::new();
    while let Some(parent) = cur.parent() {
        if let Some(name) = cur.file_name() {
            tail.push(name.to_os_string());
        }
        if let Ok(mut base) = std::fs::canonicalize(parent) {
            for seg in tail.into_iter().rev() {
                base.push(seg);
            }
            return base;
        }
        cur = parent;
    }
    path.to_path_buf()
}

/// From a module dir, climb to the build root one server should own: a Maven
/// reactor whose `pom.xml` lists the child in `<modules>`, a Gradle build with
/// `settings.gradle*`, or the Cargo `[workspace]` that holds the member.
#[allow(clippy::disallowed_methods)]
pub(crate) fn climb_to_build_root(
    norm_lang: &str,
    module: &Path,
    allowed: &dyn Fn(&Path) -> bool,
) -> PathBuf {
    let read = |p: PathBuf| std::fs::read_to_string(p).unwrap_or_default();
    let has_settings_gradle =
        |d: &Path| d.join("settings.gradle").is_file() || d.join("settings.gradle.kts").is_file();
    let is_cargo_workspace = |d: &Path| {
        read(d.join("Cargo.toml"))
            .lines()
            .any(|l| l.trim() == "[workspace]")
    };
    if norm_lang == "rust" && is_cargo_workspace(module) {
        return module.to_path_buf();
    }
    let mut top = module.to_path_buf();
    let mut cur = module.to_path_buf();
    for _ in 0..16 {
        let Some(parent) = cur.parent().map(Path::to_path_buf) else {
            break;
        };
        if !allowed(&parent) {
            break;
        }
        let climb = match norm_lang {
            "java" | "kotlin" => {
                let lists_child = read(parent.join("pom.xml"))
                    .split("<module>")
                    .skip(1)
                    .filter_map(|s| s.split("</module>").next())
                    .any(|m| {
                        std::fs::canonicalize(parent.join(m.trim())).ok().as_ref() == Some(&cur)
                    });
                lists_child || (has_settings_gradle(&parent) && !has_settings_gradle(&cur))
            }
            // Cargo binds a member to its nearest `[workspace]` ancestor.
            "rust" if is_cargo_workspace(&parent) => return parent,
            "rust" => true,
            _ => false,
        };
        if !climb {
            break;
        }
        if norm_lang != "rust" {
            top = parent.clone();
        }
        cur = parent;
    }
    top
}

/// Scope an LSP root strictly to the nearest module manifest directory for `lang`
/// and refuse spawning on a multi-repository umbrella folder.
pub fn resolve_lsp_module_root(
    lang: &str,
    raw_root: &str,
    file_hint: Option<&str>,
    allowed_roots: &[PathBuf],
) -> std::io::Result<String> {
    resolve_lsp_module_root_with_markers(lang, raw_root, file_hint, allowed_roots, &[])
}

/// Scope an LSP root strictly to the nearest module manifest (built-in or plugin
/// `extra_markers`), falling back to the nearest `.git` root or `canon_root` for
/// custom plugin languages so leaf subdirectories do not spawn duplicate servers.
#[allow(clippy::disallowed_methods)]
pub fn resolve_lsp_module_root_with_markers(
    lang: &str,
    raw_root: &str,
    file_hint: Option<&str>,
    allowed_roots: &[PathBuf],
    extra_markers: &[String],
) -> std::io::Result<String> {
    let trimmed = raw_root.trim();
    if trimmed.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "LSP root path must not be empty",
        ));
    }
    let expanded_root = PathBuf::from(otto_core::paths::expand_tilde(trimmed));
    let root_path = expanded_root.as_path();
    if !root_path.exists() {
        // Preserve synthetic in-memory test roots (`/r`, `/repo-lru-test`).
        let expanded_str = root_path.to_string_lossy();
        return Ok(expanded_str.trim_end_matches('/').to_string());
    }
    let canon_root = std::fs::canonicalize(root_path).unwrap_or_else(|_| root_path.to_path_buf());
    let start_dir = if let Some(hint) = file_hint.map(str::trim).filter(|s| !s.is_empty()) {
        let hp = PathBuf::from(otto_core::paths::expand_tilde(hint));
        let full = if hp.is_absolute() {
            hp
        } else {
            canon_root.join(hp)
        };
        let resolved = canonical_or_existing_ancestor(&full);
        if resolved.is_dir() {
            resolved
        } else {
            resolved
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| canon_root.clone())
        }
    } else if canon_root.is_file() {
        canon_root
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| canon_root.clone())
    } else {
        canon_root.clone()
    };

    if otto_core::paths::is_broad_non_project_root(&start_dir) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "refusing to spawn language server on a broad non-project directory",
        ));
    }

    let within_allowed = |cand: &Path| -> bool {
        if otto_core::paths::is_broad_non_project_root(cand) {
            return false;
        }
        if allowed_roots.is_empty() {
            let home = dirs::home_dir();
            cand.parent().is_some() && home.as_deref() != Some(cand)
        } else {
            allowed_roots.iter().any(|r| cand.starts_with(r))
        }
    };

    let lower_lang = lang.trim().to_ascii_lowercase();
    let norm_lang = normalize_lsp_lang(&lower_lang);
    let manifests = manifest_names_for_lang(norm_lang);
    if !manifests.is_empty() || !extra_markers.is_empty() {
        let mut cur = start_dir.as_path();
        for _ in 0..16 {
            if !within_allowed(cur) {
                break;
            }
            if dir_has_any_manifest(cur, manifests, extra_markers) {
                let top = climb_to_build_root(norm_lang, cur, &within_allowed);
                return Ok(top.to_string_lossy().into_owned());
            }
            let Some(parent) = cur.parent() else {
                break;
            };
            cur = parent;
        }
    }

    // No language manifest matched: walk up for `.git` first, otherwise fall back
    // to `canon_root` (when `start_dir` is nested inside a non-umbrella `canon_root`)
    // so custom plugin LSP servers bind to the workspace root rather than every leaf dir.
    {
        let mut cur = start_dir.as_path();
        for _ in 0..16 {
            if !within_allowed(cur) {
                break;
            }
            if cur.join(".git").exists() {
                return Ok(cur.to_string_lossy().into_owned());
            }
            let Some(parent) = cur.parent() else {
                break;
            };
            cur = parent;
        }
    }

    if start_dir != canon_root
        && start_dir.starts_with(&canon_root)
        && canon_root.is_dir()
        && within_allowed(&canon_root)
        && !is_multi_repo_umbrella_dir(&canon_root)
    {
        return Ok(canon_root.to_string_lossy().into_owned());
    }

    if !dir_has_any_manifest(&start_dir, manifests, extra_markers)
        && is_multi_repo_umbrella_dir(&start_dir)
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "refusing to spawn language server on a multi-repository parent folder; open a file inside a specific module",
        ));
    }

    Ok(start_dir.to_string_lossy().into_owned())
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::super::servers::*;
    use super::*;
    use crate::plugins::IdePluginRegistry;

    #[test]
    fn finds_binaries_under_toolchains_and_skips_lightweight_non_lsp_formats() {
        let tmp = tempfile::tempdir().unwrap();
        let go_bin = tmp.path().join("toolchains/go/1.23.0/bin");
        std::fs::create_dir_all(&go_bin).unwrap();
        let fake_gopls = go_bin.join("gopls");
        std::fs::write(&fake_gopls, "#!/bin/sh\nexit 0\n").unwrap();

        let bins = collect_toolchain_bin_dirs(tmp.path());
        assert!(bins.contains(&go_bin));

        let reg = IdePluginRegistry::with_dirs(tmp.path().to_path_buf(), None);
        let resolved = resolve_lang_with_ide("go", tmp.path(), &reg).unwrap();
        assert!(resolved.available);
        assert_eq!(resolved.command, fake_gopls.to_string_lossy());

        assert_eq!(normalize_lsp_lang("bash"), "shell");
        assert!(resolve_remote_lang("bash", &reg).is_some());

        for non_lsp in [
            "markdown", "md", "csv", "sql", "json", "jsonc", "jsonl", "yaml", "toml",
        ] {
            assert!(is_lightweight_non_lsp_lang(non_lsp), "{non_lsp}");
            assert!(resolve_lang(non_lsp).is_none(), "{non_lsp}");
            assert!(
                resolve_lang_with_ide(non_lsp, tmp.path(), &reg).is_none(),
                "{non_lsp}"
            );
            assert!(resolve_remote_lang(non_lsp, &reg).is_none(), "{non_lsp}");
        }
    }

    #[test]
    fn includes_custom_lsp_from_enabled_ide_plugin() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join("ide-plugins/kotlin-ide");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        let fake_bin_dir = tmp.path().join("toolchains/kotlin/bin");
        std::fs::create_dir_all(&fake_bin_dir).unwrap();
        let fake_kls = fake_bin_dir.join("kotlin-language-server");
        std::fs::write(&fake_kls, "#!/bin/sh\nexit 0\n").unwrap();

        std::fs::write(
            plugin_dir.join("otto-ide-plugin.json"),
            r#"{
                "slug": "kotlin-ide",
                "name": "Kotlin IDE",
                "version": "1.0.0",
                "contributions": {
                    "lsp_servers": [{
                        "language": "kotlin",
                        "command": "kotlin-language-server",
                        "args": [],
                        "extensions": [".kt", ".kts"],
                        "install_hint": "brew install kotlin-language-server"
                    }]
                }
            }"#,
        )
        .unwrap();

        let reg = IdePluginRegistry::with_dirs(tmp.path().to_path_buf(), None);
        assert!(
            resolve_lang_with_ide("kotlin", tmp.path(), &reg).is_none(),
            "unrecorded on-disk plugin directory must remain disabled until explicitly toggled"
        );
        reg.toggle("kotlin-ide", true).unwrap();
        let resolved = resolve_lang_with_ide("kotlin", tmp.path(), &reg).unwrap();
        assert!(resolved.available);
        assert_eq!(resolved.command, fake_kls.to_string_lossy());
    }

    #[test]
    fn plugin_lsp_overrides_lightweight_filter_and_falls_back_to_canon_root() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join("ide-plugins/docker-lsp");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        let fake_bin_dir = tmp.path().join("toolchains/docker/bin");
        std::fs::create_dir_all(&fake_bin_dir).unwrap();
        let fake_dls = fake_bin_dir.join("docker-langserver");
        std::fs::write(&fake_dls, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::write(
            plugin_dir.join("otto-ide-plugin.json"),
            r#"{
                "slug": "docker-lsp",
                "name": "Docker LSP",
                "version": "1.0.0",
                "contributions": {
                    "lsp_servers": [{
                        "language": "dockerfile",
                        "command": "docker-langserver",
                        "args": ["--stdio"],
                        "extensions": ["Dockerfile"]
                    }]
                }
            }"#,
        )
        .unwrap();

        let reg = IdePluginRegistry::with_dirs(tmp.path().to_path_buf(), None);
        assert!(resolve_lang_with_ide("dockerfile", tmp.path(), &reg).is_none());
        reg.toggle("docker-lsp", true).unwrap();
        let resolved = resolve_lang_with_ide("dockerfile", tmp.path(), &reg).unwrap();
        assert!(resolved.available);
        assert_eq!(resolved.command, fake_dls.to_string_lossy());

        let ws = tmp.path().join("cue-workspace");
        let nested = ws.join("pkg/policy/sub");
        std::fs::create_dir_all(&nested).unwrap();
        let canon_ws = std::fs::canonicalize(&ws).unwrap();
        let scoped = resolve_lsp_module_root(
            "cue",
            &ws.to_string_lossy(),
            Some(&nested.join("rules.cue").to_string_lossy()),
            &[],
        )
        .unwrap();
        assert_eq!(PathBuf::from(scoped), canon_ws);
    }

    #[test]
    fn module_root_scoping_resolves_nested_dir_and_rejects_umbrella_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let umbrella = std::fs::canonicalize(tmp.path()).unwrap();
        let repo_a = umbrella.join("repo-a");
        let repo_b = umbrella.join("repo-b");
        std::fs::create_dir_all(repo_a.join(".git")).unwrap();
        std::fs::create_dir_all(repo_a.join("internal/handler")).unwrap();
        std::fs::write(repo_a.join("go.mod"), "module example.com/a\n").unwrap();
        std::fs::create_dir_all(repo_b.join(".git")).unwrap();
        std::fs::write(repo_b.join("go.mod"), "module example.com/b\n").unwrap();

        let err = resolve_lsp_module_root("go", &umbrella.to_string_lossy(), None, &[])
            .expect_err("umbrella folder with multiple child repos must be rejected");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);

        let scoped = resolve_lsp_module_root(
            "go",
            &repo_a.join("internal/handler").to_string_lossy(),
            None,
            &[umbrella],
        )
        .unwrap();
        assert_eq!(PathBuf::from(scoped), repo_a);
    }

    #[test]
    fn file_hint_outside_primary_root_resolves_sibling_go_module() {
        let tmp = tempfile::tempdir().unwrap();
        let umbrella = std::fs::canonicalize(tmp.path()).unwrap();
        let evoplay = umbrella.join("koala-evoplay-go");
        let rebeltech = umbrella.join("koala-rebeltech-go");
        std::fs::create_dir_all(evoplay.join("app")).unwrap();
        std::fs::write(evoplay.join("go.mod"), "module example.com/evoplay\n").unwrap();
        std::fs::create_dir_all(rebeltech.join("app")).unwrap();
        std::fs::write(rebeltech.join("go.mod"), "module example.com/rebeltech\n").unwrap();
        let rebel_file = rebeltech.join("app/rebeltech_controller.go");
        std::fs::write(&rebel_file, "package app\n").unwrap();

        // Opening a file in sibling `koala-rebeltech-go` while `raw_root` points to
        // `koala-evoplay-go` (or the umbrella dir) walks up from `file_hint` to
        // `koala-rebeltech-go` rather than attaching to `koala-evoplay-go`'s gopls.
        let resolved_from_sibling = resolve_lsp_module_root(
            "go",
            &evoplay.to_string_lossy(),
            Some(&rebel_file.to_string_lossy()),
            &[],
        )
        .unwrap();
        assert_eq!(PathBuf::from(resolved_from_sibling), rebeltech);

        let resolved_from_umbrella = resolve_lsp_module_root(
            "go",
            &umbrella.to_string_lossy(),
            Some(&rebel_file.to_string_lossy()),
            &[],
        )
        .unwrap();
        assert_eq!(PathBuf::from(resolved_from_umbrella), rebeltech);

        // Even an unsaved file path inside `koala-rebeltech-go/app` resolves to `koala-rebeltech-go`.
        let unsaved = rebeltech.join("app/unsaved_handler.go");
        let resolved_unsaved = resolve_lsp_module_root(
            "go",
            &evoplay.to_string_lossy(),
            Some(&unsaved.to_string_lossy()),
            &[],
        )
        .unwrap();
        assert_eq!(PathBuf::from(resolved_unsaved), rebeltech);
    }

    #[test]
    fn jvm_and_cargo_roots_climb_to_the_reactor_and_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let top = std::fs::canonicalize(tmp.path()).unwrap();
        let w = |p: &str, body: &str| {
            let f = top.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, body).unwrap();
        };
        w(
            "mvn/pom.xml",
            "<project><modules>\n<module>svc</module>\n</modules></project>",
        );
        w(
            "mvn/svc/pom.xml",
            "<project><modules><module>core</module></modules></project>",
        );
        w("mvn/svc/core/pom.xml", "<project/>");
        w("mvn/svc/core/src/main/java/A.java", "class A {}");
        // A pom next door that does NOT list `lone` keeps it standalone.
        w(
            "lone-parent/pom.xml",
            "<project><modules><module>other</module></modules></project>",
        );
        w("lone-parent/lone/pom.xml", "<project/>");
        w(
            "cargo/Cargo.toml",
            "[workspace]\nmembers = [\"crates/*\"]\n",
        );
        w("cargo/crates/app/Cargo.toml", "[package]\nname = \"app\"\n");
        w("cargo/crates/app/src/lib.rs", "");
        w("solo/Cargo.toml", "[package]\nname = \"solo\"\n");
        w("gradle/settings.gradle.kts", "include(\"core\")");
        w("gradle/core/build.gradle.kts", "");

        let root = |lang: &str, file: &str| {
            let f = top.join(file);
            PathBuf::from(
                resolve_lsp_module_root(
                    lang,
                    &f.to_string_lossy(),
                    Some(&f.to_string_lossy()),
                    &[top.clone()],
                )
                .unwrap(),
            )
        };
        assert_eq!(
            root("java", "mvn/svc/core/src/main/java/A.java"),
            top.join("mvn")
        );
        assert_eq!(
            root("java", "lone-parent/lone/pom.xml"),
            top.join("lone-parent/lone")
        );
        assert_eq!(
            root("rust", "cargo/crates/app/src/lib.rs"),
            top.join("cargo")
        );
        assert_eq!(root("rust", "solo/Cargo.toml"), top.join("solo"));
        assert_eq!(
            root("kotlin", "gradle/core/build.gradle.kts"),
            top.join("gradle")
        );
        // Never above the allowed root.
        let core = top.join("mvn/svc/core");
        let scoped = resolve_lsp_module_root(
            "java",
            &core.to_string_lossy(),
            None,
            &[top.join("mvn/svc")],
        )
        .unwrap();
        assert_eq!(PathBuf::from(scoped), top.join("mvn/svc"));
    }
}
