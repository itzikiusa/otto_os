//! Multi-language Gutter Run & Debug detector (`▶ Run` / `🐞 Debug`).
//!
//! Scans source files in Go, Java, Rust, Python, JavaScript/TypeScript
//! (including `package.json` scripts), plus runtime [`PluginGutterDetector`]
//! rules, returning 1-based [`IdeGutterItem`] entries with ready-to-launch
//! [`IdeQuickRunSpec`] configurations for both test runners and DAP debugging.

use std::sync::OnceLock;

use regex::Regex;

use crate::types::{
    detect_language, IdeGutterItem, IdeGutterKind, IdeQuickRunSpec, IdeRunnerKind,
    PluginGutterDetector,
};

/// Detect runnable and debuggable gutter items (`main`, unit tests, suites,
/// benchmarks, `package.json` scripts, and active plugin detectors).
pub fn detect_gutter_items(
    path: &str,
    content: &str,
    extra_detectors: &[PluginGutterDetector],
) -> Vec<IdeGutterItem> {
    let normalized_path = path.replace('\\', "/");
    let base_name = normalized_path
        .rsplit('/')
        .next()
        .unwrap_or(&normalized_path);
    let dir = parent_dir(&normalized_path);
    let lang = detect_language(&normalized_path).unwrap_or("");

    let mut items = Vec::new();
    match base_name {
        "package.json" => {
            detect_package_json_scripts(&normalized_path, &dir, content, &mut items);
        }
        "deno.json" | "deno.jsonc" => {
            detect_json_task_map(
                &normalized_path,
                &dir,
                content,
                "tasks",
                IdeRunnerKind::Custom,
                "deno",
                &["task"],
                &mut items,
            );
        }
        "composer.json" => {
            detect_json_task_map(
                &normalized_path,
                &dir,
                content,
                "scripts",
                IdeRunnerKind::Custom,
                "composer",
                &["run-script"],
                &mut items,
            );
        }
        _ => match lang {
            "go" => detect_go(&normalized_path, &dir, content, &mut items),
            "java" => detect_java(&normalized_path, &dir, content, &mut items),
            "rust" => detect_rust(&normalized_path, &dir, content, &mut items),
            "python" => detect_python(&normalized_path, &dir, content, &mut items),
            "javascript" | "typescript" => {
                detect_js_ts(&normalized_path, &dir, lang, content, &mut items);
            }
            "shell" => detect_shell(&normalized_path, &dir, base_name, content, &mut items),
            _ => {}
        },
    }

    if !extra_detectors.is_empty() {
        apply_plugin_detectors(
            &normalized_path,
            &dir,
            base_name,
            lang,
            content,
            extra_detectors,
            &mut items,
        );
    }

    items.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.label.cmp(&b.label)));
    items.dedup_by(|a, b| a.line == b.line && a.label == b.label);
    items
}

fn parent_dir(path: &str) -> String {
    match path.rsplit_once('/') {
        Some(("", _)) => "/".to_string(),
        Some((dir, _)) => dir.to_string(),
        None => ".".to_string(),
    }
}

// ---------------------------------------------------------------------------
// 1. Go (`func main`, `func TestXxx`, `func BenchmarkXxx`, `func FuzzXxx`)
// ---------------------------------------------------------------------------

