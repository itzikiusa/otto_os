use super::*;
use std::sync::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

async fn command<S: AsyncRead + Unpin>(read: &mut BufReader<S>) -> io::Result<Option<Vec<String>>> {
    let mut line = String::new();
    if read.read_line(&mut line).await? == 0 {
        return Ok(None);
    }
    let count: usize = line.trim().strip_prefix('*').unwrap().parse().unwrap();
    let mut out = Vec::new();
    for _ in 0..count {
        line.clear();
        read.read_line(&mut line).await?;
        let size: usize = line.trim().strip_prefix('$').unwrap().parse().unwrap();
        let mut value = vec![0; size + 2];
        read.read_exact(&mut value).await?;
        out.push(String::from_utf8(value[..size].to_vec()).unwrap());
    }
    Ok(Some(out))
}
fn eval_simple_cmd(cmd: &[String], db: &mut String, seen: &Arc<Mutex<Vec<Vec<String>>>>) -> String {
    match cmd[0].as_str() {
        "SCAN" => {
            let scan_count = seen
                .lock()
                .unwrap()
                .iter()
                .filter(|v| v[0] == "SCAN")
                .count();
            let key = if scan_count == 1 {
                "oversize-type"
            } else {
                "healthy-type"
            };
            format!("*2\r\n$1\r\n0\r\n*1\r\n${}\r\n{key}\r\n", key.len())
        }
        "TYPE" if cmd[1] == "oversize-type" => "$1000000000\r\n".to_string(),
        "TYPE" if cmd[1] == "oversize-hash" => "+hash\r\n".to_string(),
        "TYPE" if cmd[1] == "stream-key" => "+stream\r\n".to_string(),
        "TYPE" if cmd[1] == "json-key" => "+ReJSON-RL\r\n".to_string(),
        "TYPE" if cmd[1] == "ts-key" => "+TSDB-TYPE\r\n".to_string(),
        "TYPE" => "+list\r\n".to_string(),
        "TTL" | "LLEN" | "HLEN" => ":1\r\n".to_string(),
        "XLEN" => ":2\r\n".to_string(),
        "INCR" => ":11\r\n".to_string(),
        "OBJECT" => "+listpack\r\n".to_string(),
        "LRANGE" if cmd[1] == "oversize-preview" => "*1\r\n$1000000000\r\n".to_string(),
        "LRANGE" => "*1\r\n$3\r\nabc\r\n".to_string(),
        "XRANGE" => "*1\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$2\r\nf1\r\n$2\r\nv1\r\n".to_string(),
        "JSON.GET" => "$13\r\n{\"ok\":\"json\"}\r\n".to_string(),
        "TS.GET" => "*2\r\n:1700000000\r\n$4\r\n42.5\r\n".to_string(),
        "HSCAN" => "*2\r\n:0\r\n*2\r\n+field\r\n$1000000000\r\n".to_string(),
        "SELECT" => {
            *db = cmd[1].clone();
            "+OK\r\n".to_string()
        }
        "BLPOP" => "*2\r\n$4\r\njobs\r\n$5\r\njob-1\r\n".to_string(),
        "GET" if cmd[1] == "large" => "$1000000000\r\n".to_string(),
        "GET" if cmd[1] == "nested" => "*1000000000\r\n".to_string(),
        "SET" if cmd[1] == "large-reply" => "$1000000000\r\n".to_string(),
        "GET" => format!("${}\r\n{}\r\n", db.len(), db),
        "PING" => "+PONG\r\n".to_string(),
        "ECHO" => format!("${}\r\n{}\r\n", cmd[1].len(), cmd[1]),
        "FAIL" => "-ERR fixture rejected command\r\n".to_string(),
        _ => "+OK\r\n".to_string(),
    }
}

