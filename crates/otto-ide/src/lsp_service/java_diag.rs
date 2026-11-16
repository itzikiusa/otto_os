//! Stateless `< 30ms` / `< 5 MB` Java Unrecognized-Import, Unknown-Type, Unused-Import,
//! Package-Mismatch, and Lombok (`@Slf4j`, `@Data`, `@Builder`, …) Diagnostics + Quick Fixes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

use crate::java_workspace::{global_jdk_index, JavaWorkspaceGraph};
use crate::types::{IdeDiagnostic, IdeQuickFix, IdeTextEdit};

use super::code_mask_for_line;
use super::java_diag_index::{collect_same_package_types, load_or_build_java_project_index};
pub(crate) use super::java_diag_index::{
    expected_package_from_path, find_import_candidates_by_simple,
};

pub(super) const KNOWN_LOMBOK_CLASSES: &[&str] = &[
    "lombok.AllArgsConstructor",
    "lombok.Builder",
    "lombok.Cleanup",
    "lombok.CustomLog",
    "lombok.Data",
    "lombok.EqualsAndHashCode",
    "lombok.Generated",
    "lombok.Getter",
    "lombok.Locked",
    "lombok.NoArgsConstructor",
    "lombok.NonNull",
    "lombok.RequiredArgsConstructor",
    "lombok.Setter",
    "lombok.Singular",
    "lombok.SneakyThrows",
    "lombok.StandardException",
    "lombok.Synchronized",
    "lombok.ToString",
    "lombok.Value",
    "lombok.With",
    "lombok.val",
    "lombok.var",
    "lombok.extern.apachecommons.CommonsLog",
    "lombok.extern.flogger.Flogger",
    "lombok.extern.java.Log",
    "lombok.extern.jbosslog.JBossLog",
    "lombok.extern.log4j.Log4j",
    "lombok.extern.log4j.Log4j2",
    "lombok.extern.slf4j.Slf4j",
    "lombok.extern.slf4j.XSlf4j",
    "lombok.experimental.Accessors",
    "lombok.experimental.Delegate",
    "lombok.experimental.ExtensionMethod",
    "lombok.experimental.FieldDefaults",
    "lombok.experimental.FieldNameConstants",
    "lombok.experimental.Helper",
    "lombok.experimental.Jacksonized",
    "lombok.experimental.NonFinal",
    "lombok.experimental.PackagePrivate",
    "lombok.experimental.StandardException",
    "lombok.experimental.SuperBuilder",
    "lombok.experimental.Tolerate",
    "lombok.experimental.UtilityClass",
];

const LOMBOK_LOG_ANNOTATIONS: &[&str] = &[
    "Slf4j",
    "Log4j2",
    "Log4j",
    "CommonsLog",
    "Log",
    "JBossLog",
    "Flogger",
    "XSlf4j",
    "CustomLog",
];

const BUILTIN_JAVA_TYPES: &[&str] = &[
    "Boolean",
    "Byte",
    "Character",
    "Class",
    "ClassLoader",
    "Cloneable",
    "Comparable",
    "Compiler",
    "Deprecated",
    "Double",
    "Enum",
    "Error",
    "Exception",
    "Float",
    "FunctionalInterface",
    "IllegalArgumentException",
    "IllegalStateException",
    "IndexOutOfBoundsException",
    "Integer",
    "InterruptedException",
    "Iterable",
    "Long",
    "Math",
    "NullPointerException",
    "Number",
    "NumberFormatException",
    "Object",
    "Override",
    "Package",
    "Process",
    "ProcessBuilder",
    "Readable",
    "Record",
    "Runnable",
    "Runtime",
    "RuntimeException",
    "SafeVarargs",
    "SecurityException",
    "Short",
    "StackTraceElement",
    "StrictMath",
    "String",
    "StringBuffer",
    "StringBuilder",
    "SuppressWarnings",
    "System",
    "Thread",
    "ThreadLocal",
    "Throwable",
    "UnsupportedOperationException",
    "Void",
    "var",
];

