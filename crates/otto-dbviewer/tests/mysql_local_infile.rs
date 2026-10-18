//! Native LOCAL INFILE protocol tests against the isolated dev MySQL fixture.
use otto_dbviewer::{
    driver::Driver,
    drivers::mysql::MysqlDriver,
    types::{Engine, QueryRequest, ResolvedConfig, TlsConfig},
};
use serde_json::json;
use sqlx::Connection as _;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

fn cfg() -> ResolvedConfig {
    ResolvedConfig {
        lifecycle: None,
        engine: Engine::Mysql,
        host: "127.0.0.1".into(),
        port: 13306,
        user: Some("otto".into()),
        password: Some("ottopw".into()),
        database: Some("shopdb".into()),
        tls: TlsConfig::default(),
        params: json!({"__mysql_local_infile_authorized": true}),
    }
}
fn request(statement: String) -> QueryRequest {
    QueryRequest {
        statement,
        timeout_ms: Some(5000),
        ..Default::default()
    }
}

/// Relay the real fixture handshake/auth, but replace the LOCAL INFILE filename
/// with a different existing file. Count any upload bytes after that request.
async fn rewrite_filename_proxy(
    expected: Vec<u8>,
    replacement: Vec<u8>,
) -> (
    u16,
    Arc<AtomicBool>,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let replaced = Arc::new(AtomicBool::new(false));
    let uploaded = Arc::new(AtomicUsize::new(0));
    let replaced_out = replaced.clone();
    let uploaded_out = uploaded.clone();
    let task = tokio::spawn(async move {
        let mut relays = tokio::task::JoinSet::new();
        loop {
            let Ok((client, _)) = listener.accept().await else {
                break;
            };
            let replaced = replaced.clone();
            let uploaded = uploaded.clone();
            let expected = expected.clone();
            let replacement = replacement.clone();
            relays.spawn(async move {
                let server = tokio::net::TcpStream::connect("127.0.0.1:13306").await?;
                let (mut cr, mut cw) = client.into_split();
                let (mut sr, mut sw) = server.into_split();
                let upload_started = Arc::new(AtomicBool::new(false));
                let flag = upload_started.clone();
                let toward_client = async {
                    loop {
                        let mut header = [0u8; 4];
                        sr.read_exact(&mut header).await?;
                        let len = u32::from_le_bytes([header[0], header[1], header[2], 0]) as usize;
                        let mut packet = vec![0; len];
                        sr.read_exact(&mut packet).await?;
                        if packet.first() == Some(&0xfb) && packet[1..] == expected {
                            packet = [vec![0xfb], replacement.clone()].concat();
                            let len = (packet.len() as u32).to_le_bytes();
                            header[..3].copy_from_slice(&len[..3]);
                            replaced.store(true, Ordering::SeqCst);
                            flag.store(true, Ordering::SeqCst);
                        }
                        cw.write_all(&header).await?;
                        cw.write_all(&packet).await?;
                    }
                    #[allow(unreachable_code)]
                    Ok::<(), std::io::Error>(())
                };
                let toward_server = async {
                    let mut buf = [0; 8192];
                    loop {
                        let n = cr.read(&mut buf).await?;
                        if n == 0 {
                            break;
                        }
                        if upload_started.load(Ordering::SeqCst) {
                            uploaded.fetch_add(n, Ordering::SeqCst);
                        }
                        sw.write_all(&buf[..n]).await?;
                    }
                    sw.shutdown().await?;
                    Ok::<(), std::io::Error>(())
                };
                tokio::try_join!(toward_client, toward_server)?;
                Ok::<(), std::io::Error>(())
            });
        }
    });
    (port, replaced_out, uploaded_out, task)
}

