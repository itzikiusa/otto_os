//! Round 5 unit tests for `LspPool`: non-root `/ws/lsp` method & `file://` URI
//! confinement, Go build-tag `PoolKey` isolation, and stale `gopls` eviction on
//! project/module Go build-tag updates.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::BufReader;

use super::framing;
use super::pool::{
    Attached, LspPool, PoolKey, ServerIo, SpawnLaunch, Spawner, IDLE_TIMEOUT, WRITE_QUEUE_MAX_BYTES,
};
use crate::types::IdeProjectEnvironment;

struct RecordedPool {
    spawns: Arc<AtomicUsize>,
    forwarded_methods: Arc<Mutex<Vec<String>>>,
    init_params: Arc<Mutex<Vec<Value>>>,
}

fn recording_pool() -> (Arc<LspPool>, RecordedPool) {
    let spawns = Arc::new(AtomicUsize::new(0));
    let forwarded_methods = Arc::new(Mutex::new(Vec::new()));
    let init_params = Arc::new(Mutex::new(Vec::new()));
    let (s, fm, ip) = (
        spawns.clone(),
        forwarded_methods.clone(),
        init_params.clone(),
    );

    let spawner: Spawner = Arc::new(move |_key: &PoolKey, _launch: &SpawnLaunch| {
        s.fetch_add(1, Ordering::SeqCst);
        let (pool_side_w, server_r) = tokio::io::duplex(1 << 16);
        let (server_w, pool_side_r) = tokio::io::duplex(1 << 16);
        let (fm, ip) = (fm.clone(), ip.clone());
        tokio::spawn(async move {
            let mut r = BufReader::new(server_r);
            let mut w = server_w;
            while let Ok(Some(body)) = framing::read_message(&mut r).await {
                let msg: Value = serde_json::from_slice(&body).unwrap();
                let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
                fm.lock().unwrap().push(method.to_string());
                let reply = match method {
                    "initialize" => {
                        ip.lock()
                            .unwrap()
                            .push(msg.get("params").cloned().unwrap_or(Value::Null));
                        Some(json!({
                            "jsonrpc": "2.0",
                            "id": msg["id"],
                            "result": { "capabilities": { "hoverProvider": true } }
                        }))
                    }
                    "textDocument/hover" => Some(json!({
                        "jsonrpc": "2.0",
                        "id": msg["id"],
                        "result": { "contents": "confined-hover-ok" }
                    })),
                    _ => None,
                };
                if let Some(reply) = reply {
                    let bytes = serde_json::to_vec(&reply).unwrap();
                    if framing::write_message(&mut w, &bytes).await.is_err() {
                        break;
                    }
                }
            }
        });
        Ok(ServerIo {
            reader: Box::new(BufReader::new(pool_side_r)),
            writer: Box::new(pool_side_w),
            child: None,
        })
    });

    (
        LspPool::new_inner(spawner, IDLE_TIMEOUT, WRITE_QUEUE_MAX_BYTES, None, None),
        RecordedPool {
            spawns,
            forwarded_methods,
            init_params,
        },
    )
}

async fn recv_msg(a: &mut Attached) -> Value {
    let text = tokio::time::timeout(Duration::from_secs(2), a.rx.recv())
        .await
        .expect("timed out waiting for pooled LSP reply")
        .expect("pooled LSP channel closed");
    serde_json::from_str(&text).unwrap()
}

