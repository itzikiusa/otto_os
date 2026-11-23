//! Unit tests for `LspPool`: multiplexing, owner isolation, `MAX_SERVERS_PER_OWNER = 3`,
//! `1.2 GiB` RSS budget eviction with `active_clients == 0` priority, lightweight
//! format rejection, module-root scoping, and per-language heap caps.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::BufReader;

use super::framing;
use super::pool::{
    Attached, LspPool, PoolKey, RssProbe, ServerIo, SpawnLaunch, Spawner, DATA_CAPACITY,
    IDLE_TIMEOUT, LSP_RSS_BUDGET_BYTES, MAX_GLOBAL_SERVERS, MAX_SERVERS_PER_OWNER,
    WRITE_QUEUE_MAX_BYTES,
};
use super::pool_spawn::{apply_language_memory_caps, build_remote_lsp_script, JDTLS_JVM_FLAGS};
use crate::types::IdeSshSpec;

struct Fake {
    spawns: Arc<AtomicUsize>,
    inits: Arc<AtomicUsize>,
    doc_log: Arc<Mutex<Vec<String>>>,
}

fn fake_pool_with_rss(idle: Duration, rss_probe: Option<RssProbe>) -> (Arc<LspPool>, Fake) {
    let spawns = Arc::new(AtomicUsize::new(0));
    let inits = Arc::new(AtomicUsize::new(0));
    let doc_log = Arc::new(Mutex::new(Vec::new()));
    let (s, i, d) = (spawns.clone(), inits.clone(), doc_log.clone());
    let spawner: Spawner = Arc::new(move |_key: &PoolKey, _launch: &SpawnLaunch| {
        s.fetch_add(1, Ordering::SeqCst);
        let (pool_side_w, server_r) = tokio::io::duplex(1 << 16);
        let (server_w, pool_side_r) = tokio::io::duplex(1 << 16);
        let (inits, doc_log) = (i.clone(), d.clone());
        tokio::spawn(async move {
            let mut r = BufReader::new(server_r);
            let mut w = server_w;
            while let Ok(Some(body)) = framing::read_message(&mut r).await {
                let msg: Value = serde_json::from_slice(&body).unwrap();
                let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
                let reply = match method {
                    "initialize" => {
                        inits.fetch_add(1, Ordering::SeqCst);
                        Some(json!({
                            "jsonrpc":"2.0",
                            "id":msg["id"],
                            "result":{"capabilities":{"hoverProvider":true}}
                        }))
                    }
                    "textDocument/hover" => {
                        Some(json!({"jsonrpc":"2.0","id":msg["id"],"result":{"contents":"hi"}}))
                    }
                    "textDocument/didOpen" | "textDocument/didClose" => {
                        let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap();
                        doc_log.lock().unwrap().push(format!("{method} {uri}"));
                        (method == "textDocument/didOpen").then(|| {
                            json!({
                                "jsonrpc":"2.0","method":"textDocument/publishDiagnostics",
                                "params":{"uri":uri,"diagnostics":[]}
                            })
                        })
                    }
                    "workspace/didChangeWatchedFiles" => {
                        let uri = msg["params"]["changes"][0]["uri"]
                            .as_str()
                            .unwrap_or_default();
                        doc_log.lock().unwrap().push(format!("watched {uri}"));
                        None
                    }
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
        LspPool::new_inner(spawner, idle, WRITE_QUEUE_MAX_BYTES, None, rss_probe),
        Fake {
            spawns,
            inits,
            doc_log,
        },
    )
}

fn fake_pool(idle: Duration) -> (Arc<LspPool>, Fake) {
    fake_pool_with_rss(idle, None)
}

fn key(root: &str) -> PoolKey {
    key_for("u1", root)
}

fn key_for(owner: &str, root: &str) -> PoolKey {
    PoolKey {
        owner: owner.into(),
        lang: "typescript".into(),
        root: root.into(),
        ssh: None,
        go_build_tags: Vec::new(),
    }
}

async fn recv(a: &mut Attached) -> Value {
    let text = tokio::time::timeout(Duration::from_secs(2), a.rx.recv())
        .await
        .expect("timed out waiting for a server message")
        .expect("socket closed");
    serde_json::from_str(&text).unwrap()
}

async fn nothing(a: &mut Attached) {
    let got = tokio::time::timeout(Duration::from_millis(100), a.rx.recv()).await;
    assert!(got.is_err(), "unexpected message: {got:?}");
}

async fn init(a: &mut Attached, id: i64) {
    let req =
        json!({"jsonrpc":"2.0","id":id,"method":"initialize","params":{"rootUri":"file:///r"}});
    assert!(a.send(req.to_string()).await);
    let resp = recv(a).await;
    assert_eq!(resp["id"], json!(id));
    assert_eq!(resp["result"]["capabilities"]["hoverProvider"], json!(true));
    a.send(json!({"jsonrpc":"2.0","method":"initialized","params":{}}).to_string())
        .await;
}

fn open(uri: &str) -> String {
    json!({"jsonrpc":"2.0","method":"textDocument/didOpen",
        "params":{"textDocument":{"uri":uri,"languageId":"typescript","version":1,"text":"x"}}})
    .to_string()
}

fn close(uri: &str) -> String {
    json!({"jsonrpc":"2.0","method":"textDocument/didClose",
        "params":{"textDocument":{"uri":uri}}})
    .to_string()
}

#[tokio::test]
async fn two_sockets_for_the_same_lang_and_root_share_one_server() {
    let (pool, fake) = fake_pool(IDLE_TIMEOUT);
    let mut a = pool.attach(key("/r"), "tsls", &[]).unwrap();
    let mut b = pool.attach(key("/r"), "tsls", &[]).unwrap();
    assert_eq!(fake.spawns.load(Ordering::SeqCst), 1);
    assert_eq!(pool.live_servers(), 1);
    assert_eq!(
        pool.status_snapshot(None, None, None).servers[0].active_clients,
        2
    );

    init(&mut a, 1).await;
    init(&mut b, 1).await;
    assert_eq!(fake.inits.load(Ordering::SeqCst), 1);

    let hover = |id: i64| {
        json!({"jsonrpc":"2.0","id":id,"method":"textDocument/hover","params":{}}).to_string()
    };
    assert!(b.send(hover(7)).await);
    assert!(a.send(hover(7)).await);
    let rb = recv(&mut b).await;
    let ra = recv(&mut a).await;
    assert_eq!((rb["id"].clone(), ra["id"].clone()), (json!(7), json!(7)));
    assert_eq!(rb["result"]["contents"], json!("hi"));
    nothing(&mut a).await;
    nothing(&mut b).await;

    let _c = pool.attach(key("/other"), "tsls", &[]).unwrap();
    assert_eq!(fake.spawns.load(Ordering::SeqCst), 2);
    assert_eq!(pool.live_servers(), 2);
}

#[tokio::test]
async fn local_and_ssh_roots_never_share_a_pooled_server() {
    let (pool, fake) = fake_pool(IDLE_TIMEOUT);
    let _local = pool.attach(key("/srv/app"), "gopls", &[]).unwrap();
    let mut remote_key = key("/srv/app");
    remote_key.ssh = Some(IdeSshSpec {
        connection_id: "conn_1".into(),
        name: "box".into(),
        host: "127.0.0.1".into(),
        port: Some(2222),
        user: "dev".into(),
        identity_file: None,
        proxy_jump: None,
        environment: None,
    });
    let _remote = pool.attach(remote_key, "gopls", &[]).unwrap();
    assert_eq!(fake.spawns.load(Ordering::SeqCst), 2);
    assert_eq!(pool.live_servers(), 2);
}

#[test]
fn remote_script_escapes_paths_and_applies_memory_budgets() {
    let launch = SpawnLaunch {
        cmd: "pyright-langserver".into(),
        args: vec!["--stdio".into()],
        fallback_cmd: Some("pylsp".into()),
        fallback_args: vec![],
    };
    let script = build_remote_lsp_script("/srv/my 'app'", &launch);
    assert!(script.contains("cd '/srv/my '\\''app'\\'''"));
    assert!(script.contains("GOMEMLIMIT=\"${GOMEMLIMIT:-512MiB}\""));
    assert!(script.contains("--max-old-space-size=384"));
    assert!(script.contains("exec 'pyright-langserver' '--stdio'"));
    assert!(
        script.contains("mkfifo -m 600") && script.contains("kill -TERM \"$pid\""),
        "remote LSP script must wrap stdio with EOF-kill watcher: {script}"
    );
}

#[tokio::test]
async fn shared_documents_are_ref_counted_and_diagnostics_routed() {
    let (pool, fake) = fake_pool(IDLE_TIMEOUT);
    let mut a = pool.attach(key("/r"), "tsls", &[]).unwrap();
    let mut b = pool.attach(key("/r"), "tsls", &[]).unwrap();
    init(&mut a, 1).await;
    init(&mut b, 1).await;

    a.send(open("file:///r/x.ts")).await;
    let diag = recv(&mut a).await;
    assert_eq!(diag["params"]["uri"], json!("file:///r/x.ts"));
    nothing(&mut b).await;

    b.send(open("file:///r/x.ts")).await;
    let _ = recv(&mut a).await;
    let _ = recv(&mut b).await;
    a.send(close("file:///r/x.ts")).await;
    tokio::task::yield_now().await;
    drop(b);
    let expected = vec![
        "textDocument/didOpen file:///r/x.ts".to_string(),
        "textDocument/didClose file:///r/x.ts".to_string(),
        "textDocument/didOpen file:///r/x.ts".to_string(),
        "textDocument/didClose file:///r/x.ts".to_string(),
    ];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let log = fake.doc_log.lock().unwrap().clone();
        if log.len() >= expected.len() || tokio::time::Instant::now() >= deadline {
            assert_eq!(log, expected);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn idle_server_is_reaped_and_a_quick_reattach_reuses_it() {
    let (pool, fake) = fake_pool(Duration::from_millis(80));
    let mut a = pool.attach(key("/r"), "tsls", &[]).unwrap();
    init(&mut a, 1).await;
    drop(a);
    tokio::time::sleep(Duration::from_millis(15)).await;
    let mut b = pool.attach(key("/r"), "tsls", &[]).unwrap();
    init(&mut b, 3).await;
    assert_eq!(fake.spawns.load(Ordering::SeqCst), 1);
    assert_eq!(fake.inits.load(Ordering::SeqCst), 1);
    drop(b);
    for _ in 0..40 {
        if pool.live_servers() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(pool.live_servers(), 0);
    let mut c = pool.attach(key("/r"), "tsls", &[]).unwrap();
    init(&mut c, 1).await;
    assert_eq!(fake.spawns.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_stalled_server_backpressures_client_sends() {
    let spawner: Spawner = Arc::new(|_key: &PoolKey, _launch: &SpawnLaunch| {
        let (pool_side_w, server_r) = tokio::io::duplex(4 * 1024);
        let (_server_w, pool_side_r) = tokio::io::duplex(1024);
        tokio::spawn(async move {
            let _hold = (server_r, _server_w);
            std::future::pending::<()>().await;
        });
        Ok(ServerIo {
            reader: Box::new(BufReader::new(pool_side_r)),
            writer: Box::new(pool_side_w),
            child: None,
        })
    });
    let pool = LspPool::with_write_cap(spawner, IDLE_TIMEOUT, 16 * 1024);
    let a = pool.attach(key("/r"), "tsls", &[]).unwrap();
    let body = "x".repeat(1024);
    let mut accepted = 0usize;
    let mut blocked = false;
    for i in 0..(DATA_CAPACITY * 3) {
        let msg = json!({"jsonrpc":"2.0","method":"textDocument/didChange",
            "params":{"n": i, "text": body}})
        .to_string();
        match tokio::time::timeout(Duration::from_millis(200), a.send(msg)).await {
            Ok(true) => accepted += 1,
            Ok(false) => panic!("server went away"),
            Err(_) => {
                blocked = true;
                break;
            }
        }
    }
    assert!(blocked, "sends never blocked: {accepted} accepted");
    assert!(accepted <= DATA_CAPACITY + 64, "{accepted} accepted");
}

#[tokio::test]
async fn max_three_servers_evicts_inactive_clients_first_and_skips_non_code_langs() {
    assert_eq!(IDLE_TIMEOUT, Duration::from_secs(45));
    assert_eq!(MAX_SERVERS_PER_OWNER, 3);
    assert_eq!(MAX_GLOBAL_SERVERS, 3);
    assert_eq!(LSP_RSS_BUDGET_BYTES, 1_288_490_188);

    let (pool, fake) = fake_pool(IDLE_TIMEOUT);
    let lang_key = |lang: &str, root: &str| PoolKey {
        owner: "u1".into(),
        lang: lang.into(),
        root: root.into(),
        ssh: None,
        go_build_tags: Vec::new(),
    };

    let mut s_rust = pool.attach(lang_key("rust", "/repo-1"), "ra", &[]).unwrap();
    init(&mut s_rust, 1).await;
    let s_go = pool
        .attach(lang_key("go", "/repo-2"), "gopls", &[])
        .unwrap();
    drop(s_go); // inactive (`active_clients == 0`)
    let mut s_java = pool
        .attach(lang_key("java", "/repo-3"), "jdtls", &[])
        .unwrap();
    init(&mut s_java, 1).await;

    assert_eq!(pool.live_servers(), 3);

    // Lightweight data/doc formats fail fast without spawning or evicting any code server.
    for non_lsp in ["markdown", "csv", "sql", "json"] {
        let err = pool
            .attach(lang_key(non_lsp, "/repo-1"), "noop", &[])
            .err()
            .expect("lightweight format must be rejected");
        assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);
    }
    assert_eq!(fake.spawns.load(Ordering::SeqCst), 3);
    assert_eq!(pool.live_servers(), 3);

    // Attaching a 4th code language (`typescript`) evicts the inactive `go` server (`active_clients == 0`).
    let mut s_ts = pool
        .attach(lang_key("typescript", "/repo-4"), "tsls", &[])
        .unwrap();
    init(&mut s_ts, 1).await;
    assert_eq!(pool.live_servers(), 3);

    let status = pool.status_snapshot(None, None, None);
    let langs: Vec<&str> = status.servers.iter().map(|s| s.language.as_str()).collect();
    assert_eq!(langs, vec!["java", "rust", "typescript"]);
    assert!(status.servers.iter().all(|s| s.active_clients == 1));

    // Watched manifest notification reaches `/repo-1` without matching sibling `/repo-1-other`.
    pool.notify_watched_file_if_build_manifest(Path::new("/repo-1-other/Cargo.toml"), 2);
    pool.notify_watched_file_if_build_manifest(Path::new("/repo-1/Cargo.toml"), 2);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let log = fake.doc_log.lock().unwrap().clone();
        if log.iter().any(|l| l == "watched file:///repo-1/Cargo.toml")
            || tokio::time::Instant::now() >= deadline
        {
            assert!(log.iter().any(|l| l == "watched file:///repo-1/Cargo.toml"));
            assert!(!log
                .iter()
                .any(|l| l == "watched file:///repo-1-other/Cargo.toml"));
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    drop(s_rust);
    drop(s_java);
    drop(s_ts);
    pool.evict_detached_other_roots_for_user(
        "u1",
        Path::new("/repo-4"),
        Path::new("/repo-4"),
        &[PathBuf::from("/repo-1")],
    );
    assert_eq!(pool.live_servers(), 2);
}

#[tokio::test]
async fn rss_budget_evicts_inactive_servers_and_never_evicts_active_requested_server() {
    let rss_table: Arc<Mutex<HashMap<String, u64>>> = Arc::new(Mutex::new(HashMap::from([
        ("/go-idle".to_string(), 520 * 1024 * 1024),
        ("/java-active".to_string(), 580 * 1024 * 1024),
        ("/rust-new".to_string(), 420 * 1024 * 1024),
    ])));
    let tbl = Arc::clone(&rss_table);
    let probe: RssProbe =
        Arc::new(move |k: &PoolKey, _pid| tbl.lock().unwrap().get(&k.root).copied().unwrap_or(0));

    let (pool, _fake) = fake_pool_with_rss(IDLE_TIMEOUT, Some(probe));
    let s_idle = pool
        .attach(
            PoolKey {
                owner: "u1".into(),
                lang: "go".into(),
                root: "/go-idle".into(),
                ssh: None,
                go_build_tags: Vec::new(),
            },
            "gopls",
            &[],
        )
        .unwrap();
    drop(s_idle); // `active_clients == 0`

    let _s_java = pool
        .attach(
            PoolKey {
                owner: "u1".into(),
                lang: "java".into(),
                root: "/java-active".into(),
                ssh: None,
                go_build_tags: Vec::new(),
            },
            "jdtls",
            &[],
        )
        .unwrap();

    // Spawning `/rust-new` brings combined RSS to 1,520 MiB (> 1.2 GiB); `/go-idle` (`active_clients == 0`)
    // must be evicted while both `/java-active` and `/rust-new` stay alive under budget.
    let _s_rust = pool
        .attach(
            PoolKey {
                owner: "u1".into(),
                lang: "rust".into(),
                root: "/rust-new".into(),
                ssh: None,
                go_build_tags: Vec::new(),
            },
            "rust-analyzer",
            &[],
        )
        .unwrap();

    let status = pool.status_snapshot(None, None, None);
    assert_eq!(status.active_server_count, 2);
    assert!(status.total_rss_bytes <= LSP_RSS_BUDGET_BYTES);
    let roots: Vec<&str> = status.servers.iter().map(|s| s.root.as_str()).collect();
    assert_eq!(roots, vec!["/java-active", "/rust-new"]);
}

#[test]
fn language_memory_caps_set_go_java_rust_and_node_flags() {
    let empty = BTreeMap::new();
    let mut go_cmd = tokio::process::Command::new("gopls");
    apply_language_memory_caps(&mut go_cmd, "go", "/repo", "gopls", &[], &empty);
    let go_envs: HashMap<_, _> = go_cmd.as_std().get_envs().collect();
    assert_eq!(
        go_envs
            .get(std::ffi::OsStr::new("GOMEMLIMIT"))
            .copied()
            .flatten(),
        Some(std::ffi::OsStr::new("512MiB"))
    );
    assert_eq!(
        go_envs.get(std::ffi::OsStr::new("GOGC")).copied().flatten(),
        Some(std::ffi::OsStr::new("75"))
    );

    let mut java_cmd = tokio::process::Command::new("jdtls");
    apply_language_memory_caps(&mut java_cmd, "java", "/repo-java", "jdtls", &[], &empty);
    let java_args: Vec<String> = java_cmd
        .as_std()
        .get_args()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    for flag in JDTLS_JVM_FLAGS {
        assert!(
            java_args.contains(&format!("--jvm-arg={flag}")),
            "missing {flag}"
        );
    }
    assert!(java_args.contains(&"-data".to_string()));
}

#[test]
fn jdtls_gets_the_newest_lombok_javaagent_from_the_local_m2() {
    let m2 = tempfile::tempdir().unwrap();
    let base = m2.path().join("org/projectlombok/lombok");
    for v in ["1.18.9", "1.18.30", "1.18.32"] {
        std::fs::create_dir_all(base.join(v)).unwrap();
        if v != "1.18.32" {
            // 1.18.32 has only a pom (an incomplete download), so it is skipped.
            std::fs::write(base.join(v).join(format!("lombok-{v}.jar")), b"jar").unwrap();
        }
    }
    let args_with = |env: &BTreeMap<String, String>| -> Vec<String> {
        let mut cmd = tokio::process::Command::new("jdtls");
        apply_language_memory_caps(&mut cmd, "java", "/repo-java", "jdtls", &[], env);
        cmd.as_std()
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect()
    };
    let env = BTreeMap::from([(
        "MAVEN_REPO_LOCAL".to_string(),
        m2.path().to_string_lossy().into_owned(),
    )]);
    let want = format!(
        "--jvm-arg=-javaagent:{}",
        base.join("1.18.30/lombok-1.18.30.jar").display()
    );
    assert!(args_with(&env).contains(&want), "{:?}", args_with(&env));

    let empty_m2 = tempfile::tempdir().unwrap();
    let env = BTreeMap::from([(
        "MAVEN_REPO_LOCAL".to_string(),
        empty_m2.path().to_string_lossy().into_owned(),
    )]);
    assert!(!args_with(&env).iter().any(|a| a.contains("javaagent")));
}

/// The periodic RSS sweep never kills a server an editor is attached to: being
/// over budget is the lesser harm (the count caps still bound the pool).
#[tokio::test]
async fn rss_sweep_never_evicts_servers_with_attached_sockets() {
    let probe: RssProbe = Arc::new(|_k: &PoolKey, _pid| 600 * 1024 * 1024);
    let (pool, _fake) = fake_pool_with_rss(IDLE_TIMEOUT, Some(probe));
    let lang_key = |lang: &str, root: &str| PoolKey {
        owner: "u1".into(),
        lang: lang.into(),
        root: root.into(),
        ssh: None,
        go_build_tags: Vec::new(),
    };
    let _a = pool
        .attach(lang_key("go", "/svc-go"), "gopls", &[])
        .unwrap();
    let _b = pool
        .attach(lang_key("java", "/svc-java"), "jdtls", &[])
        .unwrap();
    let _c = pool
        .attach(lang_key("rust", "/svc-rust"), "rust-analyzer", &[])
        .unwrap();
    pool.enforce_rss_budget(None);
    let status = pool.status_snapshot(None, None, None);
    assert_eq!(status.active_server_count, 3, "{:?}", status.servers);
    assert!(status.total_rss_bytes > LSP_RSS_BUDGET_BYTES);
}

/// When a new server needs a slot and every server is in use, the least
/// recently used one goes — and its socket is told why before it closes.
#[tokio::test]
async fn evicting_an_in_use_server_tells_its_socket_before_closing() {
    let (pool, _fake) = fake_pool(IDLE_TIMEOUT);
    let lang_key = |lang: &str, root: &str| PoolKey {
        owner: "u1".into(),
        lang: lang.into(),
        root: root.into(),
        ssh: None,
        go_build_tags: Vec::new(),
    };
    let mut oldest = pool.attach(lang_key("go", "/m1"), "gopls", &[]).unwrap();
    let _b = pool.attach(lang_key("java", "/m2"), "jdtls", &[]).unwrap();
    let _c = pool
        .attach(lang_key("rust", "/m3"), "rust-analyzer", &[])
        .unwrap();
    let _d = pool
        .attach(lang_key("typescript", "/m4"), "tsls", &[])
        .unwrap();
    assert_eq!(pool.live_servers(), MAX_GLOBAL_SERVERS);

    let notice = recv(&mut oldest).await;
    assert_eq!(notice["method"], json!("otto/lspEvicted"));
    assert_eq!(notice["params"]["reason"], json!("server_cap"));
    assert_eq!(notice["params"]["lang"], json!("go"));
    assert_eq!(notice["params"]["root"], json!("/m1"));
    let closed = tokio::time::timeout(Duration::from_secs(2), oldest.rx.recv())
        .await
        .expect("socket should close after the notice");
    assert!(closed.is_none());
}

#[tokio::test]
async fn a_source_file_write_reaches_the_server_as_a_watched_file_change() {
    let (pool, fake) = fake_pool(IDLE_TIMEOUT);
    let mut a = pool.attach(key("/repo-w"), "tsls", &[]).unwrap();
    init(&mut a, 1).await;
    // A closed `.ts`/`.go` file renamed or replaced under the server: it must hear of it.
    pool.notify_watched_file(Path::new("/repo-w/src/money.ts"), 2);
    // The manifest-only entry point still drops source files (deps actions use it).
    pool.notify_watched_file_if_build_manifest(Path::new("/repo-w/src/other.ts"), 2);
    let want = "watched file:///repo-w/src/money.ts";
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while !fake.doc_log.lock().unwrap().iter().any(|l| l == want) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no didChangeWatchedFiles for money.ts"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let log = fake.doc_log.lock().unwrap().clone();
    assert!(!log.iter().any(|l| l.ends_with("/src/other.ts")));
}
