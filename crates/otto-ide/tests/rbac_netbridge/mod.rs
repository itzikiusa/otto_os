//! Non-root RBAC, cross-tenant SSH/K8s/namespace rejection, stale-disabled
//! tolerance, per-user `evict_user` isolation, and `NetBridgeLease::drop`
//! idle eviction tests for `otto-ide`'s Network Bridge (`[B1]`).

use std::path::Path;
use std::time::{Duration, Instant};

use axum::http::{Method, StatusCode};
use otto_core::{Error, Id};
use otto_ide::netbridge::{IdeK8sSpec, NetBridgeManager};
use otto_ide::types::{IdeHostMapping, IdeNetworkBridgeConfig, IdeSshSpec, IdeTargetSpec};
use serde_json::json;

use crate::common::{
    build_test_app, non_root_user, router_as, send_json, TestK8sEntry, TestSshEntry,
};

fn write_mock_kubectl(dir: &Path) -> String {
    let bin = dir.join("mock-kubectl.sh");
    let script = r#"#!/bin/sh
case "$*" in
  *services*)
    printf '%s\n' '{"items":[{"metadata":{"name":"confsrv","namespace":"cbo"},"spec":{"clusterIP":"10.96.1.10","ports":[{"name":"http","port":80,"targetPort":8080}]}}]}'
    ;;
  *ingresses*)
    printf '%s\n' '{"items":[]}'
    ;;
  *)
    printf '%s\n' '{"items":[]}'
    ;;
esac
"#;
    std::fs::write(&bin, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin.to_string_lossy().into_owned()
}

fn make_k8s_spec(id: &str, name: &str, ns: &str, kubectl: &str) -> IdeK8sSpec {
    IdeK8sSpec {
        cluster_id: Id::from(id),
        cluster_name: name.to_string(),
        environment: None,
        namespace: ns.to_string(),
        allow_cross_namespace: false,
        kubectl_program: kubectl.to_string(),
        kubectl_query_args: Vec::new(),
        kubectl_base_args: Vec::new(),
        kubectl_env: Vec::new(),
    }
}

