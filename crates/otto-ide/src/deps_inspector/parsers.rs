//! Manifest line-span parsers for `package.json`, `Cargo.toml`, and Python
//! `requirements.txt` / `pyproject.toml`.

use std::sync::OnceLock;

use regex::Regex;

use crate::types::IdeDepItem;

/// 1-based line and column of byte offset `off` in `content`.
fn line_col(content: &str, off: usize) -> (u32, u32) {
    let before = &content[..off];
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    (
        before.matches('\n').count() as u32 + 1,
        before[line_start..].chars().count() as u32 + 1,
    )
}

/// One dependency whose name and version sit at the given byte spans.
fn dep_at(
    content: &str,
    name: &str,
    version: &str,
    range: bool,
    indirect: bool,
    name_span: (usize, usize),
    ver_span: (usize, usize),
) -> IdeDepItem {
    let (line, name_col) = line_col(content, name_span.0);
    let (_, name_end_col) = line_col(content, name_span.1);
    let (version_line, version_col) = line_col(content, ver_span.0);
    let (_, version_end_col) = line_col(content, ver_span.1);
    IdeDepItem {
        name: name.to_string(),
        current_version: version.to_string(),
        latest_version: None,
        latest_patch: None,
        indirect,
        deprecated: None,
        line,
        name_col,
        name_end_col,
        version_line,
        version_col,
        version_end_col,
        block_start_line: line,
        block_end_line: version_line,
        property_name: None,
        managed_in: None,
        vulnerabilities: Vec::new(),
        range,
    }
}

/// Byte offset just past `"key"` followed by `:` at or after `from` (the key's
/// own quotes excluded from the returned start).
fn find_json_key(content: &str, from: usize, key: &str) -> Option<usize> {
    let needle = format!("\"{key}\"");
    let mut at = from;
    while let Some(i) = content.get(at..)?.find(&needle) {
        let start = at + i;
        if content[start + needle.len()..]
            .trim_start()
            .starts_with(':')
        {
            return Some(start + 1);
        }
        at = start + needle.len();
    }
    None
}

/// `package.json` `dependencies` / `devDependencies`, parsed as JSON (a
/// minified file has them all on one line), then located in the text.
pub fn parse_package_json(content: &str) -> Vec<IdeDepItem> {
    let Ok(serde_json::Value::Object(root)) = serde_json::from_str(content) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    for (key, indirect) in [("dependencies", false), ("devDependencies", true)] {
        let (Some(deps), Some(section)) = (
            root.get(key).and_then(|v| v.as_object()),
            find_json_key(content, 0, key),
        ) else {
            continue;
        };
        for (name, raw) in deps {
            let Some(raw_v) = raw.as_str() else { continue };
            if raw_v.starts_with("workspace:") || raw_v.starts_with("file:") {
                continue;
            }
            let Some(n_start) = find_json_key(content, section, name) else {
                continue;
            };
            let n_end = n_start + name.len();
            let quoted = format!("\"{raw_v}\"");
            let Some(v_rel) = content[n_end..].find(&quoted) else {
                continue;
            };
            let v_start = n_end + v_rel + 1;
            let clean = raw_v.trim_start_matches(['^', '~', '>', '=', '<', ' ']);
            // `^1.0.0` is a range: `1.0.0` is its floor, not what is installed.
            let range = clean != raw_v || clean.contains(['x', '*', ' ', '|']);
            items.push(dep_at(
                content,
                name,
                clean,
                range,
                indirect,
                (n_start, n_end),
                (v_start, v_start + raw_v.len()),
            ));
        }
    }
    items.sort_by_key(|i| (i.line, i.name_col));
    items
}

/// `deps` / `dev` for a Cargo dependency table header (without brackets),
/// including `workspace.` and `target.<cfg>.` prefixed tables.
fn cargo_dep_section(header: &str) -> Option<&'static str> {
    if header == "dependencies" || header.ends_with(".dependencies") {
        Some("deps")
    } else if ["dev-dependencies", "build-dependencies"]
        .iter()
        .any(|k| header == *k || header.ends_with(&format!(".{k}")))
    {
        Some("dev")
    } else {
        None
    }
}

