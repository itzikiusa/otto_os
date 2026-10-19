use super::*;
use serde_json::json;

#[test]
fn ipv6_endpoints_are_bracketed_once() {
    assert_eq!(host_port("::1", 8123), "[::1]:8123");
    assert_eq!(host_port("[::1]", 9000), "[::1]:9000");
    assert_eq!(host_port("db.example", 8123), "db.example:8123");
}

#[test]
fn streamed_clickhouse_cells_keep_truncation_provenance() {
    let mut stream = CompactStream::new(Some(1));
    stream.phase = StreamPhase::Data;
    stream.accept_row(vec![
        serde_json::json!({"nested": "x".repeat(types::MAX_CELL_CHARS + 1)}),
    ]);
    assert!(stream.finish().unwrap().cells_truncated);
}

#[test]
fn mid_stream_exception_field_is_an_error_not_partial_rows() {
    let body = r#"{"meta":[{"name":"x","type":"UInt8"}],"data":[[1],[2],[3]],"rows":3,"exception":"Code: 241. DB::Exception: Memory limit (for query) exceeded. (MEMORY_LIMIT_EXCEEDED) (version 24.8.4.13 (official build))"}"#;
    let err = decode_json_reply(body).unwrap_err();
    assert_eq!(
            err,
            "Code: 241. DB::Exception: Memory limit (for query) exceeded. (MEMORY_LIMIT_EXCEEDED)\nSTREAMED_ROWS: 3"
        );
}

