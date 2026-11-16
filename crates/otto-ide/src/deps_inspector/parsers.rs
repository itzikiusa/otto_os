//! Manifest line-span parsers for `package.json`, `Cargo.toml`, and Python
//! `requirements.txt` / `pyproject.toml`.

use std::sync::OnceLock;

use regex::Regex;

use crate::types::IdeDepItem;

pub fn parse_package_json(content: &str) -> Vec<IdeDepItem> {
    static RE_KV: OnceLock<Regex> = OnceLock::new();
    let kv_re = RE_KV.get_or_init(|| Regex::new(r#"^\s*"([^"]+)"\s*:\s*"([^"]+)""#).unwrap());
    let mut section = "";
    let mut items = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let t = line.trim();
        if t.starts_with("\"dependencies\"") {
            section = if t.contains('}') { "" } else { "deps" };
            continue;
        } else if t.starts_with("\"devDependencies\"") {
            section = if t.contains('}') { "" } else { "dev" };
            continue;
        } else if !section.is_empty() && t.starts_with('}') {
            section = "";
            continue;
        }
        if !section.is_empty() {
            if let Some(cap) = kv_re.captures(line) {
                let n = cap.get(1).unwrap();
                let v = cap.get(2).unwrap();
                let raw_v = v.as_str();
                if raw_v.starts_with("workspace:") || raw_v.starts_with("file:") {
                    continue;
                }
                let clean_v = raw_v
                    .trim_start_matches(['^', '~', '>', '=', '<', ' '])
                    .to_string();
                let line_no = (idx + 1) as u32;
                items.push(IdeDepItem {
                    name: n.as_str().to_string(),
                    current_version: clean_v,
                    latest_version: None,
                    latest_patch: None,
                    indirect: section == "dev",
                    deprecated: None,
                    line: line_no,
                    name_col: (line[..n.start()].chars().count() + 1) as u32,
                    name_end_col: (line[..n.end()].chars().count() + 1) as u32,
                    version_line: line_no,
                    version_col: (line[..v.start()].chars().count() + 1) as u32,
                    version_end_col: (line[..v.end()].chars().count() + 1) as u32,
                    block_start_line: line_no,
                    block_end_line: line_no,
                    property_name: None,
                    managed_in: None,
                    vulnerabilities: Vec::new(),
                });
            }
        }
    }
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

pub fn parse_python_deps(fname: &str, content: &str) -> Vec<IdeDepItem> {
    static RE_PY: OnceLock<Regex> = OnceLock::new();
    let req_re = RE_PY.get_or_init(|| {
        Regex::new(r#"^\s*"?([A-Za-z0-9_.-]+)\s*(?:==|>=|~=)\s*([0-9][A-Za-z0-9_.-]*)"#).unwrap()
    });
    let mut items = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') || (fname == "pyproject.toml" && t.starts_with('[')) {
            continue;
        }
        if let Some(cap) = req_re.captures(line) {
            let n = cap.get(1).unwrap();
            let v = cap.get(2).unwrap();
            let line_no = (idx + 1) as u32;
            items.push(IdeDepItem {
                name: n.as_str().to_string(),
                current_version: v.as_str().to_string(),
                latest_version: None,
                latest_patch: None,
                indirect: false,
                deprecated: None,
                line: line_no,
                name_col: (line[..n.start()].chars().count() + 1) as u32,
                name_end_col: (line[..n.end()].chars().count() + 1) as u32,
                version_line: line_no,
                version_col: (line[..v.start()].chars().count() + 1) as u32,
                version_end_col: (line[..v.end()].chars().count() + 1) as u32,
                block_start_line: line_no,
                block_end_line: line_no,
                property_name: None,
                managed_in: None,
                vulnerabilities: Vec::new(),
            });
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

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
