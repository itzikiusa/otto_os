use super::*;
use crate::toolchains_catalog::JAVA_DEBUG_JAR_NAME;
use crate::types::{IdeInstallComponent, IdeInstallJobStatus};
use installer::make_executable;

#[test]
fn discovers_otto_managed_sdks_lsps_daps_and_marks_active_selection() {
    let tmp = tempfile::tempdir().unwrap();
    let tc = tmp.path().join("toolchains");

    let jdk_home = tc.join("java/jdks/jdk-21/Contents/Home");
    std::fs::create_dir_all(jdk_home.join("bin")).unwrap();
    std::fs::write(jdk_home.join("bin/java"), "#!/bin/sh\n").unwrap();
    make_executable(&jdk_home.join("bin/java")).unwrap();
    std::fs::write(jdk_home.join("release"), "JAVA_VERSION=\"21.0.6\"\n").unwrap();

    let jdtls = tc.join("java/lsp/bin/jdtls");
    std::fs::create_dir_all(jdtls.parent().unwrap()).unwrap();
    std::fs::write(&jdtls, "#!/bin/sh\n").unwrap();
    make_executable(&jdtls).unwrap();

    let java_dbg = tc.join("java/dap").join(JAVA_DEBUG_JAR_NAME);
    std::fs::create_dir_all(java_dbg.parent().unwrap()).unwrap();
    std::fs::write(&java_dbg, b"jar").unwrap();

    let goroot = tc.join("go/sdks/1.24.1/go");
    std::fs::create_dir_all(goroot.join("bin")).unwrap();
    std::fs::write(goroot.join("bin/go"), "#!/bin/sh\n").unwrap();
    make_executable(&goroot.join("bin/go")).unwrap();
    std::fs::write(goroot.join("VERSION"), "go1.24.1\n").unwrap();

    let dlv = tc.join("go/bin/dlv");
    std::fs::create_dir_all(dlv.parent().unwrap()).unwrap();
    std::fs::write(&dlv, "#!/bin/sh\n").unwrap();
    make_executable(&dlv).unwrap();

    let shfmt = tc.join("shell/bin/shfmt");
    std::fs::create_dir_all(shfmt.parent().unwrap()).unwrap();
    std::fs::write(&shfmt, "#!/bin/sh\n").unwrap();
    make_executable(&shfmt).unwrap();

    let jq = tc.join("json/bin/jq");
    std::fs::create_dir_all(jq.parent().unwrap()).unwrap();
    std::fs::write(&jq, "#!/bin/sh\n").unwrap();
    make_executable(&jq).unwrap();

    let roots = vec![tc];
    let mut java_sdks = discover_local_sdks(&roots, EcosystemKind::Java);
    let env = IdeProjectEnvironment {
        java_home: Some(jdk_home.to_string_lossy().into_owned()),
        ..Default::default()
    };
    mark_active_sdk(&mut java_sdks, EcosystemKind::Java, Some(&env));
    let otto_java = java_sdks.iter().find(|s| s.source == "otto").unwrap();
    assert_eq!(otto_java.version, "21.0.6");
    assert!(otto_java.active);
    assert!(discover_local_lsp(&roots, EcosystemKind::Java).available);
    assert!(discover_local_dap(&roots, EcosystemKind::Java).available);

    let go_sdks = discover_local_sdks(&roots, EcosystemKind::Go);
    assert!(go_sdks
        .iter()
        .any(|s| s.source == "otto" && s.version == "1.24.1"));
    assert!(discover_local_dap(&roots, EcosystemKind::Go).available);

    let shell_sdks = discover_local_sdks(&roots, EcosystemKind::Shell);
    assert!(shell_sdks.iter().any(|s| s.source == "otto"));
    let json_sdks = discover_local_sdks(&roots, EcosystemKind::Json);
    assert!(json_sdks.iter().any(|s| s.source == "otto"));

    let detected = scan_detected_sdks(&roots, &[]);
    assert!(detected
        .iter()
        .any(|d| d.ecosystem == IdeEcosystemKind::Java && d.version == "21.0.6"));
    assert!(detected
        .iter()
        .any(|d| d.ecosystem == IdeEcosystemKind::Go && d.version == "1.24.1"));
}

