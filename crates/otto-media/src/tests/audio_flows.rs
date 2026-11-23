//! Sounds and music end to end over the router, on a fake Otto Forge audio
//! stage (`forge-sfx` / `forge-music` beside a fake `forge`): progress lines, a
//! small valid WAV and its manifest, a failing mode and a slow mode for cancel.
//! No real model runs.

use super::*;

/// One script for both commands: logs its name and argv, prints progress,
/// fails on "boom", waits on "slow" (its pid in `pid`), else writes a 1 s WAV
/// and its manifest.
pub(super) const FAKE_AUDIO: &str = "#!/bin/sh\nDIR=$(dirname \"$0\")\necho \"$(basename \"$0\") $*\" >> \"$DIR/log\"\n\
while [ $# -gt 0 ]; do case \"$1\" in --prompt) P=\"$2\"; shift;; --output) OUT=\"$2\"; shift;; --loop) L=true;; esac; shift; done\n\
echo '{\"stage\":\"audio\",\"pct\":5,\"message\":\"Loading the model\"}' >&2\n\
case \"$P\" in *boom*) echo '{\"error\":\"the model ran out of memory\"}' >&2; exit 1;; esac\n\
case \"$P\" in *slow*) echo $$ > \"$DIR/pid\"; trap 'exit 130' TERM; sleep 30 & wait $!;; esac\n\
sleep 0.3\n\
echo '{\"stage\":\"audio\",\"pct\":60,\"message\":\"Generating audio.wav\"}' >&2\n\
sleep 0.3\n\
printf 'RIFF\\044\\167\\001\\000WAVEfmt \\020\\000\\000\\000\\001\\000\\001\\000\\200\\273\\000\\000\\000\\167\\001\\000\\002\\000\\020\\000data\\000\\167\\001\\000' > \"$OUT\"\n\
head -c 96000 /dev/zero >> \"$OUT\"\n\
printf '{\"tool\":\"%s\",\"model\":{\"id\":\"fake\",\"licence\":\"MIT\"},\"loop\":{\"enabled\":%s,\"seam_ok\":true}}' \"$(basename \"$0\")\" \"${L:-false}\" > \"$OUT.manifest.json\"\n";