#[test]
fn mid_stream_broken_json_surfaces_the_server_error() {
    let body = "{\"meta\":[{\"name\":\"x\",\"type\":\"UInt8\"}],\"data\":[[1],[2]\nCode: 159. DB::Exception: Timeout exceeded: elapsed 30.0001 seconds, maximum: 30. (TIMEOUT_EXCEEDED) (version 24.8.4.13 (official build))\n";
    let err = decode_json_reply(body).unwrap_err();
    assert!(
        err.starts_with("Code: 159. DB::Exception: Timeout exceeded"),
        "{err}"
    );
    assert!(
        err.ends_with("(TIMEOUT_EXCEEDED)\nSTREAMED_ROWS: unknown"),
        "{err}"
    );
    assert!(!err.contains("trailing characters"));
    // A plain broken body with no server error keeps serde's text.
    assert!(decode_json_reply("{\"data\":[[1]").is_err());
    // A clean reply still decodes.
    let ok = decode_json_reply(r#"{"meta":[],"data":[[1]]}"#).unwrap();
    assert_eq!(ok.data.len(), 1);
}

#[test]
fn capabilities_are_honest() {
    let c = ClickhouseDriver::default().capabilities();
    assert_eq!(c.engine, Engine::Clickhouse);
    assert!(c.sql && c.joins && c.multi_statement);
    assert!(!c.transactions);
    assert!(c.cancel, "KILL QUERY WHERE query_id supported (HTTP)");
    assert!(c.explain);
}

fn base_cfg(port: u16) -> ResolvedConfig {
    ResolvedConfig {
        lifecycle: None,
        engine: Engine::Clickhouse,
        host: "127.0.0.1".into(),
        port,
        user: None,
        password: None,
        database: None,
        tls: types::TlsConfig::default(),
        params: json!({}),
    }
}

/// The MCP path's server-derived flag turns on the native read-only mode;
/// ordinary runs never send it.
#[test]
fn read_only_execution_flag_selects_server_readonly_mode() {
    let plain = ClickhouseDriver::connection_from_client(
        &base_cfg(8123),
        None,
        None,
        None,
        reqwest::Client::new(),
    )
    .unwrap();
    assert!(!plain.readonly);
    let mut cfg = base_cfg(8123);
    cfg.params = json!({ "__read_only_execution": true });
    let guarded =
        ClickhouseDriver::connection_from_client(&cfg, None, None, None, reqwest::Client::new())
            .unwrap();
    assert!(guarded.readonly);
    assert!(is_readonly_setting_refused(
        "Code: 164. DB::Exception: Cannot modify 'readonly' setting in readonly mode. (READONLY)"
    ));
    assert!(!is_readonly_setting_refused(
        "Code: 164. DB::Exception: Cannot execute query in readonly mode. (READONLY)"
    ));
}

#[test]
fn output_format_preserves_settings_comments_and_literals() {
    assert_eq!(
        output_format_sql("SELECT x FROM format csv", "JSONCompact"),
        "SELECT x FROM format csv\nFORMAT JSONCompact"
    );

    assert_eq!(
        output_format_sql("SELECT 1 FORMAT CSV", "JSONCompact"),
        "SELECT 1 FORMAT JSONCompact"
    );
    assert_eq!(
        output_format_sql(
            "SELECT 'é FORMAT CSV' FORMAT CSV SETTINGS max_threads=1",
            "JSONCompact"
        ),
        "SELECT 'é FORMAT CSV' FORMAT JSONCompact SETTINGS max_threads=1"
    );
    assert_eq!(
        output_format_sql("SELECT 1 -- FORMAT CSV", "JSONCompact"),
        "SELECT 1 -- FORMAT CSV\nFORMAT JSONCompact"
    );
    assert_eq!(
        output_format_sql("SELECT 1 AS format", "JSONCompact"),
        "SELECT 1 AS format\nFORMAT JSONCompact"
    );
    assert_eq!(
        output_format_sql("SELECT 1 FORMAT /* wire */ CSV;", "JSONCompact"),
        "SELECT 1 FORMAT /* wire */ JSONCompact"
    );
}

#[test]
fn http_duplicate_map_keys_survive_both_decoders() {
    let row = r#"[{"key":1,"key":2}, {"nested":{"x":3,"x":4}}]"#;
    let body = compact_reply(&[row], "");
    let parsed = decode_json_reply(&body).unwrap();
    let expected = json!([["key", 1], ["key", 2]]);
    assert_eq!(parsed.data[0][0], expected);
    let mut stream = CompactStream::new(Some(10));
    feed_all(&mut stream, &body).unwrap();
    let raw = stream.finish().unwrap();
    assert_eq!(raw.data[0][0], expected);
    assert_eq!(raw.data[0][1], json!({"nested": [["x", 3], ["x", 4]]}));
}

#[test]
fn native_map_enums_use_labels_and_duplicate_keys_are_not_lost() {
    use klickhouse::{Type as T, Value as V};
    let ty = T::Map(
        Box::new(T::String),
        Box::new(T::Enum8(vec![("yes".into(), 1)])),
    );
    let key = V::String(b"key".to_vec());
    assert_eq!(
        typed_value_to_json(&V::Map(vec![key.clone()], vec![V::Enum8(1)]), &ty),
        json!([["key", "yes"]])
    );
    assert_eq!(
        typed_value_to_json(
            &V::Map(vec![key.clone(), key], vec![V::Enum8(1), V::Enum8(1)]),
            &ty
        ),
        json!([["key", "yes"], ["key", "yes"]])
    );
}

#[test]
fn native_wide_integers_preserve_browser_precision() {
    assert_eq!(
        value_to_json(&klickhouse::Value::UInt64(u64::MAX)),
        json!(u64::MAX.to_string())
    );
    assert_eq!(
        value_to_json(&klickhouse::Value::Int64(i64::MIN)),
        json!(i64::MIN.to_string())
    );
}

#[test]
fn detects_statements_that_return_rows() {
    assert!(returns_rows("SELECT 1"));
    assert!(returns_rows("  select count() from t"));
    assert!(returns_rows("WITH x AS (SELECT 1) SELECT * FROM x"));
    assert!(returns_rows("SHOW TABLES"));
    assert!(returns_rows("DESCRIBE TABLE t"));
    assert!(returns_rows("desc t"));
    assert!(returns_rows("EXPLAIN SELECT 1"));
    assert!(returns_rows("(SELECT 1)"));
    assert!(returns_rows(
        "-- diagnostic\n/* explanation */ EXISTS TABLE t"
    ));
    assert!(returns_rows("CHECK TABLE t"));
    assert!(returns_rows("BACKUP TABLE t TO Disk('backups','b') ASYNC"));
    assert!(returns_rows(
        "RESTORE TABLE t FROM Disk('backups','b') ASYNC"
    ));
    assert!(returns_rows("KILL QUERY WHERE query_id = 'gone' TEST"));

    assert!(!returns_rows("INSERT INTO t VALUES (1)"));
    assert!(!returns_rows("CREATE TABLE t (a Int32) ENGINE = Memory"));
    assert!(!returns_rows("ALTER TABLE t ADD COLUMN b Int32"));
    assert!(!returns_rows("DROP TABLE t"));
    assert!(!returns_rows("  optimize table t final"));
    assert!(!returns_rows(""));
}

#[test]
fn escapes_identifier_quotes() {
    assert_eq!(esc("plain"), "plain");
    assert_eq!(esc("o'brien"), "o''brien");
    assert_eq!(esc("a'b'c"), "a''b''c");
    assert_eq!(esc_ident("plain"), "plain");
    assert_eq!(esc_ident("we`ird"), "we``ird");
}

/// CH honours `\` escapes in string literals AND backtick idents — an
/// unescaped trailing backslash was a quote break-out (injection).
#[test]
fn escapes_backslashes_in_literals_and_idents() {
    // `\'` alone would read as an ESCAPED quote; `\\''` stays inert.
    assert_eq!(esc("evil\\"), "evil\\\\");
    assert_eq!(esc("a\\'b"), "a\\\\''b");
    assert_eq!(esc_ident("t\\"), "t\\\\");
    assert_eq!(esc_ident("a\\`b"), "a\\\\``b");
}

#[test]
fn transport_selected_by_port() {
    // HTTP ports → HTTP.
    assert_eq!(transport_for(&base_cfg(8123)), Transport::Http);
    assert_eq!(transport_for(&base_cfg(8443)), Transport::Http);
    assert_eq!(transport_for(&base_cfg(18123)), Transport::Http);
    // Native protocol ports → native.
    assert_eq!(transport_for(&base_cfg(9000)), Transport::Native);
    assert_eq!(transport_for(&base_cfg(9440)), Transport::Native);
    // Forwarded native ports (tunnel local forward keeps the trailing
    // 9000/9440) → still native.
    assert_eq!(transport_for(&base_cfg(19000)), Transport::Native);
    assert_eq!(transport_for(&base_cfg(19440)), Transport::Native);
    // An arbitrary local-forward port falls back to HTTP unless overridden.
    assert_eq!(transport_for(&base_cfg(34567)), Transport::Http);
}

#[test]
fn native_port_recognition() {
    assert!(is_native_port(9000));
    assert!(is_native_port(9440));
    assert!(is_native_port(19000));
    assert!(is_native_port(19440));
    assert!(!is_native_port(8123));
    assert!(!is_native_port(8443));
    assert!(!is_native_port(18123));
    assert!(!is_native_port(3306));
}

#[test]
fn transport_param_forces_native() {
    // An explicit transport=native param overrides the port heuristic — for
    // any forward port that doesn't carry a recognizable native suffix.
    let mut cfg = base_cfg(34567);
    cfg.params = json!({ "transport": "native" });
    assert_eq!(transport_for(&cfg), Transport::Native);

    // Case-insensitive.
    cfg.params = json!({ "transport": "Native" });
    assert_eq!(transport_for(&cfg), Transport::Native);
}

#[test]
fn value_scalars_to_json() {
    use klickhouse::Value as V;
    assert_eq!(value_to_json(&V::Null), Value::Null);
    assert_eq!(value_to_json(&V::Int32(-7)), json!(-7));
    assert_eq!(value_to_json(&V::UInt64(5)), json!("5"));
    assert_eq!(value_to_json(&V::Float64(1.5)), json!(1.5));
    assert_eq!(value_to_json(&V::String(b"hello".to_vec())), json!("hello"));
    // 128-bit ints render as strings (out of f64/i64 range safely).
    assert_eq!(
        value_to_json(&V::Int128(170141183460469231731687303715884105727i128)),
        json!("170141183460469231731687303715884105727")
    );
}

#[test]
fn value_array_and_tuple_to_json() {
    use klickhouse::Value as V;
    let arr = V::Array(vec![V::Int32(1), V::Int32(2), V::Int32(3)]);
    assert_eq!(value_to_json(&arr), json!([1, 2, 3]));

    let tup = V::Tuple(vec![V::String(b"a".to_vec()), V::Int32(9)]);
    assert_eq!(value_to_json(&tup), json!(["a", 9]));
}

#[test]
fn value_map_to_json_object() {
    use klickhouse::Value as V;
    let map = V::Map(
        vec![V::String(b"k1".to_vec()), V::String(b"k2".to_vec())],
        vec![V::Int32(1), V::Int32(2)],
    );
    assert_eq!(value_to_json(&map), json!({ "k1": 1, "k2": 2 }));
}

#[test]
fn strips_surrounding_quotes() {
    assert_eq!(strip_quotes("'1.2.3.4'".to_string()), "1.2.3.4");
    assert_eq!(strip_quotes("plain".to_string()), "plain");
}

#[test]
fn json_cell_to_text_unwraps_strings() {
    assert_eq!(
        json_cell_to_text(&json!("CREATE TABLE x")),
        "CREATE TABLE x"
    );
    assert_eq!(json_cell_to_text(&json!(42)), "42");
    assert_eq!(json_cell_to_text(&Value::Null), "");
}

// --- Streaming run reader / readonly memo / bulk graph (perf wave) ------

/// A `FORMAT JSONCompact` reply laid out the way ClickHouse writes it.
fn compact_reply(rows: &[&str], tail_extra: &str) -> String {
    let mut s = String::from(
            "{\n\t\"meta\":\n\t[\n\t\t{\n\t\t\t\"name\": \"n\",\n\t\t\t\"type\": \"UInt64\"\n\t\t},\n\t\t{\n\t\t\t\"name\": \"s\",\n\t\t\t\"type\": \"String\"\n\t\t}\n\t],\n\n\t\"data\":\n\t[\n",
        );
    s.push_str(
        &rows
            .iter()
            .map(|r| format!("\t\t{r}"))
            .collect::<Vec<_>>()
            .join(",\n"),
    );
    s.push_str("\n\t],\n\n\t\"rows\": 2,\n");
    s.push_str(tail_extra);
    s.push_str("\n\t\"statistics\":\n\t{\n\t\t\"elapsed\": 0.001,\n\t\t\"rows_read\": 2,\n\t\t\"bytes_read\": 16\n\t}\n}\n");
    s
}

fn feed_all(state: &mut CompactStream, body: &str) -> std::result::Result<(), String> {
    state.push_lines(body.as_bytes())
}

#[test]
fn compact_stream_decodes_rows_meta_and_statistics() {
    let body = compact_reply(&[r#"[1, "a"]"#, r#"[2, "b,]"]"#], "");
    let mut st = CompactStream::new(Some(10));
    feed_all(&mut st, &body).unwrap();
    let raw = st.finish().unwrap();
    assert_eq!(
        raw.meta,
        vec![("n".into(), "UInt64".into()), ("s".into(), "String".into())]
    );
    assert_eq!(
        raw.data,
        vec![vec![json!(1), json!("a")], vec![json!(2), json!("b,]")]]
    );
    assert_eq!(raw.bytes_read, 16);
    assert!(raw.prepared && !raw.partial && !raw.truncated_bytes);
}

#[test]
fn compact_stream_stops_at_the_row_cap() {
    let body = compact_reply(&["[1, \"a\"]", "[2, \"b\"]"], "");
    let mut st = CompactStream::new(Some(1));
    feed_all(&mut st, &body).unwrap();
    assert!(st.stopped);
    let raw = st.finish().unwrap();
    assert_eq!(raw.data.len(), 1);
    // The whole reply was in hand, so the trailer still yields statistics.
    assert!(!raw.partial && raw.prepared);
    assert_eq!(raw.bytes_read, 16);

    // Hanging up inside the rows: rows kept, statistics unknown.
    let cut = &body[..body.rfind("\n\t],").unwrap()];
    let mut st = CompactStream::new(Some(1));
    feed_all(&mut st, cut).unwrap();
    let raw = st.finish().unwrap();
    assert_eq!(raw.data.len(), 1);
    assert!(raw.partial);
}

#[test]
fn compact_stream_stops_at_the_byte_budget_keeping_at_least_one_row() {
    let body = compact_reply(&["[1, \"aaaaaaaaaa\"]", "[2, \"bbbbbbbbbb\"]"], "");
    let mut st = CompactStream::new(Some(10));
    st.budget = types::ByteBudget::new(5);
    feed_all(&mut st, &body).unwrap();
    let raw = st.finish().unwrap();
    assert_eq!(raw.data.len(), 1);
    assert!(raw.truncated_bytes && !raw.partial);
}

#[test]
fn compact_stream_in_band_exception_is_an_error() {
    let body = compact_reply(
            &["[1, \"a\"]"],
            "\t\"exception\": \"Code: 241. DB::Exception: Memory limit exceeded. (MEMORY_LIMIT_EXCEEDED)\",",
        );
    let mut st = CompactStream::new(Some(10));
    feed_all(&mut st, &body).unwrap();
    let err = st.finish().unwrap_err();
    assert!(err.contains("MEMORY_LIMIT_EXCEEDED"), "{err}");
    assert!(err.ends_with("STREAMED_ROWS: 1"), "{err}");
}

#[test]
fn compact_stream_raw_trailing_exception_mid_rows_is_an_error() {
    let mut body = compact_reply(&["[1, \"a\"]"], "");
    body.truncate(body.rfind("\n\t],").unwrap());
    body.push_str(",\nCode: 241. DB::Exception: Memory limit exceeded. (MEMORY_LIMIT_EXCEEDED) (version 24.8)\n");
    let mut st = CompactStream::new(Some(10));
    let err = feed_all(&mut st, &body).unwrap_err();
    assert!(err.contains("MEMORY_LIMIT_EXCEEDED"), "{err}");
}

#[test]
fn compact_stream_falls_back_to_whole_body_for_single_line_json() {
    let body = r#"{"meta":[{"name":"x","type":"UInt8"}],"data":[[1],[2]],"rows":2,"statistics":{"bytes_read":7}}"#;
    let mut st = CompactStream::new(Some(10));
    feed_all(&mut st, body).unwrap();
    let raw = st.finish().unwrap();
    assert_eq!(raw.data.len(), 2);
    assert_eq!(raw.bytes_read, 7);
    // The whole-body path leaves capping/budgeting to `exec_ch_read`.
    assert!(!raw.prepared);
}

#[test]
fn single_select_counts_select_keywords() {
    assert!(single_select("SELECT * FROM t FORMAT JSON"));
    assert!(single_select("select 1 settings max_threads = 1"));
    assert!(!single_select("SELECT 1 UNION ALL SELECT 2"));
    assert!(!single_select(
        "SELECT * FROM t WHERE id IN (SELECT id FROM u)"
    ));
    assert!(!single_select("SHOW TABLES"));
    assert!(single_select("SELECT selected_at FROM t"));
}

#[test]
fn bulk_graph_groups_columns_and_flags_truncation() {
    let tables = RawRows {
        data: vec![
            vec![json!("a"), json!("MergeTree")],
            vec![json!("b_mv"), json!("MaterializedView")],
            vec![json!("c"), json!("MergeTree")],
        ],
        ..RawRows::default()
    };
    let columns = RawRows {
        data: vec![
            vec![json!("a"), json!("id"), json!("UInt64"), json!(1)],
            vec![
                json!("a"),
                json!("note"),
                json!("Nullable(String)"),
                json!(0),
            ],
            vec![json!("b_mv"), json!("x"), json!("Int32"), json!("0")],
        ],
        ..RawRows::default()
    };
    let g = build_bulk_graph("shop", &tables, &columns, 2);
    assert!(g.truncated);
    assert_eq!(g.tables.len(), 2);
    assert_eq!(g.tables[0].id, "db:shop/table:a");
    assert_eq!(g.tables[0].columns.len(), 2);
    assert!(g.tables[0].columns[0].primary_key);
    assert!(g.tables[0].columns[1].nullable && !g.tables[0].columns[1].primary_key);
    assert_eq!(g.tables[1].kind, NodeKind::View);
    assert!(g.edges.is_empty());
    let g = build_bulk_graph("shop", &tables, &columns, 3);
    assert!(!g.truncated);
    assert!(g.tables[2].columns.is_empty());
}

/// One request seen by [`FakeCh`]: (request target, body).
type Seen = Arc<std::sync::Mutex<Vec<(String, String)>>>;

/// Minimal HTTP/1.1 server standing in for ClickHouse: records every
/// request and hands the socket to `respond` to write the reply.
async fn fake_ch<F, Fut>(respond: F) -> (std::net::SocketAddr, Seen)
where
    F: Fn(String, String, tokio::net::TcpStream) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    use tokio::io::AsyncReadExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen: Seen = Arc::default();
    let seen2 = Arc::clone(&seen);
    let respond = Arc::new(respond);
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let seen = Arc::clone(&seen2);
            let respond = Arc::clone(&respond);
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                let head_end = loop {
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let len = head
                    .lines()
                    .find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                while buf.len() < head_end + len {
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                }
                let target = head.split_whitespace().nth(1).unwrap_or("").to_string();
                let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
                seen.lock().unwrap().push((target.clone(), body.clone()));
                respond(target, body, sock).await;
            });
        }
    });
    (addr, seen)
}

