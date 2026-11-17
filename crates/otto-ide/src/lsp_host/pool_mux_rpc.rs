//! JSON-RPC initialize enrichment, configuration replies, Go build-tag
//! diagnostic rewriting, and fast-path client/server routing for [`super::Mux`].

use std::borrow::Cow;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::Ordering;

use serde::de::IgnoredAny;
use serde::Deserialize;
use serde_json::{json, Value};

use super::super::servers;
use super::{Init, Mux};
use crate::go_build_tags::is_gopls_build_tag_exclusion_msg;

#[derive(Deserialize)]
struct RpcEnvelopeProbe<'a> {
    #[serde(borrow)]
    method: Option<Cow<'a, str>>,
    #[serde(default)]
    id: Option<IgnoredAny>,
}

fn str_at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for p in path {
        cur = cur.get(*p)?;
    }
    cur.as_str()
}

pub(crate) fn default_gopls_settings(go_build_tags: &[String]) -> Value {
    let mut build_flags = vec![Value::String("-mod=mod".into())];
    if !go_build_tags.is_empty() {
        build_flags.push(Value::String(format!("-tags={}", go_build_tags.join(","))));
    }
    json!({
        "buildFlags": build_flags,
        "directoryFilters": [
            "-**/node_modules",
            "-**/target",
            "-**/.git",
            "-**/dist",
            "-**/build",
            "-**/.svelte-kit",
            "-**/__pycache__"
        ],
        "completeUnimported": true,
        // gopls answers `textDocument/semanticTokens/full` only with this on (off by default).
        "semanticTokens": true
    })
}

/// rust-analyzer for a non-root owner runs no workspace code: no build scripts,
/// no proc macros, and no `cargo check` on save (which would run `build.rs`
/// and any `.cargo/config.toml` `rustc-wrapper`).
/// jdtls settings Otto always applies: keep Eclipse `.project` / `.classpath`
/// / `.settings` out of the user's repo (they go to the `-data` dir instead).
fn jdtls_settings() -> Value {
    json!({ "java": { "import": { "generatesMetadataFilesAtProjectRoot": false } } })
}

/// Merge `extra` into `base` object-by-object (`extra` wins on leaves).
fn merge_json(base: &mut Value, extra: Value) {
    match (base, extra) {
        (Value::Object(b), Value::Object(e)) => {
            for (k, v) in e {
                merge_json(b.entry(k).or_insert(Value::Null), v);
            }
        }
        (b, e) => *b = e,
    }
}

fn non_root_rust_settings() -> Value {
    json!({
        "cargo": { "buildScripts": { "enable": false } },
        "procMacro": { "enable": false },
        "checkOnSave": false
    })
}

