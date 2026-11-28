//! Live Unity Editor attach (rc.6 A1) without a Unity licence: a plain Mono
//! process with the soft debugger on, laid out like a Unity project
//! (`tests/fixtures/unity-mono`: `Assets/Scripts` built into
//! `Library/ScriptAssemblies/Assembly-CSharp.dll` + portable PDB, `Host/Host.exe`
//! calling `Player.Update(frame)` every 100 ms as the engine does). Mono listens on
//! Unity's Editor port `56000 + pid % 1000` and the test writes
//! `Library/EditorInstance.json` the way the Editor does. Everything goes through
//! the IDE HTTP routes (the handlers the daemon mounts) with the real DotRush
//! `monodbg`, installed by Otto's own installer (pinned, SHA-256 checked; it adds
//! the .NET 10 SDK when no .NET 10 runtime is found).
//!
//! What this cannot show: a real Unity Editor (script reload, Play mode, Release
//! code optimization, players).
//!
//! Run by hand (downloads the Unity debugger, and .NET 10 when `OTTO_LIVE_DATA`
//! has none yet):
//! `OTTO_LIVE_MONO=<mono prefix: bin/mono, lib/mono/4.5/csc.exe> OTTO_LIVE_DATA=<throwaway data dir>
//!  cargo test -p otto-ide --test unity_attach_live -- --ignored --nocapture`

#![allow(clippy::disallowed_methods)] // fixture builds and process checks in a live test

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use axum::http::Method;
use axum::Router;
use common::{build_test_app, send_json};
use serde_json::{json, Value};

const PLAYER_UPDATE_LINE: u32 = 12; // string name = "player-" + frame;
const PLAYER_COMPUTE_LINE: u32 = 14; // int doubled = Helper.Compute(frame);
const PLAYER_PRINT_LINE: u32 = 20; // Console.WriteLine(...)

struct Live {
    app: Router,
    root: PathBuf,
    mono: PathBuf,
    _tmp: tempfile::TempDir,
}

struct Editor {
    child: Child,
    pid: u32,
    out: PathBuf,
}

impl Editor {
    fn alive(&mut self) -> bool {
        self.child.try_wait().ok().flatten().is_none()
    }
    fn printed(&self) -> String {
        std::fs::read_to_string(&self.out).unwrap_or_default()
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            std::fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}

fn csc(mono: &Path, cwd: &Path, args: &[&str]) {
    let out = Command::new(mono.join("bin/mono"))
        .arg(mono.join("lib/mono/4.5/csc.exe"))
        .args(["-nologo", "-debug:portable", "-optimize-"])
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "csc: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

async fn live() -> Option<Live> {
    let (Ok(mono), Ok(data)) = (
        std::env::var("OTTO_LIVE_MONO"),
        std::env::var("OTTO_LIVE_DATA"),
    ) else {
        eprintln!("skip: set OTTO_LIVE_MONO and OTTO_LIVE_DATA");
        return None;
    };
    let mono = PathBuf::from(mono).canonicalize().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("MyGame");
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/unity-mono"),
        &root,
    );
    let sa = root.join("Library/ScriptAssemblies");
    std::fs::create_dir_all(&sa).unwrap();
    let player = root.join("Assets/Scripts/Player.cs");
    let helper = root.join("Assets/Scripts/Helper.cs");
    csc(
        &mono,
        &root,
        &[
            "-target:library",
            &format!("-out:{}", sa.join("Assembly-CSharp.dll").display()),
            &player.to_string_lossy(),
            &helper.to_string_lossy(),
        ],
    );
    csc(
        &mono,
        &root,
        &[
            &format!("-r:{}", sa.join("Assembly-CSharp.dll").display()),
            &format!("-out:{}", root.join("Host/Host.exe").display()),
            &root.join("Host/Host.cs").to_string_lossy(),
        ],
    );
    let (_ctx, app) = build_test_app(PathBuf::from(data), &tmp.path().join("state.db")).await;
    Some(Live {
        app,
        root,
        mono,
        _tmp: tmp,
    })
}

impl Live {
    fn target(&self) -> Value {
        json!({ "kind": "local", "root": self.root.to_string_lossy() })
    }

