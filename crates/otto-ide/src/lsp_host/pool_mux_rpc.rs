//! JSON-RPC initialize enrichment, configuration replies, Go build-tag
//! diagnostic rewriting, and fast-path client/server routing for [`super::Mux`].

use std::borrow::Cow;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use serde::de::IgnoredAny;
use serde::Deserialize;
use serde_json::{json, Value};

use super::super::servers;
use super::{Init, Mux};

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
        "completeUnimported": true
    })
}

fn detect_local_python_path(root: &str) -> Option<String> {
    let r = Path::new(root);
    for rel in [".venv/bin/python", "venv/bin/python", ".env/bin/python"] {
        let p = r.join(rel);
        if p.is_file() {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    None
}

fn decode_local_file_uri(uri: &str) -> Option<PathBuf> {
    let raw = uri.strip_prefix("file://")?;
    let mut bytes = Vec::with_capacity(raw.len());
    let src = raw.as_bytes();
    let mut i = 0;
    while i < src.len() {
        if src[i] == b'%' && i + 2 < src.len() {
            if let (Some(hi), Some(lo)) = (
                (src[i + 1] as char).to_digit(16),
                (src[i + 2] as char).to_digit(16),
            ) {
                bytes.push(((hi << 4) | lo) as u8);
                i += 3;
                continue;
            }
        }
        bytes.push(src[i]);
        i += 1;
    }
    let decoded = String::from_utf8(bytes).ok()?;
    let p = PathBuf::from(decoded);
    p.is_absolute().then_some(p)
}

fn is_gopls_build_tag_exclusion_msg(message: &str) -> bool {
    message.contains("No packages found for open file")
        || message.contains("excluded due to its build tags")
        || message.contains("build constraints exclude all Go files")
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

    let Some(file_path) = decode_local_file_uri(&uri) else {
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
    fn workspace_folders_json(&self) -> Vec<Value> {
        let primary_uri = if self.key.root.starts_with("file://") {
            self.key.root.clone()
        } else {
            format!("file://{}", self.key.root)
        };
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
        let primary_uri = if self.key.root.starts_with("file://") {
            self.key.root.clone()
        } else {
            format!("file://{}", self.key.root)
        };
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
        } else if norm_lang == "python" && self.key.ssh.is_none() {
            if let Some(py_path) = detect_local_python_path(&self.key.root) {
                params.insert(
                    "initializationOptions".into(),
                    json!({ "pythonPath": py_path }),
                );
            } else {
                params.remove("initializationOptions");
            }
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
        let py_path = if norm_lang == "python" && self.key.ssh.is_none() {
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
                } else if (section == "python" || section == "python.analysis") && py_path.is_some()
                {
                    json!({ "pythonPath": py_path })
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

        // Fast-path high-frequency buffer notifications without constructing a Value DOM.
        if let Ok(probe) = serde_json::from_str::<RpcEnvelopeProbe<'_>>(text) {
            if probe.id.is_none()
                && matches!(
                    probe.method.as_deref(),
                    Some(
                        "textDocument/didChange" | "textDocument/didSave" | "textDocument/willSave"
                    )
                )
            {
                self.to_server_raw(text);
                return;
            }
        }

        let Ok(mut msg) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let method = msg.get("method").and_then(Value::as_str).map(str::to_owned);
        let id = msg.get("id").filter(|v| !v.is_null()).cloned();
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
                } else if norm_lang == "python" && self.key.ssh.is_none() {
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
