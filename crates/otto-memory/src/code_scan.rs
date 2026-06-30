//! Code scanner — the extraction half of Vault v2's code intelligence.
//!
//! Walks a repository (honoring `.gitignore`, bounded by file count / size /
//! wall-clock) and, per source file, uses **tree-sitter** to find symbol
//! definitions (with their line ranges) and **content heuristics** to find the
//! dependency signals that matter for understanding a flow:
//!
//! - **http_call** — service-locator lookups (`GetBrandService(ctx, id, "X")`),
//!   typed client constructors (`NewWalletGatewayClient`), URL/`.local` literals,
//!   and raw HTTP verbs — attributed to the enclosing function.
//! - **db_call**   — SQL string literals (`SELECT … FROM t`) and query-builder
//!   calls (`ExecuteSingleResultQuery`, `squirrel.Select`, `GetContext`, …),
//!   with the table name pulled out when present.
//! - **imports**   — a file's imports, with cross-repo imports (e.g. importing
//!   `go_casino_kit`) surfaced as `service` dependency nodes.
//! - **calls**     — intra-/cross-file function references resolved to defs.
//!
//! The result is a set of graph nodes (by natural key) + typed edges + a flat
//! symbol list + the file texts to embed. The service layer persists it via
//! [`otto_state::CodeIndexRepo`]. Panic-free and bounded: a parse error skips
//! the file, never unwinds.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use tree_sitter::{Language, Node, Parser};

/// Tunables for a scan.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub max_files: usize,
    pub max_file_bytes: u64,
    pub budget: Duration,
    /// Whether to collect file texts for embedding (can be large).
    pub collect_texts: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            max_files: 4000,
            max_file_bytes: 512 * 1024,
            budget: Duration::from_secs(20),
            collect_texts: true,
        }
    }
}

/// A symbol definition found in the repo (1-based lines).
#[derive(Clone, Debug)]
pub struct ScannedSymbol {
    pub name: String,
    pub kind: String,
    pub lang: String,
    pub file: String,
    pub line: usize,
    pub end_line: usize,
    pub signature: String,
}

/// A graph node addressed by its natural key (kind, key).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NodeKey {
    pub kind: String,
    pub key: String,
}

/// A graph node spec (pre-persistence).
#[derive(Clone, Debug)]
pub struct ScannedNode {
    pub key: NodeKey,
    pub label: String,
    pub file: Option<String>,
    pub line: Option<usize>,
    pub meta_json: String,
}

/// A typed edge between two node keys.
#[derive(Clone, Debug)]
pub struct ScannedEdge {
    pub src: NodeKey,
    pub dst: NodeKey,
    pub rel: String,
    pub detail: String,
    pub file: Option<String>,
    pub line: Option<usize>,
}

/// The full scan output.
#[derive(Clone, Debug, Default)]
pub struct ScanResult {
    pub files: usize,
    pub symbols: Vec<ScannedSymbol>,
    pub nodes: Vec<ScannedNode>,
    pub edges: Vec<ScannedEdge>,
    /// (repo-relative path, content) for files to embed.
    pub texts: Vec<(String, String)>,
}

// --- tree-sitter grammar plumbing (mirrors otto-context::repomap) -----------

const DEF_KINDS: &[&str] = &[
    "function_item", "struct_item", "enum_item", "trait_item", "impl_item", "mod_item",
    "type_item", "macro_definition", "function_declaration", "generator_function_declaration",
    "class_declaration", "abstract_class_declaration", "method_definition", "interface_declaration",
    "type_alias_declaration", "enum_declaration", "function_definition", "class_definition",
    "method_declaration", "type_spec",
];

const IDENT_KINDS: &[&str] = &["identifier", "type_identifier", "field_identifier", "property_identifier"];

fn language_for(ext: &str) -> Option<(Language, &'static str)> {
    let lang = match ext {
        "rs" => (tree_sitter_rust::LANGUAGE.into(), "rs"),
        "ts" => (tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), "ts"),
        "tsx" => (tree_sitter_typescript::LANGUAGE_TSX.into(), "ts"),
        "js" | "jsx" | "mjs" | "cjs" => (tree_sitter_javascript::LANGUAGE.into(), "js"),
        "py" => (tree_sitter_python::LANGUAGE.into(), "py"),
        "go" => (tree_sitter_go::LANGUAGE.into(), "go"),
        _ => return None,
    };
    Some(lang)
}

