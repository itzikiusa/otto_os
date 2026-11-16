//! Multi-language Gutter Run & Debug detector (`▶ Run` / `🐞 Debug`).
//!
//! Scans source files in Go, Java, Rust, Python, JavaScript/TypeScript
//! (including `package.json` scripts), plus runtime [`PluginGutterDetector`]
//! rules, returning 1-based [`IdeGutterItem`] entries with ready-to-launch
//! `IdeQuickRunSpec` configurations for both test runners and DAP debugging.

mod compiled;
mod scripting;
#[cfg(test)]
mod tests;

use crate::plugins::IdePluginRegistry;
use crate::types::{detect_language, IdeGutterItem, IdeRunnerKind, PluginGutterDetector};

/// Detect runnable and debuggable gutter items (`main`, unit tests, suites,
/// benchmarks, `package.json` scripts, and active plugin detectors), consulting
/// runtime plugin language contributions before static extension fallback.
pub fn detect_gutter_items_with_plugins(
    path: &str,
    content: &str,
    plugins: &IdePluginRegistry,
) -> Vec<IdeGutterItem> {
    let normalized_path = path.replace('\\', "/");
    let plugin_lang = plugins
        .detect_language_for_path(&normalized_path)
        .map(|l| l.id);
    let extra = plugins.active_gutter_detectors();
    detect_gutter_items_inner(&normalized_path, content, plugin_lang.as_deref(), &extra)
}

/// Detect runnable and debuggable gutter items (`main`, unit tests, suites,
/// benchmarks, `package.json` scripts, and active plugin detectors).
pub fn detect_gutter_items(
    path: &str,
    content: &str,
    extra_detectors: &[PluginGutterDetector],
) -> Vec<IdeGutterItem> {
    let normalized_path = path.replace('\\', "/");
    detect_gutter_items_inner(&normalized_path, content, None, extra_detectors)
}

fn detect_gutter_items_inner(
    normalized_path: &str,
    content: &str,
    plugin_lang: Option<&str>,
    extra_detectors: &[PluginGutterDetector],
) -> Vec<IdeGutterItem> {
    let base_name = normalized_path
        .rsplit('/')
        .next()
        .unwrap_or(normalized_path);
    let dir = parent_dir(normalized_path);
    let lang = plugin_lang
        .or_else(|| detect_language(normalized_path))
        .unwrap_or("");

    let mut items = Vec::new();
    match base_name {
        "package.json" => {
            scripting::detect_package_json_scripts(normalized_path, &dir, content, &mut items);
        }
        "deno.json" | "deno.jsonc" => {
            scripting::detect_json_task_map(
                normalized_path,
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
            scripting::detect_json_task_map(
                normalized_path,
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
            "go" => compiled::detect_go(normalized_path, &dir, content, &mut items),
            "java" => compiled::detect_java(normalized_path, &dir, content, &mut items),
            "rust" => compiled::detect_rust(normalized_path, &dir, content, &mut items),
            "python" => scripting::detect_python(normalized_path, &dir, content, &mut items),
            "javascript" | "typescript" => {
                scripting::detect_js_ts(normalized_path, &dir, lang, content, &mut items);
            }
            "shell" => {
                scripting::detect_shell(normalized_path, &dir, base_name, content, &mut items);
            }
            _ => {}
        },
    }

    if !extra_detectors.is_empty() {
        scripting::apply_plugin_detectors(
            normalized_path,
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

pub(super) fn strip_line_comment(line: &str) -> &str {
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}
