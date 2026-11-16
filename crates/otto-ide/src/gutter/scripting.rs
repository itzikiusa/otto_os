//! Gutter Run & Debug detectors for scripting languages, JSON task maps, and plugins.

use std::sync::OnceLock;

use regex::Regex;

use crate::types::{
    IdeGutterItem, IdeGutterKind, IdeQuickRunSpec, IdeRunnerKind, PluginGutterDetector,
};

// ---------------------------------------------------------------------------
// 4. Python (`if __name__ == "__main__":`, `class Test*`, `def test_*`)
// ---------------------------------------------------------------------------

pub(super) fn detect_python(path: &str, dir: &str, content: &str, out: &mut Vec<IdeGutterItem>) {
    static RE_MAIN: OnceLock<Regex> = OnceLock::new();
    static RE_CLASS: OnceLock<Regex> = OnceLock::new();
    static RE_DEF: OnceLock<Regex> = OnceLock::new();

    let re_main = RE_MAIN
        .get_or_init(|| Regex::new(r#"^\s*if\s+__name__\s*==\s*['"]__main__['"]\s*:"#).unwrap());
    let re_class = RE_CLASS.get_or_init(|| {
        Regex::new(r"^(\s*)class\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?:\(([^)]*)\))?\s*:").unwrap()
    });
    let re_def = RE_DEF.get_or_init(|| {
        Regex::new(r"^(\s*)(?:async\s+)?def\s+(test_[A-Za-z0-9_]*|[A-Za-z0-9_]+_test)\s*\(")
            .unwrap()
    });

    let mut active_test_class: Option<(String, usize)> = None;

    for (idx, line) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.chars().take_while(|c| c.is_whitespace()).count();
        if let Some((_, cls_indent)) = &active_test_class {
            if indent <= *cls_indent && !trimmed.starts_with('@') {
                active_test_class = None;
            }
        }

        if re_main.is_match(line) {
            out.push(IdeGutterItem {
                line: line_no,
                kind: IdeGutterKind::Main,
                // The menu reads "Run 'review.py'" / "Debug 'review.py'": labels
                // name the target; the verb is the menu's.
                label: std::path::Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("__main__")
                    .to_string(),
                language: "python".into(),
                run_spec: IdeQuickRunSpec {
                    runner: IdeRunnerKind::Python,
                    command: Some("python3".into()),
                    args: vec![path.to_string()],
                    cwd: Some(dir.to_string()),
                    target_path: Some(path.to_string()),
                    test_filter: None,
                    debug_program: Some(path.to_string()),
                    debug_test_name: None,
                },
            });
            continue;
        }

        if let Some(cap) = re_class.captures(line) {
            let cls_indent = cap[1].chars().count();
            let cls_name = &cap[2];
            let bases = cap.get(3).map(|m| m.as_str()).unwrap_or("");
            if cls_name.starts_with("Test") || bases.contains("TestCase") {
                active_test_class = Some((cls_name.to_string(), cls_indent));
                let node_id = format!("{path}::{cls_name}");
                out.push(IdeGutterItem {
                    line: line_no,
                    kind: IdeGutterKind::Suite,
                    label: cls_name.to_string(),
                    language: "python".into(),
                    run_spec: IdeQuickRunSpec {
                        runner: IdeRunnerKind::Pytest,
                        command: Some("pytest".into()),
                        args: vec![node_id.clone(), "-v".into()],
                        cwd: Some(dir.to_string()),
                        target_path: Some(path.to_string()),
                        test_filter: Some(node_id.clone()),
                        debug_program: Some(path.to_string()),
                        debug_test_name: Some(node_id),
                    },
                });
            }
            continue;
        }

        if let Some(cap) = re_def.captures(line) {
            let def_indent = cap[1].chars().count();
            let fn_name = &cap[2];
            let node_id = match &active_test_class {
                Some((cls, cls_indent)) if def_indent > *cls_indent => {
                    format!("{path}::{cls}::{fn_name}")
                }
                _ => format!("{path}::{fn_name}"),
            };
            out.push(IdeGutterItem {
                line: line_no,
                kind: IdeGutterKind::Test,
                label: fn_name.to_string(),
                language: "python".into(),
                run_spec: IdeQuickRunSpec {
                    runner: IdeRunnerKind::Pytest,
                    command: Some("pytest".into()),
                    args: vec![node_id.clone(), "-v".into()],
                    cwd: Some(dir.to_string()),
                    target_path: Some(path.to_string()),
                    test_filter: Some(node_id.clone()),
                    debug_program: Some(path.to_string()),
                    debug_test_name: Some(node_id),
                },
            });
        }
    }
}

// ---------------------------------------------------------------------------
// 5. JavaScript / TypeScript (`describe`, `it`, `test`, `package.json`)
// ---------------------------------------------------------------------------

pub(super) fn detect_js_ts(
    path: &str,
    dir: &str,
    lang: &str,
    content: &str,
    out: &mut Vec<IdeGutterItem>,
) {
    static RE_SUITE: OnceLock<Regex> = OnceLock::new();
    static RE_TEST: OnceLock<Regex> = OnceLock::new();

    let re_suite = RE_SUITE.get_or_init(|| {
        Regex::new(
            r#"^\s*(?:(?:test\.)?describe|suite)(?:\.(?:only|skip|concurrent|sequential))?\s*\(\s*(?:'([^']+)'|"([^"]+)"|`([^`]+)`)"#,
        )
        .unwrap()
    });
    let re_test = RE_TEST.get_or_init(|| {
        Regex::new(
            r#"^\s*(?:it|test)(?:\.(?:only|skip|concurrent|todo))?\s*\(\s*(?:'([^']+)'|"([^"]+)"|`([^`]+)`)"#,
        )
        .unwrap()
    });

    let uses_node_test = content.contains("node:test");
    let extract_title = |cap: &regex::Captures<'_>| -> Option<String> {
        cap.get(1)
            .or_else(|| cap.get(2))
            .or_else(|| cap.get(3))
            .map(|m| m.as_str().to_string())
    };

    for (idx, line) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        if let Some(title) = re_suite.captures(line).and_then(|c| extract_title(&c)) {
            out.push(build_js_test_item(
                path,
                dir,
                lang,
                line_no,
                IdeGutterKind::Suite,
                &title,
                uses_node_test,
            ));
        } else if let Some(title) = re_test.captures(line).and_then(|c| extract_title(&c)) {
            out.push(build_js_test_item(
                path,
                dir,
                lang,
                line_no,
                IdeGutterKind::Test,
                &title,
                uses_node_test,
            ));
        }
    }
}