async fn reply(mut sock: tokio::net::TcpStream, status: &str, body: &str) {
    use tokio::io::AsyncWriteExt;
    let msg = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = sock.write_all(msg.as_bytes()).await;
    let _ = sock.shutdown().await;
}

fn test_conn(addr: std::net::SocketAddr) -> Conn {
    Conn {
        client: reqwest::Client::builder().no_proxy().build().unwrap(),
        base: format!("http://{addr}/"),
        headers: HeaderMap::new(),
        timezone: None,
        database: None,
        query_id: Some("otto-q1".into()),
        session_id: None,
        map_arrays: true,
        timeout_secs: None,
        readonly: false,
        readonly_refused: Arc::new(AtomicBool::new(false)),
        result_row_cap: None,
    }
}

#[tokio::test]
async fn explicit_format_and_admin_rows_reach_grid() {
    let (addr, seen) = fake_ch(|_, _, sock| async move {
        reply(
            sock,
            "200 OK",
            r#"{"meta":[{"name":"result","type":"UInt8"}],"data":[[1]]}"#,
        )
        .await;
    })
    .await;
    let mut cfg = base_cfg(addr.port());
    cfg.host = addr.ip().to_string();
    cfg.params = json!({"transport": "http"});
    let driver = ClickhouseDriver::default();
    for statement in [
        "SELECT 1 FORMAT CSV SETTINGS max_threads=1",
        "-- diagnostic\nEXISTS TABLE t",
        "CHECK TABLE t",
        "BACKUP TABLE t TO Disk('backups','b') ASYNC",
        "RESTORE TABLE t FROM Disk('backups','b') ASYNC",
    ] {
        let result = driver
            .run(
                &cfg,
                &QueryRequest {
                    statement: statement.into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(result.rows, vec![vec![json!(1)]]);
        assert_eq!(result.columns[0].name, "result");
    }
    let seen = seen.lock().unwrap();
    let requests: Vec<_> = seen
        .iter()
        .filter(|(_, sql)| !sql.contains("FROM system.settings WHERE name ="))
        .collect();
    assert!(requests[0]
        .1
        .contains("FORMAT JSONCompact SETTINGS max_threads=1"));
    assert!(!requests[0].1.contains("FORMAT CSV"));
    assert!(requests
        .iter()
        .all(|(_, body)| body.contains("FORMAT JSONCompact")));
}

/// DB-01: a ~200 MB reply resolves to `row_cap` rows, the reader hangs up
/// long before the server finishes writing, and the query is KILLed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn capped_run_reader_stops_early_on_a_huge_reply_and_kills_the_query() {
    use std::sync::atomic::AtomicUsize;
    use tokio::io::AsyncWriteExt;
    const TOTAL_ROWS: usize = 200_000;
    let written = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicBool::new(false));
    let (w, f) = (Arc::clone(&written), Arc::clone(&finished));
    let (addr, seen) = fake_ch(move |_target, body, mut sock| {
        let (written, finished) = (Arc::clone(&w), Arc::clone(&f));
        async move {
            if body.starts_with("KILL QUERY") {
                return reply(sock, "200 OK", "").await;
            }
            let head = compact_reply(&[], "");
            let head = &head[..head.rfind("\t[\n").unwrap() + 3];
            let pad = "x".repeat(1000);
            let mut out = format!("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n{head}");
            for i in 0..TOTAL_ROWS {
                out.push_str(&format!("\t\t[{i}, \"{pad}\"],\n"));
                if out.len() >= 64 * 1024 {
                    if sock.write_all(out.as_bytes()).await.is_err() {
                        break;
                    }
                    out.clear();
                }
                written.store(i + 1, Ordering::Relaxed);
            }
            finished.store(true, Ordering::Relaxed);
        }
    })
    .await;

    let conn = test_conn(addr);
    let raw = conn
        .query_json_capped("SELECT * FROM big", 101)
        .await
        .unwrap();
    assert_eq!(raw.data.len(), 101);
    assert!(raw.prepared && raw.partial && !raw.truncated_bytes);
    assert_eq!(raw.data[100][0], json!(100));

    // The server's writer sees the hang-up and stops well short of the end.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !finished.load(Ordering::Relaxed) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let rows_written = written.load(Ordering::Relaxed);
    assert!(
        rows_written < TOTAL_ROWS / 4,
        "reader consumed too much: server wrote {rows_written} rows"
    );
    // …and the server-side query is killed by its query_id.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let killed = seen
            .lock()
            .unwrap()
            .iter()
            .any(|(_, b)| b == "KILL QUERY WHERE query_id = 'otto-q1'");
        if killed {
            break;
        }
        assert!(Instant::now() < deadline, "no KILL QUERY was sent");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// DB-01 server-side cap: `max_result_rows` + `result_overflow_mode=break`
/// ride on the request only when `result_row_cap` is set.
#[tokio::test]
async fn server_row_cap_is_sent_as_request_settings() {
    let body = compact_reply(&["[1, \"a\"]"], "");
    let (addr, seen) = fake_ch(move |_t, _b, sock| {
        let body = body.clone();
        async move { reply(sock, "200 OK", &body).await }
    })
    .await;
    let mut conn = test_conn(addr);
    conn.query_json_capped("SELECT 1", 5).await.unwrap();
    conn.result_row_cap = Some(5);
    let raw = conn.query_json_capped("SELECT 1", 5).await.unwrap();
    assert_eq!(raw.data.len(), 1);
    assert_eq!(raw.bytes_read, 16);
    let seen = seen.lock().unwrap();
    assert!(!seen[0].0.contains("max_result_rows"), "{}", seen[0].0);
    assert!(seen[1].0.contains("max_result_rows=5"), "{}", seen[1].0);
    assert!(
        seen[1].0.contains("result_overflow_mode=break"),
        "{}",
        seen[1].0
    );
}

/// DB2-05: a zstd body decodes chunk by chunk to exactly the original bytes
/// however the network splits it; identity passes through; an encoding we
/// never asked for is a clear error, not garbage rows.
#[test]
fn zstd_body_decodes_across_arbitrary_chunk_splits() {
    let rows: Vec<String> = (0..5_000).map(|i| format!("[{i}, \"row {i}\"]")).collect();
    let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
    let plain = compact_reply(&refs, "");
    let packed = zstd::stream::encode_all(plain.as_bytes(), 3).unwrap();
    assert!(packed.len() * 5 < plain.len(), "JSONCompact compresses ≥5×");
    for split in [1usize, 7, 4096, packed.len()] {
        let mut dec = BodyDecoder::for_encoding(Some("zstd")).unwrap();
        let mut out = Vec::new();
        for chunk in packed.chunks(split) {
            out.extend_from_slice(&dec.decode(chunk).unwrap());
        }
        assert_eq!(out, plain.as_bytes(), "split {split}");
    }
    let mut id = BodyDecoder::for_encoding(None).unwrap();
    assert!(matches!(
        id.decode(b"abc").unwrap(),
        std::borrow::Cow::Borrowed(b"abc")
    ));
    assert!(BodyDecoder::for_encoding(Some("br")).is_err());
    let mut dec = BodyDecoder::for_encoding(Some("zstd")).unwrap();
    assert!(dec.decode(b"definitely not zstd").is_err());
}

/// DB2-05 end to end: every request asks for compression, and a compressed
/// JSONCompact reply streams into the same rows as a plain one.
#[tokio::test]
async fn compressed_reply_streams_into_rows() {
    let plain = compact_reply(&["[1, \"a\"]", "[2, \"b\"]"], "");
    let packed = zstd::stream::encode_all(plain.as_bytes(), 3).unwrap();
    let (addr, seen) = fake_ch(move |_t, _b, mut sock| {
            let packed = packed.clone();
            async move {
                use tokio::io::AsyncWriteExt;
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Encoding: zstd\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    packed.len()
                );
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(&packed).await;
                let _ = sock.shutdown().await;
            }
        })
        .await;
    let conn = test_conn(addr);
    let raw = conn.query_json_capped("SELECT 1", 10).await.unwrap();
    assert_eq!(raw.data.len(), 2);
    assert_eq!(raw.data[1][1], json!("b"));
    let seen = seen.lock().unwrap();
    assert!(
        seen[0].0.contains("enable_http_compression=1"),
        "{}",
        seen[0].0
    );
}

