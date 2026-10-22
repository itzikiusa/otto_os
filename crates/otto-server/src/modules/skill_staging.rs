//! Staging bundled skill packages (SKILL.md + support files) into a temp root
//! that review / docs / workflow agents read their skills from.

/// Materialize complete skill packages into `bundle`, with two views:
/// `.claude/skills/<name>/` for Claude's first-class `--add-dir` loader and
/// `skills/<name>/` for provider-neutral, explicit file reads from prompts.
/// References, scripts, examples, assets, and eval metadata travel with
/// `SKILL.md`; no provider is limited to a lossy body-only copy.
///
/// Why a dedicated bundle (not the repo cwd): review sessions deliberately skip
/// context materialization (see `SessionManager` spawn), and a reviewed repo
/// usually has no `.claude/skills` of its own, so `Skill(<lens>)` otherwise errors
/// "Unknown skill". `--add-dir=<dir>` loading `<dir>/.claude/skills` is verified
/// against the live CLI. The bundle holds ONLY review skills — never the data dir
/// (secrets/DB) — so add-dir grants no sensitive access.
///
/// Sources, per skill: the Otto Library, the operator's global Claude skills,
/// then the compiled-in `otto-skills` tree. Re-copied cleanly so edits and
/// bundled upgrades propagate. Best-effort: unknown skills are skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StagedSkillPackages {
    pub(crate) root: String,
    pub(crate) files: std::collections::HashMap<String, Vec<String>>,
}

pub(crate) fn stage_skill_packages_at(
    library: &otto_context::Library,
    names: &[String],
    bundle: &std::path::Path,
) -> Option<StagedSkillPackages> {
    use std::path::Path;
    let neutral_root = bundle.join("skills");
    let claude_root = bundle.join(".claude").join("skills");
    let mut files = std::collections::HashMap::new();
    let mut seen = std::collections::HashSet::<&str>::new();
    for name in names {
        // Path::join replaces its base for absolute paths. Validate before any
        // destination join or cleanup so a name can never escape `bundle`.
        if !is_safe_skill_package_name(name) || !seen.insert(name.as_str()) {
            continue;
        }
        // Resolve the skill's on-disk source dir: Library first (skill_path()
        // rejects unsafe names), then the operator's global Claude skills dir.
        let src = library
            .skill_path(name)
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .filter(|d| d.is_dir())
            .or_else(|| {
                if name.contains(['/', '\\']) || name.contains("..") {
                    return None;
                }
                let home = std::env::var_os("HOME")?;
                let d = Path::new(&home).join(".claude/skills").join(name);
                d.is_dir().then_some(d)
            });
        let neutral = neutral_root.join(name);
        if let Err(e) = remove_staged_package_path(&neutral) {
            tracing::warn!(skill = %name, "stage_skill_packages: neutral cleanup failed: {e}");
            continue;
        }
        let copied = match src {
            Some(src) => crate::plugins::copy_dir(&src, &neutral).map(|_| true),
            None => otto_skills::copy_bundled_into(name, &neutral).map_err(|e| e.to_string()),
        };
        match copied {
            Ok(true) => {}
            Ok(false) => {
                let _ = remove_staged_package_path(&neutral);
                continue;
            }
            Err(e) => {
                let _ = remove_staged_package_path(&neutral);
                tracing::warn!(skill = %name, "stage_skill_packages: copy failed: {e}");
                continue;
            }
        }
        let claude = claude_root.join(name);
        if let Err(e) = remove_staged_package_path(&claude) {
            let _ = remove_staged_package_path(&neutral);
            tracing::warn!(skill = %name, "stage_skill_packages: claude cleanup failed: {e}");
            continue;
        }
        if let Err(e) = crate::plugins::copy_dir(&neutral, &claude) {
            let _ = remove_staged_package_path(&neutral);
            let _ = remove_staged_package_path(&claude);
            tracing::warn!(skill = %name, "stage_skill_packages: claude view failed: {e}");
            continue;
        }
        let mut package_files = Vec::new();
        collect_staged_package_files(&neutral, &neutral, &mut package_files);
        package_files.sort();
        package_files.dedup();
        files.insert(name.clone(), package_files);
    }
    (!files.is_empty()).then(|| StagedSkillPackages {
        root: bundle.to_string_lossy().into_owned(),
        files,
    })
}

pub(crate) fn is_safe_skill_package_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[allow(clippy::disallowed_methods)] // pre-existing sync fs reached from async code without offload (perf2 N3 follow-up)
pub(crate) fn remove_staged_package_path(
    path: &std::path::Path,
) -> std::result::Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {
            std::fs::remove_dir_all(path).map_err(|e| format!("remove {}: {e}", path.display()))
        }
        Ok(_) => std::fs::remove_file(path).map_err(|e| format!("remove {}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("inspect {}: {e}", path.display())),
    }
}

