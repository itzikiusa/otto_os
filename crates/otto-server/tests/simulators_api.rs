//! End-to-end REST + WebSocket + RBAC integration tests for `/api/v1/simulators/*`
//! and `WS /ws/simulators/{id}/stream|logs` driven through the real daemon router
//! with deterministic `FakeCommandRunner` + `FakeHostProbe`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use otto_core::domain::{Capability, Feature};
use otto_rbac::AuthRepo;
use otto_server::ServerCtx;
use otto_simulators::android::BATCHED_ADB_PROBE_CMD;
use otto_simulators::exec::fake::{ok_out, FakeCommandRunner, FakeHostProbe};
use otto_simulators::types::decode_frame;
use otto_simulators::SimulatorsService;
use otto_state::{DbPool, GrantsRepo};
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

async fn mem_pool(dir: &std::path::Path) -> DbPool {
    let opts = SqliteConnectOptions::new()
        .filename(dir.join("sim.db"))
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(opts)
        .await
        .expect("connect sqlite");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool.into()
}

fn fake_png_bytes(w: u32, h: u32) -> Vec<u8> {
    let mut png = Vec::new();
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    png.extend_from_slice(&13u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&w.to_be_bytes());
    png.extend_from_slice(&h.to_be_bytes());
    png.extend_from_slice(&[8, 6, 0, 0, 0]);
    png
}

const SAMPLE_SIMCTL_JSON: &str = r#"{
    "runtimes": [
        {"identifier": "com.apple.CoreSimulator.SimRuntime.iOS-18-2", "name": "iOS 18.2", "version": "18.2", "isAvailable": true}
    ],
    "devicetypes": [
        {"identifier": "com.apple.CoreSimulator.SimDeviceType.iPhone-16-Pro", "name": "iPhone 16 Pro"}
    ],
    "devices": {
        "com.apple.CoreSimulator.SimRuntime.iOS-18-2": [
            {"udid": "IOS-UDID-1", "name": "iPhone 16 Pro", "state": "Booted", "isAvailable": true, "deviceTypeIdentifier": "com.apple.CoreSimulator.SimDeviceType.iPhone-16-Pro"}
        ]
    }
}"#;

