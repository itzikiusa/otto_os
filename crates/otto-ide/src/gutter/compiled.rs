//! Gutter Run & Debug detectors for compiled languages (Go, Java, Rust).

use std::sync::OnceLock;

use regex::Regex;

use crate::types::{IdeGutterItem, IdeGutterKind, IdeQuickRunSpec, IdeRunnerKind};

use super::strip_line_comment;

// ---------------------------------------------------------------------------
// 1. Go (`func main`, `func TestXxx`, `func BenchmarkXxx`, `func FuzzXxx`)
// ---------------------------------------------------------------------------

pub(super) fn detect_go(path: &str, dir: &str, content: &str, out: &mut Vec<IdeGutterItem>) {
    static RE_PKG: OnceLock<Regex> = OnceLock::new();
    static RE_FN: OnceLock<Regex> = OnceLock::new();
    static RE_SUBTEST: OnceLock<Regex> = OnceLock::new();
    let re_pkg =
        RE_PKG.get_or_init(|| Regex::new(r"^\s*package\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap());
    let re_fn = RE_FN
        .get_or_init(|| Regex::new(r"^\s*func\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(([^)]*)\)").unwrap());
    let re_subtest = RE_SUBTEST.get_or_init(|| {
        Regex::new(r#"\b[A-Za-z_][A-Za-z0-9_]*\.Run\s*\(\s*(?:"([^"\\]+)"|`([^`]+)`)"#).unwrap()
    });

    let go_tags = crate::go_build_tags::extract_go_file_build_tags(content);
    let tags_flag = (!go_tags.is_empty()).then(|| format!("-tags={}", go_tags.join(",")));

    let mut is_main_pkg = false;
    for line in content.lines() {
        if let Some(cap) = re_pkg.captures(line) {
            is_main_pkg = &cap[1] == "main";
            break;
        }
    }

    let mut brace_depth: i32 = 0;
    let mut active_test_fn: Option<(String, i32)> = None;

    for (idx, line) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            continue;
        }

        if let Some(cap) = re_fn.captures(line) {
            let fn_name = &cap[1];
            let params = cap[2].trim();

            if fn_name == "main" && params.is_empty() && is_main_pkg {
                let mut args = vec!["run".into()];
                if let Some(ref tf) = tags_flag {
                    args.push(tf.clone());
                }
                args.push(".".into());
                out.push(IdeGutterItem {
                    line: line_no,
                    kind: IdeGutterKind::Main,
                    label: "main".into(),
                    language: "go".into(),
                    run_spec: IdeQuickRunSpec {
                        runner: IdeRunnerKind::GoRun,
                        command: Some("go".into()),
                        args,
                        cwd: Some(dir.to_string()),
                        target_path: Some(path.to_string()),
                        test_filter: None,
                        debug_program: Some(path.to_string()),
                        debug_test_name: None,
                    },
                });
            } else if (fn_name.starts_with("Test") || fn_name.starts_with("Fuzz"))
                && (params.contains("*testing.T")
                    || params.contains("*testing.M")
                    || params.contains("*testing.F"))
            {
                active_test_fn = Some((fn_name.to_string(), brace_depth));
                let pattern = format!("^{fn_name}$");
                let mut args = vec!["test".into()];
                if let Some(ref tf) = tags_flag {
                    args.push(tf.clone());
                }
                args.extend(["-v".into(), "-run".into(), pattern.clone(), ".".into()]);
                out.push(IdeGutterItem {
                    line: line_no,
                    kind: IdeGutterKind::Test,
                    label: fn_name.to_string(),
                    language: "go".into(),
                    run_spec: IdeQuickRunSpec {
                        runner: IdeRunnerKind::GoTest,
                        command: Some("go".into()),
                        args,
                        cwd: Some(dir.to_string()),
                        target_path: Some(path.to_string()),
                        test_filter: Some(pattern.clone()),
                        debug_program: Some(dir.to_string()),
                        debug_test_name: Some(pattern),
                    },
                });
            } else if fn_name.starts_with("Benchmark") && params.contains("*testing.B") {
                let pattern = format!("^{fn_name}$");
                let mut args = vec!["test".into()];
                if let Some(ref tf) = tags_flag {
                    args.push(tf.clone());
                }
                args.extend([
                    "-bench".into(),
                    pattern.clone(),
                    "-run".into(),
                    "^$".into(),
                    ".".into(),
                ]);
                out.push(IdeGutterItem {
                    line: line_no,
                    kind: IdeGutterKind::Bench,
                    label: format!("Bench {fn_name}"),
                    language: "go".into(),
                    run_spec: IdeQuickRunSpec {
                        runner: IdeRunnerKind::GoTest,
                        command: Some("go".into()),
                        args,
                        cwd: Some(dir.to_string()),
                        target_path: Some(path.to_string()),
                        test_filter: Some(pattern),
                        debug_program: Some(dir.to_string()),
                        debug_test_name: Some(fn_name.to_string()),
                    },
                });
            }
        } else if let Some((parent_fn, _)) = &active_test_fn {
            if let Some(sub_cap) = re_subtest.captures(line) {
                if let Some(sub_name) = sub_cap
                    .get(1)
                    .or_else(|| sub_cap.get(2))
                    .map(|m| m.as_str().trim())
                    .filter(|s| !s.is_empty())
                {
                    let go_sub = regex::escape(&sub_name.replace(' ', "_"));
                    let pattern = format!("^{parent_fn}$/^{go_sub}$");
                    let mut args = vec!["test".into()];
                    if let Some(ref tf) = tags_flag {
                        args.push(tf.clone());
                    }
                    args.extend(["-v".into(), "-run".into(), pattern.clone(), ".".into()]);
                    out.push(IdeGutterItem {
                        line: line_no,
                        kind: IdeGutterKind::Test,
                        label: format!("{parent_fn}/{sub_name}"),
                        language: "go".into(),
                        run_spec: IdeQuickRunSpec {
                            runner: IdeRunnerKind::GoTest,
                            command: Some("go".into()),
                            args,
                            cwd: Some(dir.to_string()),
                            target_path: Some(path.to_string()),
                            test_filter: Some(pattern.clone()),
                            debug_program: Some(dir.to_string()),
                            debug_test_name: Some(pattern),
                        },
                    });
                }
            }
        }

        let code_only = strip_line_comment(line);
        for ch in code_only.chars() {
            if ch == '{' {
                brace_depth += 1;
            } else if ch == '}' {
                brace_depth -= 1;
                if active_test_fn
                    .as_ref()
                    .is_some_and(|(_, d)| brace_depth <= *d)
                {
                    active_test_fn = None;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 2. Java (`public static void main`, `@Test`, `@ParameterizedTest`, package)
// ---------------------------------------------------------------------------

pub(super) fn detect_java(path: &str, dir: &str, content: &str, out: &mut Vec<IdeGutterItem>) {
    static RE_PKG: OnceLock<Regex> = OnceLock::new();
    static RE_CLASS: OnceLock<Regex> = OnceLock::new();
    static RE_TEST_ANN: OnceLock<Regex> = OnceLock::new();
    static RE_MAIN: OnceLock<Regex> = OnceLock::new();
    static RE_METHOD: OnceLock<Regex> = OnceLock::new();

    let re_pkg =
        RE_PKG.get_or_init(|| Regex::new(r"^\s*package\s+([A-Za-z_][A-Za-z0-9_.]*)\s*;").unwrap());
    let re_class = RE_CLASS.get_or_init(|| {
        Regex::new(
            r"^\s*(?:public\s+|protected\s+|private\s+|abstract\s+|final\s+|static\s+)*(?:class|record)\s+([A-Za-z_][A-Za-z0-9_]*)",
        )
        .unwrap()
    });
    let re_test_ann = RE_TEST_ANN.get_or_init(|| {
        Regex::new(
            r"@(?:org\.junit\.jupiter\.api\.|org\.junit\.jupiter\.params\.|org\.junit\.|org\.testng\.annotations\.)?(?:Test|ParameterizedTest|RepeatedTest|TestFactory)\b",
        )
        .unwrap()
    });
    let re_main = RE_MAIN.get_or_init(|| {
        Regex::new(
            r"^\s*(?:public\s+)?static\s+void\s+main\s*\(\s*String\s*(?:\[\s*\]|\.\.\.)\s*[A-Za-z_][A-Za-z0-9_]*\s*\)",
        )
        .unwrap()
    });
    let re_method = RE_METHOD.get_or_init(|| {
        Regex::new(
            r"^\s*(?:public\s+|protected\s+|private\s+|static\s+|final\s+|synchronized\s+)*(?:[A-Za-z0-9_<>,\[\]\.?]+\s+)+([A-Za-z_][A-Za-z0-9_]*)\s*\(",
        )
        .unwrap()
    });

    // Run and debug from the module that owns the file (`<module>/src/test/java/...`), as
    // Maven/Gradle need its build file; the runner refines this on local targets.
    let dir = path
        .rfind("/src/")
        .filter(|&i| i > 0)
        .map_or(dir, |i| &path[..i]);

    let mut package_name = String::new();
    for line in content.lines() {
        if let Some(cap) = re_pkg.captures(line) {
            package_name = cap[1].to_string();
            break;
        }
    }

    let mut current_class: Option<(String, u32)> = None;
    let mut class_has_tests = false;
    let mut pending_test_ann = false;
    let mut emitted_suite_line: Option<u32> = None;

    for (idx, line) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with('*') || trimmed.starts_with("/*") {
            continue;
        }

        if let Some(cap) = re_class.captures(line) {
            let cls = cap[1].to_string();
            current_class = Some((cls, line_no));
            class_has_tests = false;
            emitted_suite_line = None;
        }

        if re_test_ann.is_match(line) {
            pending_test_ann = true;
            // Support inline `@Test void foo() {}` on the same line:
            let after_ann = re_test_ann.replace_all(line, "");
            if let Some(cap) = re_method.captures(&after_ann) {
                let method = &cap[1];
                emit_java_test(
                    path,
                    dir,
                    &package_name,
                    &current_class,
                    method,
                    line_no,
                    &mut class_has_tests,
                    &mut emitted_suite_line,
                    out,
                );
                pending_test_ann = false;
            }
            continue;
        }

        if re_main.is_match(line) {
            let cls_name = current_class
                .as_ref()
                .map(|(c, _)| c.as_str())
                .unwrap_or("Main");
            let fqcn = qualify_java(&package_name, cls_name);
            out.push(IdeGutterItem {
                line: line_no,
                kind: IdeGutterKind::Main,
                label: format!("{cls_name}.main()"),
                language: "java".into(),
                run_spec: IdeQuickRunSpec {
                    runner: IdeRunnerKind::JavaMain,
                    command: None,
                    args: Vec::new(),
                    cwd: Some(dir.to_string()),
                    target_path: Some(path.to_string()),
                    test_filter: Some(fqcn.clone()),
                    debug_program: Some(fqcn),
                    debug_test_name: None,
                },
            });
            pending_test_ann = false;
            continue;
        }

        if pending_test_ann {
            if trimmed.starts_with('@') || trimmed.is_empty() {
                continue;
            }
            if let Some(cap) = re_method.captures(line) {
                let method = &cap[1];
                emit_java_test(
                    path,
                    dir,
                    &package_name,
                    &current_class,
                    method,
                    line_no,
                    &mut class_has_tests,
                    &mut emitted_suite_line,
                    out,
                );
            }
            pending_test_ann = false;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_java_test(
    path: &str,
    dir: &str,
    package_name: &str,
    current_class: &Option<(String, u32)>,
    method: &str,
    line_no: u32,
    class_has_tests: &mut bool,
    emitted_suite_line: &mut Option<u32>,
    out: &mut Vec<IdeGutterItem>,
) {
    let (cls_name, cls_line) = current_class
        .as_ref()
        .map(|(c, l)| (c.as_str(), *l))
        .unwrap_or(("Test", 1));
    let fqcn = qualify_java(package_name, cls_name);

    if !*class_has_tests && *emitted_suite_line != Some(cls_line) {
        *class_has_tests = true;
        *emitted_suite_line = Some(cls_line);
        out.push(IdeGutterItem {
            line: cls_line,
            kind: IdeGutterKind::Suite,
            label: cls_name.to_string(),
            language: "java".into(),
            run_spec: IdeQuickRunSpec {
                runner: IdeRunnerKind::JavaJunit,
                command: None,
                args: Vec::new(),
                cwd: Some(dir.to_string()),
                target_path: Some(path.to_string()),
                test_filter: Some(fqcn.clone()),
                debug_program: Some(fqcn.clone()),
                debug_test_name: Some(fqcn.clone()),
            },
        });
    }

    let selector = format!("{fqcn}#{method}");
    out.push(IdeGutterItem {
        line: line_no,
        kind: IdeGutterKind::Test,
        label: format!("{cls_name}.{method}"),
        language: "java".into(),
        run_spec: IdeQuickRunSpec {
            runner: IdeRunnerKind::JavaJunit,
            command: None,
            args: Vec::new(),
            cwd: Some(dir.to_string()),
            target_path: Some(path.to_string()),
            test_filter: Some(selector.clone()),
            debug_program: Some(fqcn),
            debug_test_name: Some(selector),
        },
    });
}

fn qualify_java(package_name: &str, class_name: &str) -> String {
    if package_name.is_empty() {
        class_name.to_string()
    } else {
        format!("{package_name}.{class_name}")
    }
}

// ---------------------------------------------------------------------------
// 3. Rust (`fn main`, `#[test]`, `#[tokio::test]`, inline + file module path)
// ---------------------------------------------------------------------------

pub(super) fn detect_rust(path: &str, dir: &str, content: &str, out: &mut Vec<IdeGutterItem>) {
    static RE_TEST_ATTR: OnceLock<Regex> = OnceLock::new();
    static RE_BENCH_ATTR: OnceLock<Regex> = OnceLock::new();
    static RE_MOD: OnceLock<Regex> = OnceLock::new();
    static RE_FN: OnceLock<Regex> = OnceLock::new();

    let re_test_attr = RE_TEST_ATTR.get_or_init(|| {
        Regex::new(r"^\s*#\[\s*(?:[A-Za-z0-9_]+\s*::\s*)*(?:test|rstest)\b").unwrap()
    });
    let re_bench_attr = RE_BENCH_ATTR.get_or_init(|| Regex::new(r"^\s*#\[\s*bench\b").unwrap());
    let re_mod = RE_MOD.get_or_init(|| {
        Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{").unwrap()
    });
    let re_fn = RE_FN.get_or_init(|| {
        Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(")
            .unwrap()
    });

    let cargo_target = super::cargo_target::cargo_target_for(path);
    let file_mods = cargo_target
        .as_ref()
        .map_or_else(|| infer_rust_file_modules(path), |t| t.modules.clone());
    let selector = cargo_target.as_ref().map_or(&[][..], |t| &t.selector[..]);
    let run_selector = cargo_target
        .as_ref()
        .and_then(|t| t.run_selector())
        .unwrap_or(&[]);
    let mut brace_depth: i32 = 0;
    let mut mod_stack: Vec<(String, i32)> = Vec::new();
    let mut pending_kind: Option<IdeGutterKind> = None;
    let mut pending_ignored = false;

    for (idx, line) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            continue;
        }

        if re_test_attr.is_match(line) {
            pending_kind = Some(IdeGutterKind::Test);
        } else if trimmed.starts_with("#[ignore") {
            pending_ignored = true;
        } else if re_bench_attr.is_match(line) {
            pending_kind = Some(IdeGutterKind::Bench);
        } else if let Some(cap) = re_fn.captures(line) {
            let fn_name = &cap[1];
            if let Some(kind) = pending_kind.take() {
                let mut full_mods = file_mods.clone();
                for (m, _) in &mod_stack {
                    full_mods.push(m.clone());
                }
                full_mods.push(fn_name.to_string());
                let full_filter = full_mods.join("::");
                let mut args = vec!["test".to_string()];
                args.extend(selector.iter().cloned());
                args.extend([full_filter.clone(), "--".into(), "--exact".into()]);
                args.push("--nocapture".into());
                if std::mem::take(&mut pending_ignored) {
                    args.push("--include-ignored".into());
                }
                out.push(IdeGutterItem {
                    line: line_no,
                    kind,
                    label: fn_name.to_string(),
                    language: "rust".into(),
                    run_spec: IdeQuickRunSpec {
                        runner: IdeRunnerKind::CargoTest,
                        command: Some("cargo".into()),
                        args,
                        cwd: Some(dir.to_string()),
                        target_path: Some(path.to_string()),
                        test_filter: Some(full_filter.clone()),
                        debug_program: Some(path.to_string()),
                        debug_test_name: Some(full_filter),
                    },
                });
            } else if fn_name == "main" && mod_stack.is_empty() {
                out.push(IdeGutterItem {
                    line: line_no,
                    kind: IdeGutterKind::Main,
                    label: "main".into(),
                    language: "rust".into(),
                    run_spec: IdeQuickRunSpec {
                        runner: IdeRunnerKind::CargoRun,
                        command: Some("cargo".into()),
                        args: std::iter::once("run".to_string())
                            .chain(run_selector.iter().cloned())
                            .collect(),
                        cwd: Some(dir.to_string()),
                        target_path: Some(path.to_string()),
                        test_filter: None,
                        debug_program: Some(path.to_string()),
                        debug_test_name: None,
                    },
                });
            }
        } else if !trimmed.starts_with("#[") && !trimmed.is_empty() {
            pending_kind = None;
            pending_ignored = false;
        }

        if let Some(cap) = re_mod.captures(line) {
            mod_stack.push((cap[1].to_string(), brace_depth));
        }

        let code_only = strip_line_comment(line);
        for ch in code_only.chars() {
            if ch == '{' {
                brace_depth += 1;
            } else if ch == '}' {
                brace_depth -= 1;
                while mod_stack.last().is_some_and(|(_, d)| brace_depth <= *d) {
                    mod_stack.pop();
                }
            }
        }
    }
}

/// Fallback when no local `Cargo.toml` owns the file (a remote target).
fn infer_rust_file_modules(path: &str) -> Vec<String> {
    let Some((_, after_src)) = path.rsplit_once("/src/") else {
        return Vec::new();
    };
    if after_src == "lib.rs" || after_src == "main.rs" || after_src.starts_with("bin/") {
        return Vec::new();
    }
    let without_ext = after_src.strip_suffix(".rs").unwrap_or(after_src);
    without_ext
        .split('/')
        .filter(|seg| !seg.is_empty() && *seg != "mod")
        .map(str::to_string)
        .collect()
}

// ---------------------------------------------------------------------------
// 4. Zig (`pub fn main`, `test "name"`, `test decl`, `test {`)
// ---------------------------------------------------------------------------

pub(super) fn detect_zig(path: &str, dir: &str, content: &str, out: &mut Vec<IdeGutterItem>) {
    static RE_MAIN: OnceLock<Regex> = OnceLock::new();
    static RE_TEST_STR: OnceLock<Regex> = OnceLock::new();
    static RE_TEST_DECL: OnceLock<Regex> = OnceLock::new();

    let re_main = RE_MAIN.get_or_init(|| Regex::new(r"^\s*pub\s+fn\s+main\s*\(").unwrap());
    let re_test_str =
        RE_TEST_STR.get_or_init(|| Regex::new(r#"^\s*test\s+"((?:\\.|[^"\\])+)"\s*\{"#).unwrap());
    let re_test_decl = RE_TEST_DECL
        .get_or_init(|| Regex::new(r"^\s*test\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{").unwrap());

    let effective_cwd = find_zig_project_root(dir).unwrap_or_else(|| dir.to_string());

    for (idx, line) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            continue;
        }

        if re_main.is_match(line) {
            out.push(IdeGutterItem {
                line: line_no,
                kind: IdeGutterKind::Main,
                label: "main".into(),
                language: "zig".into(),
                run_spec: IdeQuickRunSpec {
                    runner: IdeRunnerKind::ZigRun,
                    command: Some("zig".into()),
                    args: vec!["run".into(), path.to_string()],
                    cwd: Some(effective_cwd.clone()),
                    target_path: Some(path.to_string()),
                    test_filter: None,
                    debug_program: Some(path.to_string()),
                    debug_test_name: None,
                },
            });
        } else if let Some(cap) = re_test_str.captures(line) {
            let test_name = cap[1].replace("\\\"", "\"");
            out.push(IdeGutterItem {
                line: line_no,
                kind: IdeGutterKind::Test,
                label: test_name.clone(),
                language: "zig".into(),
                run_spec: IdeQuickRunSpec {
                    runner: IdeRunnerKind::ZigTest,
                    command: Some("zig".into()),
                    args: vec![
                        "test".into(),
                        "--test-filter".into(),
                        test_name.clone(),
                        path.to_string(),
                    ],
                    cwd: Some(effective_cwd.clone()),
                    target_path: Some(path.to_string()),
                    test_filter: Some(test_name.clone()),
                    debug_program: Some(path.to_string()),
                    debug_test_name: Some(test_name),
                },
            });
        } else if let Some(cap) = re_test_decl.captures(line) {
            let decl_name = cap[1].to_string();
            out.push(IdeGutterItem {
                line: line_no,
                kind: IdeGutterKind::Test,
                label: decl_name.clone(),
                language: "zig".into(),
                run_spec: IdeQuickRunSpec {
                    runner: IdeRunnerKind::ZigTest,
                    command: Some("zig".into()),
                    args: vec![
                        "test".into(),
                        "--test-filter".into(),
                        decl_name.clone(),
                        path.to_string(),
                    ],
                    cwd: Some(effective_cwd.clone()),
                    target_path: Some(path.to_string()),
                    test_filter: Some(decl_name.clone()),
                    debug_program: Some(path.to_string()),
                    debug_test_name: Some(decl_name),
                },
            });
        }
    }
}

fn find_zig_project_root(dir: &str) -> Option<String> {
    let mut cur = Some(std::path::Path::new(dir));
    let mut hops = 0usize;
    while let Some(d) = cur {
        if d.join("build.zig").is_file() {
            return Some(d.to_string_lossy().into_owned());
        }
        hops += 1;
        if hops >= 8 {
            break;
        }
        cur = d.parent();
    }
    None
}

// ---------------------------------------------------------------------------
// 5. C# (`Main`, top-level `Program.cs`, xUnit / NUnit / MSTest)
// ---------------------------------------------------------------------------

pub(super) fn detect_csharp(path: &str, dir: &str, content: &str, out: &mut Vec<IdeGutterItem>) {
    static RE_NS: OnceLock<Regex> = OnceLock::new();
    static RE_CLASS: OnceLock<Regex> = OnceLock::new();
    static RE_MAIN: OnceLock<Regex> = OnceLock::new();
    static RE_TEST_ATTR: OnceLock<Regex> = OnceLock::new();
    static RE_METHOD: OnceLock<Regex> = OnceLock::new();

    let re_ns = RE_NS.get_or_init(|| {
        Regex::new(r"^\s*namespace\s+([A-Za-z_][A-Za-z0-9_.]*)\s*(?:;|\{)?").unwrap()
    });
    let re_class = RE_CLASS.get_or_init(|| {
        Regex::new(
            r"^\s*(?:(?:public|internal|private|protected|static|abstract|sealed|partial|file)\s+)*(?:class|record|struct)\s+([A-Za-z_][A-Za-z0-9_]*)",
        )
        .unwrap()
    });
    let re_main = RE_MAIN.get_or_init(|| {
        Regex::new(
            r"^\s*(?:(?:public|private|internal|protected|async|unsafe)\s+)*static\s+(?:async\s+)?(?:void|int|Task|Task<int>|System\.Threading\.Tasks\.Task(?:<int>)?)\s+Main\s*\(",
        )
        .unwrap()
    });
    let re_test_attr = RE_TEST_ATTR.get_or_init(|| {
        Regex::new(
            r"\[\s*(?:(?:Xunit|NUnit\.Framework|Microsoft\.VisualStudio\.TestTools\.UnitTesting)\.)?(?:Fact|Theory|Test|TestCase|TestCaseSource|TestMethod|DataTestMethod)\b",
        )
        .unwrap()
    });
    let re_method = RE_METHOD.get_or_init(|| {
        Regex::new(
            r"^\s*(?:(?:public|private|internal|protected|static|async|virtual|override|sealed|new|unsafe)\s+)*(?:[A-Za-z0-9_<>,\[\]\.?\s]+?\s+)([A-Za-z_][A-Za-z0-9_]*)\s*(?:<[^>]+>)?\s*\(",
        )
        .unwrap()
    });

    let proj_root = find_dotnet_project_root(dir).unwrap_or_else(|| dir.to_string());
    let mut namespace_name = String::new();
    let mut current_class: Option<(String, u32)> = None;
    let mut emitted_suite_line: Option<u32> = None;
    let mut pending_test_attr = false;
    let mut has_explicit_main = false;
    let mut has_any_type = false;
    let mut first_stmt_line: Option<u32> = None;

    for (idx, line) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with('*')
            || trimmed.starts_with("/*")
            || trimmed.starts_with('#')
        {
            continue;
        }
        if let Some(cap) = re_ns.captures(line) {
            namespace_name = cap[1].to_string();
            continue;
        }
        if let Some(cap) = re_class.captures(line) {
            has_any_type = true;
            current_class = Some((cap[1].to_string(), line_no));
            emitted_suite_line = None;
            pending_test_attr = false;
            continue;
        }
        if re_test_attr.is_match(line) {
            pending_test_attr = true;
            continue;
        }
        if trimmed.starts_with('[') {
            continue;
        }
        if !trimmed.starts_with("using ")
            && !trimmed.starts_with("global using ")
            && first_stmt_line.is_none()
        {
            first_stmt_line = Some(line_no);
        }
        if re_main.is_match(line) {
            has_explicit_main = true;
            let cls = current_class
                .as_ref()
                .map(|(c, _)| c.as_str())
                .unwrap_or("Program");
            out.push(IdeGutterItem {
                line: line_no,
                kind: IdeGutterKind::Main,
                label: format!("{cls}.Main()"),
                language: "csharp".into(),
                run_spec: IdeQuickRunSpec {
                    runner: IdeRunnerKind::DotnetRun,
                    command: Some("dotnet".into()),
                    args: vec!["run".into()],
                    cwd: Some(proj_root.clone()),
                    target_path: Some(path.to_string()),
                    test_filter: None,
                    debug_program: Some(path.to_string()),
                    debug_test_name: None,
                },
            });
            pending_test_attr = false;
            continue;
        }
        if pending_test_attr {
            if let Some(cap) = re_method.captures(line) {
                let method = cap[1].to_string();
                let fqn_prefix = match (&namespace_name.is_empty(), &current_class) {
                    (false, Some((c, _))) => format!("{namespace_name}.{c}"),
                    (true, Some((c, _))) => c.clone(),
                    (false, None) => namespace_name.clone(),
                    (true, None) => String::new(),
                };
                if let Some((ref cls, cls_line)) = current_class {
                    if emitted_suite_line != Some(cls_line) {
                        emitted_suite_line = Some(cls_line);
                        out.push(IdeGutterItem {
                            line: cls_line,
                            kind: IdeGutterKind::Suite,
                            label: format!("{cls} (all tests)"),
                            language: "csharp".into(),
                            run_spec: IdeQuickRunSpec {
                                runner: IdeRunnerKind::DotnetTest,
                                command: Some("dotnet".into()),
                                args: vec![
                                    "test".into(),
                                    "--verbosity".into(),
                                    "normal".into(),
                                    "--filter".into(),
                                    format!("FullyQualifiedName~{fqn_prefix}"),
                                ],
                                cwd: Some(proj_root.clone()),
                                target_path: Some(path.to_string()),
                                test_filter: Some(fqn_prefix.clone()),
                                debug_program: Some(path.to_string()),
                                debug_test_name: Some(fqn_prefix.clone()),
                            },
                        });
                    }
                }
                let full_test = if fqn_prefix.is_empty() {
                    method.clone()
                } else {
                    format!("{fqn_prefix}.{method}")
                };
                out.push(IdeGutterItem {
                    line: line_no,
                    kind: IdeGutterKind::Test,
                    label: method,
                    language: "csharp".into(),
                    run_spec: IdeQuickRunSpec {
                        runner: IdeRunnerKind::DotnetTest,
                        command: Some("dotnet".into()),
                        args: vec![
                            "test".into(),
                            "--verbosity".into(),
                            "normal".into(),
                            "--filter".into(),
                            format!("FullyQualifiedName~{full_test}"),
                        ],
                        cwd: Some(proj_root.clone()),
                        target_path: Some(path.to_string()),
                        test_filter: Some(full_test.clone()),
                        debug_program: Some(path.to_string()),
                        debug_test_name: Some(full_test),
                    },
                });
                pending_test_attr = false;
            }
        }
    }

    // C# 9+ top-level statements (`Program.cs` with executable statements and no explicit `Main`).
    let is_program_cs = path
        .rsplit('/')
        .next()
        .is_some_and(|n| n.eq_ignore_ascii_case("Program.cs"));
    if is_program_cs && !has_explicit_main && !has_any_type {
        if let Some(stmt_ln) = first_stmt_line {
            out.push(IdeGutterItem {
                line: stmt_ln,
                kind: IdeGutterKind::Main,
                label: "Program (top-level)".into(),
                language: "csharp".into(),
                run_spec: IdeQuickRunSpec {
                    runner: IdeRunnerKind::DotnetRun,
                    command: Some("dotnet".into()),
                    args: vec!["run".into()],
                    cwd: Some(proj_root),
                    target_path: Some(path.to_string()),
                    test_filter: None,
                    debug_program: Some(path.to_string()),
                    debug_test_name: None,
                },
            });
        }
    }
}

// ---------------------------------------------------------------------------
// 6. F# (`[<EntryPoint>]`, `[<Fact>]`, `[<Test>]`, `[<TestMethod>]`)
// ---------------------------------------------------------------------------

pub(super) fn detect_fsharp(path: &str, dir: &str, content: &str, out: &mut Vec<IdeGutterItem>) {
    static RE_LET: OnceLock<Regex> = OnceLock::new();
    let re_let = RE_LET.get_or_init(|| {
        Regex::new(r"^\s*let\s+(?:``([^`]+)``|([A-Za-z_][A-Za-z0-9_']*)\b)").unwrap()
    });
    let proj_root = find_dotnet_project_root(dir).unwrap_or_else(|| dir.to_string());
    let mut pending_entry = false;
    let mut pending_test = false;

    for (idx, line) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if trimmed.contains("[<EntryPoint>]") {
            pending_entry = true;
            continue;
        }
        if trimmed.contains("[<Fact")
            || trimmed.contains("[<Theory")
            || trimmed.contains("[<Test")
            || trimmed.contains("[<TestMethod")
        {
            pending_test = true;
            continue;
        }
        if trimmed.starts_with("[<") {
            continue;
        }
        if pending_entry {
            if let Some(cap) = re_let.captures(line) {
                let name = cap.get(1).or_else(|| cap.get(2)).unwrap().as_str();
                out.push(IdeGutterItem {
                    line: line_no,
                    kind: IdeGutterKind::Main,
                    label: format!("{name} (EntryPoint)"),
                    language: "fsharp".into(),
                    run_spec: IdeQuickRunSpec {
                        runner: IdeRunnerKind::DotnetRun,
                        command: Some("dotnet".into()),
                        args: vec!["run".into()],
                        cwd: Some(proj_root.clone()),
                        target_path: Some(path.to_string()),
                        test_filter: None,
                        debug_program: Some(path.to_string()),
                        debug_test_name: None,
                    },
                });
                pending_entry = false;
            }
        } else if pending_test {
            if let Some(cap) = re_let.captures(line) {
                let name = cap
                    .get(1)
                    .or_else(|| cap.get(2))
                    .unwrap()
                    .as_str()
                    .to_string();
                out.push(IdeGutterItem {
                    line: line_no,
                    kind: IdeGutterKind::Test,
                    label: name.clone(),
                    language: "fsharp".into(),
                    run_spec: IdeQuickRunSpec {
                        runner: IdeRunnerKind::DotnetTest,
                        command: Some("dotnet".into()),
                        args: vec![
                            "test".into(),
                            "--verbosity".into(),
                            "normal".into(),
                            "--filter".into(),
                            format!("FullyQualifiedName~{name}"),
                        ],
                        cwd: Some(proj_root.clone()),
                        target_path: Some(path.to_string()),
                        test_filter: Some(name.clone()),
                        debug_program: Some(path.to_string()),
                        debug_test_name: Some(name),
                    },
                });
                pending_test = false;
            }
        }
    }
}

#[allow(clippy::disallowed_methods)] // bounded <=8-hop ancestor check executed inside spawn_blocking
fn find_dotnet_project_root(dir: &str) -> Option<String> {
    let mut cur = Some(std::path::Path::new(dir));
    let mut hops = 0usize;
    while let Some(d) = cur {
        let has_proj = std::fs::read_dir(d).ok().is_some_and(|rd| {
            rd.flatten().any(|e| {
                e.file_name().to_str().is_some_and(|n| {
                    n.ends_with(".csproj")
                        || n.ends_with(".fsproj")
                        || n.ends_with(".sln")
                        || n.ends_with(".slnx")
                })
            })
        });
        if has_proj {
            return Some(d.to_string_lossy().into_owned());
        }
        hops += 1;
        if hops >= 8 {
            break;
        }
        cur = d.parent();
    }
    None
}