async fn peer<S: AsyncRead + AsyncWrite + Unpin>(stream: S, seen: Arc<Mutex<Vec<Vec<String>>>>) {
    let (read, mut write) = tokio::io::split(stream);
    let mut read = BufReader::new(read);
    let mut db = "0".to_string();
    let mut queued: Option<Vec<Vec<String>>> = None;
    while let Ok(Some(cmd)) = command(&mut read).await {
        seen.lock().unwrap().push(cmd.clone());
        let response = match cmd[0].as_str() {
            "MULTI" => {
                queued = Some(Vec::new());
                "+OK\r\n".to_string()
            }
            "DISCARD" => {
                queued = None;
                "+OK\r\n".to_string()
            }
            "EXEC" => {
                if let Some(cmds) = queued.take() {
                    let mut out = format!("*{}\r\n", cmds.len());
                    for qcmd in &cmds {
                        out.push_str(&eval_simple_cmd(qcmd, &mut db, &seen));
                    }
                    out
                } else {
                    "-ERR EXEC without MULTI\r\n".to_string()
                }
            }
            _ if queued.is_some() => {
                queued.as_mut().unwrap().push(cmd);
                "+QUEUED\r\n".to_string()
            }
            "BLPOP" if cmd.get(1).map(String::as_str) == Some("slow-jobs") => {
                tokio::time::sleep(Duration::from_secs(35)).await;
                "*2\r\n$9\r\nslow-jobs\r\n$8\r\nafter-35\r\n".to_string()
            }
            "GET" if cmd[1] == "delay" => {
                tokio::time::sleep(Duration::from_millis(80)).await;
                "+ready\r\n".to_string()
            }
            "GET" if cmd[1] == "slow" => {
                let mut eof = [0];
                let _ = read.read(&mut eof).await;
                return;
            }
            _ => eval_simple_cmd(&cmd, &mut db, &seen),
        };
        // Force actual headers/body/footer fragmentation across writes.
        for chunk in response.as_bytes().chunks(3) {
            if write.write_all(chunk).await.is_err() {
                return;
            }
            tokio::task::yield_now().await;
        }
    }
}
fn limits() -> Limits {
    Limits {
        bytes: 128,
        nodes: 16,
        depth: 4,
    }
}
async fn duplex() -> (
    Connection,
    Arc<Mutex<Vec<Vec<String>>>>,
    tokio::task::JoinHandle<()>,
) {
    let (client, server) = tokio::io::duplex(32);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let task = tokio::spawn(peer(server, seen.clone()));
    let info = redis::RedisConnectionInfo::default()
        .set_username("fixture")
        .set_password("fixture-secret")
        .set_db(3);
    let conn = Connection::from_stream(client, &info, limits())
        .await
        .unwrap();
    (conn, seen, task)
}
#[tokio::test]
async fn auth_select_fragmented_pipeline_and_server_errors_keep_the_cache_alive() {
    let (mut conn, seen, task) = duplex().await;
    let values: Vec<String> = redis::pipe()
        .cmd("PING")
        .cmd("ECHO")
        .arg("hello")
        .cmd("GET")
        .arg("db")
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(values, ["PONG", "hello", "3"]);
    assert!(redis::cmd("FAIL")
        .query_async::<String>(&mut conn)
        .await
        .is_err());
    assert!(conn.is_alive());
    let seen = seen.lock().unwrap().clone();
    assert!(seen
        .iter()
        .any(|v| v == &["AUTH", "fixture", "fixture-secret"]));
    assert!(seen.iter().any(|v| v == &["SELECT", "3"]));
    drop(conn);
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn oversized_declarations_retire_without_payload_or_write_retry() {
    for key in ["large", "nested"] {
        let (mut conn, seen, task) = duplex().await;
        assert!(redis::cmd("GET")
            .arg(key)
            .query_async::<Value>(&mut conn)
            .await
            .is_err());
        assert!(!conn.is_alive());
        assert!(redis::cmd("SET")
            .arg("key")
            .arg("value")
            .query_async::<Value>(&mut conn)
            .await
            .is_err());
        assert_eq!(
            seen.lock()
                .unwrap()
                .iter()
                .filter(|v| v[0] == "GET")
                .count(),
            1
        );
        assert!(!seen.lock().unwrap().iter().any(|v| v[0] == "SET"));
        drop(conn);
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
    }
}
#[tokio::test]
async fn cancelling_inflight_request_retires_clones_and_stops_driver() {
    let (mut conn, _, task) = duplex().await;
    let clone = conn.clone();
    let outcome = tokio::time::timeout(
        Duration::from_millis(20),
        redis::cmd("GET")
            .arg("slow")
            .query_async::<Value>(&mut conn),
    )
    .await;
    assert!(outcome.is_err());
    assert!(!clone.is_alive());
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
}

fn cfg(port: u16) -> ResolvedConfig {
    ResolvedConfig {
        lifecycle: None,
        engine: types::Engine::Redis,
        host: "127.0.0.1".into(),
        port,
        user: Some("fixture".into()),
        password: Some("fixture-secret".into()),
        database: Some("1".into()),
        tls: Default::default(),
        params: serde_json::json!({}),
    }
}
#[tokio::test]
async fn cached_metadata_and_console_share_bounds_and_reconnect_without_database_cross_talk() {
    use crate::driver::Driver;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cfg = cfg(listener.local_addr().unwrap().port());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let peer_seen = seen.clone();
    let task = tokio::spawn(async move {
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..3 {
            let (socket, _) = listener.accept().await.unwrap();
            tasks.spawn(peer(socket, peer_seen.clone()));
        }
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
    });
    let driver = super::super::RedisDriver::default();
    let mut one = driver.connect(&cfg, 1).await.unwrap();
    let mut same = driver.connect(&cfg, 1).await.unwrap();
    assert!(Arc::ptr_eq(&one.owner, &same.owner));
    let mut two = driver.connect(&cfg, 2).await.unwrap();
    let a: String = redis::cmd("GET")
        .arg("db")
        .query_async(&mut one)
        .await
        .unwrap();
    let b: String = redis::cmd("GET")
        .arg("db")
        .query_async(&mut two)
        .await
        .unwrap();
    assert_eq!((a.as_str(), b.as_str()), ("1", "2"));
    // Metadata's pipelined LRANGE/HSCAN replies use this exact ConnectionLike
    // path. One oversized response aborts the pipeline before allocation.
    assert!(redis::pipe()
        .cmd("PING")
        .cmd("GET")
        .arg("large")
        .query_async::<Vec<Value>>(&mut same)
        .await
        .is_err());
    assert!(!one.is_alive());
    let mut fresh = driver.connect(&cfg, 1).await.unwrap();
    assert!(!Arc::ptr_eq(&one.owner, &fresh.owner));
    let a: String = redis::cmd("GET")
        .arg("db")
        .query_async(&mut fresh)
        .await
        .unwrap();
    assert_eq!(a, "1");
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(|v| v.first().map(String::as_str) == Some("AUTH"))
            .count(),
        3
    );
    drop((one, same, two, fresh));
    driver.close(&cfg.cache_key()).await;
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn tls_custom_ca_client_auth_tunnel_sni_and_selected_database_are_preserved() {
    let server = rcgen::generate_simple_self_signed(vec!["redis.fixture".into()]).unwrap();
    let client = rcgen::generate_simple_self_signed(vec!["client.fixture".into()]).unwrap();
    let mut trust = rustls::RootCertStore::empty();
    trust.add(client.cert.der().clone()).unwrap();
    let verify_client = rustls::server::WebPkiClientVerifier::builder_with_provider(
        Arc::new(trust),
        Arc::new(rustls::crypto::ring::default_provider()),
    )
    .build()
    .unwrap();
    let server_config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_client_cert_verifier(verify_client)
    .with_single_cert(
        vec![server.cert.der().clone()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(server.signing_key.serialize_der()).into(),
    )
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut cfg = cfg(listener.local_addr().unwrap().port());
    cfg.tls.mode = types::TlsMode::Required;
    cfg.tls.verify = true;
    cfg.tls.ca_cert = Some(server.cert.pem());
    cfg.tls.client_cert = Some(client.cert.pem());
    cfg.tls.client_key = Some(client.signing_key.serialize_pem());
    cfg.params = serde_json::json!({"__tunnel_host":"redis.fixture"});
    let seen = Arc::new(Mutex::new(Vec::new()));
    let peer_seen = seen.clone();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let tls = acceptor.accept(socket).await.unwrap();
        assert_eq!(tls.get_ref().1.server_name(), Some("redis.fixture"));
        assert_eq!(tls.get_ref().1.peer_certificates().unwrap().len(), 1);
        peer(tls, peer_seen).await;
    });
    let driver = super::super::RedisDriver::default();
    let mut conn = driver.connect(&cfg, 7).await.unwrap();
    let db: String = redis::cmd("GET")
        .arg("db")
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(db, "7");
    drop(conn);
    drop(driver);
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn aggregate_pipeline_budget_rejects_individually_small_responses() {
    for nodes in [false, true] {
        let (mut conn, _, task) = duplex().await;
        let mut pipeline = redis::pipe();
        if nodes {
            for _ in 0..17 {
                pipeline.cmd("PING");
            }
        } else {
            for _ in 0..3 {
                pipeline.cmd("ECHO").arg("x".repeat(50));
            }
        }
        assert!(pipeline.query_async::<Vec<Value>>(&mut conn).await.is_err());
        assert!(!conn.is_alive());
        drop(conn);
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
    }
}
#[tokio::test]
async fn cancelling_a_queued_request_keeps_the_active_request_and_cache_alive() {
    let (mut active, seen, task) = duplex().await;
    let mut queued = active.clone();
    let first = tokio::spawn(async move {
        redis::cmd("GET")
            .arg("delay")
            .query_async::<String>(&mut active)
            .await
            .unwrap()
    });
    while !seen.lock().unwrap().iter().any(|v| v == &["GET", "delay"]) {
        tokio::task::yield_now().await;
    }
    assert!(tokio::time::timeout(
        Duration::from_millis(10),
        redis::cmd("PING").query_async::<String>(&mut queued)
    )
    .await
    .is_err());
    assert!(queued.is_alive());
    assert_eq!(first.await.unwrap(), "ready");
    // More total bytes than one operation's budget remain safe across requests.
    for _ in 0..30 {
        assert_eq!(
            redis::cmd("PING")
                .query_async::<String>(&mut queued)
                .await
                .unwrap(),
            "PONG"
        );
    }
    drop(queued);
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn failed_write_response_is_never_replayed() {
    let (mut conn, seen, task) = duplex().await;
    assert!(redis::cmd("SET")
        .arg("large-reply")
        .arg("already-applied")
        .query_async::<String>(&mut conn)
        .await
        .is_err());
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(|v| v[0] == "SET")
            .count(),
        1
    );
    assert!(!conn.is_alive());
    drop(conn);
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn metadata_list_and_detail_report_budget_failures_then_reconnect() {
    use crate::driver::Driver;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cfg = cfg(listener.local_addr().unwrap().port());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let peer_seen = seen.clone();
    let task = tokio::spawn(async move {
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..4 {
            let (socket, _) = listener.accept().await.unwrap();
            tasks.spawn(peer(socket, peer_seen.clone()));
        }
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
    });
    let driver = super::super::RedisDriver::default();
    let scope = types::NodePath::parse("kdb:1");
    let error = driver
        .schema_children(&cfg, &scope, Some("fixture"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("receive budget"), "{error}");
    let nodes = driver
        .schema_children(&cfg, &scope, Some("fixture"))
        .await
        .unwrap();
    assert!(nodes.iter().any(|n| n.label == "healthy-type"));
    for key in ["oversize-preview", "oversize-hash"] {
        let path = types::NodePath::parse(&format!("kdb:1/key:{key}"));
        let error = driver.object_detail(&cfg, &path).await.unwrap_err();
        assert!(error.to_string().contains("receive budget"), "{error}");
    }
    let detail = driver
        .object_detail(&cfg, &types::NodePath::parse("kdb:1/key:healthy"))
        .await
        .unwrap();
    assert_eq!(detail.extra["preview"], serde_json::json!(["abc"]));
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(|v| v[0] == "AUTH")
            .count(),
        4
    );
    drop(driver);
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn tls_verification_rejects_wrong_ca_and_hostname() {
    let server = rcgen::generate_simple_self_signed(vec!["redis.fixture".into()]).unwrap();
    let other = rcgen::generate_simple_self_signed(vec!["unrelated.fixture".into()]).unwrap();
    let server_config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![server.cert.der().clone()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(server.signing_key.serialize_der()).into(),
    )
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        for _ in 0..2 {
            let (socket, _) = listener.accept().await.unwrap();
            assert!(acceptor.accept(socket).await.is_err());
        }
    });
    for wrong_ca in [false, true] {
        let mut cfg = cfg(port);
        cfg.tls.mode = types::TlsMode::Required;
        cfg.tls.verify = true;
        cfg.tls.ca_cert = Some(if wrong_ca {
            other.cert.pem()
        } else {
            server.cert.pem()
        });
        cfg.tls.server_name = Some(
            if wrong_ca {
                "redis.fixture"
            } else {
                "wrong.fixture"
            }
            .into(),
        );
        let driver = super::super::RedisDriver::default();
        assert!(driver.connect(&cfg, 1).await.is_err());
    }
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn stream_and_module_object_detail_plus_batch_run_with_more_results_and_error_entry() {
    use crate::driver::Driver;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cfg = cfg(listener.local_addr().unwrap().port());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let peer_seen = seen.clone();
    let task = tokio::spawn(async move {
        let mut handles = Vec::new();
        // 1 cached connection for object_detail + 3 operation-isolated connections for run()
        for _ in 0..4 {
            let (socket, _) = listener.accept().await.unwrap();
            let s = peer_seen.clone();
            handles.push(tokio::spawn(peer(socket, s)));
        }
        for h in handles {
            let _ = h.await;
        }
    });
    let driver = super::super::RedisDriver::default();

    // L13: stream, ReJSON-RL, and TSDB-TYPE object_detail previews.
    let stream_detail = driver
        .object_detail(&cfg, &types::NodePath::parse("kdb:1/key:stream-key"))
        .await
        .unwrap();
    assert_eq!(stream_detail.extra["type"], serde_json::json!("stream"));
    assert_eq!(stream_detail.extra["length"], serde_json::json!(2));
    assert_eq!(
        stream_detail.extra["preview"],
        serde_json::json!([["1-0", ["f1", "v1"]]])
    );

    let json_detail = driver
        .object_detail(&cfg, &types::NodePath::parse("kdb:1/key:json-key"))
        .await
        .unwrap();
    assert_eq!(json_detail.extra["type"], serde_json::json!("ReJSON-RL"));
    assert_eq!(
        json_detail.extra["preview"],
        serde_json::json!("{\"ok\":\"json\"}")
    );

    let ts_detail = driver
        .object_detail(&cfg, &types::NodePath::parse("kdb:1/key:ts-key"))
        .await
        .unwrap();
    assert_eq!(ts_detail.extra["type"], serde_json::json!("TSDB-TYPE"));
    assert_eq!(
        ts_detail.extra["preview"],
        serde_json::json!([1700000000, "42.5"])
    );

    // L14: multi-command batch with single-quoted JSON, inline comment, and mid-batch failure.
    let req = types::QueryRequest {
        statement: "PING # health check\nECHO '{\"a\": \"b c\"}'\nFAIL\nPING".into(),
        node: Some("kdb:1".into()),
        ..Default::default()
    };
    let batch = driver.run(&cfg, &req).await.unwrap();
    assert_eq!(batch.rows, vec![vec![serde_json::json!("PONG")]]);
    assert_eq!(batch.statement.as_deref(), Some("PING # health check"));
    assert_eq!(batch.more_results.len(), 2);
    assert_eq!(
        batch.more_results[0].rows,
        vec![vec![serde_json::json!("{\"a\": \"b c\"}")]]
    );
    assert!(batch.more_results[1].errored);
    assert_eq!(batch.more_results[1].statement.as_deref(), Some("FAIL"));
    assert!(batch.more_results[1]
        .message
        .as_deref()
        .unwrap_or_default()
        .contains("fixture rejected command"));

    // L16: SELECT, MULTI/EXEC transaction, and BLPOP with >30s timeout on an operation-isolated connection.
    let tx_req = types::QueryRequest {
        statement: "SELECT 5\nMULTI\nINCR counter\nGET db\nEXEC\nBLPOP jobs 1".into(),
        node: Some("kdb:1".into()),
        timeout_ms: Some(45_000),
        ..Default::default()
    };
    let tx_batch = driver.run(&cfg, &tx_req).await.unwrap();
    assert_eq!(tx_batch.rows, vec![vec![serde_json::json!("OK")]]); // SELECT 5
    assert_eq!(tx_batch.more_results.len(), 5);
    assert_eq!(
        tx_batch.more_results[0].rows,
        vec![vec![serde_json::json!("OK")]]
    ); // MULTI
    assert_eq!(
        tx_batch.more_results[1].rows,
        vec![vec![serde_json::json!("QUEUED")]]
    ); // INCR counter (queued)
    assert_eq!(
        tx_batch.more_results[2].rows,
        vec![vec![serde_json::json!("QUEUED")]]
    ); // GET db (queued)
    assert_eq!(
        tx_batch.more_results[3].rows,
        vec![vec![serde_json::json!(11)], vec![serde_json::json!("5")]]
    ); // EXEC -> [11, "5"]
    assert_eq!(
        tx_batch.more_results[4].rows,
        vec![
            vec![serde_json::json!("jobs")],
            vec![serde_json::json!("job-1")]
        ]
    ); // BLPOP jobs 1

    // L16: Subsequent run() starts on a fresh connection pinned to kdb:1 (SELECT 5 did not leak).
    let check_db = types::QueryRequest {
        statement: "GET db".into(),
        node: Some("kdb:1".into()),
        ..Default::default()
    };
    let db_res = driver.run(&cfg, &check_db).await.unwrap();
    assert_eq!(db_res.rows, vec![vec![serde_json::json!("1")]]);

    drop(driver);
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn configurable_response_timeout_governs_delayed_replies() {
    let info = redis::RedisConnectionInfo::default()
        .set_username("fixture")
        .set_password("fixture-secret")
        .set_db(1);

    // 1. A short transport response_timeout (25ms) aborts an 80ms `GET delay` reply and retires the connection.
    let (client, server) = tokio::io::duplex(64);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let short_task = tokio::spawn(peer(server, seen));
    let mut short_conn = Connection::from_stream_with_timeout(
        client,
        &info,
        limits(),
        Some(Duration::from_millis(25)),
    )
    .await
    .unwrap();
    let err = redis::cmd("GET")
        .arg("delay")
        .query_async::<String>(&mut short_conn)
        .await
        .unwrap_err();
    assert!(err.is_timeout(), "expected transport timeout: {err}");
    assert!(!short_conn.is_alive());
    drop(short_conn);
    let _ = short_task.await;

    // 2. An extended transport response_timeout (500ms) allows the 80ms `GET delay` reply to succeed.
    let (client, server) = tokio::io::duplex(64);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let ext_task = tokio::spawn(peer(server, seen));
    let mut ext_conn = Connection::from_stream_with_timeout(
        client,
        &info,
        limits(),
        Some(Duration::from_millis(500)),
    )
    .await
    .unwrap();
    let val: String = redis::cmd("GET")
        .arg("delay")
        .query_async(&mut ext_conn)
        .await
        .expect("500ms transport response_timeout should allow 80ms reply");
    assert_eq!(val, "ready");
    assert!(ext_conn.is_alive());
    drop(ext_conn);
    let _ = ext_task.await;
}