#[derive(Debug, Clone)]
struct ParsedJavaImport {
    line_num: u32,
    start_col: u32,
    end_col: u32,
    is_static: bool,
    is_wildcard: bool,
    fqn: String,
    simple_or_member: String,
}

/// Build line-deletion `IdeTextEdit` (`1`-based coordinates).
pub(crate) fn make_delete_line_edit(path: &str, lines: &[&str], line_num: u32) -> IdeTextEdit {
    let total = lines.len() as u32;
    if line_num < total {
        IdeTextEdit {
            path: path.to_string(),
            start_line: line_num,
            start_col: 1,
            end_line: line_num + 1,
            end_col: 1,
            new_text: String::new(),
        }
    } else {
        let len = lines
            .get(line_num.saturating_sub(1) as usize)
            .map(|l| l.encode_utf16().count() as u32)
            .unwrap_or(0);
        IdeTextEdit {
            path: path.to_string(),
            start_line: line_num,
            start_col: 1,
            end_line: line_num,
            end_col: len + 1,
            new_text: String::new(),
        }
    }
}

/// Build line-replacement `IdeTextEdit` (`1`-based coordinates, preserving trailing newline).
pub(crate) fn make_replace_line_edit(
    path: &str,
    lines: &[&str],
    line_num: u32,
    replacement_line: &str,
) -> IdeTextEdit {
    let end_col = lines
        .get(line_num.saturating_sub(1) as usize)
        .map(|l| l.encode_utf16().count() as u32 + 1)
        .unwrap_or(1);
    IdeTextEdit {
        path: path.to_string(),
        start_line: line_num,
        start_col: 1,
        end_line: line_num,
        end_col,
        new_text: replacement_line.to_string(),
    }
}

/// Compute the 1-based line number where a new Java `import ...;\n` statement should be inserted.
pub(crate) fn find_java_import_insert_line(lines: &[&str]) -> (u32, String) {
    let mut last_import_line = None;
    let mut package_line = None;
    for (idx, raw) in lines.iter().enumerate() {
        let t = raw.trim();
        if t.starts_with("package ") && t.ends_with(';') {
            package_line = Some((idx + 1) as u32);
        } else if t.starts_with("import ") && t.ends_with(';') {
            last_import_line = Some((idx + 1) as u32);
        } else if t.starts_with("public class ")
            || t.starts_with("class ")
            || t.starts_with("public interface ")
            || t.starts_with("public enum ")
            || t.starts_with("public record ")
        {
            break;
        }
    }
    if let Some(li) = last_import_line {
        (li + 1, String::new())
    } else if let Some(pl) = package_line {
        (pl + 1, "\n".to_string())
    } else {
        (1, String::new())
    }
}

pub(crate) fn make_java_add_import_edit(path: &str, lines: &[&str], fqn: &str) -> IdeTextEdit {
    let (ins_line, leading) = find_java_import_insert_line(lines);
    let total = lines.len() as u32;
    if ins_line <= total {
        IdeTextEdit {
            path: path.to_string(),
            start_line: ins_line,
            start_col: 1,
            end_line: ins_line,
            end_col: 1,
            new_text: format!("{leading}import {fqn};\n"),
        }
    } else {
        let last_ln = total.max(1);
        let end_col = lines
            .last()
            .map(|l| l.encode_utf16().count() as u32 + 1)
            .unwrap_or(1);
        IdeTextEdit {
            path: path.to_string(),
            start_line: last_ln,
            start_col: end_col,
            end_line: last_ln,
            end_col,
            new_text: format!("\nimport {fqn};\n"),
        }
    }
}

