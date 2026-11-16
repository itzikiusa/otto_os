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

pub fn parse_cargo_toml(content: &str) -> Vec<IdeDepItem> {
    static RE_SIMPLE: OnceLock<Regex> = OnceLock::new();
    static RE_TABLE: OnceLock<Regex> = OnceLock::new();
    let simple_re =
        RE_SIMPLE.get_or_init(|| Regex::new(r#"^\s*([A-Za-z0-9_-]+)\s*=\s*"([^"]+)""#).unwrap());
    let table_re = RE_TABLE.get_or_init(|| {
        Regex::new(r#"^\s*([A-Za-z0-9_-]+)\s*=\s*\{[^}]*?\bversion\s*=\s*"([^"]+)""#).unwrap()
    });
    let mut section = "";
    let mut items = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            section = match t {
                "[dependencies]" | "[workspace.dependencies]" => "deps",
                "[dev-dependencies]" | "[build-dependencies]" => "dev",
                _ => "",
            };
            continue;
        }
        if section.is_empty() || t.starts_with('#') {
            continue;
        }
        let cap_opt = simple_re.captures(line).or_else(|| table_re.captures(line));
        if let Some(cap) = cap_opt {
            let n = cap.get(1).unwrap();
            let v = cap.get(2).unwrap();
            let line_no = (idx + 1) as u32;
            items.push(IdeDepItem {
                name: n.as_str().to_string(),
                current_version: v.as_str().trim_start_matches(['^', '~', '=']).to_string(),
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
    items
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
