//! Source ↔ test file mapping (heuristic, language-aware). Used to add `test_of`
//! edges to the code graph and to surface "the tests for this file" in context.

/// Given a path, return the source file it appears to test, or `None` when it is
/// not a recognized test file. Paths are repo-relative; the returned path keeps
/// the same directory unless the convention implies otherwise.
pub fn source_for_test(path: &str) -> Option<String> {
    let (dir, file) = split(path);
    let (stem, ext) = match file.rsplit_once('.') {
        Some((s, e)) => (s, e),
        None => return None,
    };

    let base = match ext {
        // Go: foo_test.go → foo.go
        "go" => stem.strip_suffix("_test").map(|b| format!("{b}.go"))?,
        // Python: test_foo.py → foo.py ; foo_test.py → foo.py
        "py" => {
            if let Some(b) = stem.strip_prefix("test_") {
                format!("{b}.py")
            } else if let Some(b) = stem.strip_suffix("_test") {
                format!("{b}.py")
            } else {
                return None;
            }
        }
        // Rust: foo_test.rs / foo_tests.rs → foo.rs
        "rs" => {
            let b = stem.strip_suffix("_test").or_else(|| stem.strip_suffix("_tests"))?;
            format!("{b}.rs")
        }
        // TS/JS: Foo.test.ts / Foo.spec.tsx → Foo.ts / Foo.tsx
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => {
            let b = stem.strip_suffix(".test").or_else(|| stem.strip_suffix(".spec"))?;
            format!("{b}.{ext}")
        }
        _ => return None,
    };

    // A `__tests__/` or `tests/` segment usually sits beside the source dir; try
    // both the same dir and the parent.
    let same = join(dir, &base);
    Some(strip_tests_dir(&same).unwrap_or(same))
}

/// Candidate test file paths for a source file (best-effort; the caller checks
/// which actually exist).
pub fn tests_for_source(path: &str) -> Vec<String> {
    let (dir, file) = split(path);
    let (stem, ext) = match file.rsplit_once('.') {
        Some((s, e)) => (s, e),
        None => return vec![],
    };
    match ext {
        "go" => vec![join(dir, &format!("{stem}_test.go"))],
        "py" => vec![
            join(dir, &format!("test_{stem}.py")),
            join(dir, &format!("{stem}_test.py")),
        ],
        "rs" => vec![join(dir, &format!("{stem}_test.rs"))],
        "ts" | "tsx" | "js" | "jsx" => vec![
            join(dir, &format!("{stem}.test.{ext}")),
            join(dir, &format!("{stem}.spec.{ext}")),
            join(&join(dir, "__tests__"), &format!("{stem}.test.{ext}")),
        ],
        _ => vec![],
    }
}

/// Is this a recognized test file?
pub fn is_test_file(path: &str) -> bool {
    source_for_test(path).is_some()
}

fn split(path: &str) -> (&str, &str) {
    match path.rsplit_once('/') {
        Some((d, f)) => (d, f),
        None => ("", path),
    }
}

fn join(dir: &str, file: &str) -> String {
    if dir.is_empty() {
        file.to_string()
    } else {
        format!("{dir}/{file}")
    }
}

/// `pkg/__tests__/foo.go` → `pkg/foo.go`; `pkg/tests/foo.py` → `pkg/foo.py`.
fn strip_tests_dir(path: &str) -> Option<String> {
    for seg in ["/__tests__/", "/tests/", "/test/"] {
        if let Some(idx) = path.find(seg) {
            let before = &path[..idx];
            let after = &path[idx + seg.len()..];
            return Some(join(before, after));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_test_files_to_sources() {
        assert_eq!(source_for_test("app/login_test.go").as_deref(), Some("app/login.go"));
        assert_eq!(source_for_test("pkg/test_handler.py").as_deref(), Some("pkg/handler.py"));
        assert_eq!(source_for_test("pkg/handler_test.py").as_deref(), Some("pkg/handler.py"));
        assert_eq!(source_for_test("ui/Button.test.ts").as_deref(), Some("ui/Button.ts"));
        assert_eq!(source_for_test("ui/Button.spec.tsx").as_deref(), Some("ui/Button.tsx"));
        assert_eq!(source_for_test("src/parser_test.rs").as_deref(), Some("src/parser.rs"));
        assert_eq!(source_for_test("app/login.go"), None);
    }

    #[test]
    fn suggests_test_files_for_sources() {
        let cands = tests_for_source("app/login.go");
        assert!(cands.contains(&"app/login_test.go".to_string()));
        let ts = tests_for_source("ui/Button.ts");
        assert!(ts.iter().any(|c| c == "ui/Button.test.ts"));
    }
}