#[tokio::test]
#[ignore = "requires isolated MySQL fixture (OTTO_DBV_E2E)"]
async fn mysql_local_infile_native_load_and_exact_filename_grant() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("approved.csv");
    std::fs::write(&path, "id,name\n1,alpha\n2,beta\n").unwrap();
    let secret = dir.path().join("not-authorized.txt");
    std::fs::write(&secret, "MUST_NOT_BE_UPLOADED").unwrap();
    let filename = path.to_str().unwrap();
    let driver = MysqlDriver::default();
    let script = format!(
        r"CREATE TEMPORARY TABLE infile_probe (id INT, label TEXT, total INT);
        SET @suffix='!';
        LOAD DATA LOCAL INFILE '{filename}' INTO TABLE infile_probe
        FIELDS TERMINATED BY ',' LINES TERMINATED BY '\n' IGNORE 1 LINES
        (@id,@name) SET id=@id, label=CONCAT(@name,@suffix), total=@id*10;
        SELECT id,label,total FROM infile_probe ORDER BY id"
    );
    let mut denied = cfg();
    denied.params = json!({});
    assert!(driver
        .run(&denied, &request(script.clone()))
        .await
        .unwrap_err()
        .to_string()
        .contains("authorized access"));

    let mut admin = sqlx::MySqlConnection::connect("mysql://root:rootpw@127.0.0.1:13306/shopdb")
        .await
        .unwrap();
    let previous: i64 = sqlx::query_scalar("SELECT @@global.local_infile")
        .fetch_one(&mut admin)
        .await
        .unwrap();
    sqlx::raw_sql("SET GLOBAL local_infile=ON")
        .execute(&mut admin)
        .await
        .unwrap();
    // Collect results before restoring the fixture's original server setting.
    let loaded = driver.run(&cfg(), &request(script.clone())).await;
    // A backslash in a filename previously triggered an observer SQL-mode SELECT,
    // which changed ROW_COUNT() from the preceding INSERT before LOAD evaluated it.
    let escaped_path = dir.path().join("escaped\\file.csv");
    std::fs::write(&escaped_path, "3,gamma\n").unwrap();
    let escaped_filename = escaped_path.to_str().unwrap().replace('\\', "\\\\");
    let state_script = format!(
        r"CREATE TEMPORARY TABLE infile_state (id INT, label TEXT, previous_count INT);
        INSERT INTO infile_state VALUES (1,'one',0),(2,'two',0);
        LOAD DATA LOCAL INFILE '{escaped_filename}' INTO TABLE infile_state
        FIELDS TERMINATED BY ',' LINES TERMINATED BY '\n'
        (id,label) SET previous_count=ROW_COUNT();
        SELECT previous_count FROM infile_state WHERE id=3"
    );
    let state_result = driver.run(&cfg(), &request(state_script)).await;
    let raw_filename = escaped_path.to_str().unwrap();
    let mode_script = format!(
        r"SET SESSION sql_mode='NO_BACKSLASH_ESCAPES';
        SELECT 'ends\' AS literal;
        CREATE TEMPORARY TABLE infile_mode (id INT, label TEXT);
        LOAD DATA LOCAL INFILE '{raw_filename}' INTO TABLE infile_mode
        FIELDS TERMINATED BY ',';
        SELECT id,label FROM infile_mode"
    );
    let mode_result = driver.run(&cfg(), &request(mode_script)).await;
    let large_path = dir.path().join("chunked.txt");
    std::fs::write(&large_path, format!("{}\n", "x".repeat(140_000))).unwrap();
    let empty_path = dir.path().join("empty.txt");
    std::fs::write(&empty_path, "").unwrap();
    let chunk_script = format!(
        "CREATE TEMPORARY TABLE infile_chunks (payload LONGTEXT); \
         LOAD DATA LOCAL INFILE '{}' INTO TABLE infile_chunks; \
         LOAD DATA LOCAL INFILE '{}' INTO TABLE infile_chunks; \
         SELECT LENGTH(payload) FROM infile_chunks",
        large_path.display(),
        empty_path.display()
    );
    let chunk_result = driver.run(&cfg(), &request(chunk_script)).await;

    let (port, replaced, uploaded, proxy) = rewrite_filename_proxy(
        filename.as_bytes().to_vec(),
        secret.to_str().unwrap().as_bytes().to_vec(),
    )
    .await;
    let mut proxied = cfg();
    proxied.port = port;
    let refused = driver.run(&proxied, &request(script)).await;
    proxy.abort();
    let _ = proxy.await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "SET GLOBAL local_infile={previous}"
    )))
    .execute(&mut admin)
    .await
    .unwrap();

    let state_result = state_result.unwrap();
    assert_eq!(
        state_result.more_results.last().unwrap().rows,
        vec![vec![json!(2)]],
        "{state_result:?}"
    );
    let mode_result = mode_result.unwrap();
    assert_eq!(
        mode_result.more_results.last().unwrap().rows,
        vec![vec![json!(3), json!("gamma")]],
        "{mode_result:?}"
    );
    let chunk_result = chunk_result.unwrap();
    assert_eq!(
        chunk_result.more_results[0].rows_affected,
        Some(1),
        "{chunk_result:?}"
    );
    assert_eq!(
        chunk_result.more_results[1].rows_affected,
        Some(0),
        "{chunk_result:?}"
    );
    assert_eq!(
        chunk_result.more_results.last().unwrap().rows,
        vec![vec![json!(140_000)]],
        "{chunk_result:?}"
    );
    let loaded = loaded.unwrap();
    assert_eq!(loaded.more_results[1].rows_affected, Some(2), "{loaded:?}");
    assert_eq!(
        loaded.more_results[2].rows,
        vec![
            vec![json!(1), json!("alpha!"), json!(10)],
            vec![json!(2), json!("beta!"), json!(20)]
        ]
    );
    assert!(
        replaced.load(Ordering::SeqCst),
        "fixture must actually replace server's filename request"
    );
    assert_eq!(
        uploaded.load(Ordering::SeqCst),
        0,
        "no bytes may be sent for another filename"
    );
    let refused = refused.unwrap();
    assert!(refused.more_results.last().unwrap().errored, "{refused:?}");
    assert!(refused
        .more_results
        .last()
        .unwrap()
        .message
        .as_deref()
        .unwrap()
        .contains("not explicitly authorized"));
}