#[tokio::test]
async fn non_root_netbridge_cross_tenant_ssh_k8s_and_namespace_rbac() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_alice = tmp.path().join("ws-alice");
    std::fs::create_dir_all(&ws_alice).unwrap();
    let alice_root = ws_alice.to_string_lossy().into_owned();
    let kubectl = write_mock_kubectl(tmp.path());

    let (ctx, _) = build_test_app(tmp.path().join("data"), &tmp.path().join("db.sqlite")).await;
    ctx.k8s_clusters.lock().unwrap().insert(
        "k8s_alice".into(),
        TestK8sEntry {
            owner_user_id: Some("u_alice".into()),
            allowed_namespaces: vec!["cbo".into()],
            spec: make_k8s_spec("k8s_alice", "STG-M", "cbo", &kubectl),
        },
    );
    ctx.k8s_clusters.lock().unwrap().insert(
        "k8s_bob".into(),
        TestK8sEntry {
            owner_user_id: Some("u_bob".into()),
            allowed_namespaces: vec!["bob-ns".into()],
            spec: make_k8s_spec("k8s_bob", "BOB-CLUSTER", "bob-ns", &kubectl),
        },
    );
    ctx.ssh_connections.lock().unwrap().insert(
        "ssh_bob".into(),
        TestSshEntry {
            owner_user_id: Some("u_bob".into()),
            spec: IdeSshSpec {
                connection_id: Id::from("ssh_bob"),
                name: "Bob Bastion".into(),
                host: "10.0.0.5".into(),
                port: Some(22),
                user: "bob".into(),
                identity_file: None,
                proxy_jump: None,
                environment: None,
            },
        },
    );

    let alice = non_root_user("u_alice");
    let app_alice = router_as(&ctx, alice.clone(), vec![ws_alice.clone()]);

    // 1. Direct `authorize_config` checks: cross-tenant SSH, cross-tenant K8s,
    // and disallowed namespace on Alice's own cluster must all return `Forbidden`.
    for bad_cfg in [
        IdeNetworkBridgeConfig {
            enabled: true,
            ssh_tunnel_connection_id: Some(Id::from("ssh_bob")),
            ..Default::default()
        },
        IdeNetworkBridgeConfig {
            enabled: true,
            k8s_cluster_id: Some(Id::from("k8s_bob")),
            k8s_namespace: Some("bob-ns".into()),
            ..Default::default()
        },
        IdeNetworkBridgeConfig {
            enabled: true,
            k8s_cluster_id: Some(Id::from("k8s_alice")),
            k8s_namespace: Some("kube-system".into()),
            ..Default::default()
        },
    ] {
        let err = ctx
            .ide
            .netbridge
            .authorize_config(&ctx, &alice, &bad_cfg)
            .await
            .expect_err("expected Forbidden on unauthorized bridge config");
        assert!(
            matches!(err, Error::Forbidden(_)),
            "expected Error::Forbidden, got: {err:?}"
        );
    }

    ctx.ide
        .netbridge
        .authorize_config(
            &ctx,
            &alice,
            &IdeNetworkBridgeConfig {
                enabled: true,
                k8s_cluster_id: Some(Id::from("k8s_alice")),
                k8s_namespace: Some("cbo".into()),
                ..Default::default()
            },
        )
        .await
        .expect("Alice's allowed cluster + namespace must pass authorize_config");

    // 2. HTTP `PUT /ide/environments` & `POST /ide/environments/bridge-test`
    // must reject cross-tenant SSH, cross-tenant K8s, and disallowed namespace.
    for bad_bridge in [
        json!({ "enabled": true, "ssh_tunnel_connection_id": "ssh_bob" }),
        json!({ "enabled": false, "ssh_tunnel_connection_id": "ssh_bob" }),
        json!({ "enabled": true, "k8s_cluster_id": "k8s_bob", "k8s_namespace": "bob-ns" }),
        json!({ "enabled": true, "k8s_cluster_id": "k8s_alice", "k8s_namespace": "kube-system" }),
    ] {
        let (st_put, body_put) = send_json(
            &app_alice,
            Method::PUT,
            "/ide/environments",
            Some(json!({
                "target": { "kind": "local", "root": alice_root },
                "network_bridge": bad_bridge,
            })),
        )
        .await;
        assert_eq!(
            st_put,
            StatusCode::FORBIDDEN,
            "expected 403 on PUT /ide/environments for {bad_bridge}: {body_put}"
        );
    }

    for bad_probe in [
        json!({ "enabled": true, "ssh_tunnel_connection_id": "ssh_bob" }),
        json!({ "enabled": true, "k8s_cluster_id": "k8s_bob", "k8s_namespace": "bob-ns" }),
        json!({ "enabled": true, "k8s_cluster_id": "k8s_alice", "k8s_namespace": "kube-system" }),
    ] {
        let (st_probe, body_probe) = send_json(
            &app_alice,
            Method::POST,
            "/ide/environments/bridge-test",
            Some(json!({
                "target": { "kind": "local", "root": alice_root },
                "bridge": bad_probe,
            })),
        )
        .await;
        assert_eq!(
            st_probe,
            StatusCode::FORBIDDEN,
            "expected 403 on POST /ide/environments/bridge-test for {bad_probe}: {body_probe}"
        );
    }

    // 3. Allowed cluster + namespace succeeds on both probe and save.
    let (st_ok_probe, ok_probe) = send_json(
        &app_alice,
        Method::POST,
        "/ide/environments/bridge-test",
        Some(json!({
            "target": { "kind": "local", "root": alice_root },
            "bridge": { "enabled": true, "k8s_cluster_id": "k8s_alice", "k8s_namespace": "cbo" },
        })),
    )
    .await;
    assert_eq!(st_ok_probe, StatusCode::OK, "allowed probe: {ok_probe}");
    assert_eq!(ok_probe["ok"], true);
    assert_eq!(ok_probe["services"][0]["name"], "confsrv");

    // 4. Stale disabled bridge tolerance: if Alice disables her bridge and later
    // loses access to `k8s_alice`, saving unrelated `env_vars` must NOT 403,
    // whereas re-enabling the stale bridge MUST 403.
    let (st_save_dis, _) = send_json(
        &app_alice,
        Method::PUT,
        "/ide/environments",
        Some(json!({
            "target": { "kind": "local", "root": alice_root },
            "network_bridge": {
                "enabled": false,
                "k8s_cluster_id": "k8s_alice",
                "k8s_namespace": "cbo",
            },
        })),
    )
    .await;
    assert_eq!(st_save_dis, StatusCode::OK);

    ctx.k8s_clusters
        .lock()
        .unwrap()
        .get_mut("k8s_alice")
        .unwrap()
        .owner_user_id = Some("u_bob".into());

    let (st_env_edit, body_env_edit) = send_json(
        &app_alice,
        Method::PUT,
        "/ide/environments",
        Some(json!({
            "target": { "kind": "local", "root": alice_root },
            "env_vars": [{ "key": "LOG_LEVEL", "value": "debug", "secret": false }],
            "network_bridge": {
                "enabled": false,
                "k8s_cluster_id": "k8s_alice",
                "k8s_namespace": "cbo",
            },
        })),
    )
    .await;
    assert_eq!(
        st_env_edit,
        StatusCode::OK,
        "saving unrelated env_vars with unchanged disabled stale bridge must succeed: {body_env_edit}"
    );

    let (st_reenable, _) = send_json(
        &app_alice,
        Method::PUT,
        "/ide/environments",
        Some(json!({
            "target": { "kind": "local", "root": alice_root },
            "network_bridge": {
                "enabled": true,
                "k8s_cluster_id": "k8s_alice",
                "k8s_namespace": "cbo",
            },
        })),
    )
    .await;
    assert_eq!(st_reenable, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn per_user_netbridge_pool_isolation_evict_user_and_drop_idle_ttl() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_alice = tmp.path().join("ws-alice");
    let ws_bob = tmp.path().join("ws-bob");
    std::fs::create_dir_all(&ws_alice).unwrap();
    std::fs::create_dir_all(&ws_bob).unwrap();
    let alice_root = ws_alice.to_string_lossy().into_owned();
    let bob_root = ws_bob.to_string_lossy().into_owned();

    let (ctx, _) = build_test_app(tmp.path().join("data"), &tmp.path().join("db.sqlite")).await;
    let alice = non_root_user("u_alice");
    let bob = non_root_user("u_bob");
    let app_alice = router_as(&ctx, alice.clone(), vec![ws_alice.clone()]);

    let bridge_cfg = IdeNetworkBridgeConfig {
        enabled: true,
        host_mappings: vec![IdeHostMapping {
            from_host: "confsrv.stg.local".into(),
            from_port: Some(8080),
            to_host: "example.com".into(),
            to_port: Some(80),
        }],
        ..Default::default()
    };

    let lease_alice = ctx
        .ide
        .netbridge
        .acquire_for_launch(
            &ctx,
            &alice,
            &IdeTargetSpec::local(alice_root.clone()),
            &bridge_cfg,
        )
        .await
        .unwrap()
        .expect("alice lease");
    let lease_bob = ctx
        .ide
        .netbridge
        .acquire_for_launch(&ctx, &bob, &IdeTargetSpec::local(bob_root), &bridge_cfg)
        .await
        .unwrap()
        .expect("bob lease");

    assert_eq!(ctx.ide.netbridge.pooled_count_for_user("u_alice"), 1);
    assert_eq!(ctx.ide.netbridge.pooled_count_for_user("u_bob"), 1);
    drop(lease_alice);
    drop(lease_bob);

    // Direct `evict_user("u_alice")` removes only Alice's pooled bridge, leaving Bob's warm.
    ctx.ide.netbridge.evict_user("u_alice");
    assert_eq!(ctx.ide.netbridge.pooled_count_for_user("u_alice"), 0);
    assert_eq!(ctx.ide.netbridge.pooled_count_for_user("u_bob"), 1);

    // Re-populate Alice's warm bridge, save enabled config, then disable via HTTP
    // `PUT /ide/environments` and verify HTTP eviction clears Alice's slot only.
    let (st_en, _) = send_json(
        &app_alice,
        Method::PUT,
        "/ide/environments",
        Some(json!({
            "target": { "kind": "local", "root": alice_root },
            "network_bridge": {
                "enabled": true,
                "host_mappings": [{
                    "from_host": "confsrv.stg.local",
                    "from_port": 8080,
                    "to_host": "example.com",
                    "to_port": 80,
                }],
            },
        })),
    )
    .await;
    assert_eq!(st_en, StatusCode::OK);

    let lease_alice_2 = ctx
        .ide
        .netbridge
        .acquire_for_launch(
            &ctx,
            &alice,
            &IdeTargetSpec::local(alice_root.clone()),
            &bridge_cfg,
        )
        .await
        .unwrap()
        .expect("alice lease 2");
    drop(lease_alice_2);
    assert_eq!(ctx.ide.netbridge.pooled_count_for_user("u_alice"), 1);

    let (st_dis, _) = send_json(
        &app_alice,
        Method::PUT,
        "/ide/environments",
        Some(json!({
            "target": { "kind": "local", "root": alice_root },
            "network_bridge": {
                "enabled": false,
                "host_mappings": [{
                    "from_host": "confsrv.stg.local",
                    "from_port": 8080,
                    "to_host": "example.com",
                    "to_port": 80,
                }],
            },
        })),
    )
    .await;
    assert_eq!(st_dis, StatusCode::OK);
    assert_eq!(ctx.ide.netbridge.pooled_count_for_user("u_alice"), 0);
    assert_eq!(ctx.ide.netbridge.pooled_count_for_user("u_bob"), 1);

    // Delayed `NetBridgeLease::drop` eviction: dropping the last active lease
    // evicts the idle bridge after `idle_ttl` even if `acquire_for_launch` is never called again.
    let short_mgr =
        NetBridgeManager::with_idle_ttl(tmp.path().join("short-ttl"), Duration::from_millis(50));
    let short_lease = short_mgr
        .acquire_for_launch(&ctx, &alice, &IdeTargetSpec::local(alice_root), &bridge_cfg)
        .await
        .unwrap()
        .expect("short ttl lease");
    assert_eq!(short_mgr.pooled_count_for_user("u_alice"), 1);
    drop(short_lease);

    let deadline = Instant::now() + Duration::from_secs(2);
    while short_mgr.pooled_count_for_user("u_alice") > 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        short_mgr.pooled_count_for_user("u_alice"),
        0,
        "NetBridgeLease::drop must schedule automatic idle eviction after idle_ttl"
    );
}