/// The repo's venv interpreter for pyright (which EXECUTES it to read
/// `sys.path`). A real venv (it has `pyvenv.cfg`) links `bin/python` to an
/// installed interpreter; one that resolves inside the repo is a committed
/// file, and a committed symlink to anywhere else (another repo, Downloads,
/// `/tmp`) is not an installation either — so the target must lie under a
/// known interpreter home. Anything else is never run.
#[allow(clippy::disallowed_methods)]
fn detect_local_python_path(root: &str) -> Option<String> {
    let r = Path::new(root);
    let canon_root = std::fs::canonicalize(r).ok()?;
    let bases = trusted_interpreter_bases();
    for venv in [".venv", "venv", ".env"] {
        let p = r.join(venv).join("bin/python");
        if !r.join(venv).join("pyvenv.cfg").is_file() {
            continue;
        }
        if std::fs::canonicalize(&p).is_ok_and(|c| {
            c.is_file() && !c.starts_with(&canon_root) && bases.iter().any(|b| c.starts_with(b))
        }) {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    None
}

/// Canonical homes of installed Python interpreters: system, Xcode/CLT,
/// python.org, Homebrew, pyenv/uv/mise/asdf/rye/conda, and Otto's toolchains.
#[allow(clippy::disallowed_methods)]
fn trusted_interpreter_bases() -> Vec<std::path::PathBuf> {
    let mut raw: Vec<std::path::PathBuf> = [
        "/usr/bin",
        "/usr/lib",
        "/usr/local",
        "/opt/homebrew",
        "/opt/local",
        "/Library/Frameworks/Python.framework",
        "/Library/Developer/CommandLineTools",
        "/Applications/Xcode.app",
    ]
    .iter()
    .map(std::path::PathBuf::from)
    .collect();
    if let Some(home) = otto_core::secret_paths::home_dir() {
        for rel in [
            ".pyenv",
            ".local/share/uv",
            ".local/share/mise",
            ".asdf",
            ".rye",
            "miniconda3",
            "anaconda3",
            "miniforge3",
            "mambaforge",
            ".conda",
        ] {
            raw.push(home.join(rel));
        }
    }
    for data in otto_core::secret_paths::otto_data_dirs() {
        raw.push(data.join("toolchains"));
    }
    raw.into_iter()
        .filter_map(|b| std::fs::canonicalize(b).ok())
        .collect()
}

#[allow(clippy::disallowed_methods)]
fn read_go_file_header(path: &Path) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = Vec::with_capacity(4096);
    f.by_ref().take(4096).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

fn maybe_rewrite_go_build_tag_diagnostics(
    msg: &mut Value,
    root: &str,
    active_tags: &[String],
) -> bool {
    let Some(uri) = str_at(msg, &["params", "uri"]).map(str::to_owned) else {
        return false;
    };
    let Some(diags_peek) = msg.pointer("/params/diagnostics").and_then(Value::as_array) else {
        return false;
    };
    let has_exclusion = diags_peek.iter().any(|d| {
        d.get("message")
            .and_then(Value::as_str)
            .is_some_and(is_gopls_build_tag_exclusion_msg)
    });
    if !has_exclusion {
        return false;
    }

    let Some(file_path) = otto_core::paths::file_uri_to_path(&uri) else {
        return false;
    };
    if file_path.extension().and_then(|e| e.to_str()) != Some("go") {
        return false;
    }
    let root_path = Path::new(root);
    let in_root = file_path.starts_with(root_path) || {
        #[allow(clippy::disallowed_methods)]
        let ok = match (
            std::fs::canonicalize(&file_path),
            std::fs::canonicalize(root_path),
        ) {
            (Ok(cf), Ok(cr)) => cf.starts_with(&cr),
            _ => false,
        };
        ok
    };
    if !in_root {
        return false;
    }

    let Some(header) = read_go_file_header(&file_path) else {
        return false;
    };
    let file_tags = crate::lsp_service::extract_go_file_build_tags(&header);
    let missing: Vec<String> = file_tags
        .into_iter()
        .filter(|t| !active_tags.iter().any(|at| at == t))
        .collect();
    if missing.is_empty() {
        return false;
    }

    let tags_csv = missing.join(",");
    let rewritten_msg = format!(
        "Excluded by //go:build {tags_csv} — enable Go build tag \"{tags_csv}\" in the editor bar (or click ▶ Run/Debug in the gutter) to index, run, and debug this file."
    );

    let Some(diags_mut) = msg
        .pointer_mut("/params/diagnostics")
        .and_then(Value::as_array_mut)
    else {
        return false;
    };
    let mut changed = false;
    for diag in diags_mut {
        let should_rewrite = diag
            .get("message")
            .and_then(Value::as_str)
            .is_some_and(is_gopls_build_tag_exclusion_msg);
        if should_rewrite {
            if let Some(obj) = diag.as_object_mut() {
                obj.insert("message".into(), Value::String(rewritten_msg.clone()));
                obj.insert("code".into(), Value::String("otto_go_build_tag".into()));
                changed = true;
            }
        }
    }
    changed
}

impl Mux {
    fn is_blocked_non_root_method(method: &str) -> bool {
        matches!(
            method,
            "workspace/didChangeWorkspaceFolders"
                | "workspace/executeCommand"
                | "workspace/applyEdit"
                | "rust-analyzer/runSingle"
                | "experimental/runTest"
        ) || method.starts_with("gopls/")
    }

    #[allow(clippy::disallowed_methods)]
    fn is_confined_local_file_uri(&self, uri: &str) -> bool {
        let Some(ref canon_root) = self.canon_root else {
            return false;
        };
        // An unescaped `?`/`#` is a query/fragment the server's URI parser drops,
        // so the path checked here would not be the file it opens.
        if uri.contains(['?', '#']) {
            return false;
        }
        let Some(path) = otto_core::paths::file_uri_to_path(uri) else {
            return false;
        };
        let Some(canon) = std::fs::canonicalize(&path)
            .ok()
            .or_else(|| otto_core::secret_paths::canonicalize_prospective(&path))
        else {
            return false;
        };
        canon.starts_with(canon_root)
            && !otto_core::secret_paths::is_denied_file(&canon)
            && !otto_core::secret_paths::in_protected_dir(&canon)
            && crate::target::enforce_non_root_path(&canon).is_ok()
    }

    fn check_non_root_value_uris(&self, v: &Value, depth: usize) -> bool {
        if depth > 16 {
            return false;
        }
        match v {
            Value::String(s) => {
                let bytes = s.as_bytes();
                if bytes
                    .get(..5)
                    .is_some_and(|p| p.eq_ignore_ascii_case(b"file:"))
                {
                    s.get(..7)
                        .is_some_and(|p| p.eq_ignore_ascii_case("file://"))
                        && self.is_confined_local_file_uri(s)
                } else {
                    !(bytes.get(..4).is_some_and(|p| {
                        p.eq_ignore_ascii_case(b"jar:") || p.eq_ignore_ascii_case(b"jdt:")
                    }) || bytes
                        .get(..8)
                        .is_some_and(|p| p.eq_ignore_ascii_case(b"zipfile:")))
                }
            }
            Value::Array(arr) => arr
                .iter()
                .all(|item| self.check_non_root_value_uris(item, depth + 1)),
            Value::Object(map) => map
                .values()
                .all(|val| self.check_non_root_value_uris(val, depth + 1)),
            _ => true,
        }
    }

    fn check_non_root_message_confined(&self, method: &str, msg: &Value) -> bool {
        if self.owner_is_root || self.key.ssh.is_some() {
            return true;
        }
        if matches!(
            method,
            "initialize" | "initialized" | "shutdown" | "exit" | "$/cancelRequest"
        ) {
            return true;
        }
        if method.starts_with("textDocument/") {
            if let Some(u) = str_at(msg, &["params", "textDocument", "uri"])
                .or_else(|| str_at(msg, &["params", "uri"]))
            {
                if !u
                    .get(..7)
                    .is_some_and(|p| p.eq_ignore_ascii_case("file://"))
                {
                    return false;
                }
            }
        }
        match msg.get("params") {
            Some(params) => self.check_non_root_value_uris(params, 0),
            None => true,
        }
    }

    fn primary_root_uri(&self) -> String {
        if self.key.root.starts_with("file://") {
            self.key.root.clone()
        } else {
            otto_core::paths::path_to_file_uri(Path::new(&self.key.root))
        }
    }

    fn workspace_folders_json(&self) -> Vec<Value> {
        let primary_uri = self.primary_root_uri();
        let primary_name = Path::new(&self.key.root)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("workspace")
            .to_string();
        vec![json!({ "uri": primary_uri, "name": primary_name })]
    }

    fn enrich_initialize_params(&self, msg: &mut Value) {
        let Some(params) = msg.get_mut("params").and_then(Value::as_object_mut) else {
            return;
        };
        let primary_uri = self.primary_root_uri();
        params.insert("rootUri".into(), Value::String(primary_uri));
        params.insert("rootPath".into(), Value::String(self.key.root.clone()));
        params.insert(
            "workspaceFolders".into(),
            Value::Array(self.workspace_folders_json()),
        );

        let norm_lang = servers::normalize_lsp_lang(&self.key.lang);
        if norm_lang == "go" {
            params.insert(
                "initializationOptions".into(),
                default_gopls_settings(&self.go_build_tags),
            );
        } else if norm_lang == "python" && self.key.ssh.is_none() && self.owner_is_root {
            if let Some(py_path) = detect_local_python_path(&self.key.root) {
                params.insert(
                    "initializationOptions".into(),
                    json!({ "pythonPath": py_path }),
                );
                // pyright ignores `initializationOptions`; it asks `workspace/configuration`
                // (answered below) only when the client says it can.
                let caps = params.entry("capabilities").or_insert_with(|| json!({}));
                if let Some(caps) = caps.as_object_mut() {
                    let ws = caps.entry("workspace").or_insert_with(|| json!({}));
                    if let Some(ws) = ws.as_object_mut() {
                        ws.insert("configuration".into(), Value::Bool(true));
                    }
                }
            } else {
                params.remove("initializationOptions");
            }
        } else if norm_lang == "java" {
            // Non-root owners never pass their own options through (bundles load code).
            let mut opts = params
                .remove("initializationOptions")
                .filter(|o| self.owner_is_root && o.is_object())
                .unwrap_or_else(|| json!({}));
            merge_json(&mut opts, json!({ "settings": jdtls_settings() }));
            params.insert("initializationOptions".into(), opts);
        } else if !self.owner_is_root && norm_lang == "rust" {
            params.insert("initializationOptions".into(), non_root_rust_settings());
        } else if !self.owner_is_root {
            params.remove("initializationOptions");
        }
    }

    fn build_configuration_reply(&self, msg: &Value) -> Value {
        let items = msg
            .pointer("/params/items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let norm_lang = servers::normalize_lsp_lang(&self.key.lang);
        let py_path = if norm_lang == "python" && self.key.ssh.is_none() && self.owner_is_root {
            detect_local_python_path(&self.key.root)
        } else {
            None
        };
        let replies: Vec<Value> = items
            .iter()
            .map(|it| {
                let section = it.get("section").and_then(Value::as_str).unwrap_or("");
                if section == "gopls" || (section.is_empty() && norm_lang == "go") {
                    default_gopls_settings(&self.go_build_tags)
                } else if !self.owner_is_root
                    && norm_lang == "rust"
                    && (section.is_empty() || section == "rust-analyzer")
                {
                    non_root_rust_settings()
                } else if (section == "python" || section == "python.analysis") && py_path.is_some()
                {
                    json!({ "pythonPath": py_path })
                } else if norm_lang == "java" && (section.is_empty() || section.starts_with("java"))
                {
                    let settings = jdtls_settings();
                    let ptr = format!("/{}", section.replace('.', "/"));
                    let found = if section.is_empty() {
                        Some(&settings)
                    } else {
                        settings.pointer(&ptr)
                    };
                    found.cloned().unwrap_or(Value::Null)
                } else {
                    Value::Null
                }
            })
            .collect();
        Value::Array(replies)
    }

    pub(super) fn on_client(&mut self, client: u64, text: &str) {
        if !self.clients.contains_key(&client) {
            return;
        }
        self.touch();

        // Fast-path high-frequency buffer notifications without constructing a Value DOM
        // (enabled for root or remote SSH; non-root local sessions inspect every URI).
        if self.owner_is_root || self.key.ssh.is_some() {
            if let Ok(probe) = serde_json::from_str::<RpcEnvelopeProbe<'_>>(text) {
                if probe.id.is_none()
                    && matches!(
                        probe.method.as_deref(),
                        Some(
                            "textDocument/didChange"
                                | "textDocument/didSave"
                                | "textDocument/willSave"
                        )
                    )
                {
                    self.to_server_raw(text);
                    return;
                }
            }
        }

        let Ok(mut msg) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let method = msg.get("method").and_then(Value::as_str).map(str::to_owned);
        let id = msg.get("id").filter(|v| !v.is_null()).cloned();

        if !self.owner_is_root {
            if let Some(ref m) = method {
                if Self::is_blocked_non_root_method(m) {
                    if let Some(req_id) = id {
                        let reply = json!({
                            "jsonrpc": "2.0",
                            "id": req_id,
                            "error": {
                                "code": -32600,
                                "message": format!("{m} requires root authority")
                            }
                        });
                        self.send_client(client, reply.to_string());
                    }
                    return;
                }
                if !self.check_non_root_message_confined(m, &msg) {
                    if let Some(req_id) = id {
                        let reply = json!({
                            "jsonrpc": "2.0",
                            "id": req_id,
                            "error": {
                                "code": -32600,
                                "message": "URI escapes authorized workspace root"
                            }
                        });
                        self.send_client(client, reply.to_string());
                    }
                    return;
                }
            }
        }
        match (method.as_deref(), id) {
            (Some("initialize"), Some(id)) => {
                if let Init::Done(result) = &self.init {
                    let reply = json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string();
                    self.send_client(client, reply);
                } else if let Init::Pending { waiters, .. } = &mut self.init {
                    waiters.push((client, id));
                } else {
                    let pool_id = self.alloc_id();
                    msg["id"] = json!(pool_id);
                    self.enrich_initialize_params(&mut msg);
                    self.init = Init::Pending {
                        pool_id,
                        waiters: vec![(client, id)],
                    };
                    self.to_server(&msg);
                }
            }
            (Some("shutdown"), Some(id)) => {
                let reply = json!({"jsonrpc": "2.0", "id": id, "result": null});
                self.send_client(client, reply.to_string());
            }
            (Some("workspace/executeCommand"), Some(id)) if !self.owner_is_root => {
                let reply = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": -32600,
                        "message": "workspace/executeCommand requires root authority"
                    }
                });
                self.send_client(client, reply.to_string());
            }
            (Some(_), Some(id)) => {
                let pool_id = self.alloc_id();
                self.pending.insert(pool_id, (client, id));
                msg["id"] = json!(pool_id);
                self.to_server(&msg);
            }
            (Some("initialized"), None) => {
                if !self.initialized_sent {
                    self.initialized_sent = true;
                    self.to_server_raw(text);
                }
            }
            (Some("exit"), None) => {}
            (Some("workspace/didChangeConfiguration"), None) => {
                let norm_lang = servers::normalize_lsp_lang(&self.key.lang);
                if norm_lang == "go" {
                    self.to_server(&json!({
                        "jsonrpc": "2.0",
                        "method": "workspace/didChangeConfiguration",
                        "params": {
                            "settings": {
                                "gopls": default_gopls_settings(&self.go_build_tags)
                            }
                        }
                    }));
                } else if norm_lang == "python" && self.key.ssh.is_none() && self.owner_is_root {
                    let settings = match detect_local_python_path(&self.key.root) {
                        Some(py) => json!({ "python": { "pythonPath": py } }),
                        None => json!({}),
                    };
                    self.to_server(&json!({
                        "jsonrpc": "2.0",
                        "method": "workspace/didChangeConfiguration",
                        "params": { "settings": settings }
                    }));
                } else if self.owner_is_root {
                    self.to_server_raw(text);
                }
            }
            (Some("textDocument/didOpen"), None) => {
                let Some(uri) = str_at(&msg, &["params", "textDocument", "uri"]) else {
                    return;
                };
                let uri = uri.to_owned();
                let newly = self
                    .clients
                    .get_mut(&client)
                    .is_some_and(|c| c.open.insert(uri.clone()));
                let holders = if newly {
                    let n = self.docs.entry(uri.clone()).or_insert(0);
                    *n += 1;
                    let count = *n;
                    self.open_docs.store(self.docs.len(), Ordering::Release);
                    count
                } else {
                    self.docs.get(&uri).copied().unwrap_or(1)
                };
                if newly && holders == 1 {
                    self.to_server_raw(text);
                } else {
                    self.to_server(&json!({
                        "jsonrpc": "2.0",
                        "method": "textDocument/didClose",
                        "params": {"textDocument": {"uri": uri}},
                    }));
                    self.to_server_raw(text);
                }
            }
            (Some("textDocument/didClose"), None) => {
                let Some(uri) = str_at(&msg, &["params", "textDocument", "uri"]) else {
                    return;
                };
                let uri = uri.to_owned();
                if self
                    .clients
                    .get_mut(&client)
                    .is_some_and(|c| c.open.remove(&uri))
                {
                    self.release_doc(&uri);
                }
            }
            (Some("$/cancelRequest"), None) => {
                let Some(target) = msg.pointer("/params/id").cloned() else {
                    return;
                };
                let pool_id = self
                    .pending
                    .iter()
                    .find(|(_, (c, id))| *c == client && *id == target)
                    .map(|(pid, _)| *pid);
                if let Some(pid) = pool_id {
                    msg["params"]["id"] = json!(pid);
                    self.to_server(&msg);
                }
            }
            (Some(_), None) => self.to_server_raw(text),
            (None, Some(id)) => {
                if self.server_reqs.remove(&id.to_string()).is_some() {
                    if !self.owner_is_root
                        && self.key.ssh.is_none()
                        && !self.check_non_root_value_uris(&msg, 0)
                    {
                        self.to_server(&json!({"jsonrpc": "2.0", "id": id, "result": null}));
                        return;
                    }
                    self.to_server_raw(text);
                }
            }
            (None, None) => {}
        }
    }

    pub(super) fn on_server(&mut self, body: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(body) else {
            return true;
        };

        // Fast-path non-diagnostic server notifications without allocating a Value DOM.
        if let Ok(probe) = serde_json::from_str::<RpcEnvelopeProbe<'_>>(text) {
            if probe.id.is_none()
                && probe.method.is_some()
                && probe.method.as_deref() != Some("textDocument/publishDiagnostics")
            {
                let all: Vec<u64> = self.clients.keys().copied().collect();
                for c in all {
                    self.send_client(c, text.to_owned());
                }
                return true;
            }
        }

        let Ok(mut msg) = serde_json::from_str::<Value>(text) else {
            return true;
        };
        let method = msg.get("method").and_then(Value::as_str).map(str::to_owned);
        let id = msg.get("id").filter(|v| !v.is_null()).cloned();
        match (method.as_deref(), id) {
            (Some(m), Some(id)) => match m {
                "workspace/configuration" => {
                    let result = self.build_configuration_reply(&msg);
                    self.to_server(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
                }
                "workspace/workspaceFolders" => {
                    let result = Value::Array(self.workspace_folders_json());
                    self.to_server(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
                }
                "window/workDoneProgress/create"
                | "client/registerCapability"
                | "client/unregisterCapability" => {
                    self.to_server(&json!({"jsonrpc": "2.0", "id": id, "result": null}));
                }
                _ => match self.clients.keys().next().copied() {
                    Some(target) => {
                        self.server_reqs.insert(id.to_string(), (target, id));
                        self.send_client(target, text.to_owned());
                    }
                    None => {
                        self.to_server(&json!({"jsonrpc": "2.0", "id": id, "result": null}));
                    }
                },
            },
            (Some("textDocument/publishDiagnostics"), None) => {
                let Some(uri) = str_at(&msg, &["params", "uri"]).map(str::to_owned) else {
                    return true;
                };
                let targets: Vec<u64> = self
                    .clients
                    .iter()
                    .filter(|(_, c)| c.open.contains(&uri))
                    .map(|(id, _)| *id)
                    .collect();
                if targets.is_empty() {
                    return true;
                }
                let out_payload = if self.key.ssh.is_none()
                    && servers::normalize_lsp_lang(&self.key.lang) == "go"
                    && maybe_rewrite_go_build_tag_diagnostics(
                        &mut msg,
                        &self.key.root,
                        &self.go_build_tags,
                    ) {
                    msg.to_string()
                } else {
                    text.to_owned()
                };
                for c in targets {
                    self.send_client(c, out_payload.clone());
                }
            }
            (Some(_), None) => {
                let all: Vec<u64> = self.clients.keys().copied().collect();
                for c in all {
                    self.send_client(c, text.to_owned());
                }
            }
            (None, Some(id)) => {
                let Some(pid) = id.as_u64() else {
                    return true;
                };
                if let Init::Pending { pool_id, .. } = &self.init {
                    if *pool_id == pid {
                        let Init::Pending { waiters, .. } =
                            std::mem::replace(&mut self.init, Init::NotStarted)
                        else {
                            unreachable!()
                        };
                        let ok = msg.get("result").cloned();
                        let err = msg.get("error").cloned();
                        for (c, wid) in waiters {
                            let reply = match (&ok, &err) {
                                (Some(r), _) => json!({"jsonrpc": "2.0", "id": wid, "result": r}),
                                (None, Some(e)) => json!({"jsonrpc": "2.0", "id": wid, "error": e}),
                                (None, None) => {
                                    json!({"jsonrpc": "2.0", "id": wid, "result": null})
                                }
                            };
                            self.send_client(c, reply.to_string());
                        }
                        return match ok {
                            Some(r) => {
                                self.init = Init::Done(r);
                                true
                            }
                            None => false,
                        };
                    }
                }
                if let Some((c, orig)) = self.pending.remove(&pid) {
                    msg["id"] = orig;
                    self.send_client(c, msg.to_string());
                }
            }
            (None, None) => {}
        }
        true
    }
}