#[test]
fn parses_batched_ssh_probe_for_remote_linux_aarch64() {
    let sample = concat!(
        "UNAME:Linux aarch64\n",
        "SDK:go:otto:1.24.1:/home/dev/.otto/toolchains/go/sdks/1.24.1/go\n",
        "SDK:go:system:1.23.4:/usr/local/go/bin/go\n",
        "SDK:java:system:21.0.6:/usr/lib/jvm/java-21-openjdk\n",
        "SDK:python:system:3.12.8:/usr/bin/python3\n",
        "LSP:go:/home/dev/.otto/toolchains/go/bin/gopls\n",
        "DAP:go:/usr/local/bin/dlv\n",
        "LSP:shell:/usr/local/bin/bash-language-server\n",
    );
    let (platform, map) = parse_ssh_probe_output(sample);
    assert_eq!(platform.os.as_str(), "linux");
    assert_eq!(platform.arch.as_str(), "aarch64");
    let go = map.get(&EcosystemKind::Go).unwrap();
    assert_eq!(go.installed[0].source, "otto");
    assert_eq!(go.installed[0].version, "1.24.1");
    assert_eq!(
        go.installed[0].path,
        "/home/dev/.otto/toolchains/go/sdks/1.24.1/go"
    );
    assert_eq!(go.installed[1].source, "system");
    assert_eq!(go.installed[1].version, "1.23.4");
    assert_eq!(go.installed[1].path, "/usr/local/go");
    assert!(go.lsp.as_ref().unwrap().available);
    assert!(go.dap.as_ref().unwrap().available);

    let java = map.get(&EcosystemKind::Java).unwrap();
    assert_eq!(java.installed[0].version, "21.0.6");
    assert_eq!(java.installed[0].path, "/usr/lib/jvm/java-21-openjdk");

    let py = map.get(&EcosystemKind::Python).unwrap();
    assert_eq!(py.installed[0].version, "3.12.8");
    assert_eq!(py.installed[0].path, "/usr/bin/python3");

    let sh = map.get(&EcosystemKind::Shell).unwrap();
    assert!(sh.lsp.as_ref().unwrap().available);
}

#[test]
fn parses_legacy_three_field_ssh_probe_lines_and_normalizes_sdk_home() {
    let sample = "UNAME:Linux x86_64\nSDK:go:system:/usr/local/go/bin/go\nSDK:python:system:/usr/bin/python3\n";
    let (platform, map) = parse_ssh_probe_output(sample);
    assert_eq!(platform.arch.as_str(), "x86_64");
    let go = map.get(&EcosystemKind::Go).unwrap();
    assert_eq!(go.installed[0].path, "/usr/local/go");
    assert_eq!(go.installed[0].version, "system");
    let py = map.get(&EcosystemKind::Python).unwrap();
    assert_eq!(py.installed[0].path, "/usr/bin/python3");
    assert_eq!(py.installed[0].version, "3");
}

#[tokio::test]
async fn register_and_remove_local_sdk_round_trip_persists_and_surfaces_in_fast_list() {
    let tmp = tempfile::tempdir().unwrap();
    let db = otto_state::open(&tmp.path().join("ide_tc.db"))
        .await
        .unwrap();
    let ws = tmp.path().join("workspace");
    std::fs::create_dir_all(&ws).unwrap();

    let custom_go = tmp.path().join("custom-go-1.23.7");
    std::fs::create_dir_all(custom_go.join("bin")).unwrap();
    std::fs::write(custom_go.join("bin/go"), "#!/bin/sh\n").unwrap();
    make_executable(&custom_go.join("bin/go")).unwrap();
    std::fs::write(custom_go.join("VERSION"), "go1.23.7\n").unwrap();

    let mgr = ToolchainManager::new(tmp.path().join("data"));
    let targets = TargetManager::new();
    let target = IdeTargetSpec::local(ws.to_string_lossy().into_owned());

    let registered = mgr
        .register_local_sdk(
            &db,
            IdeRegisterLocalSdkReq {
                target: target.clone(),
                ecosystem: IdeEcosystemKind::Go,
                path: custom_go.to_string_lossy().into_owned(),
            },
        )
        .await
        .unwrap();
    assert_eq!(registered.source, "local");
    assert_eq!(registered.version, "1.23.7");

    let t0 = std::time::Instant::now();
    let resp = mgr
        .list_toolchains_with_opts(Some(&db), &targets, &target, None, None, false)
        .await
        .unwrap();
    let elapsed = t0.elapsed();
    assert!(
        elapsed < Duration::from_millis(500),
        "non-catalog list_toolchains must not make network requests (took {elapsed:?})"
    );
    assert_eq!(resp.ecosystems.len(), 7);
    assert!(!resp.detected.is_empty());
    let go_eco = resp
        .ecosystems
        .iter()
        .find(|e| e.ecosystem == IdeEcosystemKind::Go)
        .unwrap();
    assert!(go_eco
        .installed
        .iter()
        .any(|s| s.source == "local" && s.version == "1.23.7"));

    mgr.remove_local_sdk(&db, &target, None, Some(&registered.path))
        .await
        .unwrap();
    let resp_after = mgr
        .list_toolchains_with_opts(Some(&db), &targets, &target, None, None, false)
        .await
        .unwrap();
    let go_after = resp_after
        .ecosystems
        .iter()
        .find(|e| e.ecosystem == IdeEcosystemKind::Go)
        .unwrap();
    assert!(!go_after
        .installed
        .iter()
        .any(|s| s.source == "local" && s.version == "1.23.7"));
}