#[tokio::test]
async fn non_root_pooled_mux_blocks_workspace_escape_methods_and_out_of_root_file_uris() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().join("workspace");
    let sibling_outside = tmp.path().join("workspace-secret");
    std::fs::create_dir_all(ws_root.join("src")).unwrap();
    std::fs::create_dir_all(&sibling_outside).unwrap();
    std::fs::write(ws_root.join("tsconfig.json"), "{}").unwrap();
    let allowed_file = ws_root.join("src/main.ts");
    let denied_dotenv = ws_root.join(".env");
    let outside_file = sibling_outside.join("leak.ts");
    std::fs::write(&allowed_file, "export const x = 1;\n").unwrap();
    std::fs::write(&denied_dotenv, "SECRET_TOKEN=xyz\n").unwrap();
    std::fs::write(&outside_file, "export const s = 'top-secret';\n").unwrap();

    let (pool, rec) = recording_pool();
    pool.record_owner_privilege("u_editor", false);

    let canon_ws = std::fs::canonicalize(&ws_root).unwrap();
    let key = PoolKey {
        owner: "u_editor".into(),
        lang: "typescript".into(),
        root: canon_ws.to_string_lossy().into_owned(),
        ssh: None,
        go_build_tags: Vec::new(),
    };
    let mut client = pool.attach(key, "tsls", &[]).unwrap();

    // 1. Client tries to spoof rootUri=/etc in `initialize` -> Mux forces confined root.
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "rootUri": "file:///etc",
            "rootPath": "/etc",
            "workspaceFolders": [{ "uri": "file:///etc", "name": "etc" }]
        }
    });
    assert!(client.send(init_req.to_string()).await);
    let init_resp = recv_msg(&mut client).await;
    assert_eq!(init_resp["id"], json!(1));
    let expected_root_uri = otto_core::paths::path_to_file_uri(&canon_ws);
    let sent_init = rec.init_params.lock().unwrap().first().cloned().unwrap();
    assert_eq!(sent_init["rootUri"], json!(expected_root_uri));
    assert_eq!(
        sent_init["rootPath"],
        json!(canon_ws.to_string_lossy().as_ref())
    );
    assert!(
        client
            .send(json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}).to_string())
            .await
    );

    // 2. `workspace/didChangeWorkspaceFolders` notification is silently dropped.
    let escape_folders = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWorkspaceFolders",
        "params": {
            "event": {
                "added": [{ "uri": otto_core::paths::path_to_file_uri(&sibling_outside), "name": "leak" }],
                "removed": []
            }
        }
    });
    assert!(client.send(escape_folders.to_string()).await);

    // 3. `workspace/executeCommand` request returns JSON-RPC -32600 without reaching server.
    let exec_req = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "workspace/executeCommand",
        "params": { "command": "gopls.run_tests" }
    });
    assert!(client.send(exec_req.to_string()).await);
    let exec_err = recv_msg(&mut client).await;
    assert_eq!(exec_err["id"], json!(2));
    assert_eq!(exec_err["error"]["code"], json!(-32600));

    // 4. Out-of-workspace `textDocument/didOpen` (notification) and `hover` (request) are blocked.
    let outside_uri = otto_core::paths::path_to_file_uri(&outside_file);
    assert!(client
        .send(
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": { "textDocument": { "uri": outside_uri, "languageId": "typescript", "version": 1, "text": "x" } }
            })
            .to_string(),
        )
        .await);
    assert!(
        client
            .send(
                json!({
                    "jsonrpc": "2.0",
                    "id": 3,
                    "method": "textDocument/hover",
                    "params": {
                        "textDocument": { "uri": outside_uri },
                        "position": { "line": 0, "character": 0 }
                    }
                })
                .to_string(),
            )
            .await
    );
    let outside_err = recv_msg(&mut client).await;
    assert_eq!(outside_err["id"], json!(3));
    assert_eq!(outside_err["error"]["code"], json!(-32600));

    // 5. Protected `.env` inside workspace is also blocked for non-root.
    let dotenv_uri = otto_core::paths::path_to_file_uri(&denied_dotenv);
    assert!(
        client
            .send(
                json!({
                    "jsonrpc": "2.0",
                    "id": 4,
                    "method": "textDocument/hover",
                    "params": {
                        "textDocument": { "uri": dotenv_uri },
                        "position": { "line": 0, "character": 0 }
                    }
                })
                .to_string(),
            )
            .await
    );
    let dotenv_err = recv_msg(&mut client).await;
    assert_eq!(dotenv_err["id"], json!(4));
    assert_eq!(dotenv_err["error"]["code"], json!(-32600));

    // 6. Confined in-workspace file URI succeeds and reaches the server.
    let allowed_uri = otto_core::paths::path_to_file_uri(&allowed_file);
    assert!(client
        .send(
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": { "textDocument": { "uri": allowed_uri, "languageId": "typescript", "version": 1, "text": "export const x = 1;\n" } }
            })
            .to_string(),
        )
        .await);
    assert!(
        client
            .send(
                json!({
                    "jsonrpc": "2.0",
                    "id": 5,
                    "method": "textDocument/hover",
                    "params": {
                        "textDocument": { "uri": allowed_uri },
                        "position": { "line": 0, "character": 7 }
                    }
                })
                .to_string(),
            )
            .await
    );
    let ok_hover = recv_msg(&mut client).await;
    assert_eq!(ok_hover["id"], json!(5));
    assert_eq!(ok_hover["result"]["contents"], json!("confined-hover-ok"));

    let methods = rec.forwarded_methods.lock().unwrap().clone();
    assert!(
        !methods
            .iter()
            .any(|m| m == "workspace/didChangeWorkspaceFolders" || m == "workspace/executeCommand"),
        "blocked workspace methods must never reach the LSP server: {methods:?}"
    );
    assert_eq!(
        methods,
        vec![
            "initialize",
            "initialized",
            "textDocument/didOpen",
            "textDocument/hover",
        ]
    );
}