fn build_js_test_item(
    path: &str,
    dir: &str,
    lang: &str,
    line_no: u32,
    kind: IdeGutterKind,
    title: &str,
    uses_node_test: bool,
) -> IdeGutterItem {
    let (runner, command, args) = if uses_node_test {
        (
            IdeRunnerKind::Node,
            Some("node".into()),
            vec![
                "--test".into(),
                format!("--test-name-pattern={title}"),
                path.to_string(),
            ],
        )
    } else {
        (
            IdeRunnerKind::Vitest,
            Some("npx".into()),
            vec![
                "vitest".into(),
                "run".into(),
                path.to_string(),
                "-t".into(),
                title.to_string(),
            ],
        )
    };
    IdeGutterItem {
        line: line_no,
        kind,
        label: title.to_string(),
        language: lang.to_string(),
        run_spec: IdeQuickRunSpec {
            runner,
            command,
            args,
            cwd: Some(dir.to_string()),
            target_path: Some(path.to_string()),
            test_filter: Some(title.to_string()),
            debug_program: Some(path.to_string()),
            debug_test_name: Some(title.to_string()),
        },
    }
}

pub(super) fn detect_package_json_scripts(
    path: &str,
    dir: &str,
    content: &str,
    out: &mut Vec<IdeGutterItem>,
) {
    detect_json_task_map(
        path,
        dir,
        content,
        "scripts",
        IdeRunnerKind::Npm,
        "npm",
        &["run"],
        out,
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn detect_json_task_map(
    path: &str,
    dir: &str,
    content: &str,
    section_key: &str,
    runner: IdeRunnerKind,
    cmd_bin: &str,
    sub_args: &[&str],
    out: &mut Vec<IdeGutterItem>,
) {
    let cleaned = strip_jsonc_line_comments(content);
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&cleaned) else {
        return;
    };
    let Some(scripts) = parsed.get(section_key).and_then(|v| v.as_object()) else {
        return;
    };
    if scripts.is_empty() {
        return;
    }

    let lines: Vec<&str> = content.lines().collect();
    let header_needle = format!("\"{section_key}\"");
    let scripts_header_idx = lines
        .iter()
        .position(|l| l.contains(&header_needle))
        .unwrap_or(0);

    for key in scripts.keys() {
        let needle = format!("\"{key}\"");
        let found_line = lines
            .get(scripts_header_idx + 1..)
            .unwrap_or(&[])
            .iter()
            .position(|l| {
                l.trim_start()
                    .strip_prefix(&needle)
                    .is_some_and(|rest| rest.trim_start().starts_with(':'))
            })
            .map(|rel| (scripts_header_idx + 1 + rel + 1) as u32)
            .unwrap_or((scripts_header_idx + 1) as u32);

        let kind = if key.contains("test") {
            IdeGutterKind::Test
        } else {
            IdeGutterKind::Main
        };

        let mut args: Vec<String> = sub_args.iter().map(|s| (*s).to_string()).collect();
        args.push(key.clone());
        let label_sub = sub_args.first().copied().unwrap_or("run");

        out.push(IdeGutterItem {
            line: found_line,
            kind,
            label: format!("{cmd_bin} {label_sub} {key}"),
            language: "json".into(),
            run_spec: IdeQuickRunSpec {
                runner,
                command: Some(cmd_bin.to_string()),
                args,
                cwd: Some(dir.to_string()),
                target_path: Some(path.to_string()),
                test_filter: Some(key.clone()),
                debug_program: Some(path.to_string()),
                debug_test_name: Some(key.clone()),
            },
        });
    }
}

fn strip_jsonc_line_comments(content: &str) -> String {
    content
        .lines()
        .map(|l| {
            let trimmed = l.trim_start();
            if trimmed.starts_with("//") {
                ""
            } else {
                l
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// 6. Shell (`.sh`, `.bash`, `.zsh`, `.bats` — script entrypoint, functions, Bats)
// ---------------------------------------------------------------------------

pub(super) fn detect_shell(
    path: &str,
    dir: &str,
    base_name: &str,
    content: &str,
    out: &mut Vec<IdeGutterItem>,
) {
    static RE_BATS: OnceLock<Regex> = OnceLock::new();
    static RE_SH_FN: OnceLock<Regex> = OnceLock::new();
    let re_bats =
        RE_BATS.get_or_init(|| Regex::new(r#"^\s*@test\s+(?:"([^"]+)"|'([^']+)')\s*\{"#).unwrap());
    let re_sh_fn = RE_SH_FN.get_or_init(|| {
        Regex::new(
            r"^\s*(?:function\s+([A-Za-z_][A-Za-z0-9_-]*)(?:\s*\(\s*\))?|([A-Za-z_][A-Za-z0-9_-]*)\s*\(\s*\))\s*\{?",
        )
        .unwrap()
    });

    if base_name.ends_with(".bats") {
        let mut has_any = false;
        for (idx, line) in content.lines().enumerate() {
            let line_no = (idx + 1) as u32;
            if let Some(cap) = re_bats.captures(line) {
                let Some(desc) = cap.get(1).or_else(|| cap.get(2)).map(|m| m.as_str()) else {
                    continue;
                };
                if !has_any {
                    has_any = true;
                    out.push(IdeGutterItem {
                        line: 1,
                        kind: IdeGutterKind::Suite,
                        label: base_name.to_string(),
                        language: "shell".into(),
                        run_spec: IdeQuickRunSpec {
                            runner: IdeRunnerKind::Shell,
                            command: Some("bats".into()),
                            args: vec![path.to_string()],
                            cwd: Some(dir.to_string()),
                            target_path: Some(path.to_string()),
                            test_filter: None,
                            debug_program: Some(path.to_string()),
                            debug_test_name: None,
                        },
                    });
                }
                out.push(IdeGutterItem {
                    line: line_no,
                    kind: IdeGutterKind::Test,
                    label: desc.to_string(),
                    language: "shell".into(),
                    run_spec: IdeQuickRunSpec {
                        runner: IdeRunnerKind::Shell,
                        command: Some("bats".into()),
                        args: vec!["-f".into(), desc.to_string(), path.to_string()],
                        cwd: Some(dir.to_string()),
                        target_path: Some(path.to_string()),
                        test_filter: Some(desc.to_string()),
                        debug_program: Some(path.to_string()),
                        debug_test_name: Some(desc.to_string()),
                    },
                });
            }
        }
        return;
    }

    let shell_bin = infer_shell_interpreter(base_name, content);
    let non_empty = content
        .lines()
        .any(|l| !l.trim().is_empty() && (!l.trim().starts_with('#') || l.starts_with("#!")));
    if non_empty {
        out.push(IdeGutterItem {
            line: 1,
            kind: IdeGutterKind::Main,
            label: base_name.to_string(),
            language: "shell".into(),
            run_spec: IdeQuickRunSpec {
                runner: IdeRunnerKind::Shell,
                command: Some(shell_bin.to_string()),
                args: vec![path.to_string()],
                cwd: Some(dir.to_string()),
                target_path: Some(path.to_string()),
                test_filter: None,
                debug_program: Some(path.to_string()),
                debug_test_name: None,
            },
        });
    }

    for (idx, line) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(cap) = re_sh_fn.captures(line) {
            let Some(fn_name) = cap.get(1).or_else(|| cap.get(2)).map(|m| m.as_str()) else {
                continue;
            };
            if matches!(fn_name, "if" | "for" | "while" | "case" | "select") {
                continue;
            }
            let invoke = format!(". {} && {}", shell_words::quote(path), fn_name);
            out.push(IdeGutterItem {
                line: line_no,
                kind: IdeGutterKind::Main,
                label: format!("{fn_name}()"),
                language: "shell".into(),
                run_spec: IdeQuickRunSpec {
                    runner: IdeRunnerKind::Shell,
                    command: Some(shell_bin.to_string()),
                    args: vec!["-c".into(), invoke],
                    cwd: Some(dir.to_string()),
                    target_path: Some(path.to_string()),
                    test_filter: Some(fn_name.to_string()),
                    debug_program: Some(path.to_string()),
                    debug_test_name: Some(fn_name.to_string()),
                },
            });
        }
    }
}

fn infer_shell_interpreter(base_name: &str, content: &str) -> &'static str {
    if let Some(first) = content.lines().next() {
        if let Some(shebang) = first.strip_prefix("#!") {
            if shebang.contains("zsh") {
                return "zsh";
            }
            if shebang.contains("bash") {
                return "bash";
            }
            if shebang.contains("/sh") || shebang.ends_with(" sh") {
                return "sh";
            }
        }
    }
    if base_name.ends_with(".zsh") || base_name.starts_with(".zsh") {
        "zsh"
    } else {
        "bash"
    }
}

// ---------------------------------------------------------------------------
// 7. Runtime Plugin Gutter Detectors
// ---------------------------------------------------------------------------

pub(super) fn apply_plugin_detectors(
    path: &str,
    dir: &str,
    base_name: &str,
    lang: &str,
    content: &str,
    detectors: &[PluginGutterDetector],
    out: &mut Vec<IdeGutterItem>,
) {
    let ext = base_name
        .rsplit_once('.')
        .map(|(_, e)| format!(".{}", e.to_ascii_lowercase()));

    for det in detectors {
        let lang_matches = !det.language.is_empty() && det.language.eq_ignore_ascii_case(lang);
        let ext_matches = det.extensions.iter().any(|candidate| {
            let c = candidate.trim();
            if c.starts_with('.') {
                ext.as_deref().is_some_and(|e| e.eq_ignore_ascii_case(c))
            } else {
                base_name.eq_ignore_ascii_case(c)
            }
        });
        if !lang_matches && !ext_matches {
            continue;
        }
        let Ok(re) = Regex::new(&det.pattern) else {
            continue;
        };

        for (idx, line) in content.lines().enumerate() {
            let Some(cap) = re.captures(line) else {
                continue;
            };
            let name = cap
                .get(1)
                .or_else(|| cap.get(0))
                .map(|m| m.as_str().trim())
                .unwrap_or("target");
            let subst = |tmpl: &str| {
                tmpl.replace("{file}", path)
                    .replace("{dir}", dir)
                    .replace("{name}", name)
            };
            let prefix = det.label_prefix.as_deref().unwrap_or("").trim();
            let cmd = if det.command_template.is_empty() {
                None
            } else {
                Some(subst(&det.command_template))
            };
            let args: Vec<String> = det.args_template.iter().map(|a| subst(a)).collect();

            out.push(IdeGutterItem {
                line: (idx + 1) as u32,
                kind: det.kind,
                label: if prefix.is_empty() {
                    name.to_string()
                } else {
                    format!("{prefix} {name}")
                },
                language: det.language.clone(),
                run_spec: IdeQuickRunSpec {
                    runner: det.runner,
                    command: cmd,
                    args,
                    cwd: Some(dir.to_string()),
                    target_path: Some(path.to_string()),
                    test_filter: Some(name.to_string()),
                    debug_program: Some(path.to_string()),
                    debug_test_name: Some(name.to_string()),
                },
            });
        }
    }
}
