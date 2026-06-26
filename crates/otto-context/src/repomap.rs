//! Aider-style repo map: a ranked, concise list of a repository's most
//! important symbols, built with tree-sitter + a PageRank over the file
//! reference graph. Injected (opt-in) into the agent's context so it gets an
//! architectural overview without reading the whole tree.
//!
//! Design notes (deliberately defensive — this runs on the synchronous spawn
//! path):
//! - **panic-free**: every tree-sitter call is fallible-handled; a grammar/parse
//!   error skips the file, never unwinds.
//! - **bounded**: caps on file count, per-file size and a wall-clock budget, so a
//!   huge repo can't stall a session spawn.
//! - **grammar-agnostic extraction**: a single tree walk collects definitions
//!   (named nodes whose kind is a known def kind) and references (identifier
//!   nodes), so no per-language query has to compile.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tree_sitter::{Language, Node, Parser};

/// Tunables for [`build_repo_map`].
#[derive(Debug, Clone)]
pub struct RepoMapOptions {
    /// Max files to parse (cheapest cap; a huge repo stops here).
    pub max_files: usize,
    /// Skip files larger than this (generated/minified blobs).
    pub max_file_bytes: u64,
    /// Cap on emitted markdown lines (keeps the injected block small).
    pub max_lines: usize,
    /// Wall-clock budget; the walk stops early and returns a partial map.
    pub budget: Duration,
}

impl Default for RepoMapOptions {
    fn default() -> Self {
        Self {
            max_files: 400,
            max_file_bytes: 256 * 1024,
            max_lines: 100,
            budget: Duration::from_millis(400),
        }
    }
}

/// A definition discovered in a file.
struct Def {
    name: String,
    line: usize,      // 0-based
    signature: String,
}

/// One file's extracted symbols.
struct FileSyms {
    rel: String,
    defs: Vec<Def>,
    refs: HashMap<String, usize>, // identifier -> occurrence count
}

/// Named-node kinds we treat as a definition across the supported grammars. A
/// kind that doesn't exist in a given grammar simply never matches.
const DEF_KINDS: &[&str] = &[
    // Rust
    "function_item",
    "struct_item",
    "enum_item",
    "trait_item",
    "impl_item",
    "mod_item",
    "type_item",
    "macro_definition",
    // TS / JS
    "function_declaration",
    "generator_function_declaration",
    "class_declaration",
    "abstract_class_declaration",
    "method_definition",
    "interface_declaration",
    "type_alias_declaration",
    "enum_declaration",
    // Python
    "function_definition",
    "class_definition",
    // Go
    "method_declaration",
    "type_spec",
];

/// Node kinds that name a referenced symbol.
const IDENT_KINDS: &[&str] = &[
    "identifier",
    "type_identifier",
    "field_identifier",
    "property_identifier",
];

/// Resolve a tree-sitter `Language` for a file extension, or `None` when the
/// extension isn't a supported source language.
fn language_for(ext: &str) -> Option<Language> {
    let lang = match ext {
        "rs" => tree_sitter_rust::LANGUAGE.into(),
        "ts" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "js" | "jsx" | "mjs" | "cjs" => tree_sitter_javascript::LANGUAGE.into(),
        "py" => tree_sitter_python::LANGUAGE.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        _ => return None,
    };
    Some(lang)
}

/// Pull the defined name out of a def node: prefer the `name` field, fall back
/// to the `type` field (Rust `impl`) or the first identifier-ish named child.
fn def_name<'a>(node: Node<'a>, src: &'a [u8]) -> Option<(String, Node<'a>)> {
    let name_node = node
        .child_by_field_name("name")
        .or_else(|| node.child_by_field_name("type"))
        .or_else(|| {
            (0..node.named_child_count())
                .filter_map(|i| node.named_child(i))
                .find(|c| IDENT_KINDS.contains(&c.kind()))
        })?;
    let text = name_node.utf8_text(src).ok()?.to_string();
    if text.trim().is_empty() {
        return None;
    }
    Some((text, name_node))
}

/// Walk a parsed tree collecting definitions + references. Node-count bounded so
/// a pathological tree can't run away.
fn collect(root: Node, src: &[u8]) -> (Vec<Def>, HashMap<String, usize>) {
    let mut defs: Vec<Def> = Vec::new();
    let mut refs: HashMap<String, usize> = HashMap::new();
    let mut stack = vec![root];
    let mut visited = 0usize;
    let lines: Vec<&str> = std::str::from_utf8(src)
        .unwrap_or("")
        .lines()
        .collect();

    while let Some(node) = stack.pop() {
        visited += 1;
        if visited > 200_000 {
            break;
        }
        let kind = node.kind();
        if DEF_KINDS.contains(&kind) {
            if let Some((name, name_node)) = def_name(node, src) {
                let line = name_node.start_position().row;
                let sig = lines
                    .get(line)
                    .map(|l| {
                        let t = l.trim();
                        if t.len() > 120 {
                            format!("{}…", &t[..120])
                        } else {
                            t.to_string()
                        }
                    })
                    .unwrap_or_default();
                defs.push(Def { name, line, signature: sig });
            }
        } else if IDENT_KINDS.contains(&kind) {
            if let Ok(t) = node.utf8_text(src) {
                if !t.is_empty() {
                    *refs.entry(t.to_string()).or_insert(0) += 1;
                }
            }
        }
        for i in 0..node.named_child_count() {
            if let Some(c) = node.named_child(i) {
                stack.push(c);
            }
        }
    }
    (defs, refs)
}