#[cfg(test)]
mod python_path_tests {
    #[test]
    fn a_committed_venv_python_is_never_handed_to_pyright() {
        let ws = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let bin = ws.path().join(".venv/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("python"), "#!/bin/sh\ntouch /tmp/pwned\n").unwrap();
        let root = ws.path().to_str().unwrap();
        assert_eq!(super::detect_local_python_path(root), None);

        // E5: a committed symlink to an attacker file outside the repo (another
        // repo, Downloads, /tmp) is not an installed interpreter either.
        std::fs::write(ws.path().join(".venv/pyvenv.cfg"), "home = /tmp\n").unwrap();
        let interp = outside.path().join("python3");
        std::fs::write(&interp, "").unwrap();
        std::fs::remove_file(bin.join("python")).unwrap();
        std::os::unix::fs::symlink(&interp, bin.join("python")).unwrap();
        assert_eq!(super::detect_local_python_path(root), None);

        // A real venv on an installed interpreter is still found.
        if let Some(sys) = ["/usr/bin/python3", "/opt/homebrew/bin/python3"]
            .into_iter()
            .find(|p| std::path::Path::new(p).is_file())
        {
            std::fs::remove_file(bin.join("python")).unwrap();
            std::os::unix::fs::symlink(sys, bin.join("python")).unwrap();
            assert!(super::detect_local_python_path(root).is_some());
        }
    }
}