fn def_name<'a>(node: Node<'a>, src: &'a [u8]) -> Option<String> {
    let name_node = node
        .child_by_field_name("name")
        .or_else(|| node.child_by_field_name("type"))
        .or_else(|| {
            (0..node.named_child_count())
                .filter_map(|i| node.named_child(i))
                .find(|c| IDENT_KINDS.contains(&c.kind()))
        })?;
    let text = name_node.utf8_text(src).ok()?.trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Map a tree-sitter def-node kind to a stable, human symbol kind.
fn symbol_kind(ts_kind: &str) -> &'static str {
    match ts_kind {
        "function_item" | "function_declaration" | "generator_function_declaration"
        | "function_definition" => "function",
        "method_definition" | "method_declaration" => "method",
        "struct_item" => "struct",
        "class_declaration" | "abstract_class_declaration" | "class_definition" => "class",
        "interface_declaration" => "interface",
        "trait_item" => "trait",
        "enum_item" | "enum_declaration" => "enum",
        "impl_item" => "impl",
        "mod_item" => "module",
        "type_item" | "type_alias_declaration" | "type_spec" => "type",
        "macro_definition" => "macro",
        _ => "symbol",
    }
}

/// Collect symbol defs (with line ranges) + referenced identifiers from a tree.
fn collect(root: Node, src: &[u8], lang: &str, lines: &[&str]) -> (Vec<ScannedSymbol>, HashSet<String>) {
    let mut defs: Vec<ScannedSymbol> = Vec::new();
    let mut refs: HashSet<String> = HashSet::new();
    let mut stack = vec![root];
    let mut visited = 0usize;
    while let Some(node) = stack.pop() {
        visited += 1;
        if visited > 300_000 {
            break;
        }
        let kind = node.kind();
        if DEF_KINDS.contains(&kind) {
            if let Some(name) = def_name(node, src) {
                let line = node.start_position().row;
                let end = node.end_position().row;
                let sig = lines
                    .get(line)
                    .map(|l| {
                        let t = l.trim();
                        if t.len() > 160 {
                            format!("{}…", &t[..160])
                        } else {
                            t.to_string()
                        }
                    })
                    .unwrap_or_default();
                defs.push(ScannedSymbol {
                    name,
                    kind: symbol_kind(kind).to_string(),
                    lang: lang.to_string(),
                    file: String::new(), // filled by caller
                    line: line + 1,
                    end_line: end + 1,
                    signature: sig,
                });
            }
        } else if IDENT_KINDS.contains(&kind) {
            if let Ok(t) = node.utf8_text(src) {
                if t.len() >= 3 {
                    refs.insert(t.to_string());
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

// --- content heuristics -----------------------------------------------------

/// Extract string literals (both "..." and `...`) from a line.
fn quoted_strings(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    for delim in ['"', '`', '\''] {
        let mut rest = line;
        while let Some(start) = rest.find(delim) {
            let after = &rest[start + 1..];
            if let Some(end) = after.find(delim) {
                let s = &after[..end];
                if !s.is_empty() {
                    out.push(s.to_string());
                }
                rest = &after[end + 1..];
            } else {
                break;
            }
        }
    }
    out
}

/// An UPPER_SNAKE service token (e.g. "WALLET_GATEWAY", "LIMITS").
fn is_service_token(s: &str) -> bool {
    s.len() >= 3
        && s.len() <= 48
        && s.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false)
        && s.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && s.contains(|c: char| c.is_ascii_uppercase())
}

/// Pull a table name out of a SQL fragment after FROM/INTO/UPDATE/JOIN, keeping
/// only the last dotted segment (`pr_bo.players` → `players`). Returns the raw
/// token; validity is decided by [`valid_table`].
fn sql_table(line_lower: &str, original: &str) -> Option<String> {
    for kw in ["from ", "into ", "update ", "join "] {
        if let Some(pos) = line_lower.find(kw) {
            let tail = &original[pos + kw.len()..];
            let name: String = tail
                .trim_start()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
                .collect();
            let last = name.rsplit('.').next().unwrap_or(&name).to_string();
            if last.len() >= 2 {
                return Some(last);
            }
        }
    }
    None
}

/// High-precision table filter: a real table name is an identifier that contains
/// an underscore (casino tables are prefixed: `MdlGm_tblPlayers`, `fin_accounts`,
/// `tbl_auditLog`, `players_buffer`, …). This drops the SQL-keyword / prose
/// false positives (`of`, `the`, `DESC`, `DUAL`) at the cost of single-word
/// tables — a worthwhile trade for a clean graph.
fn valid_table(name: &str) -> bool {
    let n = name.len();
    (3..=64).contains(&n)
        && name.contains('_')
        && name.chars().next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false)
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A plausible HTTP service node: an UPPER_SNAKE service key, a hostname
/// (dotted, no regex escapes), or a typed client name. Drops `http`, format
/// strings, and regex literals.
fn valid_service(s: &str) -> bool {
    if s.is_empty() || s.contains('\\') || s.contains('%') || s.eq_ignore_ascii_case("http") {
        return false;
    }
    if is_service_token(s) {
        return true;
    }
    // hostname: at least one dot, sane chars, a non-numeric label.
    let host_ok = s.contains('.')
        && s.split('.').count() >= 2
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
        && s.chars().any(|c| c.is_ascii_alphabetic());
    host_ok
}

/// Identifiers too common/generic to make a meaningful cross-file `calls` edge.
fn is_noise_call(name: &str) -> bool {
    if name.len() < 4 {
        return true;
    }
    const COMMON: &[&str] = &[
        "args", "Run", "err", "ctx", "New", "Get", "Set", "nil", "len", "make", "append",
        "String", "Error", "Errorf", "Sprintf", "Printf", "Println", "Test", "Main", "init",
        "this", "self", "Background", "Context", "context", "Wrap", "Wrapf", "Now", "Add",
        "Equal", "NoError", "Nil", "True", "False", "Len", "Contains", "Marshal", "Unmarshal",
        "Close", "Lock", "Unlock", "Done", "Some", "None", "Ok", "Err", "Value", "Field",
        "InitServices", "Result", "Response", "Request", "Handle", "Parse", "Format",
    ];
    COMMON.contains(&name)
}

/// What a single line signals about external dependencies.
enum LineSignal {
    Http { service: String, detail: String },
    Db { table: String, detail: String },
}

fn detect_signal(line: &str) -> Option<LineSignal> {
    let lower = line.to_lowercase();
    let strings = quoted_strings(line);

    // --- DB first (SQL is unambiguous) ---
    let has_sql = ["select ", "insert into", "update ", "delete from", " from ", "create table"]
        .iter()
        .any(|k| lower.contains(k))
        && (line.contains('"') || line.contains('`'));
    let has_query_builder = [
        "executesingleresultquery",
        "executequery",
        "squirrel.",
        ".getcontext(",
        ".querycontext(",
        ".execcontext(",
        ".queryrow(",
        "db.query",
        "conn.query",
    ]
    .iter()
    .any(|k| lower.contains(k));
    if has_sql || has_query_builder {
        // Only emit a db edge when we can name a real (underscored) table — keeps
        // the graph precise rather than littered with SQL-keyword false positives.
        let table = strings
            .iter()
            .find_map(|s| sql_table(&s.to_lowercase(), s))
            .or_else(|| sql_table(&lower, line))
            .filter(|t| valid_table(t));
        if let Some(table) = table {
            return Some(LineSignal::Db {
                table,
                detail: if has_sql { first_sql_clause(line) } else { "query".to_string() },
            });
        }
        return None;
    }

    // --- HTTP ---
    // 1) Service-locator lookups → UPPER_SNAKE service token.
    let locator = ["getbrandservice", "getservice", ".locate(", "discover", "servicelocator"]
        .iter()
        .any(|k| lower.contains(k));
    if locator {
        if let Some(svc) = strings.iter().find(|s| is_service_token(s)) {
            return Some(LineSignal::Http {
                service: svc.clone(),
                detail: "service discovery".to_string(),
            });
        }
    }
    // 2) Typed client constructor: New<Name>Client(
    if let Some(svc) = client_constructor(line) {
        return Some(LineSignal::Http {
            service: svc,
            detail: "client".to_string(),
        });
    }
    // 3) URL / *.local literal — only when the host is a real (validated) host.
    if let Some(url) = strings
        .iter()
        .find(|s| s.starts_with("http://") || s.starts_with("https://") || s.contains(".local"))
    {
        let host = url_host(url);
        if valid_service(&host) {
            return Some(LineSignal::Http {
                service: host,
                detail: trim_detail(url),
            });
        }
    }
    // 4) Raw HTTP verbs on a client — only attribute when a real service token
    // is present on the line (otherwise it's an un-namable call, skip).
    let verb = [
        "http.get(", "http.post(", "http.newrequest(", ".getrequest(", ".postrequest(",
        "restclient.", ".do(req", "resty.",
    ]
    .iter()
    .any(|k| lower.contains(k));
    if verb {
        if let Some(svc) = strings.iter().find(|s| is_service_token(s)) {
            return Some(LineSignal::Http {
                service: svc.clone(),
                detail: "http".to_string(),
            });
        }
    }
    None
}

fn first_sql_clause(line: &str) -> String {
    let t = line.trim();
    let t = t.trim_start_matches(['"', '`', '\t', ' ']);
    let mut s: String = t.chars().take(60).collect();
    if t.len() > 60 {
        s.push('…');
    }
    s
}

fn trim_detail(s: &str) -> String {
    if s.len() > 60 {
        format!("{}…", &s[..60])
    } else {
        s.to_string()
    }
}

/// Detect `New<Name>Client(` → "Name" (CamelCase service name).
fn client_constructor(line: &str) -> Option<String> {
    let idx = line.find("New")?;
    let after = &line[idx + 3..];
    let name: String = after.chars().take_while(|c| c.is_alphanumeric()).collect();
    if name.ends_with("Client") && name.len() > 6 {
        Some(name[..name.len() - 6].to_string())
    } else {
        None
    }
}

/// Host portion of a URL (or a `*.local` service host).
fn url_host(url: &str) -> String {
    let no_scheme = url.split("://").last().unwrap_or(url);
    let host = no_scheme.split(['/', ':']).next().unwrap_or(no_scheme);
    host.to_string()
}

/// Find which symbol's [line,end_line] range contains a 1-based line.
fn enclosing(syms: &[ScannedSymbol], line: usize) -> Option<&ScannedSymbol> {
    syms.iter()
        .filter(|s| line >= s.line && line <= s.end_line && s.kind != "impl" && s.kind != "module")
        .min_by_key(|s| s.end_line - s.line)
}

// --- the scan ---------------------------------------------------------------

/// Scan `root`, returning symbols + a dependency graph + texts to embed.
pub fn scan_repo(root: &Path, opts: &ScanOptions) -> ScanResult {
    let start = Instant::now();
    let mut result = ScanResult::default();
    // (file rel -> its symbols) so we can resolve calls + attribute signals.
    let mut per_file: Vec<(String, Vec<ScannedSymbol>)> = Vec::new();
    // global def name -> set of files defining it (for cross-file calls).
    let mut def_files: HashMap<String, HashSet<String>> = HashMap::new();
    let mut file_refs: Vec<(String, HashSet<String>)> = Vec::new();

    let mut nodes: HashMap<NodeKey, ScannedNode> = HashMap::new();
    let mut edges: Vec<ScannedEdge> = Vec::new();
    let mut edge_seen: HashSet<(NodeKey, NodeKey, String, String)> = HashSet::new();

    let add_node = |nodes: &mut HashMap<NodeKey, ScannedNode>, n: ScannedNode| {
        nodes.entry(n.key.clone()).or_insert(n);
    };
    let add_edge =
        |edges: &mut Vec<ScannedEdge>, seen: &mut HashSet<_>, e: ScannedEdge| {
            let k = (e.src.clone(), e.dst.clone(), e.rel.clone(), e.detail.clone());
            if seen.insert(k) {
                edges.push(e);
            }
        };

    let walker = ignore::WalkBuilder::new(root)
        .standard_filters(true)
        .max_filesize(Some(opts.max_file_bytes))
        .build();

    for entry in walker {
        if start.elapsed() > opts.budget || result.files >= opts.max_files {
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
        let Some((lang, lang_id)) = language_for(ext) else {
            continue;
        };
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy().to_string();

        // Parse for symbols + refs.
        let mut parser = Parser::new();
        if parser.set_language(&lang).is_err() {
            continue;
        }
        let Some(tree) = parser.parse(&source, None) else {
            continue;
        };
        let lines: Vec<&str> = source.lines().collect();
        let (mut syms, refs) = collect(tree.root_node(), source.as_bytes(), lang_id, &lines);
        for s in &mut syms {
            s.file = rel.clone();
        }
        result.files += 1;

        // File node + symbol nodes + defined_in edges.
        let file_key = NodeKey { kind: "file".into(), key: rel.clone() };
        add_node(
            &mut nodes,
            ScannedNode {
                key: file_key.clone(),
                label: rel.clone(),
                file: Some(rel.clone()),
                line: None,
                meta_json: format!("{{\"lang\":\"{lang_id}\"}}"),
            },
        );
        for s in &syms {
            def_files.entry(s.name.clone()).or_default().insert(rel.clone());
            let sk = NodeKey { kind: "symbol".into(), key: format!("{}#{}", rel, s.name) };
            add_node(
                &mut nodes,
                ScannedNode {
                    key: sk.clone(),
                    label: s.name.clone(),
                    file: Some(rel.clone()),
                    line: Some(s.line),
                    meta_json: format!("{{\"kind\":\"{}\",\"sig\":{}}}", s.kind, json_str(&s.signature)),
                },
            );
            add_edge(
                &mut edges,
                &mut edge_seen,
                ScannedEdge { src: sk, dst: file_key.clone(), rel: "defined_in".into(), detail: String::new(), file: Some(rel.clone()), line: Some(s.line) },
            );
        }

        // Per-line dependency signals → http_call / db_call / imports.
        for (i, line) in lines.iter().enumerate() {
            let ln = i + 1;
            // imports: cross-repo dependency detection.
            for imp in import_targets(line, lang_id) {
                let svc = cross_repo_service(&imp);
                if let Some(svc) = svc {
                    let dst = NodeKey { kind: "service".into(), key: svc.clone() };
                    add_node(&mut nodes, ScannedNode { key: dst.clone(), label: svc.clone(), file: None, line: None, meta_json: "{\"origin\":\"import\"}".into() });
                    add_edge(&mut edges, &mut edge_seen, ScannedEdge { src: file_key.clone(), dst, rel: "imports".into(), detail: imp.clone(), file: Some(rel.clone()), line: Some(ln) });
                }
            }
            if let Some(sig) = detect_signal(line) {
                let src_key = enclosing(&syms, ln)
                    .map(|s| NodeKey { kind: "symbol".into(), key: format!("{}#{}", rel, s.name) })
                    .unwrap_or_else(|| file_key.clone());
                match sig {
                    LineSignal::Http { service, detail } => {
                        let dst = NodeKey { kind: "service".into(), key: service.clone() };
                        add_node(&mut nodes, ScannedNode { key: dst.clone(), label: service, file: None, line: None, meta_json: "{\"origin\":\"http\"}".into() });
                        add_edge(&mut edges, &mut edge_seen, ScannedEdge { src: src_key, dst, rel: "http_call".into(), detail, file: Some(rel.clone()), line: Some(ln) });
                    }
                    LineSignal::Db { table, detail } => {
                        let dst = NodeKey { kind: "db_table".into(), key: table.clone() };
                        add_node(&mut nodes, ScannedNode { key: dst.clone(), label: table, file: None, line: None, meta_json: "{\"origin\":\"db\"}".into() });
                        add_edge(&mut edges, &mut edge_seen, ScannedEdge { src: src_key, dst, rel: "db_call".into(), detail, file: Some(rel.clone()), line: Some(ln) });
                    }
                }
            }
        }

        if opts.collect_texts {
            result.texts.push((rel.clone(), source));
        }
        file_refs.push((rel.clone(), refs));
        per_file.push((rel.clone(), syms));
    }

    // Cross-file calls: a file referencing a symbol defined in ANOTHER file →
    // file → symbol `calls` edge (bounded; symbol-level src would need range
    // attribution per ref which we skip for cost).
    for (rel, refs) in &file_refs {
        let src = NodeKey { kind: "file".into(), key: rel.clone() };
        let mut added = 0usize;
        for r in refs {
            if added >= 40 {
                break;
            }
            // Skip generic identifiers + symbols defined in many files (ambiguous):
            // these produce hub noise that drowns the real call structure.
            if is_noise_call(r) {
                continue;
            }
            if let Some(files) = def_files.get(r) {
                if files.len() > 4 {
                    continue;
                }
                for f in files {
                    if f != rel {
                        let dst = NodeKey { kind: "symbol".into(), key: format!("{}#{}", f, r) };
                        if nodes.contains_key(&dst) {
                            add_edge(&mut edges, &mut edge_seen, ScannedEdge { src: src.clone(), dst, rel: "calls".into(), detail: String::new(), file: Some(rel.clone()), line: None });
                            added += 1;
                        }
                    }
                }
            }
        }
    }

    // test_of edges: a *_test / test_* file → the source file it tests.
    let file_set: HashSet<String> = per_file.iter().map(|(r, _)| r.clone()).collect();
    for (rel, _) in &per_file {
        if let Some(target) = crate::test_map::source_for_test(rel) {
            if file_set.contains(&target) {
                add_edge(
                    &mut edges,
                    &mut edge_seen,
                    ScannedEdge {
                        src: NodeKey { kind: "file".into(), key: rel.clone() },
                        dst: NodeKey { kind: "file".into(), key: target.clone() },
                        rel: "test_of".into(),
                        detail: String::new(),
                        file: Some(rel.clone()),
                        line: None,
                    },
                );
            }
        }
    }

    for (_, syms) in per_file {
        result.symbols.extend(syms);
    }
    result.nodes = nodes.into_values().collect();
    result.edges = edges;
    result
}

/// Extract import target strings from a line for a language.
fn import_targets(line: &str, lang: &str) -> Vec<String> {
    let lower = line.trim_start();
    match lang {
        "go" => {
            // `import "x"` or a line inside an import (...) block: a quoted path.
            if lower.starts_with("import ") || lower.starts_with('"') || lower.contains('"') {
                quoted_strings(line)
                    .into_iter()
                    .filter(|s| s.contains('/') || s.contains('.'))
                    .collect()
            } else {
                vec![]
            }
        }
        "ts" | "js" => {
            if lower.starts_with("import ") || lower.contains("require(") {
                quoted_strings(line)
            } else {
                vec![]
            }
        }
        "py" => {
            if lower.starts_with("import ") || lower.starts_with("from ") {
                line.split_whitespace().nth(1).map(|s| vec![s.to_string()]).unwrap_or_default()
            } else {
                vec![]
            }
        }
        "rs" => {
            if lower.starts_with("use ") {
                line.trim_start()[4..]
                    .split(['{', ':', ';', ' '])
                    .next()
                    .filter(|s| !s.is_empty())
                    .map(|s| vec![s.to_string()])
                    .unwrap_or_default()
            } else {
                vec![]
            }
        }
        _ => vec![],
    }
}

/// If an import path names a sibling repo / known service kit, return a service
/// node name. Heuristic: a path segment that looks like a repo (`go_*`, `*_kit`,
/// `*_service`, `*_gateway`).
fn cross_repo_service(path: &str) -> Option<String> {
    for seg in path.split('/') {
        let s = seg.to_lowercase();
        if (s.starts_with("go_") || s.ends_with("_kit") || s.ends_with("_service") || s.ends_with("_gateway"))
            && s.len() >= 4
            && !s.contains('.')
        {
            return Some(seg.to_string());
        }
    }
    None
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
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
    fn login_flow_graph_is_extracted() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(
            root,
            "app/login.go",
            r#"package app
import (
    "bitbucket.org/gamescale-rnd/go_casino_kit/clients"
)
func Login(ctx context.Context, brandId int) error {
    limits := GetLimits(ctx, brandId)
    _ = limits
    return nil
}
"#,
        );
        write(
            root,
            "app/limits.go",
            r#"package app
func GetLimits(ctx context.Context, brandId int) (int, error) {
    url, _ := serviceLocator.GetBrandService(ctx, brandId, "LIMITS")
    resp, _ := restClient.GetRequest(ctx, url)
    _ = resp
    row, _ := conn.GetContext(ctx, "SELECT max_limit FROM MdlGm_tblLimits WHERE id = ?")
    _ = row
    return 0, nil
}
"#,
        );
        let r = scan_repo(root, &ScanOptions::default());

        // Symbols
        assert!(r.symbols.iter().any(|s| s.name == "Login"));
        assert!(r.symbols.iter().any(|s| s.name == "GetLimits"));

        // http_call from GetLimits → LIMITS service
        let http = r.edges.iter().find(|e| e.rel == "http_call").expect("http_call edge");
        assert_eq!(http.dst.key, "LIMITS");
        assert!(http.src.key.ends_with("#GetLimits"), "attributed to GetLimits: {}", http.src.key);

        // db_call from GetLimits → MdlGm_tblLimits
        let db = r.edges.iter().find(|e| e.rel == "db_call").expect("db_call edge");
        assert_eq!(db.dst.key, "MdlGm_tblLimits");

        // import of go_casino_kit → service node + imports edge
        assert!(r.nodes.iter().any(|n| n.key.kind == "service" && n.key.key == "go_casino_kit"));
        assert!(r.edges.iter().any(|e| e.rel == "imports" && e.dst.key == "go_casino_kit"));

        // calls edge: login.go references GetLimits defined in limits.go
        assert!(r
            .edges
            .iter()
            .any(|e| e.rel == "calls" && e.dst.key.ends_with("#GetLimits")));
    }
}