fn detect_go(path: &str, dir: &str, content: &str, out: &mut Vec<IdeGutterItem>) {
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
                out.push(IdeGutterItem {
                    line: line_no,
                    kind: IdeGutterKind::Main,
                    label: "Run main".into(),
                    language: "go".into(),
                    run_spec: IdeQuickRunSpec {
                        runner: IdeRunnerKind::GoRun,
                        command: Some("go".into()),
                        args: vec!["run".into(), ".".into()],
                        cwd: Some(dir.to_string()),
                        target_path: Some(path.to_string()),
                        test_filter: None,
                        debug_program: Some(dir.to_string()),
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
                out.push(IdeGutterItem {
                    line: line_no,
                    kind: IdeGutterKind::Test,
                    label: format!("Run {fn_name}"),
                    language: "go".into(),
                    run_spec: IdeQuickRunSpec {
                        runner: IdeRunnerKind::GoTest,
                        command: Some("go".into()),
                        args: vec![
                            "test".into(),
                            "-v".into(),
                            "-run".into(),
                            pattern.clone(),
                            ".".into(),
                        ],
                        cwd: Some(dir.to_string()),
                        target_path: Some(path.to_string()),
                        test_filter: Some(pattern.clone()),
                        debug_program: Some(dir.to_string()),
                        debug_test_name: Some(pattern),
                    },
                });
            } else if fn_name.starts_with("Benchmark") && params.contains("*testing.B") {
                let pattern = format!("^{fn_name}$");
                out.push(IdeGutterItem {
                    line: line_no,
                    kind: IdeGutterKind::Bench,
                    label: format!("Bench {fn_name}"),
                    language: "go".into(),
                    run_spec: IdeQuickRunSpec {
                        runner: IdeRunnerKind::GoTest,
                        command: Some("go".into()),
                        args: vec![
                            "test".into(),
                            "-bench".into(),
                            pattern.clone(),
                            "-run".into(),
                            "^$".into(),
                            ".".into(),
                        ],
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
                    out.push(IdeGutterItem {
                        line: line_no,
                        kind: IdeGutterKind::Test,
                        label: format!("Run {parent_fn}/{sub_name}"),
                        language: "go".into(),
                        run_spec: IdeQuickRunSpec {
                            runner: IdeRunnerKind::GoTest,
                            command: Some("go".into()),
                            args: vec![
                                "test".into(),
                                "-v".into(),
                                "-run".into(),
                                pattern.clone(),
                                ".".into(),
                            ],
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

fn detect_java(path: &str, dir: &str, content: &str, out: &mut Vec<IdeGutterItem>) {
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
                label: format!("Run {cls_name}.main()"),
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
            label: format!("Run {cls_name}"),
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
        label: format!("Run {cls_name}.{method}"),
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

fn detect_rust(path: &str, dir: &str, content: &str, out: &mut Vec<IdeGutterItem>) {
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

    let file_mods = infer_rust_file_modules(path);
    let mut brace_depth: i32 = 0;
    let mut mod_stack: Vec<(String, i32)> = Vec::new();
    let mut pending_kind: Option<IdeGutterKind> = None;

    for (idx, line) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            continue;
        }

        if re_test_attr.is_match(line) {
            pending_kind = Some(IdeGutterKind::Test);
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
                out.push(IdeGutterItem {
                    line: line_no,
                    kind,
                    label: format!("Run {fn_name}"),
                    language: "rust".into(),
                    run_spec: IdeQuickRunSpec {
                        runner: IdeRunnerKind::CargoTest,
                        command: Some("cargo".into()),
                        args: vec![
                            "test".into(),
                            full_filter.clone(),
                            "--".into(),
                            "--exact".into(),
                            "--nocapture".into(),
                        ],
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
                    label: "Run main".into(),
                    language: "rust".into(),
                    run_spec: IdeQuickRunSpec {
                        runner: IdeRunnerKind::CargoRun,
                        command: Some("cargo".into()),
                        args: vec!["run".into()],
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

fn strip_line_comment(line: &str) -> &str {
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

// ---------------------------------------------------------------------------
// 4. Python (`if __name__ == "__main__":`, `class Test*`, `def test_*`)
// ---------------------------------------------------------------------------

fn detect_python(path: &str, dir: &str, content: &str, out: &mut Vec<IdeGutterItem>) {
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
                label: "Run __main__".into(),
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
                    label: format!("Run {cls_name}"),
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
                label: format!("Run {fn_name}"),
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

fn detect_js_ts(path: &str, dir: &str, lang: &str, content: &str, out: &mut Vec<IdeGutterItem>) {
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
        label: format!("Run '{title}'"),
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

fn detect_package_json_scripts(path: &str, dir: &str, content: &str, out: &mut Vec<IdeGutterItem>) {
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
fn detect_json_task_map(
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

fn detect_shell(
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
                        label: format!("Run {base_name}"),
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
                    label: format!("Run '{desc}'"),
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
            label: format!("Run {base_name}"),
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
                label: format!("Run {fn_name}()"),
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
// 6. Runtime Plugin Gutter Detectors
// ---------------------------------------------------------------------------

fn apply_plugin_detectors(
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
            let prefix = det.label_prefix.as_deref().unwrap_or("Run");
            let cmd = if det.command_template.is_empty() {
                None
            } else {
                Some(subst(&det.command_template))
            };
            let args: Vec<String> = det.args_template.iter().map(|a| subst(a)).collect();

            out.push(IdeGutterItem {
                line: (idx + 1) as u32,
                kind: det.kind,
                label: format!("{prefix} {name}"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_detects_main_tests_and_benchmarks() {
        let main_src = "\
package main

import \"fmt\"

func main() {
    fmt.Println(\"hi\")
}
";
        let main_items = detect_gutter_items("/ws/cmd/app/main.go", main_src, &[]);
        assert_eq!(main_items.len(), 1);
        assert_eq!(main_items[0].line, 5);
        assert_eq!(main_items[0].kind, IdeGutterKind::Main);
        assert_eq!(main_items[0].run_spec.runner, IdeRunnerKind::GoRun);
        assert_eq!(main_items[0].run_spec.cwd.as_deref(), Some("/ws/cmd/app"));

        let test_src = "\
package calc

import \"testing\"

func helper() {}

func TestAdd(t *testing.T) {
    if 1+1 != 2 { t.Fail() }
}

func BenchmarkAdd(b *testing.B) {
    for i := 0; i < b.N; i++ {}
}
";
        let items = detect_gutter_items("/ws/pkg/calc/calc_test.go", test_src, &[]);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].line, 7);
        assert_eq!(items[0].kind, IdeGutterKind::Test);
        assert_eq!(items[0].run_spec.test_filter.as_deref(), Some("^TestAdd$"));
        assert_eq!(items[1].line, 11);
        assert_eq!(items[1].kind, IdeGutterKind::Bench);
        assert_eq!(
            items[1].run_spec.test_filter.as_deref(),
            Some("^BenchmarkAdd$")
        );
    }

    #[test]
    fn java_detects_package_main_junit_test_and_parameterized_test() {
        let src = "\
package com.acme.billing;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;

public class InvoiceServiceTest {
    public static void main(String[] args) {
        System.out.println(\"boot\");
    }

    @Test
    void computesTax() {
    }

    @ParameterizedTest
    @ValueSource(ints = {1, 2})
    public void validatesQuantities(int qty) {
    }
}
";
        let items = detect_gutter_items(
            "/ws/src/test/java/com/acme/billing/InvoiceServiceTest.java",
            src,
            &[],
        );
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].line, 6);
        assert_eq!(items[0].kind, IdeGutterKind::Suite);
        assert_eq!(
            items[0].run_spec.test_filter.as_deref(),
            Some("com.acme.billing.InvoiceServiceTest")
        );

        assert_eq!(items[1].line, 7);
        assert_eq!(items[1].kind, IdeGutterKind::Main);
        assert_eq!(items[1].run_spec.runner, IdeRunnerKind::JavaMain);
        assert_eq!(
            items[1].run_spec.test_filter.as_deref(),
            Some("com.acme.billing.InvoiceServiceTest")
        );

        assert_eq!(items[2].line, 12);
        assert_eq!(items[2].kind, IdeGutterKind::Test);
        assert_eq!(
            items[2].run_spec.test_filter.as_deref(),
            Some("com.acme.billing.InvoiceServiceTest#computesTax")
        );

        assert_eq!(items[3].line, 17);
        assert_eq!(items[3].kind, IdeGutterKind::Test);
        assert_eq!(
            items[3].run_spec.test_filter.as_deref(),
            Some("com.acme.billing.InvoiceServiceTest#validatesQuantities")
        );
    }

    #[test]
    fn rust_detects_main_unit_tests_tokio_tests_and_module_path() {
        let main_rs = "\
fn main() {
    println!(\"hello\");
}
";
        let main_items = detect_gutter_items("/ws/src/main.rs", main_rs, &[]);
        assert_eq!(main_items.len(), 1);
        assert_eq!(main_items[0].kind, IdeGutterKind::Main);
        assert_eq!(main_items[0].run_spec.runner, IdeRunnerKind::CargoRun);

        let mod_rs = "\
pub fn add(a: i32, b: i32) -> i32 { a + b }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_positive_numbers() {
        assert_eq!(add(2, 3), 5);
    }

    #[tokio::test]
    async fn async_roundtrip() {
        assert_eq!(add(1, 1), 2);
    }
}
";
        let items = detect_gutter_items("/ws/src/math/ops.rs", mod_rs, &[]);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].line, 8);
        assert_eq!(items[0].kind, IdeGutterKind::Test);
        assert_eq!(
            items[0].run_spec.test_filter.as_deref(),
            Some("math::ops::tests::adds_positive_numbers")
        );
        assert_eq!(items[1].line, 13);
        assert_eq!(
            items[1].run_spec.test_filter.as_deref(),
            Some("math::ops::tests::async_roundtrip")
        );
    }

    #[test]
    fn python_detects_main_guard_test_classes_sync_and_async_tests() {
        let src = "\
def test_top_level():
    assert 1 == 1

class TestOrderFlow:
    def test_checkout(self):
        assert True

    async def test_webhook_delivery(self):
        assert True

if __name__ == \"__main__\":
    print('run')
";
        let items = detect_gutter_items("/ws/tests/test_orders.py", src, &[]);
        assert_eq!(items.len(), 5);
        assert_eq!(
            items[0].run_spec.test_filter.as_deref(),
            Some("/ws/tests/test_orders.py::test_top_level")
        );
        assert_eq!(items[1].kind, IdeGutterKind::Suite);
        assert_eq!(
            items[1].run_spec.test_filter.as_deref(),
            Some("/ws/tests/test_orders.py::TestOrderFlow")
        );
        assert_eq!(
            items[2].run_spec.test_filter.as_deref(),
            Some("/ws/tests/test_orders.py::TestOrderFlow::test_checkout")
        );
        assert_eq!(
            items[3].run_spec.test_filter.as_deref(),
            Some("/ws/tests/test_orders.py::TestOrderFlow::test_webhook_delivery")
        );
        assert_eq!(items[4].kind, IdeGutterKind::Main);
        assert_eq!(items[4].run_spec.runner, IdeRunnerKind::Python);
    }

    #[test]
    fn js_ts_detects_describe_it_test_and_package_json_scripts() {
        let ts_src = "\
import { describe, it, test } from 'vitest';

describe('CartEngine', () => {
  it('computes subtotal', () => {});
  test('applies coupon code', async () => {});
});
";
        let items = detect_gutter_items("/ws/src/cart.test.ts", ts_src, &[]);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].kind, IdeGutterKind::Suite);
        assert_eq!(items[0].run_spec.runner, IdeRunnerKind::Vitest);
        assert_eq!(items[1].kind, IdeGutterKind::Test);
        assert_eq!(
            items[1].run_spec.test_filter.as_deref(),
            Some("computes subtotal")
        );
        assert_eq!(
            items[2].run_spec.test_filter.as_deref(),
            Some("applies coupon code")
        );

        let pkg_json = r#"{
  "name": "demo",
  "scripts": {
    "dev": "vite",
    "test:unit": "vitest run"
  }
}"#;
        let pkg_items = detect_gutter_items("/ws/package.json", pkg_json, &[]);
        assert_eq!(pkg_items.len(), 2);
        assert_eq!(pkg_items[0].line, 4);
        assert_eq!(pkg_items[0].kind, IdeGutterKind::Main);
        assert_eq!(pkg_items[0].run_spec.runner, IdeRunnerKind::Npm);
        assert_eq!(pkg_items[1].line, 5);
        assert_eq!(pkg_items[1].kind, IdeGutterKind::Test);
    }

    #[test]
    fn plugin_detectors_extend_gutter_for_kotlin_and_makefile() {
        let kotlin_det = PluginGutterDetector {
            language: "kotlin".into(),
            extensions: vec![".kt".into()],
            pattern: r"^\s*fun\s+(main)\s*\(".into(),
            kind: IdeGutterKind::Main,
            runner: IdeRunnerKind::Custom,
            command_template: "kotlinc".into(),
            args_template: vec!["{file}".into()],
            label_prefix: Some("Run Kotlin".into()),
        };
        let kt_items = detect_gutter_items(
            "/ws/src/App.kt",
            "package demo\nfun main() {\n  println(\"ok\")\n}\n",
            &[kotlin_det],
        );
        assert_eq!(kt_items.len(), 1);
        assert_eq!(kt_items[0].line, 2);
        assert_eq!(kt_items[0].label, "Run Kotlin main");
        assert_eq!(kt_items[0].run_spec.args, vec!["/ws/src/App.kt"]);
    }

    #[test]
    fn go_subtests_shell_functions_bats_and_deno_json_tasks_detected_instantly() {
        let go_sub = "\
package admission

import \"testing\"

func TestAdmissionService(t *testing.T) {
    t.Run(\"allows vip player\", func(t *testing.T) {})
    t.Run(`rejects banned ip`, func(t *testing.T) {})
}
";
        let go_items = detect_gutter_items("/ws/service_test.go", go_sub, &[]);
        assert_eq!(go_items.len(), 3);
        assert_eq!(
            go_items[1].run_spec.test_filter.as_deref(),
            Some("^TestAdmissionService$/^allows_vip_player$")
        );
        assert_eq!(
            go_items[2].run_spec.test_filter.as_deref(),
            Some("^TestAdmissionService$/^rejects_banned_ip$")
        );

        let sh_src = "\
#!/usr/bin/env bash
set -euo pipefail

deploy_staging() {
  echo deploying
}

function rollback_release {
  echo rolling back
}
";
        let sh_items = detect_gutter_items("/ws/scripts/deploy.sh", sh_src, &[]);
        assert_eq!(sh_items.len(), 3);
        assert_eq!(sh_items[0].line, 1);
        assert_eq!(sh_items[0].run_spec.runner, IdeRunnerKind::Shell);
        assert_eq!(sh_items[1].label, "Run deploy_staging()");
        assert_eq!(sh_items[2].label, "Run rollback_release()");

        let bats_src = "\
#!/usr/bin/env bats
@test \"health endpoint returns 200\" {
  [ 1 -eq 1 ]
}
";
        let bats_items = detect_gutter_items("/ws/tests/smoke.bats", bats_src, &[]);
        assert_eq!(bats_items.len(), 2);
        assert_eq!(bats_items[0].kind, IdeGutterKind::Suite);
        assert_eq!(bats_items[1].kind, IdeGutterKind::Test);

        let deno_jsonc =
            "// deno config\n{\n  \"tasks\": {\n    \"dev\": \"deno run -A main.ts\"\n  }\n}";
        let deno_items = detect_gutter_items("/ws/deno.jsonc", deno_jsonc, &[]);
        assert_eq!(deno_items.len(), 1);
        assert_eq!(deno_items[0].label, "deno task dev");
    }
}