fn build_ready_fakes() -> (Arc<FakeCommandRunner>, Arc<FakeHostProbe>) {
    let runner = Arc::new(FakeCommandRunner::new());
    let probe = Arc::new(FakeHostProbe::new());

    // iOS setup
    probe.add_path("/Applications/Xcode.app");
    probe.add_which("xcrun", "/usr/bin/xcrun");
    runner.on_exact(
        "xcode-select",
        &["-p"],
        ok_out(b"/Applications/Xcode.app/Contents/Developer\n"),
    );
    runner.on_exact(
        "xcodebuild",
        &["-version"],
        ok_out(b"Xcode 16.2\nBuild version 16C5032a\n"),
    );
    runner.on_exact(
        "xcrun",
        &["simctl", "list", "runtimes", "-j"],
        ok_out(
            br#"{"runtimes":[{"identifier":"com.apple.CoreSimulator.SimRuntime.iOS-18-2","name":"iOS 18.2","isAvailable":true}]}"#,
        ),
    );
    runner.on_exact(
        "xcrun",
        &["simctl", "list", "devices", "-j"],
        ok_out(
            br#"{"devices":{"com.apple.CoreSimulator.SimRuntime.iOS-18-2":[{"udid":"IOS-UDID-1","name":"iPhone 16 Pro","state":"Booted","isAvailable":true}]}}"#,
        ),
    );
    runner.on_exact(
        "xcrun",
        &["simctl", "list", "-j"],
        ok_out(SAMPLE_SIMCTL_JSON.as_bytes()),
    );
    runner.on_exact(
        "xcrun",
        &["simctl", "openurl", "IOS-UDID-1", "http://localhost:5173/"],
        ok_out(b""),
    );
    runner.on(|spec| {
        if spec.program.ends_with("xcrun")
            && spec.args.len() == 6
            && spec.args[0] == "simctl"
            && spec.args[1] == "io"
            && spec.args[2] == "IOS-UDID-1"
            && spec.args[3] == "screenshot"
            && spec.args[4] == "--type=png"
        {
            assert_ne!(spec.args[5], "-", "simctl screenshot must use temp file");
            std::fs::write(&spec.args[5], fake_png_bytes(1179, 2556)).unwrap();
            Some(ok_out(
                format!("Wrote screenshot to: {}\n", spec.args[5]).into_bytes(),
            ))
        } else {
            None
        }
    });
    runner.on_exact(
        "xcrun",
        &["simctl", "ui", "IOS-UDID-1", "appearance", "dark"],
        ok_out(b""),
    );
    runner.on_exact(
        "xcrun",
        &["simctl", "pbpaste", "IOS-UDID-1"],
        ok_out(b"copied-text"),
    );
    runner.on_exact("xcrun", &["simctl", "pbcopy", "IOS-UDID-1"], ok_out(b""));
    runner.on_exact(
        "xcrun",
        &[
            "simctl",
            "status_bar",
            "IOS-UDID-1",
            "override",
            "--time",
            "9:41",
        ],
        ok_out(b""),
    );
    runner.on_exact(
        "xcrun",
        &[
            "simctl",
            "create",
            "Test iPhone",
            "com.apple.CoreSimulator.SimDeviceType.iPhone-16-Pro",
            "com.apple.CoreSimulator.SimRuntime.iOS-18-2",
        ],
        ok_out(b"IOS-UDID-1\n"),
    );
    runner.on_exact("xcrun", &["simctl", "delete", "IOS-UDID-1"], ok_out(b""));

    // Android setup
    let sdk = PathBuf::from("/Users/tester/Library/Android/sdk");
    probe.add_path(&sdk);
    probe.add_executable(sdk.join("platform-tools/adb"));
    probe.add_executable(sdk.join("emulator/emulator"));
    probe.add_executable(sdk.join("cmdline-tools/latest/bin/avdmanager"));
    probe.add_file(
        "/Users/tester/.android/avd/Pixel_9_API_35.avd/config.ini",
        "avd.ini.displayname=Pixel 9\nhw.device.name=pixel_9\nhw.lcd.width=1080\nhw.lcd.height=2424\nhw.lcd.density=420\nimage.sysdir.1=system-images/android-35/google_apis/arm64-v8a/\n",
    );
    runner.on_exact(
        "adb",
        &["version"],
        ok_out(b"Android Debug Bridge 1.0.41\n"),
    );
    runner.on_exact("emulator", &["-list-avds"], ok_out(b"Pixel_9_API_35\n"));
    runner.on_exact(
        "adb",
        &["devices"],
        ok_out(b"List of devices attached\nemulator-5554\tdevice\n"),
    );
    runner.on_exact(
        "adb",
        &["devices", "-l"],
        ok_out(b"List of devices attached\nemulator-5554 device model:Pixel_9\n"),
    );
    runner.on_exact(
        "adb",
        &["-s", "emulator-5554", "emu", "avd", "name"],
        ok_out(b"Pixel_9_API_35\nOK\n"),
    );
    runner.on_exact(
        "adb",
        &["-s", "emulator-5554", "shell", BATCHED_ADB_PROBE_CMD],
        ok_out(
            b"15\n__OTTO__\n35\n__OTTO__\nPhysical size: 1080x2424\n__OTTO__\nPhysical density: 420\n__OTTO__\n0\n",
        ),
    );
    runner.on_exact(
        "adb",
        &["-s", "emulator-5554", "reverse", "tcp:5173", "tcp:5173"],
        ok_out(b"5173\n"),
    );
    runner.on_exact(
        "adb",
        &["-s", "emulator-5554", "reverse", "--list"],
        ok_out(b"(reverse) tcp:5173 tcp:5173\n"),
    );
    runner.on_exact(
        "adb",
        &["-s", "emulator-5554", "reverse", "--remove", "tcp:5173"],
        ok_out(b""),
    );
    runner.on_exact(
        "adb",
        &[
            "-s",
            "emulator-5554",
            "shell",
            "am",
            "start",
            "-W",
            "-a",
            "android.intent.action.VIEW",
            "-d",
            "'http://localhost:5173/dev'",
        ],
        ok_out(b"Starting: Intent\nStatus: ok\n"),
    );
    runner.on_exact(
        "adb",
        &["-s", "emulator-5554", "exec-out", "screencap", "-p"],
        ok_out(fake_png_bytes(1080, 2424)),
    );
    runner.on_exact(
        "adb",
        &[
            "-s",
            "emulator-5554",
            "shell",
            "input",
            "tap",
            "540",
            "1212",
        ],
        ok_out(b""),
    );
    runner.on_exact(
        "adb",
        &[
            "-s",
            "emulator-5554",
            "shell",
            "input",
            "keyevent",
            "KEYCODE_HOME",
        ],
        ok_out(b""),
    );
    runner.set_log_lines(vec![
        "11-19 17:30:01.100 D/Noise( 100): debug noise".into(),
        "11-19 17:30:01.200 E/AppTag( 200): critical boom".into(),
    ]);

    (runner, probe)
}

