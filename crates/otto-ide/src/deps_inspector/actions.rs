//! Manifest dependency actions (`POST /ide/deps/action`) with automatic Local
//! History snapshots before and after mutation.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use otto_core::{Error, Result};
use regex::Regex;

use crate::local_history::LocalHistoryStore;
use crate::types::{IdeDepsActionReq, IdeDepsActionResp};

use super::{
    parse_cargo_toml, parse_go_mod, parse_gradle_dependencies, parse_package_json,
    parse_pom_dependencies, parse_python_deps,
};

pub async fn run_deps_action(
    canon_root: &Path,
    manifest_path: &Path,
    req: &IdeDepsActionReq,
    env_map: &BTreeMap<String, String>,
    history: &LocalHistoryStore,
) -> Result<IdeDepsActionResp> {
    if !manifest_path.is_file() {
        return Err(Error::NotFound(format!(
            "manifest '{}' does not exist",
            manifest_path.display()
        )));
    }
    let fname = manifest_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let dir = manifest_path.parent().unwrap_or(canon_root).to_path_buf();

    let raw_action = req.action.trim();
    let action = match raw_action {
        "update_single" | "upgrade" => "update_one",
        "update_all" => "update_all_latest",
        "update_direct" => "update_direct_latest",
        other => other,
    };
    let is_readonly_action = matches!(
        action,
        "why" | "maven_tree" | "sync" | "maven_sync" | "gradle_sync" | "vuln_scan"
    );
    if !is_readonly_action {
        let _ = history.record_existing_file_sync(
            canon_root,
            manifest_path,
            "before_deps_action",
            &format!("Before dependency action ({action})"),
            true,
        );
    }

    let (ok, output) = match (fname, action) {
        ("go.mod", "tidy") => exec_cmd("go", &["mod", "tidy"], &dir, env_map).await?,
        ("go.mod", "vendor") => exec_cmd("go", &["mod", "vendor"], &dir, env_map).await?,
        ("go.mod", "sync") => exec_cmd("go", &["mod", "download"], &dir, env_map).await?,
        ("go.mod", "why") => {
            let m = validate_dep_spec("module", req.module.as_deref().unwrap_or(""))?;
            exec_cmd("go", &["mod", "why", "-m", "--", m], &dir, env_map).await?
        }
        ("go.mod", "merge_directives") => {
            let raw = fs::read_to_string(manifest_path)
                .map_err(|e| Error::Internal(format!("read go.mod: {e}")))?;
            let merged = merge_go_mod_requires(&raw);
            fs::write(manifest_path, &merged)
                .map_err(|e| Error::Internal(format!("write go.mod: {e}")))?;
            (true, "Merged require directives".into())
        }
        ("go.mod", "update_one") => {
            let m = validate_dep_spec("module", req.module.as_deref().unwrap_or(""))?;
            let v = match req.version.as_deref().filter(|s| !s.trim().is_empty()) {
                Some(rv) => validate_dep_spec("version", rv)?,
                None => "latest",
            };
            // Perform instant deterministic in-place edit first when an exact version is given,
            // then run `go get` when non-isolated so `go.sum` stays in sync.
            if v != "latest" && v != "patch" {
                let raw = fs::read_to_string(manifest_path)
                    .map_err(|e| Error::Internal(format!("read go.mod: {e}")))?;
                if let Some(updated) = bump_go_mod_in_place(&raw, m, v) {
                    fs::write(manifest_path, updated)
                        .map_err(|e| Error::Internal(format!("write go.mod: {e}")))?;
                }
            }
            if !otto_core::e2e::isolated() {
                let spec = format!("{m}@{v}");
                let _ = exec_cmd("go", &["get", "--", &spec], &dir, env_map).await;
            }
            (true, format!("Updated {m} to {v}"))
        }
        (
            "go.mod",
            "update_direct_latest"
            | "update_direct_patch"
            | "update_all_latest"
            | "update_all_patch",
        ) => {
            let patch_only = action.ends_with("_patch");
            let direct_only = action.contains("_direct_");
            if direct_only {
                let raw = fs::read_to_string(manifest_path).unwrap_or_default();
                let direct_mods: Vec<String> = parse_go_mod(&raw)
                    .into_iter()
                    .filter(|d| !d.indirect && validate_dep_spec("module", &d.name).is_ok())
                    .map(|d| {
                        if patch_only {
                            format!("{}@patch", d.name)
                        } else {
                            format!("{}@latest", d.name)
                        }
                    })
                    .collect();
                if direct_mods.is_empty() {
                    (true, "No direct dependencies to update".into())
                } else {
                    let mut args: Vec<&str> = vec!["get", "--"];
                    for dm in &direct_mods {
                        args.push(dm.as_str());
                    }
                    exec_cmd("go", &args, &dir, env_map).await?
                }
            } else {
                let flag = if patch_only { "-u=patch" } else { "-u" };
                exec_cmd("go", &["get", flag, "./..."], &dir, env_map).await?
            }
        }
        ("pom.xml", "maven_sync" | "sync") => {
            exec_cmd("mvn", &["dependency:resolve", "-q"], &dir, env_map).await?
        }
        ("pom.xml", "maven_tree") => exec_cmd("mvn", &["dependency:tree"], &dir, env_map).await?,
        ("build.gradle" | "build.gradle.kts", "gradle_sync" | "maven_sync" | "sync") => {
            let bin = if dir.join("gradlew").is_file() {
                "./gradlew"
            } else {
                "gradle"
            };
            exec_cmd(
                bin,
                &["dependencies", "--configuration", "compileClasspath"],
                &dir,
                env_map,
            )
            .await?
        }
        (_, "vuln_scan") => {
            let inspect = super::inspect_dependencies(manifest_path, None, env_map).await;
            let cves: usize = inspect.items.iter().map(|i| i.vulnerabilities.len()).sum();
            (
                true,
                format!(
                    "Scanned {} dependencies ({cves} vulnerabilities)",
                    inspect.items.len()
                ),
            )
        }
        (
            _,
            "update_direct_latest"
            | "update_direct_patch"
            | "update_all_latest"
            | "update_all_patch",
        ) => {
            let patch_only = action.ends_with("_patch");
            let direct_only = action.contains("_direct_");
            let inspect = super::inspect_dependencies(manifest_path, None, env_map).await;
            let mut text = fs::read_to_string(manifest_path)
                .map_err(|e| Error::Internal(format!("read {}: {e}", manifest_path.display())))?;
            let mut count = 0usize;
            for item in &inspect.items {
                if direct_only && item.indirect {
                    continue;
                }
                let target_ver = if patch_only {
                    item.latest_patch.as_deref()
                } else {
                    item.latest_version.as_deref()
                };
                if let Some(v) = target_ver.filter(|v| *v != item.current_version) {
                    if let Some(next) =
                        bump_manifest_version_in_place(manifest_path, fname, &text, &item.name, v)
                    {
                        text = next;
                        count += 1;
                    }
                }
            }
            if count > 0 {
                fs::write(manifest_path, &text).map_err(|e| {
                    Error::Internal(format!("write {}: {e}", manifest_path.display()))
                })?;
            }
            (true, format!("Updated {count} dependencies"))
        }
        (_, "update_one") => {
            let m = validate_dep_spec("module", req.module.as_deref().unwrap_or(""))?;
            let v = validate_dep_spec("version", req.version.as_deref().unwrap_or(""))?;
            let raw = fs::read_to_string(manifest_path)
                .map_err(|e| Error::Internal(format!("read {}: {e}", manifest_path.display())))?;
            let updated = bump_manifest_version_in_place(manifest_path, fname, &raw, m, v)
                .ok_or_else(|| Error::Invalid(format!("dependency '{m}' not found in {fname}")))?;
            fs::write(manifest_path, &updated)
                .map_err(|e| Error::Internal(format!("write {}: {e}", manifest_path.display())))?;
            (true, format!("Updated {m} to {v}"))
        }
        _ => {
            return Err(Error::Invalid(format!(
                "unsupported dependency action '{action}' for '{fname}'"
            )));
        }
    };

    let updated_content = fs::read_to_string(manifest_path).ok();
    if !is_readonly_action {
        if let Some(text) = &updated_content {
            let _ = history.record_bytes_sync(
                canon_root,
                manifest_path,
                text.as_bytes(),
                "saved",
                &format!("Dependency action: {action}"),
                true,
            );
        }
    }

    Ok(IdeDepsActionResp {
        ok,
        updated_content,
        output,
    })
}