/// Parse `source` for `ext`, returning its definitions + references. Returns
/// `None` (no panic) on any unsupported language or parse failure.
fn extract(ext: &str, source: &str) -> Option<(Vec<Def>, HashMap<String, usize>)> {
    let lang = language_for(ext)?;
    let mut parser = Parser::new();
    if parser.set_language(&lang).is_err() {
        return None;
    }
    let tree = parser.parse(source, None)?;
    Some(collect(tree.root_node(), source.as_bytes()))
}

/// PageRank over the file reference graph. `edges[i]` maps target file -> weight.
/// Returns a rank per file (higher = more depended-upon).
fn pagerank(n: usize, edges: &[HashMap<usize, f64>]) -> Vec<f64> {
    if n == 0 {
        return Vec::new();
    }
    const DAMPING: f64 = 0.85;
    const ITERS: usize = 30;
    let out_weight: Vec<f64> = edges.iter().map(|e| e.values().sum::<f64>()).collect();
    let mut rank = vec![1.0 / n as f64; n];
    for _ in 0..ITERS {
        let mut next = vec![(1.0 - DAMPING) / n as f64; n];
        for (i, e) in edges.iter().enumerate() {
            if out_weight[i] <= 0.0 {
                continue;
            }
            let share = DAMPING * rank[i] / out_weight[i];
            for (&j, &w) in e {
                next[j] += share * w;
            }
        }
        rank = next;
    }
    rank
}

/// Render the ranked map to markdown, capped at `max_lines`.
fn render(files: Vec<FileSyms>, ranks: Vec<f64>, ref_totals: &HashMap<String, usize>, max_lines: usize) -> String {
    let mut order: Vec<usize> = (0..files.len()).filter(|&i| !files[i].defs.is_empty()).collect();
    order.sort_by(|&a, &b| {
        ranks[b]
            .partial_cmp(&ranks[a])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| files[a].rel.cmp(&files[b].rel))
    });

    let mut out = String::new();
    let mut lines = 0usize;
    for fi in order {
        if lines >= max_lines {
            break;
        }
        let f = &files[fi];
        // Top defs within the file by how widely the symbol is referenced.
        let mut defs: Vec<&Def> = f.defs.iter().collect();
        defs.sort_by(|a, b| {
            ref_totals
                .get(&b.name)
                .unwrap_or(&0)
                .cmp(ref_totals.get(&a.name).unwrap_or(&0))
                .then_with(|| a.line.cmp(&b.line))
        });
        defs.dedup_by(|a, b| a.name == b.name && a.line == b.line);
        out.push_str(&format!("{}:\n", f.rel));
        lines += 1;
        for d in defs.into_iter().take(8) {
            if lines >= max_lines {
                break;
            }
            out.push_str(&format!("  {}: {}\n", d.line + 1, d.signature));
            lines += 1;
        }
    }
    out.trim_end().to_string()
}