#[allow(clippy::disallowed_methods)] // pre-existing sync fs reached from async code without offload (perf2 N3 follow-up)
pub(crate) fn collect_staged_package_files(
    dir: &std::path::Path,
    root: &std::path::Path,
    out: &mut Vec<String>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            collect_staged_package_files(&path, root, out);
        } else if kind.is_file() {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}

/// Backward-compatible shared bundle for the PR/skill review engine. Vault
/// docs runs use [`stage_skill_packages_at`] with a run-specific directory.
pub(crate) fn stage_review_skills(
    library: &otto_context::Library,
    names: &[String],
) -> Option<String> {
    let bundle = otto_context::materialize::default_context_root().join("review-skills");
    stage_skill_packages_at(library, names, &bundle).map(|staged| staged.root)
}

/// Stage the WHOLE library for a review run, on the blocking pool: it lists
/// every skill and copies each tree twice (hundreds of file ops), which ran
/// inline on the worker driving the review — once per run and per retry.
pub(crate) async fn stage_all_review_skills(library: &otto_context::Library) -> Option<String> {
    let library = library.clone();
    tokio::task::spawn_blocking(move || {
        let names: Vec<String> = library.list_skills().into_iter().map(|s| s.name).collect();
        stage_review_skills(&library, &names)
    })
    .await
    .ok()
    .flatten()
}

#[cfg(test)]
mod staged_skill_package_tests {
    use super::stage_skill_packages_at;

    #[test]
    fn bundled_fallback_materializes_native_and_neutral_views() {
        let tmp = tempfile::tempdir().unwrap();
        let library = otto_context::Library::new(tmp.path().join("library"));
        let bundle = tmp.path().join("bundle");
        let staged =
            stage_skill_packages_at(&library, &["skills-reviewer".to_string()], &bundle).unwrap();
        assert_eq!(staged.root, bundle.to_string_lossy());
        assert!(staged.files["skills-reviewer"]
            .iter()
            .any(|path| path == "SKILL.md"));
        assert!(staged.files["skills-reviewer"]
            .iter()
            .any(|path| path == "references/review-rubric.md"));
        for root in [bundle.join("skills"), bundle.join(".claude/skills")] {
            assert!(root.join("skills-reviewer/SKILL.md").is_file());
            assert!(root
                .join("skills-reviewer/references/review-rubric.md")
                .is_file());
            assert!(root
                .join("skills-reviewer/scripts/skill_review.py")
                .is_file());
        }
    }

    #[test]
    fn library_package_takes_precedence_over_bundled_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let library_root = tmp.path().join("library");
        let library_skill = library_root.join("skills/skills-reviewer");
        std::fs::create_dir_all(library_skill.join("references")).unwrap();
        std::fs::write(library_skill.join("SKILL.md"), "LIBRARY COPY").unwrap();
        std::fs::write(library_skill.join("references/local.md"), "local").unwrap();
        let library = otto_context::Library::new(&library_root);
        let bundle = tmp.path().join("bundle");

        let staged =
            stage_skill_packages_at(&library, &["skills-reviewer".to_string()], &bundle).unwrap();

        assert_eq!(
            std::fs::read_to_string(bundle.join("skills/skills-reviewer/SKILL.md")).unwrap(),
            "LIBRARY COPY"
        );
        assert_eq!(
            staged.files["skills-reviewer"],
            vec!["SKILL.md".to_string(), "references/local.md".to_string()]
        );
    }

    #[test]
    fn partial_staging_reports_each_successful_package() {
        let tmp = tempfile::tempdir().unwrap();
        let library_root = tmp.path().join("library");
        let broken = library_root.join("skills/broken-skill");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("SKILL.md"), "BROKEN FALLBACK BODY").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(broken.join("missing"), broken.join("dangling")).unwrap();
        let library = otto_context::Library::new(library_root);
        let bundle = tmp.path().join("bundle");
        let staged = stage_skill_packages_at(
            &library,
            &["skills-reviewer".to_string(), "broken-skill".to_string()],
            &bundle,
        )
        .unwrap();

        assert!(staged.files.contains_key("skills-reviewer"));
        #[cfg(unix)]
        {
            assert!(!staged.files.contains_key("broken-skill"));
            assert!(!bundle.join("skills/broken-skill").exists());
            assert!(!bundle.join(".claude/skills/broken-skill").exists());
        }
    }

    #[test]
    fn unsafe_names_never_escape_or_remove_outside_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let library = otto_context::Library::new(tmp.path().join("library"));
        let bundle = tmp.path().join("bundle");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let sentinel = outside.join("keep.txt");
        std::fs::write(&sentinel, "keep").unwrap();

        let names = vec![
            outside.to_string_lossy().into_owned(),
            "../outside".to_string(),
            "safe/../../outside".to_string(),
        ];
        assert!(stage_skill_packages_at(&library, &names, &bundle).is_none());
        assert_eq!(std::fs::read_to_string(sentinel).unwrap(), "keep");
    }
}