/// Run stateless `< 30ms` Java diagnostics on `content`.
#[allow(clippy::disallowed_methods)]
pub(crate) fn analyze_java_diagnostics(
    path: &str,
    content: &str,
    project_roots: &[PathBuf],
    preferred_java_home: Option<&Path>,
    env_map: &std::collections::BTreeMap<String, String>,
) -> Vec<IdeDiagnostic> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    let project_idx = load_or_build_java_project_index(project_roots, env_map);
    let ws = &project_idx.ws;
    let jdk = global_jdk_index(preferred_java_home);
    let jdk = jdk.as_ref();
    let ext_classes = &project_idx.ext_classes;
    let ext_packages = &project_idx.ext_packages;
    let ext_by_simple = &project_idx.ext_by_simple;
    let ws_by_simple = &project_idx.ws_by_simple;

    let mut pkg_decl: Option<(u32, String)> = None;
    let mut imports = Vec::new();
    let mut annotations_used = HashSet::new();
    let mut declared_types = HashSet::new();
    let mut declared_vars = HashSet::new();

    static RE_TYPE_DECL: OnceLock<Regex> = OnceLock::new();
    static RE_ANNOT: OnceLock<Regex> = OnceLock::new();
    static RE_GENERICS: OnceLock<Regex> = OnceLock::new();
    let type_decl_re = RE_TYPE_DECL.get_or_init(|| {
        Regex::new(r"\b(?:class|interface|enum|record|@interface)\s+([A-Za-z_$][A-Za-z0-9_$]*)")
            .unwrap()
    });
    let annot_re = RE_ANNOT.get_or_init(|| Regex::new(r"@([A-Za-z_$][A-Za-z0-9_$.]*)").unwrap());
    let generics_re = RE_GENERICS
        .get_or_init(|| Regex::new(r"<([A-Z][A-Za-z0-9_$,\s?&extendsSuper]*)>").unwrap());

    let mut in_block = false;
    let mut code_lines: Vec<String> = Vec::with_capacity(lines.len());

    for (idx, &raw) in lines.iter().enumerate() {
        let ln = (idx + 1) as u32;
        let mask = code_mask_for_line("java", raw, &mut in_block);
        let code_only: String = raw
            .char_indices()
            .map(|(bi, ch)| {
                if mask.get(bi).copied().unwrap_or(false) {
                    ch
                } else {
                    ' '
                }
            })
            .collect();
        let trimmed = code_only.trim();

        if pkg_decl.is_none() {
            if let Some(rest) = trimmed
                .strip_prefix("package ")
                .and_then(|s| s.strip_suffix(';'))
            {
                pkg_decl = Some((ln, rest.trim().to_string()));
            }
        }
        if let Some(imp) = parse_java_import_line(raw, trimmed, ln) {
            imports.push(imp);
        }
        for cap in annot_re.captures_iter(&code_only) {
            let ann = cap[1].rsplit('.').next().unwrap_or(&cap[1]).to_string();
            annotations_used.insert(ann);
        }
        for cap in type_decl_re.captures_iter(&code_only) {
            declared_types.insert(cap[1].to_string());
            // Also record single-letter or PascalCase generic type params (`class Box<T, R>`).
            let after = &code_only[cap.get(0).map(|m| m.end()).unwrap_or(0)..];
            if let Some(gcap) = generics_re.captures(after) {
                for part in gcap[1].split(',') {
                    if let Some(tp) = part.split_whitespace().next() {
                        let clean =
                            tp.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_');
                        if !clean.is_empty() {
                            declared_types.insert(clean.to_string());
                        }
                    }
                }
            }
        }
        collect_declared_vars_on_line(trimmed, &mut declared_vars, &mut declared_types);
        code_lines.push(code_only);
    }

    // Lombok `@Slf4j` / `@Log4j2` / `@CommonsLog` / `@Log` synthesizes `log` field!
    let has_lombok_logger = LOMBOK_LOG_ANNOTATIONS
        .iter()
        .any(|ann| annotations_used.contains(*ann));
    if has_lombok_logger {
        declared_vars.insert("log".to_string());
    }

    let mut diagnostics = Vec::new();

    // 1. Check `package` declaration vs `src/main/java/...` or `src/test/java/...` directory layout.
    if let Some((pkg_ln, ref declared_pkg)) = pkg_decl {
        if let Some(expected_pkg) = expected_package_from_path(path) {
            if !expected_pkg.is_empty() && declared_pkg != &expected_pkg {
                let replacement = format!("package {expected_pkg};");
                let edit = make_replace_line_edit(path, &lines, pkg_ln, &replacement);
                diagnostics.push(IdeDiagnostic {
                    line: pkg_ln,
                    col: 1,
                    end_line: pkg_ln,
                    end_col: (lines[(pkg_ln - 1) as usize].encode_utf16().count() as u32).max(1),
                    severity: "warning".into(),
                    code: "package_mismatch".into(),
                    source: "otto-java".into(),
                    message: format!(
                        "Package '{declared_pkg}' does not match expected directory package '{expected_pkg}'"
                    ),
                    quick_fixes: vec![IdeQuickFix {
                        title: format!("Change package to '{expected_pkg}'"),
                        line: pkg_ln,
                        end_line: pkg_ln,
                        replacement,
                        delete_line: false,
                        is_preferred: true,
                        edits: vec![edit],
                    }],
                });
            }
        }
    }

    // 2. Validate every `import` statement & track resolved vs unresolved simple names + wildcards.
    let mut resolved_imported_types = HashSet::new();
    let mut unresolved_imported_simples: HashMap<String, String> = HashMap::new();
    let mut wildcard_packages = Vec::new();

    // Under Maven/Gradle the classpath is transitive (parents, BOMs, starters),
    // which this index can't see: an import it doesn't know is trusted, not an error.
    let build_managed = has_build_file_above(path, project_roots);
    // A wildcard from such a package may supply any simple name, so step 5 can't judge.
    let unknown_wildcard = build_managed
        && imports.iter().any(|i| {
            i.is_wildcard && !resolve_java_import_exists(i, ws, jdk, ext_classes, ext_packages)
        });
    for imp in &imports {
        let exists =
            build_managed || resolve_java_import_exists(imp, ws, jdk, ext_classes, ext_packages);
        if !exists {
            if !imp.is_wildcard && !imp.is_static {
                unresolved_imported_simples.insert(imp.simple_or_member.clone(), imp.fqn.clone());
            }
            let del_edit = make_delete_line_edit(path, &lines, imp.line_num);
            let mut fixes = vec![IdeQuickFix {
                title: format!("Remove unresolved import '{}'", imp.fqn),
                line: imp.line_num,
                end_line: imp.line_num,
                replacement: String::new(),
                delete_line: true,
                is_preferred: true,
                edits: vec![del_edit],
            }];
            for cand in find_import_candidates_by_simple(
                &imp.simple_or_member,
                ws_by_simple,
                ext_by_simple,
                jdk,
            ) {
                if cand != imp.fqn {
                    let rep_line = format!("import {cand};");
                    let rep_edit = make_replace_line_edit(path, &lines, imp.line_num, &rep_line);
                    fixes.push(IdeQuickFix {
                        title: format!("Import '{cand}'"),
                        line: imp.line_num,
                        end_line: imp.line_num,
                        replacement: rep_line,
                        delete_line: false,
                        is_preferred: false,
                        edits: vec![rep_edit],
                    });
                }
            }
            diagnostics.push(IdeDiagnostic {
                line: imp.line_num,
                col: imp.start_col,
                end_line: imp.line_num,
                end_col: imp.end_col,
                severity: "error".into(),
                code: "unresolved_import".into(),
                source: "otto-java".into(),
                message: format!(
                    "Cannot resolve symbol '{}' in import '{}'",
                    imp.simple_or_member, imp.fqn
                ),
                quick_fixes: fixes,
            });
        } else if imp.is_wildcard {
            wildcard_packages.push(imp.fqn.trim_end_matches(".*").to_string());
        } else {
            resolved_imported_types.insert(imp.simple_or_member.clone());
        }
    }

    // 3. Collect identifier references outside `package` and `import` lines (plus Javadoc `@link`/`@see`).
    let import_line_nums: HashSet<u32> = imports.iter().map(|i| i.line_num).collect();
    let pkg_line_num = pkg_decl.as_ref().map(|(l, _)| *l).unwrap_or(0);
    let mut body_ident_counts: HashMap<String, usize> = HashMap::new();
    static RE_IDENT: OnceLock<Regex> = OnceLock::new();
    let ident_re = RE_IDENT.get_or_init(|| Regex::new(r"\b([A-Za-z_$][A-Za-z0-9_$]*)\b").unwrap());

    for (idx, code_ln) in code_lines.iter().enumerate() {
        let ln = (idx + 1) as u32;
        if ln == pkg_line_num || import_line_nums.contains(&ln) {
            continue;
        }
        for cap in ident_re.captures_iter(code_ln) {
            *body_ident_counts.entry(cap[1].to_string()).or_insert(0) += 1;
        }
        // Also count Javadoc `@link Foo` / `@see Foo` references so Javadoc imports aren't marked unused!
        let raw_line = lines[idx];
        if raw_line.contains("@link") || raw_line.contains("@see") || raw_line.contains("@throws") {
            for cap in ident_re.captures_iter(raw_line) {
                *body_ident_counts.entry(cap[1].to_string()).or_insert(0) += 1;
            }
        }
    }

    // 4. Unused import warnings (`code: "unused_import"`, `severity: "warning"`).
    for imp in &imports {
        if imp.is_wildcard {
            continue;
        }
        // Skip if already flagged as unresolved.
        if diagnostics
            .iter()
            .any(|d| d.line == imp.line_num && d.code == "unresolved_import")
        {
            continue;
        }
        if !body_ident_counts.contains_key(&imp.simple_or_member) {
            let del_edit = make_delete_line_edit(path, &lines, imp.line_num);
            diagnostics.push(IdeDiagnostic {
                line: imp.line_num,
                col: imp.start_col,
                end_line: imp.line_num,
                end_col: imp.end_col,
                severity: "warning".into(),
                code: "unused_import".into(),
                source: "otto-java".into(),
                message: format!("Unused import '{}'", imp.fqn),
                quick_fixes: vec![IdeQuickFix {
                    title: format!("Remove unused import '{}'", imp.fqn),
                    line: imp.line_num,
                    end_line: imp.line_num,
                    replacement: String::new(),
                    delete_line: true,
                    is_preferred: true,
                    edits: vec![del_edit],
                }],
            });
        }
    }

    // 5. Flag body usages of unresolved imports AND unknown `PascalCase` type references!
    let current_pkg = pkg_decl.as_ref().map(|(_, p)| p.as_str()).unwrap_or("");
    let same_pkg_types = collect_same_package_types(path, current_pkg, ws);
    let mut emitted_type_spans = HashSet::new();

    for (idx, code_ln) in code_lines.iter().enumerate() {
        let ln = (idx + 1) as u32;
        if unknown_wildcard || ln == pkg_line_num || import_line_nums.contains(&ln) {
            continue;
        }
        for m in ident_re.find_iter(code_ln) {
            let tok = m.as_str();
            let byte_start = m.start();
            // Skip member accesses (`foo.Bar` where `.` precedes `Bar`).
            if byte_start > 0 && code_ln.as_bytes()[byte_start - 1] == b'.' {
                continue;
            }
            let is_explicitly_unresolved = unresolved_imported_simples.contains_key(tok);
            let is_pascal_type_candidate = tok.len() >= 2
                && tok.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                && tok.chars().any(|c| c.is_ascii_lowercase())
                && is_type_position(code_ln, byte_start, m.end());

            if !is_explicitly_unresolved && !is_pascal_type_candidate {
                continue;
            }
            if !is_explicitly_unresolved
                && is_type_in_scope(
                    tok,
                    &declared_types,
                    &declared_vars,
                    &resolved_imported_types,
                    &same_pkg_types,
                    &wildcard_packages,
                    ws,
                    jdk,
                    ext_classes,
                )
            {
                continue;
            }

            let start_col = code_ln[..byte_start].encode_utf16().count() as u32 + 1;
            let end_col = start_col + tok.encode_utf16().count() as u32;
            if !emitted_type_spans.insert((ln, start_col)) {
                continue;
            }
            let candidates =
                find_import_candidates_by_simple(tok, ws_by_simple, ext_by_simple, jdk);
            let fixes: Vec<IdeQuickFix> = candidates
                .into_iter()
                .enumerate()
                .map(|(i, cand)| {
                    let edit = make_java_add_import_edit(path, &lines, &cand);
                    let ins_ln = edit.start_line;
                    IdeQuickFix {
                        title: format!("Import '{cand}'"),
                        line: ins_ln,
                        end_line: ins_ln,
                        replacement: format!("import {cand};"),
                        delete_line: false,
                        is_preferred: i == 0,
                        edits: vec![edit],
                    }
                })
                .collect();

            diagnostics.push(IdeDiagnostic {
                line: ln,
                col: start_col,
                end_line: ln,
                end_col,
                severity: "error".into(),
                code: "unresolved_type".into(),
                source: "otto-java".into(),
                message: format!("Cannot resolve symbol '{tok}'"),
                quick_fixes: fixes,
            });
        }
    }

    diagnostics
}