fn validate_dep_spec<'a>(label: &str, raw: &'a str) -> Result<&'a str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Error::Invalid(format!("{label} is required")));
    }
    if trimmed.starts_with('-')
        || trimmed
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '"' | '\''))
    {
        return Err(Error::Invalid(format!(
            "invalid {label} '{trimmed}': must not start with '-' or contain whitespace, quotes, or control characters"
        )));
    }
    Ok(trimmed)
}

async fn exec_cmd(
    bin: &str,
    args: &[&str],
    dir: &Path,
    env_map: &BTreeMap<String, String>,
) -> Result<(bool, String)> {
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(args).current_dir(dir).kill_on_drop(true);
    for (k, v) in env_map {
        cmd.env(k, v);
    }
    let out = tokio::time::timeout(Duration::from_secs(30), cmd.output())
        .await
        .map_err(|_| Error::Internal(format!("'{bin}' timed out after 30s")))?
        .map_err(|e| Error::Internal(format!("failed to run '{bin}': {e}")))?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok((out.status.success(), combined.trim().to_string()))
}

pub fn bump_go_mod_in_place(content: &str, module: &str, new_ver: &str) -> Option<String> {
    let items = parse_go_mod(content);
    let hit = items.iter().find(|i| i.name == module)?;
    Some(replace_line_col_span(
        content,
        hit.version_line,
        hit.version_col,
        hit.version_end_col,
        new_ver,
    ))
}