#[tokio::test]
async fn gopls_pool_keys_by_sorted_build_tags_and_evicts_on_module_tag_updates() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("gorepo");
    let submod = ws.join("services/billing");
    std::fs::create_dir_all(&submod).unwrap();
    std::fs::write(ws.join("go.mod"), "module example.com/root\n\ngo 1.23\n").unwrap();
    std::fs::write(
        submod.join("go.mod"),
        "module example.com/billing\n\ngo 1.23\n",
    )
    .unwrap();

    let canon_ws = std::fs::canonicalize(&ws).unwrap();
    let canon_sub = std::fs::canonicalize(&submod).unwrap();
    let ws_str = canon_ws.to_string_lossy().into_owned();
    let sub_str = canon_sub.to_string_lossy().into_owned();

    let (pool, rec) = recording_pool();
    pool.record_owner_privilege("root_u", true);

    // Seed workspace `go_build_tags = ["integration"]`, `submod = []` (explicit empty override!).
    let mut mod_tags = BTreeMap::new();
    mod_tags.insert(sub_str.clone(), Vec::new());
    pool.sync_project_go_build_tags(
        &ws_str,
        &IdeProjectEnvironment {
            go_build_tags: vec!["integration".into()],
            module_go_build_tags: mod_tags,
            ..Default::default()
        },
    );
    assert_eq!(pool.go_build_tags(&ws_str), vec!["integration".to_string()]);
    assert!(
        pool.go_build_tags(&sub_str).is_empty(),
        "explicit empty [] module_go_build_tags must clear inherited workspace tags"
    );

    let _root_srv = pool
        .attach(
            PoolKey {
                owner: "root_u".into(),
                lang: "go".into(),
                root: ws_str.clone(),
                ssh: None,
                go_build_tags: Vec::new(),
            },
            "gopls",
            &[],
        )
        .unwrap();
    let _sub_srv = pool
        .attach(
            PoolKey {
                owner: "root_u".into(),
                lang: "go".into(),
                root: sub_str.clone(),
                ssh: None,
                go_build_tags: Vec::new(),
            },
            "gopls",
            &[],
        )
        .unwrap();
    assert_eq!(rec.spawns.load(Ordering::SeqCst), 2);
    assert_eq!(pool.live_servers(), 2);

    // Updating `module_go_build_tags` on `submod` from `[]` to `["component"]` evicts ONLY `submod`'s stale `gopls`!
    let mut updated_mod_tags = BTreeMap::new();
    updated_mod_tags.insert(sub_str.clone(), vec!["component".into()]);
    pool.sync_project_go_build_tags(
        &ws_str,
        &IdeProjectEnvironment {
            go_build_tags: vec!["integration".into()],
            module_go_build_tags: updated_mod_tags,
            ..Default::default()
        },
    );
    assert_eq!(
        pool.live_servers(),
        1,
        "only the module whose effective go_build_tags changed should be evicted"
    );
    assert_eq!(pool.go_build_tags(&sub_str), vec!["component".to_string()]);
}