/// Whether a `pom.xml` / `build.gradle*` sits in the file's dir or above it
/// (inside the project roots when there are any).
fn has_build_file_above(path: &str, project_roots: &[PathBuf]) -> bool {
    Path::new(path)
        .ancestors()
        .skip(1)
        .take(16)
        .take_while(|d| project_roots.is_empty() || project_roots.iter().any(|r| d.starts_with(r)))
        .any(|d| {
            ["pom.xml", "build.gradle", "build.gradle.kts"]
                .iter()
                .any(|f| d.join(f).is_file())
        })
}

fn parse_java_import_line(raw: &str, trimmed: &str, line_num: u32) -> Option<ParsedJavaImport> {
    let rest = trimmed.strip_prefix("import ")?.strip_suffix(';')?.trim();
    let (is_static, target) = match rest.strip_prefix("static ") {
        Some(s) => (true, s.trim()),
        None => (false, rest),
    };
    if target.is_empty() || target.contains(' ') {
        return None;
    }
    let is_wildcard = target.ends_with(".*");
    let simple = if is_wildcard {
        "*".to_string()
    } else {
        target.rsplit('.').next()?.to_string()
    };
    let start_col = raw
        .find(target)
        .map(|b| raw[..b].encode_utf16().count() as u32 + 1)
        .unwrap_or(1);
    let end_col = start_col + target.encode_utf16().count() as u32;
    Some(ParsedJavaImport {
        line_num,
        start_col,
        end_col,
        is_static,
        is_wildcard,
        fqn: target.to_string(),
        simple_or_member: simple,
    })
}

