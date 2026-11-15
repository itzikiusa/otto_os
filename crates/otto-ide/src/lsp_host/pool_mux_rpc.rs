//! JSON-RPC initialize enrichment, configuration replies, and client/server
//! message routing for [`super::Mux`].

use std::path::Path;
use std::sync::atomic::Ordering;

use serde_json::{json, Value};

use super::super::servers;
use super::{Init, Mux};

fn str_at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for p in path {
        cur = cur.get(*p)?;
    }
    cur.as_str()
}

fn default_gopls_settings() -> Value {
    json!({
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
            let defaults = default_gopls_settings();
            match params.get_mut("initializationOptions") {
                Some(Value::Object(existing)) => {
                    if let Value::Object(def_map) = defaults {
                        for (k, v) in def_map {
                            existing.entry(k).or_insert(v);
                        }
                    }
                }
                None | Some(Value::Null) => {
                    params.insert("initializationOptions".into(), defaults);
                }
                _ => {}
            }
        } else if norm_lang == "python" && self.key.ssh.is_none() {
            if let Some(py_path) = detect_local_python_path(&self.key.root) {
                if params.get("initializationOptions").is_none() {
                    params.insert(
                        "initializationOptions".into(),
                        json!({ "pythonPath": py_path }),
                    );
                }
            }
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
                    default_gopls_settings()
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
                let Some(uri) = str_at(&msg, &["params", "uri"]) else {
                    return true;
                };
                let targets: Vec<u64> = self
                    .clients
                    .iter()
                    .filter(|(_, c)| c.open.contains(uri))
                    .map(|(id, _)| *id)
                    .collect();
                for c in targets {
                    self.send_client(c, text.to_owned());
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