#[tokio::test]
#[allow(clippy::disallowed_methods)]
async fn go_build_tags_overwrite_init_options_rewrite_excluded_diagnostic_and_evict_on_update() {
    let dir = tempfile::tempdir().unwrap();
    let canon_dir = std::fs::canonicalize(dir.path()).unwrap();
    let root_str = canon_dir.to_string_lossy().into_owned();
    let comp_file = canon_dir.join("assign_frb_component_test.go");
    std::fs::write(
        &comp_file,
        "//go:build component\n\npackage tests\n\nfunc TestComponent() {}\n",
    )
    .unwrap();
    let comp_uri = format!("file://{}", comp_file.display());

    let seen_msgs: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_cl = Arc::clone(&seen_msgs);
    let spawns = Arc::new(AtomicUsize::new(0));
    let spawns_cl = Arc::clone(&spawns);

    let spawner: Spawner = Arc::new(move |_key: &PoolKey, _launch: &SpawnLaunch| {
        spawns_cl.fetch_add(1, Ordering::SeqCst);
        let (pool_side_w, server_r) = tokio::io::duplex(1 << 16);
        let (server_w, pool_side_r) = tokio::io::duplex(1 << 16);
        let seen = Arc::clone(&seen_cl);
        tokio::spawn(async move {
            let mut r = BufReader::new(server_r);
            let mut w = server_w;
            while let Ok(Some(body)) = framing::read_message(&mut r).await {
                let msg: Value = serde_json::from_slice(&body).unwrap();
                seen.lock().unwrap().push(msg.clone());
                let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
                let reply = match method {
                    "initialize" => Some(json!({
                        "jsonrpc": "2.0",
                        "id": msg["id"],
                        "result": { "capabilities": { "hoverProvider": true } }
                    })),
                    "textDocument/didOpen" => {
                        let uri = msg["params"]["textDocument"]["uri"]
                            .as_str()
                            .unwrap_or_default();
                        Some(json!({
                            "jsonrpc": "2.0",
                            "method": "textDocument/publishDiagnostics",
                            "params": {
                                "uri": uri,
                                "diagnostics": [{
                                    "range": {
                                        "start": { "line": 0, "character": 0 },
                                        "end": { "line": 0, "character": 10 }
                                    },
                                    "severity": 1,
                                    "message": "No packages found for open file /tmp/x.go: file is excluded due to its build tags."
                                }]
                            }
                        }))
                    }
                    "workspace/executeCommand" => Some(json!({
                        "jsonrpc": "2.0",
                        "id": msg["id"],
                        "result": { "ran": true }
                    })),
                    _ => None,
                };
                if let Some(reply) = reply {
                    let bytes = serde_json::to_vec(&reply).unwrap();
                    if framing::write_message(&mut w, &bytes).await.is_err() {
                        break;
                    }
                }
            }
        });
        Ok(ServerIo {
            reader: Box::new(BufReader::new(pool_side_r)),
            writer: Box::new(pool_side_w),
            child: None,
        })
    });

    let pool = LspPool::new_inner(spawner, IDLE_TIMEOUT, WRITE_QUEUE_MAX_BYTES, None, None);
    pool.record_owner_privilege("viewer1", false);
    pool.record_owner_privilege("root1", true);

    let go_key = PoolKey {
        owner: "viewer1".into(),
        lang: "go".into(),
        root: root_str.clone(),
        ssh: None,
        go_build_tags: Vec::new(),
    };
    let mut a = pool.attach(go_key, "gopls", &[]).unwrap();
    assert_eq!(a.lang, "go");
    assert_eq!(a.root, root_str);

    // Untrusted client initializationOptions must be overwritten with default_gopls_settings.
    let malicious_init = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "rootUri": "file:///evil",
            "initializationOptions": {
                "env": { "LD_PRELOAD": "/tmp/evil.so" },
                "buildFlags": ["-exec=/tmp/evil"]
            }
        }
    });
    assert!(a.send(malicious_init.to_string()).await);
    let init_resp = recv_msg(&mut a).await;
    assert_eq!(init_resp["id"], json!(1));

    {
        let log = seen_msgs.lock().unwrap();
        let srv_init = log
            .iter()
            .find(|m| m["method"] == "initialize")
            .expect("initialize sent");
        assert_eq!(
            srv_init["params"]["initializationOptions"]["buildFlags"],
            json!(["-mod=mod"])
        );
        assert!(srv_init["params"]["initializationOptions"]
            .get("env")
            .is_none());
    }

    // Non-root workspace/executeCommand is rejected by the mux before reaching gopls.
    assert!(
        a.send(
            json!({
                "jsonrpc": "2.0",
                "id": 99,
                "method": "workspace/executeCommand",
                "params": { "command": "gopls.run_tests" }
            })
            .to_string()
        )
        .await
    );
    let exec_err = recv_msg(&mut a).await;
    assert_eq!(exec_err["id"], json!(99));
    assert_eq!(exec_err["error"]["code"], json!(-32600));

    // Opening a `//go:build component` file rewrites gopls's exclusion diagnostic.
    assert!(
        a.send(
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": comp_uri,
                        "languageId": "go",
                        "version": 1,
                        "text": "//go:build component\n\npackage tests\n"
                    }
                }
            })
            .to_string()
        )
        .await
    );
    let diag_msg = recv_msg(&mut a).await;
    let first_diag = &diag_msg["params"]["diagnostics"][0];
    assert_eq!(first_diag["code"], json!("otto_go_build_tag"));
    assert!(
        first_diag["message"]
            .as_str()
            .unwrap_or_default()
            .contains("Excluded by //go:build component — enable Go build tag \"component\""),
        "unexpected diagnostic: {first_diag}"
    );

    // Updating Go build tags evicts the active gopls instance so the next attach gets `-tags=component`.
    pool.set_go_build_tags(&root_str, vec!["component".into()]);
    assert_eq!(pool.go_build_tags(&root_str), vec!["component".to_string()]);
    assert_eq!(pool.live_servers(), 0);

    seen_msgs.lock().unwrap().clear();
    let root_key = PoolKey {
        owner: "root1".into(),
        lang: "go".into(),
        root: root_str.clone(),
        ssh: None,
        go_build_tags: Vec::new(),
    };
    let mut b = pool.attach(root_key, "gopls", &[]).unwrap();
    assert_eq!(spawns.load(Ordering::SeqCst), 2);
    assert!(
        b.send(
            json!({
                "jsonrpc": "2.0",
                "id": 10,
                "method": "initialize",
                "params": { "rootUri": format!("file://{root_str}") }
            })
            .to_string()
        )
        .await
    );
    let b_init = recv_msg(&mut b).await;
    assert_eq!(b_init["id"], json!(10));
    b.send(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }).to_string())
        .await;

    // Root user may invoke workspace/executeCommand, and workspace/didChangeConfiguration uses server settings.
    assert!(
        b.send(
            json!({
                "jsonrpc": "2.0",
                "method": "workspace/didChangeConfiguration",
                "params": { "settings": { "gopls": { "buildFlags": ["-exec=/bin/sh"] } } }
            })
            .to_string()
        )
        .await
    );
    assert!(
        b.send(
            json!({
                "jsonrpc": "2.0",
                "id": 11,
                "method": "workspace/executeCommand",
                "params": { "command": "gopls.tidy" }
            })
            .to_string()
        )
        .await
    );
    let root_exec = recv_msg(&mut b).await;
    assert_eq!(root_exec["id"], json!(11));
    assert_eq!(root_exec["result"]["ran"], json!(true));

    let log = seen_msgs.lock().unwrap();
    let srv_init2 = log
        .iter()
        .find(|m| m["method"] == "initialize")
        .expect("second initialize sent");
    assert_eq!(
        srv_init2["params"]["initializationOptions"]["buildFlags"],
        json!(["-mod=mod", "-tags=component"])
    );
    let srv_cfg = log
        .iter()
        .find(|m| m["method"] == "workspace/didChangeConfiguration")
        .expect("sanitized didChangeConfiguration sent");
    assert_eq!(
        srv_cfg["params"]["settings"]["gopls"]["buildFlags"],
        json!(["-mod=mod", "-tags=component"])
    );
}