fn resolve_java_import_exists(
    imp: &ParsedJavaImport,
    ws: &JavaWorkspaceGraph,
    jdk: &crate::java_workspace::JdkClassIndex,
    ext_classes: &HashSet<String>,
    ext_packages: &HashSet<String>,
) -> bool {
    let fqn = imp.fqn.as_str();
    if imp.is_wildcard {
        let prefix = fqn.trim_end_matches(".*");
        if prefix == "lombok"
            || prefix.starts_with("lombok.extern")
            || prefix == "lombok.experimental"
        {
            return true;
        }
        if imp.is_static {
            if jdk.fqns.contains(prefix) || ext_classes.contains(prefix) {
                return true;
            }
            let rel_java = format!("{}.java", prefix.replace('.', "/"));
            return ws
                .all_source_roots(true)
                .iter()
                .any(|sr| sr.join(&rel_java).is_file());
        }
        if jdk.packages.contains(prefix) || ext_packages.contains(prefix) {
            return true;
        }
        let rel = prefix.replace('.', "/");
        return ws
            .all_source_roots(true)
            .iter()
            .any(|sr| sr.join(&rel).is_dir());
    }

    if KNOWN_LOMBOK_CLASSES.contains(&fqn) {
        return true;
    }

    let class_candidates: Vec<String> = if imp.is_static {
        // `import static pkg.Class.MEMBER;` -> check `pkg.Class` (or `pkg.Outer.Inner`)
        let mut v = vec![fqn.to_string()];
        if let Some((parent, _)) = fqn.rsplit_once('.') {
            v.push(parent.to_string());
        }
        v
    } else {
        let mut v = vec![fqn.to_string()];
        // Nested static class import `pkg.Outer.Inner` -> `<src_root>/pkg/Outer.java`
        if let Some((parent, _)) = fqn.rsplit_once('.') {
            if parent
                .rsplit('.')
                .next()
                .and_then(|s| s.chars().next())
                .is_some_and(|c| c.is_ascii_uppercase())
            {
                v.push(parent.to_string());
            }
        }
        v
    };

    for cand in &class_candidates {
        if jdk.fqns.contains(cand) || ext_classes.contains(cand) {
            return true;
        }
        let rel_java = format!("{}.java", cand.replace('.', "/"));
        let rel_kt = format!("{}.kt", cand.replace('.', "/"));
        for sr in ws.all_source_roots(true) {
            if sr.join(&rel_java).is_file() || sr.join(&rel_kt).is_file() {
                return true;
            }
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn is_type_in_scope(
    tok: &str,
    declared_types: &HashSet<String>,
    declared_vars: &HashSet<String>,
    resolved_imported_types: &HashSet<String>,
    same_pkg_types: &HashSet<String>,
    wildcard_packages: &[String],
    ws: &JavaWorkspaceGraph,
    jdk: &crate::java_workspace::JdkClassIndex,
    ext_classes: &HashSet<String>,
) -> bool {
    if BUILTIN_JAVA_TYPES.contains(&tok)
        || declared_types.contains(tok)
        || declared_vars.contains(tok)
        || resolved_imported_types.contains(tok)
        || same_pkg_types.contains(tok)
        || jdk.fqns.contains(&format!("java.lang.{tok}"))
    {
        return true;
    }
    for wp in wildcard_packages {
        let fqn = format!("{wp}.{tok}");
        if jdk.fqns.contains(&fqn)
            || ext_classes.contains(&fqn)
            || KNOWN_LOMBOK_CLASSES.contains(&fqn.as_str())
        {
            return true;
        }
        let rel = format!("{}.java", fqn.replace('.', "/"));
        if ws
            .all_source_roots(true)
            .iter()
            .any(|sr| sr.join(&rel).is_file())
        {
            return true;
        }
    }
    false
}

fn is_type_position(code_line: &str, start: usize, end: usize) -> bool {
    let before = code_line[..start].trim_end();
    let after = code_line[end..].trim_start();
    if before.ends_with('@')
        || before.ends_with("new")
        || before.ends_with("extends")
        || before.ends_with("implements")
        || before.ends_with("throws")
        || before.ends_with("instanceof")
        || before.ends_with('<')
        || before.ends_with('(')
        || before.ends_with(',')
    {
        return true;
    }
    // `Foo bar` or `Foo<Bar> baz` or `Foo.method(...)` or `Foo[] arr`
    after.starts_with('<')
        || after.starts_with("[]")
        || after.starts_with('.')
        || after
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
}

fn collect_declared_vars_on_line(
    trimmed: &str,
    declared_vars: &mut HashSet<String>,
    declared_types: &mut HashSet<String>,
) {
    static RE_METHOD_GEN: OnceLock<Regex> = OnceLock::new();
    let mg_re = RE_METHOD_GEN.get_or_init(|| Regex::new(r"<([A-Z][A-Za-z0-9_]*)>").unwrap());
    for cap in mg_re.captures_iter(trimmed) {
        if cap[1].len() <= 2 {
            declared_types.insert(cap[1].to_string());
        }
    }
    static RE_VAR: OnceLock<Regex> = OnceLock::new();
    let var_re = RE_VAR.get_or_init(|| {
        Regex::new(
            r"\b(?:[A-Za-z_$][A-Za-z0-9_$<>,.\[\]?]*)\s+([a-z_$][A-Za-z0-9_$]*)\s*(?:=|;|,|\)|:)",
        )
        .unwrap()
    });
    for cap in var_re.captures_iter(trimmed) {
        declared_vars.insert(cap[1].to_string());
    }
}

#[cfg(test)]
mod utf16_col_tests {
    #[test]
    fn line_replacement_spans_the_whole_line_in_utf16_units() {
        let lines = ["package wrong; // 🚀"];
        let edit = super::make_replace_line_edit("A.java", &lines, 1, "package ok;");
        // 18 BMP chars + one surrogate pair = 20 UTF-16 units (19 chars); EOL + 1.
        assert_eq!(edit.end_col, 21);
    }
}