struct Rig {
    ctx: TestCtx,
    ws_id: String,
    app: Router,
    dir: tempfile::TempDir,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

/// The built-in Otto Forge points at a fake `forge`; with `audio` the two
/// audio commands sit beside it.
async fn rig(audio: bool) -> Rig {
    let (ctx, ws_id, dd, ws_dir, _secrets) = setup_test(true).await;
    let dir = tempfile::tempdir().unwrap();
    let forge = write_script(dir.path(), "forge", "#!/bin/sh\nexit 0\n");
    if audio {
        for name in ["forge-sfx", "forge-music"] {
            write_script(dir.path(), name, FAKE_AUDIO);
        }
    }
    let app = app(&ctx);
    let (st, body) = call_json(
        &app,
        Method::PATCH,
        &format!("/design/3d/providers/{BUILTIN_OTTO_FORGE_ID}"),
        Some(json!({ "cli_binary": forge.to_str().unwrap() })),
        false,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    Rig {
        ctx,
        ws_id,
        app,
        dir,
        _dirs: (dd, ws_dir),
    }
}

impl Rig {
    async fn start(&self, mut body: Value) -> Value {
        body["workspace_id"] = json!(self.ws_id);
        let (st, job) = call_json(
            &self.app,
            Method::POST,
            "/product/audio/jobs",
            Some(body),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::ACCEPTED, "{job}");
        job
    }

    async fn settle(&self, id: &str) -> Value {
        for _ in 0..400 {
            let j = get_json(&self.app, &format!("/product/audio/jobs/{id}")).await;
            if j["status"] != "queued" && j["status"] != "running" {
                return j;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("audio job {id} never finished");
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("log")).unwrap_or_default()
    }
}

#[tokio::test]
async fn sound_and_music_jobs_run_to_clips_with_their_kind_and_manifest() {
    let rig = rig(true).await;
    let engine = get_json(&rig.app, "/product/audio/engine").await;
    assert_eq!(engine["state"], "ready", "{engine}");

    let job = rig
        .start(json!({"kind": "sound", "prompt": "wind on a ruined wall", "seconds": 20, "loop": true, "seed": 7}))
        .await;
    assert_eq!(job["status"], "queued", "{job}");
    let id = job["id"].as_str().unwrap();
    let mut seen = Vec::new();
    let done = loop {
        let j = get_json(&rig.app, &format!("/product/audio/jobs/{id}")).await;
        if j["status"] != "queued" && j["status"] != "running" {
            break j;
        }
        seen.push((
            j["progress_pct"].as_i64().unwrap(),
            j["stage"].as_str().unwrap().to_string(),
        ));
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    };
    assert_eq!(done["status"], "succeeded", "{done}");
    assert_eq!(done["progress_pct"], 100);
    assert!(
        seen.iter()
            .any(|(p, s)| *p == 60 && s == "Generating audio.wav"),
        "{seen:?}"
    );
    let clip = &done["clip"];
    assert_eq!(clip["kind"], "sound", "{clip}");
    assert_eq!(clip["id"], done["clip_id"]);
    assert_eq!(clip["duration_ms"], 1000);
    assert_eq!(clip["manifest"]["model"]["licence"], "MIT");
    assert_eq!(clip["manifest"]["loop"]["enabled"], true);
    let log = rig.log();
    assert!(
        log.contains("forge-sfx --prompt wind on a ruined wall --seconds 20 --output"),
        "{log}"
    );
    assert!(log.contains("--loop --seed 7"), "{log}");

    // Every voice-clip path works on it: the audio route serves the WAV.
    let req = Request::builder()
        .uri(format!(
            "/product/tts/clips/{}/audio",
            done["clip_id"].as_str().unwrap()
        ))
        .body(Body::empty())
        .unwrap();
    let resp = rig.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let wav = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&wav[0..4], b"RIFF");

    let music = rig
        .start(json!({"kind": "music", "prompt": "tavern folk, lute", "seconds": 32, "loop": true, "bpm": 110}))
        .await;
    let music = rig.settle(music["id"].as_str().unwrap()).await;
    assert_eq!(music["status"], "succeeded", "{music}");
    assert_eq!(music["clip"]["kind"], "music");
    let log = rig.log();
    assert!(
        log.contains("forge-music --prompt tavern folk, lute --seconds 32 --output"),
        "{log}"
    );
    assert!(log.contains("--loop --bpm 110"), "{log}");

    let ws = &rig.ws_id;
    for (kind, n) in [("sound", 1), ("music", 1), ("voice", 0)] {
        let clips = get_json(
            &rig.app,
            &format!("/product/tts/clips?workspace_id={ws}&kind={kind}"),
        )
        .await;
        assert_eq!(clips.as_array().unwrap().len(), n, "{kind}: {clips}");
        let jobs = get_json(
            &rig.app,
            &format!("/product/audio/jobs?workspace_id={ws}&kind={kind}"),
        )
        .await;
        assert_eq!(jobs.as_array().unwrap().len(), n, "{kind}: {jobs}");
    }
    let all = get_json(&rig.app, &format!("/product/tts/clips?workspace_id={ws}")).await;
    assert_eq!(all.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn a_failing_engine_fails_the_job_with_its_error_line_and_limits_are_checked_first() {
    let rig = rig(true).await;
    let job = rig
        .start(json!({"kind": "sound", "prompt": "boom", "seconds": 2}))
        .await;
    let failed = rig.settle(job["id"].as_str().unwrap()).await;
    assert_eq!(failed["status"], "failed", "{failed}");
    assert!(
        failed["error"]
            .as_str()
            .unwrap()
            .contains("the model ran out of memory"),
        "{failed}"
    );
    assert!(failed["clip_id"].is_null());

    for mut body in [
        json!({"kind": "sound", "prompt": "x", "seconds": 31}),
        json!({"kind": "sound", "prompt": "x", "seconds": 27, "loop": true}),
        json!({"kind": "music", "prompt": "x", "seconds": 5}),
        json!({"kind": "sound", "prompt": "x", "seconds": 2, "bpm": 100}),
        json!({"kind": "music", "prompt": "x", "seconds": 20, "bpm": 400}),
        json!({"kind": "speech", "prompt": "x", "seconds": 2}),
        json!({"kind": "sound", "prompt": " ", "seconds": 2}),
    ] {
        body["workspace_id"] = json!(rig.ws_id);
        let (st, resp) = call_json(
            &rig.app,
            Method::POST,
            "/product/audio/jobs",
            Some(body.clone()),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{body}: {resp}");
    }
}

/// Cancel stops the engine's process group: no process is left, no clip made.
#[tokio::test]
async fn cancel_stops_the_engine_and_leaves_no_process() {
    let rig = rig(true).await;
    let job = rig
        .start(json!({"kind": "music", "prompt": "slow drone", "seconds": 60}))
        .await;
    let id = job["id"].as_str().unwrap().to_string();
    let pid_file = rig.dir.path().join("pid");
    for _ in 0..200 {
        // Written by the engine's shell: present only once it holds the pid.
        if std::fs::read_to_string(&pid_file).is_ok_and(|s| !s.trim().is_empty()) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let pid = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .to_string();
    let (st, cancelled) = call_json(
        &rig.app,
        Method::POST,
        &format!("/product/audio/jobs/{id}/cancel"),
        None,
        false,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{cancelled}");
    assert_eq!(cancelled["status"], "cancelled");
    let pid: i32 = pid.parse().unwrap();
    // SAFETY: signal 0 only asks whether the process exists.
    let alive = || unsafe { libc::kill(pid, 0) == 0 };
    for _ in 0..80 {
        if !alive() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(!alive(), "the engine (pid {pid}) is still running");
    let done = get_json(&rig.app, &format!("/product/audio/jobs/{id}")).await;
    assert_eq!(done["status"], "cancelled", "{done}");
    let clips: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media_tts_clips")
        .fetch_one(&rig.ctx.pool)
        .await
        .unwrap();
    assert_eq!(clips, 0);
}

/// Without the audio commands beside `forge` the engine is `not_installed`: a
/// state, and a start records it rather than failing.
#[tokio::test]
async fn a_missing_engine_is_a_typed_state_not_an_error() {
    let rig = rig(false).await;
    let engine = get_json(&rig.app, "/product/audio/engine").await;
    assert_eq!(engine["state"], "not_installed", "{engine}");
    assert!(engine["message"]
        .as_str()
        .unwrap()
        .contains("--stage audio"));
    let job = rig
        .start(json!({"kind": "sound", "prompt": "rain", "seconds": 5}))
        .await;
    assert_eq!(job["status"], "not_installed", "{job}");
    assert!(
        job["error"].is_null() && job["finished_at"].is_string(),
        "{job}"
    );
    let again = get_json(
        &rig.app,
        &format!("/product/audio/jobs/{}", job["id"].as_str().unwrap()),
    )
    .await;
    assert_eq!(again["status"], "not_installed");
}

/// A Viewer reads jobs and the engine; starting and cancelling need an Editor.
#[tokio::test]
async fn audio_writes_need_an_editor() {
    let mut rig = rig(true).await;
    let job = rig
        .start(json!({"kind": "sound", "prompt": "rain", "seconds": 2}))
        .await;
    let id = job["id"].as_str().unwrap().to_string();
    rig.settle(&id).await;
    rig.ctx.roles = Arc::new(ViewerOnly);
    rig.app = app(&rig.ctx);
    let ws = rig.ws_id.clone();
    for uri in [
        "/product/audio/engine".to_string(),
        format!("/product/audio/jobs?workspace_id={ws}"),
        format!("/product/audio/jobs/{id}"),
    ] {
        get_json(&rig.app, &uri).await;
    }
    for (uri, body) in [
        (
            "/product/audio/jobs".to_string(),
            Some(json!({"workspace_id": ws, "kind": "sound", "prompt": "rain", "seconds": 2})),
        ),
        (format!("/product/audio/jobs/{id}/cancel"), None),
    ] {
        let (st, resp) = call_json(&rig.app, Method::POST, &uri, body, false).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{uri}: {resp}");
    }
}

/// Boot recovery fails a job a dead daemon left live.
#[tokio::test]
async fn restart_fails_a_live_audio_job() {
    let rig = rig(true).await;
    let job = rig
        .start(json!({"kind": "sound", "prompt": "slow rain", "seconds": 2}))
        .await;
    let id = job["id"].as_str().unwrap().to_string();
    rig.ctx.media.recover_interrupted().await.unwrap();
    let after = get_json(&rig.app, &format!("/product/audio/jobs/{id}")).await;
    assert_eq!(after["status"], "failed", "{after}");
    assert!(after["error"].as_str().unwrap().contains("restarted"));
}