#[tokio::test]
async fn non_root_mux_handles_multibyte_utf8_and_blocks_rfc8089_secret_uris_and_emoji_target_validate(
) {
    // 1. Multi-byte UTF-8 < 6 chars / >= 6 bytes (`"🦀🦀"`) in `IdeTargetSpec::validate` must never panic.
    let emoji_spec = crate::types::IdeTargetSpec {
        kind: crate::types::IdeTargetKind::Local,
        root: "🦀🦀".into(),
        ..Default::default()
    };
    assert!(emoji_spec.validate().is_err());

    // 2. Non-root pooled LSP mux handles Hebrew + emoji UTF-8 buffers cleanly while blocking
    //    RFC 8089 single-slash `file:/...` escapes and `~/.ssh` URIs.
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().join("workspace-utf8");
    std::fs::create_dir_all(ws_root.join("src")).unwrap();
    let allowed_file = ws_root.join("src/hebrew_emoji.ts");
    let utf8_src = "// שלום 🦀\nexport const v = 42;\n";
    std::fs::write(&allowed_file, utf8_src).unwrap();

    let (pool, _rec) = recording_pool();
    pool.record_owner_privilege("u_viewer", false);

    let canon_ws = std::fs::canonicalize(&ws_root).unwrap();
    let key = PoolKey {
        owner: "u_viewer".into(),
        lang: "typescript".into(),
        root: canon_ws.to_string_lossy().into_owned(),
        ssh: None,
        go_build_tags: Vec::new(),
    };
    let mut client = pool.attach(key, "tsls", &[]).unwrap();

    assert!(
        client
            .send(
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": { "rootUri": otto_core::paths::path_to_file_uri(&canon_ws) }
                })
                .to_string()
            )
            .await
    );
    let _ = recv_msg(&mut client).await;
    assert!(
        client
            .send(json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}).to_string())
            .await
    );

    // RFC 8089 single-slash `file:/etc/passwd` and `~/.ssh/id_rsa` URIs must be rejected (`-32600`).
    for (req_id, bad_uri) in [
        (2, "file:/etc/passwd".to_string()),
        (
            3,
            dirs::home_dir()
                .map(|h| format!("file:{}", h.join(".ssh/id_rsa").display()))
                .unwrap_or_else(|| "file:/root/.ssh/id_rsa".to_string()),
        ),
    ] {
        assert!(
            client
                .send(
                    json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "method": "textDocument/hover",
                        "params": {
                            "textDocument": { "uri": bad_uri },
                            "position": { "line": 0, "character": 0 }
                        }
                    })
                    .to_string()
                )
                .await
        );
        let err_msg = recv_msg(&mut client).await;
        assert_eq!(err_msg["id"], json!(req_id));
        assert_eq!(err_msg["error"]["code"], json!(-32600));
    }

    // Multi-byte UTF-8 in-workspace `didOpen` + `hover` succeeds cleanly.
    let allowed_uri = otto_core::paths::path_to_file_uri(&allowed_file);
    assert!(
        client
            .send(
                json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/didOpen",
                    "params": {
                        "textDocument": {
                            "uri": allowed_uri,
                            "languageId": "typescript",
                            "version": 1,
                            "text": utf8_src
                        }
                    }
                })
                .to_string()
            )
            .await
    );
    assert!(
        client
            .send(
                json!({
                    "jsonrpc": "2.0",
                    "id": 4,
                    "method": "textDocument/hover",
                    "params": {
                        "textDocument": { "uri": allowed_uri },
                        "position": { "line": 1, "character": 13 }
                    }
                })
                .to_string()
            )
            .await
    );
    let ok_msg = recv_msg(&mut client).await;
    assert_eq!(ok_msg["id"], json!(4));
    assert_eq!(ok_msg["result"]["contents"], json!("confined-hover-ok"));
}