pub fn bump_manifest_version_in_place(
    manifest_path: &Path,
    fname: &str,
    content: &str,
    module: &str,
    new_ver: &str,
) -> Option<String> {
    match fname {
        "go.mod" => bump_go_mod_in_place(content, module, new_ver),
        "pom.xml" => {
            let items = parse_pom_dependencies(manifest_path, content);
            let hit = items
                .iter()
                .find(|i| i.name == module || i.name.ends_with(&format!(":{module}")))?;
            if let Some(prop) = &hit.property_name {
                // Update `<prop>old</prop>` inside `<properties>` preserving XML layout!
                let re = Regex::new(&format!(
                    r"(<{}>\s*)[^<]*?(\s*</{}>)",
                    regex::escape(prop),
                    regex::escape(prop)
                ))
                .ok()?;
                if re.is_match(content) {
                    return Some(
                        re.replace(content, format!("${{1}}{new_ver}${{2}}"))
                            .into_owned(),
                    );
                }
            }
            if hit.managed_in.is_some() || hit.current_version.is_empty() {
                // Insert explicit `<version>` line right after `<artifactId>`
                let lines: Vec<&str> = content.lines().collect();
                let art_idx = (hit.line as usize).saturating_sub(1);
                let art_line = *lines.get(art_idx)?;
                let indent: String = art_line.chars().take_while(|c| c.is_whitespace()).collect();
                let mut out_lines: Vec<String> = lines.iter().map(|s| (*s).to_string()).collect();
                out_lines.insert(art_idx + 1, format!("{indent}<version>{new_ver}</version>"));
                return Some(
                    out_lines.join("\n") + if content.ends_with('\n') { "\n" } else { "" },
                );
            }
            Some(replace_line_col_span(
                content,
                hit.version_line,
                hit.version_col,
                hit.version_end_col,
                new_ver,
            ))
        }
        "build.gradle" | "build.gradle.kts" => {
            let items = parse_gradle_dependencies(content);
            let hit = items.iter().find(|i| i.name == module)?;
            Some(replace_line_col_span(
                content,
                hit.version_line,
                hit.version_col,
                hit.version_end_col,
                new_ver,
            ))
        }
        "package.json" => {
            let items = parse_package_json(content);
            let hit = items.iter().find(|i| i.name == module)?;
            let line = content.lines().nth((hit.version_line - 1) as usize)?;
            let cur_slice: String = line
                .chars()
                .skip((hit.version_col - 1) as usize)
                .take((hit.version_end_col - hit.version_col) as usize)
                .collect();
            let prefix = if cur_slice.starts_with('^') {
                "^"
            } else if cur_slice.starts_with('~') {
                "~"
            } else {
                ""
            };
            Some(replace_line_col_span(
                content,
                hit.version_line,
                hit.version_col,
                hit.version_end_col,
                &format!("{prefix}{new_ver}"),
            ))
        }
        "Cargo.toml" => {
            let items = parse_cargo_toml(content);
            let hit = items.iter().find(|i| i.name == module)?;
            Some(replace_line_col_span(
                content,
                hit.version_line,
                hit.version_col,
                hit.version_end_col,
                new_ver,
            ))
        }
        "requirements.txt" | "pyproject.toml" => {
            let items = parse_python_deps(fname, content);
            let hit = items.iter().find(|i| i.name == module)?;
            Some(replace_line_col_span(
                content,
                hit.version_line,
                hit.version_col,
                hit.version_end_col,
                new_ver,
            ))
        }
        _ => None,
    }
}