pub fn parse_cargo_toml(content: &str) -> Vec<IdeDepItem> {
    static RE_SIMPLE: OnceLock<Regex> = OnceLock::new();
    static RE_TABLE: OnceLock<Regex> = OnceLock::new();
    static RE_VERSION: OnceLock<Regex> = OnceLock::new();
    let simple_re =
        RE_SIMPLE.get_or_init(|| Regex::new(r#"^\s*([A-Za-z0-9_-]+)\s*=\s*"([^"]+)""#).unwrap());
    let table_re = RE_TABLE.get_or_init(|| {
        Regex::new(r#"^\s*([A-Za-z0-9_-]+)\s*=\s*\{[^}]*?\bversion\s*=\s*"([^"]+)""#).unwrap()
    });
    let version_re =
        RE_VERSION.get_or_init(|| Regex::new(r#"^\s*version\s*=\s*"([^"]+)""#).unwrap());
    let mut section = "";
    // `[dependencies.<name>]`: (name, header line no, name start/end cols).
    let mut dotted: Option<(String, u32, u32, u32)> = None;
    let mut items = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let t = line.trim();
        let line_no = (idx + 1) as u32;
        if t.starts_with('[') && t.ends_with(']') {
            let header = t.trim_matches(['[', ']']).trim();
            dotted = None;
            section = cargo_dep_section(header).unwrap_or("");
            if section.is_empty() {
                if let Some((prefix, name)) = header.rsplit_once('.') {
                    if let Some(sec) = cargo_dep_section(prefix) {
                        section = sec;
                        let col = line.find(name).unwrap_or(0);
                        let start = (line[..col].chars().count() + 1) as u32;
                        let end = start + name.chars().count() as u32;
                        dotted = Some((name.to_string(), line_no, start, end));
                    }
                }
            }
            continue;
        }
        if section.is_empty() || t.starts_with('#') {
            continue;
        }
        let (name, name_line, name_col, name_end_col, v) = if let Some((n, hl, sc, ec)) = &dotted {
            let Some(cap) = version_re.captures(line) else {
                continue;
            };
            (n.clone(), *hl, *sc, *ec, cap.get(1).unwrap())
        } else {
            let Some(cap) = simple_re.captures(line).or_else(|| table_re.captures(line)) else {
                continue;
            };
            let n = cap.get(1).unwrap();
            let start = (line[..n.start()].chars().count() + 1) as u32;
            let end = (line[..n.end()].chars().count() + 1) as u32;
            (
                n.as_str().to_string(),
                line_no,
                start,
                end,
                cap.get(2).unwrap(),
            )
        };
        items.push(IdeDepItem {
            name,
            current_version: v.as_str().trim_start_matches(['^', '~', '=']).to_string(),
            latest_version: None,
            latest_patch: None,
            indirect: section == "dev",
            deprecated: None,
            line: name_line,
            name_col,
            name_end_col,
            version_line: line_no,
            version_col: (line[..v.start()].chars().count() + 1) as u32,
            version_end_col: (line[..v.end()].chars().count() + 1) as u32,
            block_start_line: name_line,
            block_end_line: line_no,
            property_name: None,
            managed_in: None,
            vulnerabilities: Vec::new(),
            range: false,
        });
    }
    items
}

/// Replace each Cargo requirement (`1`, `0.4`) with the version `Cargo.lock`
/// resolved for it: the newest locked version with the same major (and minor
/// for `0.x`). Positions keep pointing at the `Cargo.toml` requirement.
pub fn apply_cargo_lock_versions(items: &mut [IdeDepItem], lock: &str) {
    let mut locked: Vec<(&str, &str)> = Vec::new();
    let mut name = None;
    for line in lock.lines() {
        let t = line.trim();
        if t == "[[package]]" {
            name = None;
        } else if let Some(n) = t.strip_prefix("name = ") {
            name = Some(n.trim_matches('"'));
        } else if let (Some(n), Some(v)) = (name, t.strip_prefix("version = ")) {
            locked.push((n, v.trim_matches('"')));
            name = None;
        }
    }
    use super::enrich::{compare_semver, parse_semver_tuple};
    for item in items.iter_mut() {
        let (maj, min, _) = parse_semver_tuple(&item.current_version);
        let best = locked
            .iter()
            .filter(|(n, v)| {
                let (lm, ln, _) = parse_semver_tuple(v);
                *n == item.name && lm == maj && (maj != 0 || ln == min)
            })
            .map(|(_, v)| *v)
            .max_by(|a, b| compare_semver(a, b));
        if let Some(v) = best {
            item.current_version = v.to_string();
        }
    }
}

/// `<op> <version>` at the start of a requirement's version part; `op` is
/// empty for a bare version (Poetry's exact pin).
fn version_spec_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"^\s*(===|==|>=|~=|<=|!=|\^|~|>|<)?\s*([0-9][A-Za-z0-9_.*+-]*)"#).unwrap()
    })
}

/// A dependency `name` (at `name_span`) whose version spec `spec` starts at byte `spec_at`.
fn push_spec(
    content: &str,
    name: &str,
    name_span: (usize, usize),
    spec: &str,
    spec_at: usize,
    indirect: bool,
    items: &mut Vec<IdeDepItem>,
) {
    if name.eq_ignore_ascii_case("python") {
        return;
    }
    let Some(cap) = version_spec_re().captures(spec) else {
        return;
    };
    let v = cap.get(2).unwrap();
    // Only `==` (or a bare Poetry version) names the installed version.
    let range = !matches!(cap.get(1).map(|m| m.as_str()), None | Some("==" | "==="));
    let ver_span = (spec_at + v.start(), spec_at + v.end());
    items.push(dep_at(
        content,
        name,
        v.as_str(),
        range,
        indirect,
        name_span,
        ver_span,
    ));
}

/// One PEP 508 requirement (`name[extras] <op> <version>; markers`) at byte `at`.
fn push_requirement(
    content: &str,
    at: usize,
    req: &str,
    indirect: bool,
    items: &mut Vec<IdeDepItem>,
) {
    static RE: OnceLock<Regex> = OnceLock::new();
    let name_re =
        RE.get_or_init(|| Regex::new(r#"^\s*([A-Za-z0-9_.-]+)\s*(?:\[[^\]]*\])?"#).unwrap());
    let Some(cap) = name_re.captures(req) else {
        return;
    };
    let (n, all) = (cap.get(1).unwrap(), cap.get(0).unwrap());
    let name_span = (at + n.start(), at + n.end());
    push_spec(
        content,
        n.as_str(),
        name_span,
        &req[all.end()..],
        at + all.end(),
        indirect,
        items,
    );
}

/// Every `"…"` string in `line` (byte offset `line_at` in the file), each
/// passed as a requirement.
fn push_quoted_requirements(
    content: &str,
    line_at: usize,
    line: &str,
    indirect: bool,
    items: &mut Vec<IdeDepItem>,
) {
    let mut rest = 0;
    while let Some(open) = line[rest..].find(['"', '\'']) {
        let q = line.as_bytes()[rest + open] as char;
        let s = rest + open + 1;
        let Some(len) = line[s..].find(q) else { break };
        push_requirement(content, line_at + s, &line[s..s + len], indirect, items);
        rest = s + len + 1;
    }
}

/// Whether `line` has a `]` outside a string (`"uvicorn[standard]"` keeps the array open).
fn closes_array(line: &str) -> bool {
    let mut quote = None;
    for c in line.chars() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, ']') => return true,
            _ => {}
        }
    }
    false
}