/// pyright ignores `initializationOptions`; it reads the venv from `workspace/configuration`,
/// which it asks only when the client advertises `workspace.configuration`.
#[tokio::test]
#[cfg(unix)]
async fn python_initialize_advertises_workspace_configuration_with_the_venv() {
    let system_py = std::path::Path::new("/usr/bin/python3");
    if !system_py.is_file() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("py-app");
    std::fs::create_dir_all(root.join(".venv/bin")).unwrap();
    std::fs::write(root.join(".venv/pyvenv.cfg"), "").unwrap();
    std::os::unix::fs::symlink(system_py, root.join(".venv/bin/python")).unwrap();
    let root_str = std::fs::canonicalize(&root)
        .unwrap()
        .to_string_lossy()
        .into_owned();

    let (pool, rec) = recording_pool();
    pool.record_owner_privilege("root1", true);
    let key = PoolKey {
        owner: "root1".into(),
        lang: "python".into(),
        root: root_str.clone(),
        ssh: None,
        go_build_tags: Vec::new(),
    };
    let mut a = pool.attach(key, "pyright-langserver", &[]).unwrap();
    let init =
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"capabilities": {}}});
    assert!(a.send(init.to_string()).await);
    recv_msg(&mut a).await;
    let params = rec.init_params.lock().unwrap()[0].clone();
    assert_eq!(
        params["capabilities"]["workspace"]["configuration"],
        json!(true)
    );
    assert_eq!(
        params["initializationOptions"]["pythonPath"],
        json!(format!("{root_str}/.venv/bin/python"))
    );
}