struct TestServer {
    base: String,
    ws_base: String,
    root_token: String,
    viewer_token: String,
    agent_token: String,
    http: reqwest::Client,
    _tmp: tempfile::TempDir,
}

async fn spawn_server(service: Arc<SimulatorsService>) -> TestServer {
    let tmp = tempfile::tempdir().unwrap();
    let pool = mem_pool(tmp.path()).await;
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
         VALUES ('root1', 'root1', 'x', 'Root', 1, ?),
                ('viewer1', 'viewer1', 'x', 'Viewer', 0, ?)",
    )
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();
    GrantsRepo::new(pool.clone())
        .set_grants("viewer1", &[(Feature::Browser, Capability::View)])
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, created_at) VALUES ('ws-sim-1', 'test', '/tmp', ?)",
    )
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO sessions (id, workspace_id, kind, provider, title, status, cwd, created_by, created_at, last_active_at)
         VALUES ('sim-agent-sess-1', 'ws-sim-1', 'agent', 'claude', 'test', 'idle', '/tmp', 'root1', ?, ?)",
    )
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();

    let auth_repo = AuthRepo::new(pool.clone());
    let (root_token, _) = auth_repo
        .issue_api_token(&"root1".to_string(), Some("root"))
        .await
        .unwrap();
    let (viewer_token, _) = auth_repo
        .issue_api_token(&"viewer1".to_string(), Some("viewer"))
        .await
        .unwrap();
    let (agent_token, _) = auth_repo
        .issue_session_api_token(&"root1".to_string(), &"sim-agent-sess-1".to_string())
        .await
        .unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let mut ctx = ServerCtx::for_tests(&pool, tmp.path()).await;
    ctx.base_url = base.clone();
    ctx.simulators = service;
    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx, api_extras, root_extras);
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    TestServer {
        base,
        ws_base: format!("ws://{addr}"),
        root_token,
        viewer_token,
        agent_token,
        http: reqwest::Client::new(),
        _tmp: tmp,
    }
}