    fn bp(&self, rel: &str, line: u32) -> Value {
        json!({ "path": self.root.join(rel).to_string_lossy(), "line": line })
    }

    /// Start the stand-in Editor (`debug: false` = no soft debugger, like Release
    /// code optimization) and write `EditorInstance.json` for it.
    fn start_editor(&self, debug: bool) -> Editor {
        let out = self.root.join(format!(
            "Library/editor-{}.log",
            Instant::now().elapsed().as_nanos()
        ));
        let agent = if debug {
            "--debugger-agent=transport=dt_socket,server=y,address=127.0.0.1:$((56000 + $$ % 1000)),suspend=n"
        } else {
            ""
        };
        let script = format!(
            "exec '{}' --debug {agent} Host/Host.exe",
            self.mono.join("bin/mono").display()
        );
        let child = Command::new("/bin/sh")
            .args(["-c", &script])
            .current_dir(&self.root)
            .env("MONO_PATH", self.root.join("Library/ScriptAssemblies"))
            .stdin(Stdio::null())
            .stdout(std::fs::File::create(&out).unwrap())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        std::fs::write(
            self.root.join("Library/EditorInstance.json"),
            serde_json::to_vec_pretty(&json!({
                "process_id": pid,
                "version": "6000.0.23f1",
                "app_path": self.mono,
                "app_contents_path": self.mono.join("lib"),
            }))
            .unwrap(),
        )
        .unwrap();
        let ed = Editor { child, pid, out };
        let deadline = Instant::now() + Duration::from_secs(20);
        while !ed.printed().contains("frame=") {
            assert!(
                Instant::now() < deadline,
                "the stand-in Editor never printed a frame"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        ed
    }

    async fn instances(&self) -> Value {
        let (status, resp) = send_json(
            &self.app,
            Method::POST,
            "/ide/debug/unity/instances",
            Some(json!({ "target": self.target() })),
        )
        .await;
        assert!(status.is_success(), "instances: {status} {resp}");
        resp
    }

    async fn start(&self, bps: Vec<Value>, break_on_exceptions: bool) -> (bool, Value) {
        let body = json!({
            "target": self.target(),
            "language": "csharp",
            "launch": { "program": "unity://editor", "cwd": self.root.to_string_lossy() },
            "breakpoints": bps,
            "break_on_exceptions": break_on_exceptions,
        });
        let (status, resp) =
            send_json(&self.app, Method::POST, "/ide/debug/sessions", Some(body)).await;
        (status.is_success(), resp)
    }

    async fn snap(&self, id: &str) -> Value {
        let (_, resp) = send_json(
            &self.app,
            Method::GET,
            &format!("/ide/debug/sessions/{id}"),
            None,
        )
        .await;
        resp["session"].clone()
    }

    async fn post(&self, id: &str, what: &str, body: Value) -> Value {
        let (status, resp) = send_json(
            &self.app,
            Method::POST,
            &format!("/ide/debug/sessions/{id}/{what}"),
            Some(body),
        )
        .await;
        assert!(status.is_success(), "{what}: {status} {resp}");
        resp
    }

    async fn command(&self, id: &str, action: &str) {
        self.post(id, "command", json!({ "action": action })).await;
    }

    async fn set_bps(&self, id: &str, bps: Vec<Value>, break_on_exceptions: bool) {
        let (status, resp) = send_json(
            &self.app,
            Method::PUT,
            &format!("/ide/debug/sessions/{id}/breakpoints"),
            Some(json!({ "breakpoints": bps, "break_on_exceptions": break_on_exceptions })),
        )
        .await;
        assert!(status.is_success(), "breakpoints: {status} {resp}");
    }

    async fn wait_state(&self, id: &str, want: &[&str], secs: u64) -> Value {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let s = self.snap(id).await;
            // A stop's frames and locals land just after the state flips.
            let settled = s["state"] != "paused" || !s["paused_file"].is_null();
            if settled && want.iter().any(|w| s["state"] == *w) {
                return s;
            }
            assert!(Instant::now() < deadline, "never {want:?}: {}", brief(&s));
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }

    async fn eval(&self, id: &str, frame: &Value, expr: &str) -> Value {
        self.post(
            id,
            "evaluate",
            json!({ "expression": expr, "frame_id": frame["id"], "context": "repl" }),
        )
        .await
    }

    async fn install_unity_debugger(&self) {
        let (status, resp) = send_json(
            &self.app,
            Method::POST,
            "/ide/toolchains/install",
            Some(
                json!({ "target": self.target(), "ecosystem": "dotnet", "component": "unity_dap" }),
            ),
        )
        .await;
        assert!(status.is_success(), "install: {status} {resp}");
        let job = resp["job"]["id"].as_str().unwrap().to_string();
        let deadline = Instant::now() + Duration::from_secs(900);
        loop {
            let (_, r) = send_json(
                &self.app,
                Method::GET,
                &format!("/ide/toolchains/jobs/{job}"),
                None,
            )
            .await;
            let j = &r["job"];
            if j["status"] != "running" {
                for l in j["log_lines"].as_array().into_iter().flatten() {
                    eprintln!("install: {}", l.as_str().unwrap_or(""));
                }
                assert_eq!(j["status"], "done", "{j}");
                eprintln!("installed: {}", j["installed_path"]);
                return;
            }
            assert!(Instant::now() < deadline, "install never finished");
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

fn brief(s: &Value) -> String {
    format!(
        "state={} paused={}:{} reason={} error={}",
        s["state"],
        s["paused_file"]
            .as_str()
            .unwrap_or("")
            .rsplit('/')
            .next()
            .unwrap_or(""),
        s["paused_line"],
        s["stopped_reason"],
        s["error"],
    )
}

fn console(s: &Value) -> String {
    s["output"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|o| o["text"].as_str())
        .collect()
}

fn locals(s: &Value) -> Vec<Value> {
    s["scopes"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|sc| sc["variables"].as_array().cloned().unwrap_or_default())
        .collect()
}

fn local<'a>(vars: &'a [Value], name: &str) -> &'a Value {
    vars.iter()
        .find(|v| v["name"] == name)
        .unwrap_or_else(|| panic!("no local {name}: {vars:?}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "live: needs mono and network for the Unity debugger (see the module doc)"]
async fn unity_editor_attach_against_a_mono_stand_in() {
    let Some(l) = live().await else { return };
    l.install_unity_debugger().await;

    // U1: discovery finds the running Editor with Unity's port formula.
    let mut ed = l.start_editor(true);
    let inst = l.instances().await;
    eprintln!("U1 instances: {inst}");
    assert_eq!(inst["editors"][0]["pid"], json!(ed.pid), "{inst}");
    assert_eq!(inst["editors"][0]["version"], "6000.0.23f1");
    assert_eq!(inst["editors"][0]["port"], json!(56000 + ed.pid % 1000));

    // U2: attach + a line breakpoint in Player.Update.
    let (ok, resp) = l
        .start(
            vec![l.bp("Assets/Scripts/Player.cs", PLAYER_UPDATE_LINE)],
            false,
        )
        .await;
    assert!(ok, "start: {resp}");
    let id = resp["session"]["id"].as_str().unwrap().to_string();
    let s = l
        .wait_state(&id, &["paused", "terminated", "error"], 60)
        .await;
    eprintln!("U2 {}", brief(&s));
    let text = console(&s);
    for line in text.lines().filter(|t| t.contains("Unity")) {
        eprintln!("U2 console: {line}");
    }
    assert_eq!(s["state"], "paused", "{}\n{text}", brief(&s));
    assert!(s["paused_file"]
        .as_str()
        .unwrap()
        .ends_with("Assets/Scripts/Player.cs"));
    assert_eq!(s["paused_line"], json!(PLAYER_UPDATE_LINE));
    assert!(
        text.contains("Unity Editor attached. Press Play in Unity"),
        "{text}"
    );
    let frames = s["stack_frames"].as_array().unwrap().clone();
    eprintln!(
        "U2 stack: {:?}",
        frames
            .iter()
            .map(|f| f["name"].as_str().unwrap_or(""))
            .collect::<Vec<_>>()
    );
    let top = frames[0].clone();

    // U3: locals and set value.
    let vars = locals(&s);
    let frame_no: i64 = local(&vars, "frame")["value"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    eprintln!(
        "U3 frame={frame_no} vars={:?}",
        vars.iter()
            .map(|v| format!("{}={}", v["name"], v["value"]))
            .collect::<Vec<_>>()
    );
    assert_eq!(local(&vars, "frame")["var_type"], "int");
    let hist = l.eval(&id, &top, "history").await;
    eprintln!("U3 history = {} ({})", hist["result"], hist["var_type"]);
    let kids = l
        .post(
            &id,
            "variables",
            json!({ "variables_reference": hist["variables_reference"] }),
        )
        .await;
    eprintln!(
        "U3 history children: {}",
        kids["variables"].as_array().map(|a| a.len()).unwrap_or(0)
    );
    assert!(
        hist["variables_reference"].as_i64().unwrap_or(0) > 0,
        "{hist}"
    );
    assert!(!kids["variables"].as_array().unwrap().is_empty(), "{kids}");

    // U5: evaluate, including a method call within the 1 s evaluation timeout.
    let t0 = Instant::now();
    let e = l.eval(&id, &top, "frame * 2").await;
    assert_eq!(e["result"], json!((frame_no * 2).to_string()), "{e}");
    let e = l.eval(&id, &top, "Helper.Describe(frame)").await;
    eprintln!(
        "U5 Helper.Describe(frame) = {} in {:?}",
        e["result"],
        t0.elapsed()
    );
    assert!(
        e["result"]
            .as_str()
            .unwrap()
            .contains(&format!("x={frame_no}")),
        "{e}"
    );

    // U4: step over to the Compute call, step into Helper.Compute (other file), step out.
    l.command(&id, "step_over").await;
    l.wait_state(&id, &["paused"], 20).await;
    l.command(&id, "step_over").await;
    let s = l.wait_state(&id, &["paused"], 20).await;
    assert_eq!(
        s["paused_line"],
        json!(PLAYER_COMPUTE_LINE),
        "{}",
        brief(&s)
    );
    l.command(&id, "step_into").await;
    let s = l.wait_state(&id, &["paused"], 20).await;
    eprintln!("U4 step into: {}", brief(&s));
    assert!(
        s["paused_file"]
            .as_str()
            .unwrap()
            .ends_with("Assets/Scripts/Helper.cs"),
        "{}",
        brief(&s)
    );
    l.command(&id, "step_out").await;
    let s = l.wait_state(&id, &["paused"], 20).await;
    eprintln!("U4 step out: {}", brief(&s));
    assert!(
        s["paused_file"]
            .as_str()
            .unwrap()
            .ends_with("Assets/Scripts/Player.cs"),
        "{}",
        brief(&s)
    );

    // U3 (set value): speed = 7 shows in the next printed line.
    let s = l.snap(&id).await;
    let statics = l.eval(&id, &s["stack_frames"][0], "speed").await;
    eprintln!("U3 speed before: {}", statics["result"]);
    let set = l.eval(&id, &s["stack_frames"][0], "speed = 7").await;
    eprintln!("U3 speed = 7 -> {}", set["result"]);

    // U6: conditional breakpoint + logpoint (log does not stop).
    l.set_bps(
        &id,
        vec![
            json!({ "path": l.root.join("Assets/Scripts/Player.cs").to_string_lossy(), "line": PLAYER_UPDATE_LINE, "condition": "frame % 7 == 3" }),
            json!({ "path": l.root.join("Assets/Scripts/Player.cs").to_string_lossy(), "line": PLAYER_PRINT_LINE, "log_message": "LP frame {frame}" }),
        ],
        false,
    )
    .await;
    l.command(&id, "continue").await;
    let s = l.wait_state(&id, &["paused"], 30).await;
    let f: i64 = local(&locals(&s), "frame")["value"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    eprintln!("U6 conditional stop at frame={f} {}", brief(&s));
    assert_eq!(f % 7, 3);
    assert_eq!(s["paused_line"], json!(PLAYER_UPDATE_LINE));
    let lp: Vec<String> = console(&s)
        .lines()
        .filter(|t| t.contains("LP frame"))
        .map(str::to_string)
        .collect();
    eprintln!("U6 logpoint lines: {lp:?}");
    assert!(!lp.is_empty(), "no logpoint output");
    let printed = ed.printed();
    let last = printed
        .lines()
        .rev()
        .find(|t| t.starts_with("frame="))
        .unwrap_or("");
    eprintln!("U3 last printed line: {last}");
    assert!(last.contains("speed=7"), "{last}");

    // U7: exception breakpoint ("all exceptions") on a thrown-and-caught exception.
    l.set_bps(&id, vec![], true).await;
    l.command(&id, "continue").await;
    let s = l.wait_state(&id, &["paused"], 30).await;
    eprintln!("U7 {}", brief(&s));
    assert_eq!(s["stopped_reason"], "exception", "{}", brief(&s));

    // U8: a breakpoint added while running binds and hits.
    l.set_bps(&id, vec![], false).await;
    l.command(&id, "continue").await;
    l.wait_state(&id, &["running"], 10).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    l.set_bps(&id, vec![l.bp("Assets/Scripts/Helper.cs", 5)], false)
        .await;
    let s = l.wait_state(&id, &["paused"], 30).await;
    eprintln!("U8 {}", brief(&s));
    assert!(s["paused_file"].as_str().unwrap().ends_with("Helper.cs"));

    // U9: Stop detaches: the session ends, the Editor keeps running and printing.
    l.command(&id, "stop").await;
    let s = l.wait_state(&id, &["terminated"], 20).await;
    eprintln!("U9 after stop: {}", brief(&s));
    let before = ed
        .printed()
        .lines()
        .filter(|t| t.starts_with("frame="))
        .count();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let after = ed
        .printed()
        .lines()
        .filter(|t| t.starts_with("frame="))
        .count();
    eprintln!(
        "U9 editor alive={} frames printed in 2 s after Stop: {}",
        ed.alive(),
        after - before
    );
    assert!(ed.alive(), "Stop killed the Editor");
    assert!(
        after > before + 5,
        "the Editor stopped printing: {before} -> {after}"
    );

    // U9: re-attach works.
    let (ok, resp) = l
        .start(
            vec![l.bp("Assets/Scripts/Player.cs", PLAYER_UPDATE_LINE)],
            false,
        )
        .await;
    assert!(ok, "re-attach: {resp}");
    let id2 = resp["session"]["id"].as_str().unwrap().to_string();
    let s = l.wait_state(&id2, &["paused"], 60).await;
    eprintln!("U9 re-attach {}", brief(&s));
    l.command(&id2, "stop").await;
    l.wait_state(&id2, &["terminated"], 20).await;
    assert!(ed.alive(), "the second Stop killed the Editor");

    // An Editor that exited: discovery is empty and attach says so plainly.
    drop(ed);
    let inst = l.instances().await;
    eprintln!("exited: instances {inst}");
    assert_eq!(inst["editors"], json!([]));
    let (ok, resp) = l.start(vec![], false).await;
    eprintln!("exited: start ok={ok} {resp}");
    let msg = if ok {
        let id3 = resp["session"]["id"].as_str().unwrap().to_string();
        l.wait_state(&id3, &["error", "terminated"], 20).await["error"].to_string()
    } else {
        resp.to_string()
    };
    assert!(
        msg.contains("No Unity Editor has this project open"),
        "{msg}"
    );

    // A Mono without the soft debugger (Unity's Release code optimization): a clear message.
    let _release = l.start_editor(false);
    let (ok, resp) = l.start(vec![], false).await;
    let msg = if ok {
        let id4 = resp["session"]["id"].as_str().unwrap().to_string();
        // The adapter's `terminated` lands first; the start's error follows.
        let s = l.wait_state(&id4, &["error"], 30).await;
        format!("{} {}", s["error"], console(&s))
    } else {
        resp.to_string()
    };
    eprintln!("release: {msg}");
    assert!(
        msg.contains("Couldn't attach to the Unity Editor on port"),
        "{msg}"
    );
}