/// DB-02: the first `readonly` refusal is remembered — later requests skip
/// the doomed attempt (2 round trips once, then 1 per request).
#[tokio::test]
async fn readonly_refusal_is_memoized_per_connection() {
    let (addr, seen) = fake_ch(|target, _b, sock| async move {
            if target.contains("readonly=2") {
                reply(
                    sock,
                    "500 Internal Server Error",
                    "Code: 164. DB::Exception: Cannot modify 'readonly' setting in readonly mode. (READONLY)",
                )
                .await
            } else {
                reply(sock, "200 OK", "1\n").await
            }
        })
        .await;
    let mut conn = test_conn(addr);
    conn.readonly = true;
    assert_eq!(conn.post("SELECT 1".into()).await.unwrap(), "1\n");
    assert!(conn.readonly_refused.load(Ordering::Relaxed));
    assert_eq!(conn.post("SELECT 1".into()).await.unwrap(), "1\n");
    assert_eq!(conn.post("SELECT 1".into()).await.unwrap(), "1\n");
    let seen = seen.lock().unwrap();
    // readonly attempt + fixed probe + retry, then one request each.
    assert_eq!(seen.len(), 5, "{seen:?}");
    assert_eq!(
        seen.iter()
            .filter(|(t, _)| t.contains("readonly=2"))
            .count(),
        1
    );
}