#[tokio::test]
async fn simulators_rest_rbac_and_controls_end_to_end() {
    let (runner, probe) = build_ready_fakes();
    let svc = Arc::new(SimulatorsService::with_deps(runner.clone(), probe));
    let srv = spawn_server(svc).await;

    // 1. Unauthenticated → 401
    let resp = srv
        .http
        .get(format!("{}/api/v1/simulators/status", srv.base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // 2. Browser:View can read status, devices, screenshot, ports, but CANNOT mutate (403)
    let resp = srv
        .http
        .get(format!("{}/api/v1/simulators/devices", srv.base))
        .bearer_auth(&srv.viewer_token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["devices"].as_array().unwrap().len(), 2);
    assert_eq!(body["status"]["ios"]["state"], "ready");
    assert_eq!(body["status"]["android"]["state"], "ready");

    let denied = srv
        .http
        .post(format!(
            "{}/api/v1/simulators/devices/android%3Aavd%3APixel_9_API_35/open-url",
            srv.base
        ))
        .bearer_auth(&srv.viewer_token)
        .json(&json!({"url": "http://localhost:5173/dev"}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 403);

    // 3. Root/Browser:Edit can open localhost URL on Android with automatic `adb reverse`
    let opened = srv
        .http
        .post(format!(
            "{}/api/v1/simulators/devices/android%3Aavd%3APixel_9_API_35/open-url",
            srv.base
        ))
        .bearer_auth(&srv.root_token)
        .json(&json!({"url": "http://localhost:5173/dev"}))
        .send()
        .await
        .unwrap();
    assert_eq!(opened.status(), 200);
    let opened_json: Value = opened.json().await.unwrap();
    assert_eq!(opened_json["ok"], true);
    assert_eq!(opened_json["forwarded_ports"], json!([5173]));

    // 4. Screenshot binary + base64 JSON
    let bin_shot = srv
        .http
        .get(format!(
            "{}/api/v1/simulators/devices/ios%3AIOS-UDID-1/screenshot",
            srv.base
        ))
        .bearer_auth(&srv.root_token)
        .send()
        .await
        .unwrap();
    assert_eq!(bin_shot.status(), 200);
    assert_eq!(bin_shot.headers()["content-type"], "image/png");
    let png_bytes = bin_shot.bytes().await.unwrap();
    assert!(png_bytes.starts_with(b"\x89PNG"));

    let b64_shot: Value = srv
        .http
        .get(format!(
            "{}/api/v1/simulators/devices/android%3Aavd%3APixel_9_API_35/screenshot?encoding=base64",
            srv.base
        ))
        .bearer_auth(&srv.root_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(b64_shot["width"], 1080);
    assert_eq!(b64_shot["height"], 2424);
    assert!(!b64_shot["base64"].as_str().unwrap().is_empty());

    // Invalid ?encoding= rejected with 400 (`INPUT-5`)
    let bad_enc = srv
        .http
        .get(format!(
            "{}/api/v1/simulators/devices/ios%3AIOS-UDID-1/screenshot?encoding=json",
            srv.base
        ))
        .bearer_auth(&srv.root_token)
        .send()
        .await
        .unwrap();
    assert_eq!(bad_enc.status(), 400);

    // 5. Normalized Android tap + keyevent + invalid keyevent rejection
    let tap = srv
        .http
        .post(format!(
            "{}/api/v1/simulators/devices/android%3Aavd%3APixel_9_API_35/input",
            srv.base
        ))
        .bearer_auth(&srv.root_token)
        .json(&json!({"type": "tap", "x": 0.5, "y": 0.5, "normalized": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(tap.status(), 200);

    let key = srv
        .http
        .post(format!(
            "{}/api/v1/simulators/devices/android%3Aavd%3APixel_9_API_35/input",
            srv.base
        ))
        .bearer_auth(&srv.root_token)
        .json(&json!({"type": "key_event", "key": "home"}))
        .send()
        .await
        .unwrap();
    assert_eq!(key.status(), 200);

    let bad_key = srv
        .http
        .post(format!(
            "{}/api/v1/simulators/devices/android%3Aavd%3APixel_9_API_35/input",
            srv.base
        ))
        .bearer_auth(&srv.root_token)
        .json(&json!({"type": "key_event", "key": "BAD;rm -rf /"}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_key.status(), 400);

    // 6. iOS appearance, status bar, clipboard
    let app_resp = srv
        .http
        .post(format!(
            "{}/api/v1/simulators/devices/ios%3AIOS-UDID-1/appearance",
            srv.base
        ))
        .bearer_auth(&srv.root_token)
        .json(&json!({"appearance": "dark"}))
        .send()
        .await
        .unwrap();
    assert_eq!(app_resp.status(), 200);

    let sb_resp = srv
        .http
        .post(format!(
            "{}/api/v1/simulators/devices/ios%3AIOS-UDID-1/status-bar",
            srv.base
        ))
        .bearer_auth(&srv.root_token)
        .json(&json!({"time": "9:41"}))
        .send()
        .await
        .unwrap();
    assert_eq!(sb_resp.status(), 200);

    let clip: Value = srv
        .http
        .get(format!(
            "{}/api/v1/simulators/devices/ios%3AIOS-UDID-1/clipboard",
            srv.base
        ))
        .bearer_auth(&srv.root_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(clip["text"], "copied-text");

    // 7. Android port forward list & delete
    let ports: Value = srv
        .http
        .get(format!(
            "{}/api/v1/simulators/devices/android%3Aavd%3APixel_9_API_35/ports",
            srv.base
        ))
        .bearer_auth(&srv.root_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(ports["ports"][0]["remote_port"], 5173);

    let del_port = srv
        .http
        .delete(format!(
            "{}/api/v1/simulators/devices/android%3Aavd%3APixel_9_API_35/ports/5173",
            srv.base
        ))
        .bearer_auth(&srv.root_token)
        .send()
        .await
        .unwrap();
    assert_eq!(del_port.status(), 204);

    // 8. Create & delete device RBAC + happy path (`TEST-2`) and restart rejection for
    //    bare Android serial (`BUG-12`)
    let denied_create = srv
        .http
        .post(format!("{}/api/v1/simulators/devices", srv.base))
        .bearer_auth(&srv.viewer_token)
        .json(&json!({
            "platform": "ios",
            "name": "Test iPhone",
            "device_type_id": "com.apple.CoreSimulator.SimDeviceType.iPhone-16-Pro",
            "runtime_id": "com.apple.CoreSimulator.SimRuntime.iOS-18-2"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(denied_create.status(), 403);

    let created = srv
        .http
        .post(format!("{}/api/v1/simulators/devices", srv.base))
        .bearer_auth(&srv.root_token)
        .json(&json!({
            "platform": "ios",
            "name": "Test iPhone",
            "device_type_id": "com.apple.CoreSimulator.SimDeviceType.iPhone-16-Pro",
            "runtime_id": "com.apple.CoreSimulator.SimRuntime.iOS-18-2"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201);
    let created_json: Value = created.json().await.unwrap();
    assert_eq!(created_json["id"], "ios:IOS-UDID-1");

    let denied_delete = srv
        .http
        .delete(format!(
            "{}/api/v1/simulators/devices/ios%3AIOS-UDID-1",
            srv.base
        ))
        .bearer_auth(&srv.viewer_token)
        .send()
        .await
        .unwrap();
    assert_eq!(denied_delete.status(), 403);

    let deleted = srv
        .http
        .delete(format!(
            "{}/api/v1/simulators/devices/ios%3AIOS-UDID-1",
            srv.base
        ))
        .bearer_auth(&srv.root_token)
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), 204);

    // Restarting a bare Android serial fails with 400 BEFORE shutting it down (`BUG-12`)
    runner.clear_recorded();
    let restart_serial = srv
        .http
        .post(format!(
            "{}/api/v1/simulators/devices/android%3Aserial%3Aemulator-5554/restart",
            srv.base
        ))
        .bearer_auth(&srv.root_token)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(restart_serial.status(), 400);
    assert!(
        !runner
            .recorded()
            .iter()
            .any(|(_, args)| args.iter().any(|a| a == "kill")),
        "restart on bare Android serial must not issue shutdown before failing"
    );

    // 9. `SEC-1`: Agent-session credentials (even for root) are blocked (403) on Admin
    //    host-mutating simulator routes: install, cancel, create/delete device, install app
    let agent_install = srv
        .http
        .post(format!("{}/api/v1/simulators/install", srv.base))
        .bearer_auth(&srv.agent_token)
        .json(&json!({"target": "ios_runtime"}))
        .send()
        .await
        .unwrap();
    assert_eq!(agent_install.status(), 403);

    let agent_cancel = srv
        .http
        .post(format!("{}/api/v1/simulators/install/cancel", srv.base))
        .bearer_auth(&srv.agent_token)
        .send()
        .await
        .unwrap();
    assert_eq!(agent_cancel.status(), 403);

    let agent_create = srv
        .http
        .post(format!("{}/api/v1/simulators/devices", srv.base))
        .bearer_auth(&srv.agent_token)
        .json(&json!({
            "platform": "ios",
            "name": "Test iPhone",
            "device_type_id": "com.apple.CoreSimulator.SimDeviceType.iPhone-16-Pro",
            "runtime_id": "com.apple.CoreSimulator.SimRuntime.iOS-18-2"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(agent_create.status(), 403);

    let agent_delete = srv
        .http
        .delete(format!(
            "{}/api/v1/simulators/devices/ios%3AIOS-UDID-1",
            srv.base
        ))
        .bearer_auth(&srv.agent_token)
        .send()
        .await
        .unwrap();
    assert_eq!(agent_delete.status(), 403);

    let agent_app_install = srv
        .http
        .post(format!(
            "{}/api/v1/simulators/devices/ios%3AIOS-UDID-1/apps/install",
            srv.base
        ))
        .bearer_auth(&srv.agent_token)
        .json(&json!({"path": "/tmp/app.app"}))
        .send()
        .await
        .unwrap();
    assert_eq!(agent_app_install.status(), 403);

    // 10. Built-in installer endpoints: View reads status, View cannot start (403),
    //     Root starts (202), concurrent start returns 409, and cancel transitions to
    //     stage="cancelled", running=false (`TEST-1`)
    let inst_status = srv
        .http
        .get(format!("{}/api/v1/simulators/install/status", srv.base))
        .bearer_auth(&srv.viewer_token)
        .send()
        .await
        .unwrap();
    assert_eq!(inst_status.status(), 200);
    let inst_json: Value = inst_status.json().await.unwrap();
    assert_eq!(inst_json["running"], false);
    assert_eq!(inst_json["stage"], "idle");

    let denied_start = srv
        .http
        .post(format!("{}/api/v1/simulators/install", srv.base))
        .bearer_auth(&srv.viewer_token)
        .json(&json!({"target": "ios_runtime"}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied_start.status(), 403);

    runner.set_log_delay(Duration::from_secs(5));
    let started = srv
        .http
        .post(format!("{}/api/v1/simulators/install", srv.base))
        .bearer_auth(&srv.root_token)
        .json(&json!({"target": "ios_runtime"}))
        .send()
        .await
        .unwrap();
    assert_eq!(started.status(), 202);
    let started_json: Value = started.json().await.unwrap();
    assert_eq!(started_json["target"], "ios_runtime");
    assert_eq!(started_json["running"], true);

    let conflict = srv
        .http
        .post(format!("{}/api/v1/simulators/install", srv.base))
        .bearer_auth(&srv.root_token)
        .json(&json!({"target": "ios_runtime"}))
        .send()
        .await
        .unwrap();
    assert_eq!(conflict.status(), 409);

    let cancelled = srv
        .http
        .post(format!("{}/api/v1/simulators/install/cancel", srv.base))
        .bearer_auth(&srv.root_token)
        .send()
        .await
        .unwrap();
    assert_eq!(cancelled.status(), 200);
    let cancelled_json: Value = cancelled.json().await.unwrap();
    assert_eq!(cancelled_json["stage"], "cancelled");
    assert_eq!(cancelled_json["running"], false);
}

#[tokio::test]
async fn simulators_ws_stream_and_logs_end_to_end() {
    let (runner, probe) = build_ready_fakes();
    let svc = Arc::new(SimulatorsService::with_deps(runner.clone(), probe));
    let srv = spawn_server(svc).await;

    // 1. Stream WS without subprotocol → refused (401)
    let unauthed = tokio_tungstenite::connect_async(format!(
        "{}/ws/simulators/android%3Aavd%3APixel_9_API_35/stream",
        srv.ws_base
    ))
    .await;
    assert!(unauthed.is_err());

    // 2. Stream WS with `otto-bearer` subprotocol receives hello + binary PNG frame
    let mut req = format!(
        "{}/ws/simulators/android%3Aavd%3APixel_9_API_35/stream?fps=5",
        srv.ws_base
    )
    .into_client_request()
    .unwrap();
    req.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_str(&format!("otto-bearer, {}", srv.root_token)).unwrap(),
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();

    let first = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let hello: Value = match first {
        Message::Text(t) => serde_json::from_str(&t).unwrap(),
        other => panic!("expected hello text frame, got {other:?}"),
    };
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["can_drive"], true);
    assert_eq!(hello["device"]["id"], "android:avd:Pixel_9_API_35");

    // Next non-ping frame should be a binary screencast frame
    let second = loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if matches!(msg, Message::Ping(_)) {
            continue;
        }
        break msg;
    };
    match second {
        Message::Binary(bin) => {
            let (hdr, payload) = decode_frame(&bin).expect("valid binary frame envelope");
            assert_eq!(hdr.seq, 1);
            assert_eq!(hdr.mime, "image/png");
            assert_eq!((hdr.width, hdr.height), (1080, 2424));
            assert!(payload.starts_with(b"\x89PNG"));
        }
        other => panic!("expected binary frame, got {other:?}"),
    }

    // Send a drive tap over the socket and verify `adb shell input tap` actually ran (`TEST-2`)
    runner.clear_recorded();
    ws.send(Message::Text(
        r#"{"type":"tap","x":0.5,"y":0.5,"normalized":true}"#.into(),
    ))
    .await
    .unwrap();
    let mut saw_tap = false;
    for _ in 0..40 {
        if runner.recorded().iter().any(|(_, args)| {
            args == &vec![
                "-s".to_string(),
                "emulator-5554".to_string(),
                "shell".to_string(),
                "input".to_string(),
                "tap".to_string(),
                "540".to_string(),
                "1212".to_string(),
            ]
        }) {
            saw_tap = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        saw_tap,
        "WS tap must dispatch `adb -s emulator-5554 shell input tap 540 1212`"
    );
    let _ = ws.close(None).await;

    // 3. View-only (`Browser:View`) WS gets `can_drive: false`, receives `forbidden` error
    //    on drive input, and NEVER dispatches `adb shell input` (`TEST-2`)
    let mut viewer_req = format!(
        "{}/ws/simulators/android%3Aavd%3APixel_9_API_35/stream?fps=1",
        srv.ws_base
    )
    .into_client_request()
    .unwrap();
    viewer_req.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_str(&format!("otto-bearer, {}", srv.viewer_token)).unwrap(),
    );
    let (mut viewer_ws, _) = tokio_tungstenite::connect_async(viewer_req).await.unwrap();
    let viewer_hello: Value = match tokio::time::timeout(Duration::from_secs(5), viewer_ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
    {
        Message::Text(t) => serde_json::from_str(&t).unwrap(),
        other => panic!("expected viewer hello, got {other:?}"),
    };
    assert_eq!(viewer_hello["type"], "hello");
    assert_eq!(viewer_hello["can_drive"], false);

    runner.clear_recorded();
    viewer_ws
        .send(Message::Text(
            r#"{"type":"tap","x":0.5,"y":0.5,"normalized":true}"#.into(),
        ))
        .await
        .unwrap();
    let err_msg: Value = loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), viewer_ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Text(t) = msg {
            let v: Value = serde_json::from_str(&t).unwrap();
            if v["type"] == "error" {
                break v;
            }
        }
    };
    assert_eq!(err_msg["code"], "forbidden");
    assert!(
        !runner
            .recorded()
            .iter()
            .any(|(_, args)| args.iter().any(|a| a == "input")),
        "view-only WS must never execute adb shell input"
    );
    let _ = viewer_ws.close(None).await;

    // 4. Logs WS rejects invalid `?level=bogus` with HTTP 400 (`INPUT-5`)
    let mut bad_log_req = format!(
        "{}/ws/simulators/android%3Aavd%3APixel_9_API_35/logs?level=bogus",
        srv.ws_base
    )
    .into_client_request()
    .unwrap();
    bad_log_req.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_str(&format!("otto-bearer, {}", srv.viewer_token)).unwrap(),
    );
    let bad_log_err = tokio_tungstenite::connect_async(bad_log_req)
        .await
        .expect_err("invalid log level must be rejected");
    match bad_log_err {
        tokio_tungstenite::tungstenite::Error::Http(resp) => {
            assert_eq!(resp.status().as_u16(), 400);
        }
        other => panic!("expected HTTP 400 response, got {other:?}"),
    }

    // 5. Logs WS streams filtered error lines
    let mut log_req = format!(
        "{}/ws/simulators/android%3Aavd%3APixel_9_API_35/logs?level=error&filter=boom",
        srv.ws_base
    )
    .into_client_request()
    .unwrap();
    log_req.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_str(&format!("otto-bearer, {}", srv.viewer_token)).unwrap(),
    );
    let (mut log_ws, _) = tokio_tungstenite::connect_async(log_req).await.unwrap();
    let status_msg = tokio::time::timeout(Duration::from_secs(5), log_ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let status_json: Value = match status_msg {
        Message::Text(t) => serde_json::from_str(&t).unwrap(),
        other => panic!("expected status text, got {other:?}"),
    };
    assert_eq!(status_json["type"], "status");
    assert_eq!(status_json["state"], "streaming");

    let log_msg = tokio::time::timeout(Duration::from_secs(5), log_ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let log_json: Value = match log_msg {
        Message::Text(t) => serde_json::from_str(&t).unwrap(),
        other => panic!("expected log text, got {other:?}"),
    };
    assert_eq!(log_json["type"], "log");
    assert_eq!(log_json["level"], "error");
    assert_eq!(log_json["tag"], "AppTag");
    assert_eq!(log_json["pid"], 200);
    assert_eq!(log_json["message"], "critical boom");
}

/// Every `otto.sim_*` self-call, exactly as `route_for` builds it, lands on a
/// live simulator route with a body its handler accepts: a wrong path answers
/// 404, a wrong method 405 and a missing or misnamed body field 422. The logs
/// call also pins the `adb logcat` argv (an unmocked argv answers 502).
#[tokio::test]
async fn governed_sim_tool_calls_reach_the_live_simulator_routes() {
    let (runner, probe) = build_ready_fakes();
    runner.on_exact(
        "adb",
        &[
            "-s",
            "emulator-5554",
            "logcat",
            "-d",
            "-v",
            "time",
            "-t",
            "8",
        ],
        ok_out(
            b"--------- beginning of main\n11-19 17:30:01.100 D/Net( 100): noise\n\
              11-19 17:30:01.200 E/AppTag( 200): critical boom\n",
        ),
    );
    let srv = spawn_server(Arc::new(SimulatorsService::with_deps(runner, probe))).await;
    let dev = "android:avd:Pixel_9_API_35";
    let with_dev = |mut v: Value| {
        v["device_id"] = json!(dev);
        v
    };
    let mut calls = vec![
        ("sim_list", json!({"platform": "android"})),
        ("sim_logs", with_dev(json!({"lines": 2, "level": "warn"}))),
    ];
    for action in [
        json!({"action": "boot", "wait": false}),
        json!({"action": "shutdown"}),
        json!({"action": "open_url", "url": "http://localhost:5173/dev"}),
        json!({"action": "orientation", "orientation": "landscape"}),
        json!({"action": "appearance", "appearance": "dark"}),
        json!({"action": "launch_app", "app_id": "com.example.app", "args": ["-x"]}),
        json!({"action": "terminate_app", "app_id": "com.example.app"}),
    ] {
        calls.push(("sim_action", with_dev(action)));
    }
    for input in [
        json!({"type": "tap", "x": 0.5, "y": 0.5, "normalized": true}),
        json!({"type": "swipe", "x1": 0.5, "y1": 0.8, "x2": 0.5, "y2": 0.2, "duration_ms": 200, "normalized": true}),
        json!({"type": "text", "text": "hi"}),
        json!({"type": "key_event", "key": "HOME"}),
    ] {
        calls.push(("sim_input", with_dev(input)));
    }
    for (tool, args) in calls {
        let call = otto_mcp::outward::route_for(tool, &args).unwrap();
        let method = format!("{:?}", call.method).to_uppercase();
        let mut req = srv.http.request(
            reqwest::Method::from_bytes(method.as_bytes()).unwrap(),
            format!("{}{}", srv.base, call.path),
        );
        if let Some(body) = &call.body {
            req = req.json(body);
        }
        let resp = req.bearer_auth(&srv.root_token).send().await.unwrap();
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap();
        assert!(
            !matches!(status, 404 | 405 | 415 | 422),
            "{tool} {args} → {method} {}: {status} {text}",
            call.path
        );
        if tool == "sim_logs" {
            assert_eq!(status, 200, "{text}");
            let body: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(body["lines"].as_array().unwrap().len(), 1, "{body}");
            assert_eq!(body["lines"][0]["message"], "critical boom");
        }
    }
}