/// Build the repo map for `root`. Returns `None` when there's nothing worth
/// mapping (no parseable definitions) or on any failure.
pub fn build_repo_map(root: &Path, opts: &RepoMapOptions) -> Option<String> {
    let start = Instant::now();
    let mut files: Vec<FileSyms> = Vec::new();
    let mut def_files: HashMap<String, HashSet<usize>> = HashMap::new();

    let walker = ignore::WalkBuilder::new(root)
        .standard_filters(true) // .gitignore, hidden, .git
        .max_filesize(Some(opts.max_file_bytes))
        .build();

    for entry in walker {
        if start.elapsed() > opts.budget || files.len() >= opts.max_files {
            break;
        }
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if language_for(ext).is_none() {
            continue;
        }
        let meta = entry.metadata().ok();
        if meta.map(|m| m.len() > opts.max_file_bytes).unwrap_or(false) {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let Some((defs, refs)) = extract(ext, &source) else {
            continue;
        };
        if defs.is_empty() && refs.is_empty() {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        let idx = files.len();
        for d in &defs {
            def_files.entry(d.name.clone()).or_default().insert(idx);
        }
        files.push(FileSyms { rel, defs, refs });
    }

    if files.iter().all(|f| f.defs.is_empty()) {
        return None;
    }

    // Build the file reference graph + global reference totals (excluding a
    // symbol's own defining files, so self-references don't inflate rank).
    let mut edges: Vec<HashMap<usize, f64>> = vec![HashMap::new(); files.len()];
    let mut ref_totals: HashMap<String, usize> = HashMap::new();
    for (i, f) in files.iter().enumerate() {
        for (sym, &cnt) in &f.refs {
            if let Some(defs) = def_files.get(sym) {
                let mut external = false;
                for &j in defs {
                    if j != i {
                        *edges[i].entry(j).or_insert(0.0) += cnt as f64;
                        external = true;
                    }
                }
                if external {
                    *ref_totals.entry(sym.clone()).or_insert(0) += cnt;
                }
            }
        }
    }

    let ranks = pagerank(files.len(), &edges);
    let map = render(files, ranks, &ref_totals, opts.max_lines);
    if map.trim().is_empty() {
        None
    } else {
        Some(map)
    }
}

// ---------------------------------------------------------------------------
// Cache: building the map walks + parses the repo, so memoize per root keyed by
// the repo's git HEAD (falls back to no-cache when HEAD can't be read, which is
// safe because the build is bounded).
// ---------------------------------------------------------------------------

static CACHE: Mutex<Option<HashMap<String, (String, String)>>> = Mutex::new(None);

/// A cheap signature of the repo's current state — the git HEAD commit. `None`
/// (→ recompute, no caching) when it can't be determined.
fn repo_signature(root: &Path) -> Option<String> {
    let head = std::fs::read_to_string(root.join(".git/HEAD")).ok()?;
    let head = head.trim();
    if let Some(refname) = head.strip_prefix("ref: ") {
        if let Ok(oid) = std::fs::read_to_string(root.join(".git").join(refname)) {
            return Some(oid.trim().to_string());
        }
        // Packed ref — fall back to the raw HEAD pointer string.
        return Some(refname.to_string());
    }
    Some(head.to_string()) // detached HEAD = the oid itself
}

/// Cached [`build_repo_map`]: returns the memoized map when the repo HEAD is
/// unchanged, else rebuilds and stores it.
pub fn repo_map_cached(root: &Path, opts: &RepoMapOptions) -> Option<String> {
    let key = root.to_string_lossy().to_string();
    let sig = repo_signature(root);
    if let Some(sig) = &sig {
        if let Ok(guard) = CACHE.lock() {
            if let Some(map) = guard.as_ref() {
                if let Some((cached_sig, cached_map)) = map.get(&key) {
                    if cached_sig == sig {
                        return Some(cached_map.clone());
                    }
                }
            }
        }
    }
    let map = build_repo_map(root, opts)?;
    if let Some(sig) = sig {
        if let Ok(mut guard) = CACHE.lock() {
            guard.get_or_insert_with(HashMap::new).insert(key, (sig, map.clone()));
        }
    }
    Some(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, content).unwrap();
    }

    #[test]
    fn ranks_the_widely_referenced_symbol_highest() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(root, "core.rs", "pub fn shared_helper() -> i32 { 42 }\n");
        write(root, "a.rs", "fn a() { let _ = shared_helper(); }\n");
        write(root, "b.rs", "fn b() { let _ = shared_helper(); }\n");

        let map = build_repo_map(root, &RepoMapOptions::default()).expect("a map");
        assert!(map.contains("shared_helper"), "map names the symbol:\n{map}");
        // core.rs (2 incoming refs) must out-rank the leaf files.
        let core = map.find("core.rs").expect("core listed");
        let a = map.find("a.rs").unwrap_or(usize::MAX);
        assert!(core < a, "core.rs should rank before a.rs:\n{map}");
    }

    #[test]
    fn empty_for_a_non_code_directory() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "notes.txt", "just prose, no code\n");
        write(tmp.path(), "data.json", "{\"a\":1}\n");
        assert!(build_repo_map(tmp.path(), &RepoMapOptions::default()).is_none());
    }

    #[test]
    fn output_respects_the_line_budget() {
        let tmp = tempfile::tempdir().unwrap();
        // Many symbols across many files.
        for i in 0..50 {
            let body = (0..20)
                .map(|j| format!("pub fn f_{i}_{j}() {{}}"))
                .collect::<Vec<_>>()
                .join("\n");
            write(tmp.path(), &format!("m{i}.rs"), &body);
        }
        let opts = RepoMapOptions {
            max_lines: 30,
            ..Default::default()
        };
        let map = build_repo_map(tmp.path(), &opts).expect("a map");
        assert!(
            map.lines().count() <= 30,
            "map exceeded the line budget: {} lines",
            map.lines().count()
        );
    }

    #[test]
    fn parses_multiple_languages_without_panicking() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "x.ts", "export function tsFn() {}\nexport class TsClass {}\n");
        write(tmp.path(), "y.py", "def py_fn():\n    pass\nclass PyClass:\n    pass\n");
        write(tmp.path(), "z.go", "package z\nfunc GoFn() {}\ntype GoType struct{}\n");
        let map = build_repo_map(tmp.path(), &RepoMapOptions::default()).expect("a map");
        assert!(map.contains("tsFn") || map.contains("TsClass"));
        assert!(map.contains("py_fn") || map.contains("PyClass"));
        assert!(map.contains("GoFn") || map.contains("GoType"));
    }
}