/// S6-03: a user statement that FORGES the refusal text (a read the
/// classifier passes) must not switch the native `readonly=2` barrier off
/// — the fixed probe reports a writable profile, so the error surfaces and
/// nothing is memoised.
#[tokio::test]
async fn forged_readonly_refusal_never_disables_the_barrier() {
    let (addr, seen) = fake_ch(|target, body, sock| async move {
            if body.contains("getSetting('readonly')") {
                reply(sock, "200 OK", "0\n").await
            } else if target.contains("readonly=2") {
                reply(
                    sock,
                    "500 Internal Server Error",
                    "Code: 164. DB::Exception: Cannot modify 'readonly' setting in readonly mode. (READONLY)",
                )
                .await
            } else {
                reply(sock, "200 OK", "1\n").await
            }
        })
        .await;
    let mut conn = test_conn(addr);
    conn.readonly = true;
    let forged = "SELECT throwIf(1, 'Code: 164. Cannot modify ''readonly'' setting')";
    assert!(conn.post(forged.into()).await.is_err());
    assert!(!conn.readonly_refused.load(Ordering::Relaxed));
    // The next request still asks for readonly=2 (the fake refuses every
    // readonly=2 request, so it is followed by another probe — the
    // request itself is what must carry the barrier).
    let _ = conn.post("SELECT 1".into()).await;
    let seen = seen.lock().unwrap();
    assert!(
        seen.iter()
            .any(|(t, b)| b == "SELECT 1" && t.contains("readonly=2")),
        "{seen:?}"
    );
    assert!(
        !seen
            .iter()
            .any(|(t, b)| b == "SELECT 1" && !t.contains("readonly=2")),
        "{seen:?}"
    );
    assert!(!seen
        .iter()
        .any(|(t, b)| !t.contains("readonly=2") && b.contains("throwIf")));
}