#[tokio::test]
async fn prod_netbridge_confirmation_and_idle_host_mapping_listener_teardown() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws-prod");
    std::fs::create_dir_all(&ws).unwrap();
    let ws_root = ws.to_string_lossy().into_owned();
    let kubectl = write_mock_kubectl(tmp.path());

    let (ctx, app_root) =
        build_test_app(tmp.path().join("data"), &tmp.path().join("db.sqlite")).await;
    let mut prod_spec = make_k8s_spec("k8s_prod", "PROD-EU", "cbo", &kubectl);
    prod_spec.environment = Some("production".into());
    ctx.k8s_clusters.lock().unwrap().insert(
        "k8s_prod".into(),
        TestK8sEntry {
            owner_user_id: None,
            allowed_namespaces: vec!["cbo".into()],
            spec: prod_spec,
        },
    );

    // 1. PUT /ide/environments enabling a prod K8s Network Bridge without confirm_prod_targets fails:
    let (st_no_confirm, body_no_confirm) = send_json(
        &app_root,
        Method::PUT,
        "/ide/environments",
        Some(json!({
            "target": { "kind": "local", "root": ws_root },
            "network_bridge": {
                "enabled": true,
                "k8s_cluster_id": "k8s_prod",
                "k8s_namespace": "cbo",
            },
        })),
    )
    .await;
    assert_eq!(st_no_confirm, StatusCode::BAD_REQUEST, "{body_no_confirm}");
    assert!(
        body_no_confirm
            .to_string()
            .contains("Production Network Bridge target requires confirm_prod_targets: true"),
        "expected clear prod confirmation message, got: {body_no_confirm}"
    );

    // 2. PUT /ide/environments succeeds when confirm_prod_targets: true is supplied:
    let (st_confirm, body_confirm) = send_json(
        &app_root,
        Method::PUT,
        "/ide/environments",
        Some(json!({
            "target": { "kind": "local", "root": ws_root },
            "confirm_prod_targets": true,
            "network_bridge": {
                "enabled": true,
                "k8s_cluster_id": "k8s_prod",
                "k8s_namespace": "cbo",
            },
        })),
    )
    .await;
    assert_eq!(st_confirm, StatusCode::OK, "{body_confirm}");

    // 3. POST /ide/runs and POST /ide/debug/sessions require confirm_prod_targets: true:
    let (st_run_unconfirmed, body_run_unconfirmed) = send_json(
        &app_root,
        Method::POST,
        "/ide/runs",
        Some(json!({
            "target": { "kind": "local", "root": ws_root },
            "adhoc": {
                "name": "Prod Echo",
                "runner": "shell",
                "command": "sh",
                "args": ["-c", "echo ok"],
            },
        })),
    )
    .await;
    assert_eq!(
        st_run_unconfirmed,
        StatusCode::BAD_REQUEST,
        "{body_run_unconfirmed}"
    );
    assert!(body_run_unconfirmed
        .to_string()
        .contains("confirm_prod_targets: true"));

    let script = ws.join("dbg.sh");
    std::fs::write(&script, "#!/bin/bash\necho hi\n").unwrap();
    let (st_dbg_unconfirmed, body_dbg_unconfirmed) = send_json(
        &app_root,
        Method::POST,
        "/ide/debug/sessions",
        Some(json!({
            "target": { "kind": "local", "root": ws_root },
            "language": "shell",
            "launch": { "program": script.to_string_lossy(), "cwd": ws_root },
        })),
    )
    .await;
    assert_eq!(
        st_dbg_unconfirmed,
        StatusCode::BAD_REQUEST,
        "{body_dbg_unconfirmed}"
    );
    assert!(body_dbg_unconfirmed
        .to_string()
        .contains("confirm_prod_targets: true"));

    let (st_run_confirmed, body_run_confirmed) = send_json(
        &app_root,
        Method::POST,
        "/ide/runs",
        Some(json!({
            "target": { "kind": "local", "root": ws_root },
            "confirm_prod_targets": true,
            "adhoc": {
                "name": "Prod Echo",
                "runner": "shell",
                "command": "sh",
                "args": ["-c", "echo ok"],
            },
        })),
    )
    .await;
    assert_eq!(st_run_confirmed, StatusCode::OK, "{body_run_confirmed}");

    // 4. Ephemeral localhost host_mapping TCP listener closes when active lease count hits 0
    //    and re-binds cleanly on next acquire:
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let free_port = probe.local_addr().unwrap().port();
    drop(probe);

    let alice = non_root_user("u_alice");
    let map_cfg = IdeNetworkBridgeConfig {
        enabled: true,
        host_mappings: vec![IdeHostMapping {
            from_host: "localhost".into(),
            from_port: Some(free_port),
            to_host: "127.0.0.1".into(),
            to_port: Some(9),
        }],
        ..Default::default()
    };
    let lease1 = ctx
        .ide
        .netbridge
        .acquire_for_launch(
            &ctx,
            &alice,
            &IdeTargetSpec::local(ws_root.clone()),
            &map_cfg,
        )
        .await
        .unwrap()
        .expect("lease1");
    assert!(
        tokio::net::TcpStream::connect(("127.0.0.1", free_port))
            .await
            .is_ok(),
        "localhost host_mapping listener must be bound while lease1 is active"
    );

    drop(lease1);
    let close_deadline = Instant::now() + Duration::from_secs(2);
    while tokio::net::TcpStream::connect(("127.0.0.1", free_port))
        .await
        .is_ok()
        && Instant::now() < close_deadline
    {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        tokio::net::TcpStream::connect(("127.0.0.1", free_port))
            .await
            .is_err(),
        "localhost host_mapping listener must close once active NetBridgeLease count drops to 0"
    );

    let lease2 = ctx
        .ide
        .netbridge
        .acquire_for_launch(&ctx, &alice, &IdeTargetSpec::local(ws_root), &map_cfg)
        .await
        .unwrap()
        .expect("lease2");
    assert!(
        tokio::net::TcpStream::connect(("127.0.0.1", free_port))
            .await
            .is_ok(),
        "localhost host_mapping listener must re-bind when pooled bridge is re-acquired"
    );
    drop(lease2);
}