fn replace_line_col_span(
    content: &str,
    line_1: u32,
    col_1: u32,
    end_col_1: u32,
    replacement: &str,
) -> String {
    let trailing_nl = content.ends_with('\n');
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    if let Some(target) = lines.get_mut((line_1 as usize).saturating_sub(1)) {
        let chars: Vec<char> = target.chars().collect();
        let s = (col_1 as usize).saturating_sub(1).min(chars.len());
        let e = (end_col_1 as usize).saturating_sub(1).min(chars.len());
        let mut rebuilt = String::new();
        rebuilt.extend(&chars[..s]);
        rebuilt.push_str(replacement);
        rebuilt.extend(&chars[e..]);
        *target = rebuilt;
    }
    let mut joined = lines.join("\n");
    if trailing_nl {
        joined.push('\n');
    }
    joined
}

fn merge_go_mod_requires(content: &str) -> String {
    let items = parse_go_mod(content);
    if items.is_empty() {
        return content.to_string();
    }
    let mut header_lines = Vec::new();
    let mut in_req = false;
    for line in content.lines() {
        let code_t = line.split_once("//").map_or(line, |(c, _)| c).trim();
        if let Some(rest) = code_t.strip_prefix("require") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) || rest.starts_with('(') {
                let after = rest.trim();
                if after.starts_with('(') && !after.contains(')') {
                    in_req = true;
                }
                continue;
            }
        }
        if in_req {
            if code_t.starts_with(')') || code_t.ends_with(')') {
                in_req = false;
            }
            continue;
        }
        header_lines.push(line.to_string());
    }
    while header_lines.last().is_some_and(|l| l.trim().is_empty()) {
        header_lines.pop();
    }
    let direct: Vec<_> = items.iter().filter(|d| !d.indirect).collect();
    let indirect: Vec<_> = items.iter().filter(|d| d.indirect).collect();
    if !direct.is_empty() {
        header_lines.push(String::new());
        header_lines.push("require (".into());
        for d in direct {
            header_lines.push(format!("\t{} {}", d.name, d.current_version));
        }
        header_lines.push(")".into());
    }
    if !indirect.is_empty() {
        header_lines.push(String::new());
        header_lines.push("require (".into());
        for d in indirect {
            header_lines.push(format!("\t{} {} // indirect", d.name, d.current_version));
        }
        header_lines.push(")".into());
    }
    header_lines.push(String::new());
    header_lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line_empty_package_json_deps_never_misparse_scripts() {
        let pkg = r#"{
  "name": "demo",
  "dependencies": {},
  "devDependencies": {},
  "scripts": {
    "test": "vitest",
    "build": "tsc"
  }
}
"#;
        let items = parse_package_json(pkg);
        assert!(
            items.is_empty(),
            "empty dependencies/devDependencies must not leak into scripts: {items:?}"
        );
    }

    #[test]
    fn go_mod_single_line_require_parens_preserve_replace_blocks_and_token_boundaries() {
        let go_mod = "module example.com/app\n\ngo 1.22\n\nrequire ()\n\nreplace (\n\texample.com/old => ../local_old\n)\n\nrequire (example.com/inline v1.2.0)\nrequire (\n\tgithub.com/foo/bar v1.0.0\n\tgithub.com/foo/bar-baz v2.0.0\n)\n";
        let items = parse_go_mod(go_mod);
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "example.com/inline",
                "github.com/foo/bar",
                "github.com/foo/bar-baz"
            ]
        );

        let bumped = bump_go_mod_in_place(go_mod, "github.com/foo/bar", "v1.1.0")
            .expect("bump exact module");
        assert!(bumped.contains("\tgithub.com/foo/bar v1.1.0\n"));
        assert!(bumped.contains("\tgithub.com/foo/bar-baz v2.0.0\n"));

        let merged = merge_go_mod_requires(go_mod);
        assert!(
            merged.contains("replace (\n\texample.com/old => ../local_old\n)"),
            "merge_go_mod_requires must preserve replace block after require (): {merged}"
        );
        assert!(merged.contains("\texample.com/inline v1.2.0"));
    }

    #[test]
    fn versionless_bom_pom_dependency_inserts_version_tag_after_artifact_id() {
        let pom = r#"<project>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.springframework.boot</groupId>
        <artifactId>spring-boot-dependencies</artifactId>
        <version>3.2.0</version>
        <type>pom</type>
        <scope>import</scope>
      </dependency>
    </dependencies>
  </dependencyManagement>
  <dependencies>
    <dependency>
      <groupId>org.springframework.boot</groupId>
      <artifactId>spring-boot-starter-web</artifactId>
    </dependency>
  </dependencies>
</project>
"#;
        let updated = bump_manifest_version_in_place(
            Path::new("/tmp/nonexistent/pom.xml"),
            "pom.xml",
            pom,
            "org.springframework.boot:spring-boot-starter-web",
            "3.3.1",
        )
        .expect("insert version tag");
        assert!(
            updated.contains(
                "      <artifactId>spring-boot-starter-web</artifactId>\n      <version>3.3.1</version>\n"
            ),
            "expected clean <version> insertion after <artifactId>, got:\n{updated}"
        );
        assert!(!updated.contains("3.3.1    <dependency>"));
    }

    #[tokio::test]
    async fn deps_action_rejects_flag_injection_and_whitespace_specs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let go_mod_path = root.join("go.mod");
        fs::write(&go_mod_path, "module example.com/app\n\ngo 1.22\n").unwrap();
        let history = LocalHistoryStore::new(root.join(".history"));

        for (action, bad_mod, bad_ver) in [
            ("why", Some("-toolexec=/bin/sh"), None),
            ("update_one", Some("-modfile=evil.mod"), Some("v1.0.0")),
            ("update_one", Some("example.com/pkg"), Some("-u=evil")),
            ("update_one", Some("example.com/pkg extra"), Some("v1.0.0")),
        ] {
            let req = IdeDepsActionReq {
                target: None,
                path: go_mod_path.to_string_lossy().into_owned(),
                action: action.into(),
                module: bad_mod.map(str::to_string),
                version: bad_ver.map(str::to_string),
            };
            let err = run_deps_action(&root, &go_mod_path, &req, &BTreeMap::new(), &history)
                .await
                .expect_err("flag or whitespace injection must be rejected");
            assert!(
                matches!(err, Error::Invalid(_)),
                "expected Error::Invalid, got {err:?}"
            );
        }
    }
}