#[test]
fn readonly_memo_is_shared_per_cache_key() {
    let d = ClickhouseDriver::default();
    let a = d.readonly_memo("k1");
    a.store(true, Ordering::Relaxed);
    assert!(d.readonly_memo("k1").load(Ordering::Relaxed));
    assert!(!d.readonly_memo("k2").load(Ordering::Relaxed));
}

#[tokio::test]
async fn evict_idle_drops_only_stale_handles() {
    let d = ClickhouseDriver::default();
    let cfg = base_cfg(8123);
    d.client(&cfg).await.unwrap();
    assert_eq!(d.evict_idle(Duration::from_secs(3600)).await, 0);
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert_eq!(d.evict_idle(Duration::from_millis(1)).await, 1);
    assert!(d.clients.get_ready(&cfg.cache_key()).is_none());
}

#[test]
fn export_zstd_expansion_uses_bounded_windows() {
    use std::io::Write;
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
    let megabyte = vec![b'x'; 1024 * 1024];
    for _ in 0..70 {
        encoder.write_all(&megabyte).unwrap();
    }
    let compressed = encoder.finish().unwrap();
    let mut decoder = BodyDecoder::for_encoding(Some("zstd")).unwrap();
    let mut total = 0;
    decoder
        .decode_to(&compressed, |window| {
            assert!(window.len() <= ZSTD_WINDOW);
            total += window.len();
            Ok(true)
        })
        .unwrap();
    assert_eq!(total, 70 * 1024 * 1024);
}