#[test]
fn finished_install_jobs_are_bounded_to_prevent_memory_leak() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = ToolchainManager::new(tmp.path().to_path_buf());
    for i in 0..30 {
        let id = format!("tcjob_{i:02}");
        mgr.jobs.insert(
            id.clone(),
            IdeInstallJob {
                id: id.clone(),
                ecosystem: IdeEcosystemKind::Go,
                component: IdeInstallComponent::Lsp,
                version: None,
                target_os: "darwin".into(),
                target_arch: "aarch64".into(),
                status: IdeInstallJobStatus::Running,
                progress_pct: 50,
                log_lines: vec!["working".into()],
                installed_path: None,
                error: None,
            },
        );
        mgr.finish_job(&id, Ok("/tmp/gopls".into()));
    }
    assert!(mgr.jobs.len() <= MAX_FINISHED_JOBS);
}

#[tokio::test]
async fn downloader_enforces_companion_sha256_and_removes_corrupt_archive() {
    use sha2::{Digest, Sha256};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let payload = b"fake-uv-tarball-bytes";
    let valid_hex = hex::encode(Sha256::digest(payload));
    let wrong_hex = "0000000000000000000000000000000000000000000000000000000000000000";

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let valid_sha_body = format!("{valid_hex}  uv.tar.gz\n");
    let server = tokio::spawn(async move {
        for _ in 0..3 {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let req_line = String::from_utf8_lossy(&buf[..n]);
            let body: &[u8] = if req_line.contains("GET /uv.tar.gz.sha256 ") {
                valid_sha_body.as_bytes()
            } else {
                payload.as_slice()
            };
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes()).await;
            let _ = stream.write_all(body).await;
        }
    });

    let tmp = tempfile::tempdir().unwrap();
    let mgr = ToolchainManager::new(tmp.path().join("data"));
    mgr.allow_test_loopback.store(true, Ordering::Relaxed);
    let url = format!("http://{addr}/uv.tar.gz");

    let fetched_sha = installer::fetch_companion_sha256(&mgr, &url)
        .await
        .expect("fetch .sha256 companion");
    assert_eq!(fetched_sha, valid_hex);

    let corrupt_dest = tmp.path().join("corrupt.tar.gz");
    let err = installer::download_https_to_file(
        &mgr,
        "job_bad",
        &url,
        &corrupt_dest,
        Some(wrong_hex),
        10,
    )
    .await
    .expect_err("corrupt sha256 must fail");
    assert!(err.contains("SHA-256 checksum mismatch"));
    assert!(
        !corrupt_dest.exists(),
        "corrupted download must be unlinked immediately"
    );

    let ok_dest = tmp.path().join("verified.tar.gz");
    installer::download_https_to_file(&mgr, "job_ok", &url, &ok_dest, Some(&fetched_sha), 10)
        .await
        .expect("matching sha256 must succeed");
    assert_eq!(tokio::fs::read(&ok_dest).await.unwrap(), payload);
    let _ = server.await;
}