/// `requirements.txt` lines, or `pyproject.toml`'s PEP 621 arrays
/// (`dependencies` / `optional-dependencies`, one line or many) and Poetry
/// tables (`name = "^1.2"` or `name = { version = "…" }`).
pub fn parse_python_deps(fname: &str, content: &str) -> Vec<IdeDepItem> {
    let mut items = Vec::new();
    let mut offset = 0;
    // pyproject state: the current table, and whether a requirements array is open.
    let mut table = String::new();
    let mut in_array = false;
    for line in content.split_inclusive('\n') {
        let at = offset;
        offset += line.len();
        let line = line.trim_end_matches(['\n', '\r']);
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if fname != "pyproject.toml" {
            push_requirement(content, at, line, false, &mut items);
            continue;
        }
        if in_array {
            push_quoted_requirements(content, at, line, table != "project", &mut items);
            in_array = !closes_array(line);
            continue;
        }
        if t.starts_with('[') {
            table = t.trim_matches(['[', ']']).trim().to_string();
            continue;
        }
        let (Some((key, _)), Some(eq)) = (t.split_once('='), line.find('=')) else {
            continue;
        };
        let key = key.trim();
        let (value_at, value_line) = (at + eq + 1, &line[eq + 1..]);
        let poetry = table.starts_with("tool.poetry") && table.ends_with("dependencies");
        if (table == "project" && key == "dependencies") || table == "project.optional-dependencies"
        {
            if let Some(open) = value_line.find('[') {
                let after = &value_line[open + 1..];
                push_quoted_requirements(
                    content,
                    value_at + open + 1,
                    after,
                    table != "project",
                    &mut items,
                );
                in_array = !closes_array(after);
            }
        } else if poetry {
            // `django = "^3.2"` or `requests = { version = "2.19.0", … }`.
            let name = key.trim_matches(['"', '\'']);
            let name_at = at + line.find(name).unwrap_or(0);
            let spec_at = value_line
                .find("version")
                .map_or(0, |i| i + "version".len());
            if let Some(q) = value_line[spec_at..].find('"') {
                let s = spec_at + q + 1;
                let len = value_line[s..].find('"').unwrap_or(0);
                push_spec(
                    content,
                    name,
                    (name_at, name_at + name.len()),
                    &value_line[s..s + len],
                    value_at + s,
                    table != "tool.poetry.dependencies",
                    &mut items,
                );
            }
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(content: &str, line: u32, col: u32, end: u32) -> String {
        let l = content.lines().nth((line - 1) as usize).unwrap();
        l.chars()
            .skip((col - 1) as usize)
            .take((end - col) as usize)
            .collect()
    }

    #[test]
    fn a_minified_package_json_lists_every_dependency_at_its_position() {
        let pkg = r#"{"name":"x","dependencies":{"lodash":"4.17.15","minimist":"^1.2.0"},"devDependencies":{"vitest":"^1.0.0"}}"#;
        let items = parse_package_json(pkg);
        let got: Vec<_> = items
            .iter()
            .map(|i| {
                (
                    i.name.as_str(),
                    i.current_version.as_str(),
                    i.range,
                    i.indirect,
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                ("lodash", "4.17.15", false, false),
                ("minimist", "1.2.0", true, false),
                ("vitest", "1.0.0", true, true)
            ]
        );
        let m = &items[1];
        assert_eq!(span(pkg, m.line, m.name_col, m.name_end_col), "minimist");
        assert_eq!(
            span(pkg, m.version_line, m.version_col, m.version_end_col),
            "^1.2.0"
        );
    }

    #[test]
    fn pyproject_reads_one_line_pep621_arrays_extras_and_poetry_tables() {
        let toml = r#"[project]
name = "app"
dependencies = ["requests==2.19.0", "flask>=1.0"]

[project.optional-dependencies]
web = [
  "uvicorn[standard]>=0.20",
  "jinja2==3.1.2",
]

[tool.poetry.dependencies]
python = "^3.10"
django = "^3.2.0"
urllib3 = "1.24.1"
celery = { version = "5.2.7", optional = true }
"#;
        let items = parse_python_deps("pyproject.toml", toml);
        let got: Vec<_> = items
            .iter()
            .map(|i| (i.name.as_str(), i.current_version.as_str(), i.range))
            .collect();
        assert_eq!(
            got,
            [
                ("requests", "2.19.0", false),
                ("flask", "1.0", true),
                ("uvicorn", "0.20", true),
                ("jinja2", "3.1.2", false),
                ("django", "3.2.0", true),
                ("urllib3", "1.24.1", false),
                ("celery", "5.2.7", false)
            ]
        );
        for i in &items {
            assert_eq!(
                span(toml, i.line, i.name_col, i.name_end_col),
                i.name,
                "{}",
                i.name
            );
            assert_eq!(
                span(toml, i.version_line, i.version_col, i.version_end_col),
                i.current_version,
                "{}",
                i.name
            );
        }
    }

    #[test]
    fn cargo_parser_reads_target_dotted_tables_and_lock_versions() {
        let toml = r#"[package]
name = "app"

[dependencies]
serde = "1"

[target.'cfg(unix)'.dependencies]
libc = { version = "0.2", default-features = false }

[target.x86_64-pc-windows-msvc.dev-dependencies]
winapi = "0.3"

[dependencies.smallvec]
features = ["union"]
version = "1.6"

[package.metadata.docs]
ignored = "1.0"
"#;
        let mut items = parse_cargo_toml(toml);
        let names: Vec<_> = items
            .iter()
            .map(|i| (i.name.as_str(), i.indirect))
            .collect();
        assert_eq!(
            names,
            [
                ("serde", false),
                ("libc", false),
                ("winapi", true),
                ("smallvec", false)
            ]
        );
        let sv = &items[3];
        assert_eq!(
            (sv.line, sv.version_line, sv.current_version.as_str()),
            (13, 15, "1.6")
        );
        assert_eq!(
            &toml.lines().nth(12).unwrap()
                [(sv.name_col - 1) as usize..(sv.name_end_col - 1) as usize],
            "smallvec"
        );

        let lock = "[[package]]\nname = \"smallvec\"\nversion = \"0.6.14\"\n\n[[package]]\nname = \"smallvec\"\nversion = \"1.6.0\"\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.210\"\nsource = \"registry\"\n";
        apply_cargo_lock_versions(&mut items, lock);
        assert_eq!(items[0].current_version, "1.0.210");
        assert_eq!(items[3].current_version, "1.6.0");
        assert_eq!(
            items[1].current_version, "0.2",
            "not in the lock: keep the requirement"
        );
    }
}
