//! Smart mode (productions) end to end over the router: a fake planner (no
//! agent provider), the real 3D job path on a fake Otto Forge CLI, fake TTS.

use std::collections::VecDeque;

use super::*;
use crate::productions::tests::good_plan;
use crate::productions::ProductionPlanner;

/// Answers each planner turn with the next queued reply; records the prompts.
#[derive(Default)]
pub(super) struct FakePlanner {
    replies: Mutex<VecDeque<String>>,
    pub(super) prompts: Mutex<Vec<String>>,
}

impl FakePlanner {
    fn queue(&self, replies: &[String]) {
        self.replies.lock().unwrap().extend(replies.iter().cloned());
    }
}

#[async_trait]
impl ProductionPlanner for FakePlanner {
    async fn run(&self, _production_id: &str, prompt: String) -> Result<String> {
        self.prompts.lock().unwrap().push(prompt);
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| Error::Upstream("no agent".into()))
    }
}

/// A fake `forge` (in `bin/`, its work folder `../out/work` like Otto
/// Forge's): answers `generate --help` with $HELP, logs each run's start,
/// end and any further arguments, fails on "boom", hangs on "slow", and
/// writes a GLB (a copy of the source mesh for a stage). Generate prints
/// Forge's steps (concept, matte, generate, gameready; a "cpu" log line when
/// it reaches the CPU step), drawing `concept.png` in its work folder; on
/// "queued" it waits a second in a `heavy.sh` of its own, as Forge does for
/// a heavy slot. Rig fails like the engine on "fox" and hangs once on "stiff".
const FAKE_FORGE: &str = "#!/bin/sh\nSTAGE=\"$1\"\nPROMPT=\"$2\"\nMESH=\"$3\"\nOUT=\"$4\"\nIMAGE=\"$5\"\n\
if [ \"$PROMPT\" = --help ] || [ \"$STAGE\" = --help ]; then echo '$HELP'; exit 0; fi\n\
if [ \"$STAGE\" = concept-server ]; then echo \"start server\" >> \"$LOG\"; trap 'echo \"server stopped\" >> \"$LOG\"; exit 0' TERM\n\
printf '{\"ready\": true, \"url\": \"http://127.0.0.1:%s\"}\n' \"$3\"; while :; do sleep 0.1; done; fi\n\
if [ \"$STAGE $PROMPT\" = \"generate --prompt\" ]; then P=\"$3\"; while [ $# -gt 0 ]; do case \"$1\" in --output) O=\"$2\"; shift;; --concept-server) U=\" $2\"; shift;; --kind) K=\" kind=$2\"; shift;; --body) echo \"body concept $P $2\" >> \"$LOG\"; shift;; esac; shift; done\n\
echo \"start concept $P$U$K\" >> \"$LOG\"; sleep 0.3; printf '\\211PNG %s' \"$P\" > \"$O\"; echo \"end concept $P\" >> \"$LOG\"; exit 0; fi\n\
if [ \"$STAGE\" = library ]; then echo \"library $3 $5 $9\" >> \"$LOG\"; case \"$3\" in *library*|Ranger*|\"Key. An old\"*)\n\
printf 'glTF\\002\\000\\000\\000\\014\\000\\000\\000' > \"$7\"; printf '%s' \"$3\" >> \"$7\";\n\
echo '{\"source\": \"library\", \"id\": \"fantasy-props-megakit/chest-wood\", \"licence\": \"CC0\"}' > \"$7.manifest.json\"; exit 0;; esac\n\
echo '{\"error\": \"no library match\", \"best\": {}, \"score\": 0}' >&2; exit 1; fi\n\
if [ \"$STAGE\" = pose ]; then echo \"pose $*\" >> \"$LOG\"; case \" $* \" in *\" curl \"*) echo '{\"error\": \"pose curl is not supported for this body\"}' >&2; exit 1;; esac\n\
FWD=null; case \" $* \" in *\" --swimming-fish \"*) FWD='[-1,0,0]';; esac\n\
G=null; case \" $* \" in *\",hold \"*) G='{\"hand\": \"right\", \"style\": \"raise\", \"point\": [-0.2, 0.7, 0.3], \"axis\": [0, 0.7071, 0.7071], \"up\": [0, 0.7071, -0.7071]}';; esac\n\
while [ $# -gt 0 ]; do case \"$1\" in --mesh) M=\"$2\"; shift;; --output) O=\"$2\"; shift;; esac; shift; done; cp \"$M\" \"$O\"\n\
echo '{\"pose\": {\"names\": [\"sit\"], \"clip\": \"sit\"}, \"forward\": '\"$FWD\"', \"limb_tips\": [[0.1, 0.2, 0.3]], \"default_clip\": \"sit\", \"contact\": {\"floor_y\": 0, \"seat\": [0, 0.5, -0.1], \"feet\": \"touch\"}, \"palm\": {\"hand\": \"right\", \"point\": [-0.3, 0.9, 0.3], \"normal\": [0, 1, 0]}, \"grip\": '\"$G\"', \"bounds\": {\"min\": [-0.3, 0, -0.3], \"max\": [0.3, 1.1, 0.4]}, \"check\": {\"pass\": true}}' > \"$O.manifest.json\"; exit 0; fi\n\
WORK=\"$(dirname \"$0\")/../out/work/$(basename \"$OUT\" .glb)-x-$$\"\n\
step() { echo \"{\\\"stage\\\": \\\"$1\\\", \\\"pct\\\": $2, \\\"message\\\": \\\"$1\\\"}\" >&2; sleep \"$3\"; }\n\
echo \"start $STAGE $PROMPT\" >> \"$LOG\"\n\
case \"$PROMPT\" in *boom*) echo '{\"error\": \"the model fell apart\"}' >&2; exit 1;; esac\n\
case \"$STAGE $PROMPT\" in \"rig \"*fox*) echo '{\"error\": \"bone_names failed (exit 1): rigged.glb: no humanoid body found in the skeleton (legs too short to find the feet: [[0.1]])\"}' >&2; exit 1;; esac\n\
case \"$STAGE $PROMPT\" in \"rig \"*stiff*) if [ ! -f \"$LOG.stiff\" ]; then touch \"$LOG.stiff\"; sleep 20; fi;; esac\n\
case \"$PROMPT\" in *slow*) sleep 20;; esac\n\
if [ \"$STAGE\" = generate ]; then prev=; for arg in \"$@\"; do if [ \"$prev\" = --body ]; then echo \"body generate $PROMPT $arg\" >> \"$LOG\"; fi; prev=\"$arg\"; done; mkdir -p \"$WORK\"; if [ -n \"$IMAGE\" ]; then echo \"picture $PROMPT $(tail -c +6 \"$IMAGE\")\" >> \"$LOG\"\n\
else step concept 2 0.2; printf '\\211PNG fake' > \"$WORK/concept.png\"; fi; step matte 15 0.2\n\
case \"$PROMPT\" in *queued*) /bin/sh -c 'sleep 1; :' \"$WORK/tools/heavy.sh\" \"$WORK/\";; esac\n\
printf '[3/6] shape SLAT flow\n      [flow] [##########..........]  4/8   11.7s  ~12s left\n' > \"$WORK/generate.log\"; step generate 20 0.2\n\
case \" $* \" in *\" --skip-gameready \"*) ;; *) echo \"cpu $STAGE $PROMPT\" >> \"$LOG\"; step gameready 90 0.8;; esac; rm -rf \"$WORK\"\n\
case \" $* \" in *\" --skip-gameready --kind character \"*) mkdir -p \"$OUT.finish/raw.svviews\"; echo '{}' > \"$OUT.finish/raw.svviews/transforms.json\"\n\
printf '{\"finish\": {\"kind\": \"character\", \"inputs\": \"%s.finish\"}}' \"$(basename \"$OUT\")\" > \"$OUT.manifest.json\"; echo \"draft character $PROMPT\" >> \"$LOG\";; esac\n\
elif [ \"$STAGE\" = gameready ] && [ -f \"$MESH.finish/raw.svviews/transforms.json\" ]; then\n\
grep -q \"$(basename \"$MESH\").finish\" \"$MESH.manifest.json\" && echo \"finish character $PROMPT\" >> \"$LOG\"; rm -rf \"$MESH.finish\"; sleep 0.8\n\
elif [ \"$STAGE\" = rig ]; then sleep 0.4; elif [ \"$STAGE\" = gameready ]; then sleep 0.8; else sleep 0.1; fi\n\
if [ \"$STAGE\" = rig ]; then echo '{\"body_plan\": \"biped\", \"legs\": 2, \"body\": {\"forward\":[0,0,1]}}' > \"$OUT.manifest.json\"; fi\n\
if [ -n \"$MESH\" ]; then cp \"$MESH\" \"$OUT\"; else printf 'glTF\\002\\000\\000\\000\\014\\000\\000\\000' > \"$OUT\"; printf '%s' \"$PROMPT\" >> \"$OUT\"; fi\n\
if [ \"$STAGE\" = animate ]; then echo '{\"rig\": {\"gameready\": {\"a\": {\"b\": {\"c\": {\"d\": {\"e\": {\"f\": 1}}}}}}}}' > \"$OUT.manifest.json\"; fi\n\
echo \"end $STAGE $PROMPT\" >> \"$LOG\"\n\
shift 5\nif [ -n \"$*\" ]; then echo \"args $*\" >> \"$LOG\"; fi\n";

/// An Otto Forge with `forge pose`.
const POSE_HELP: &str = "{generate,gameready,rig,animate,library,pose}  --height-m HEIGHT_M  --longest-side-m LONGEST_SIDE_M";

/// What today's Otto Forge says it takes: sizes, a picture on its own, a
/// first look without the game-ready pass, a warm image server.
const FAST_HELP: &str = "{concept-server,generate,gameready,rig,animate,library}  --height-m H  --longest-side-m L  --concept-only  --skip-gameready  --concept-server URL";

struct Rig {
    ctx: TestCtx,
    ws_id: String,
    app: Router,
    log: std::path::PathBuf,
    _dirs: (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir),
}

impl Rig {
    /// The production's scene document.
    async fn scene(&self, p: &Value) -> Value {
        let design = self.ctx.design();
        let id = p["scene_artifact_id"].as_str().unwrap();
        let a = design.store().require_artifact(id).await.unwrap();
        let (_, bytes) = design.head_content(&a).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn new() -> Rig {
        Rig::with_help("  --height-m HEIGHT_M  --longest-side-m LONGEST_SIDE_M").await
    }

    /// A rig whose fake Forge answers `generate --help` with `help`.
    async fn with_help(help: &str) -> Rig {
        let (ctx, ws_id, dd, ws_dir, _secrets) = setup_with(false, true).await;
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("forge.log");
        let body = FAKE_FORGE
            .replace("$LOG", log.to_str().unwrap())
            .replace("$HELP", help);
        std::fs::create_dir(dir.path().join("bin")).unwrap();
        let script = write_script(&dir.path().join("bin"), "forge", &body);
        let app = app(&ctx);
        let (st, body) = call_json(
            &app,
            Method::PATCH,
            &format!("/design/3d/providers/{BUILTIN_OTTO_FORGE_ID}"),
            Some(json!({
                "cli_binary": script.to_str().unwrap(),
                "cli_args_template": ["{stage}", "{prompt}", "{mesh}", "{output}", "{image}"]
            })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{body}");
        Rig {
            ctx,
            ws_id,
            app,
            log,
            _dirs: (dd, ws_dir, dir),
        }
    }

    /// A ready production whose plan is `plan`.
    async fn ready(&self, plan: &Value) -> String {
        self.ctx.planner.queue(&[plan.to_string()]);
        let (st, p) = call_json(
            &self.app,
            Method::POST,
            "/design/productions",
            Some(json!({ "workspace_id": self.ws_id, "description": "a scene" })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{p}");
        assert_eq!(p["status"], "planning");
        let id = p["id"].as_str().unwrap().to_string();
        assert_eq!(self.settle(&id).await["status"], "ready");
        id
    }

    async fn post(&self, uri: &str, body: Value) -> Value {
        let (st, v) = call_json(&self.app, Method::POST, uri, Some(body), false).await;
        assert_eq!(st, StatusCode::OK, "{uri}: {v}");
        v
    }

    /// Poll until it is no longer planning or running.
    async fn settle(&self, id: &str) -> Value {
        for _ in 0..600 {
            let p = get_json(&self.app, &format!("/design/productions/{id}")).await;
            if p["status"] != "planning" && p["status"] != "running" {
                return p;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("production {id} never settled");
    }

    /// The fake audio stage beside the fake `forge` (its own `log` file).
    fn add_audio(&self) -> std::path::PathBuf {
        let dir = &self.log.parent().unwrap().join("bin");
        for name in ["forge-sfx", "forge-music"] {
            write_script(dir, name, super::audio_flows::FAKE_AUDIO);
        }
        dir.join("log")
    }

    fn log(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

fn item<'a>(p: &'a Value, id: &str) -> &'a Value {
    p["plan"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == id)
        .unwrap()
}

fn objects(names: &[&str]) -> Value {
    let items: Vec<Value> = names
        .iter()
        .map(|n| json!({"id": n, "kind": "object", "name": n, "description": format!("A {n}."), "enabled": true, "details": {"prompt": n}}))
        .collect();
    json!({ "title": "Props", "summary": "Some props.", "items": items, "notes": [] })
}

#[tokio::test]
async fn a_plan_that_fails_validation_is_retried_once_then_fails_plainly() {
    let rig = Rig::new().await;
    rig.ctx
        .planner
        .queue(&["I made a plan!".into(), good_plan().to_string()]);
    let (_, p) = call_json(&rig.app, Method::POST, "/design/productions",
        Some(json!({ "workspace_id": rig.ws_id, "description": "a man and a monster", "use": "phone" })), false).await;
    let done = rig.settle(p["id"].as_str().unwrap()).await;
    assert_eq!(done["status"], "ready", "{done}");
    assert_eq!(done["use"], "phone");
    assert_eq!(done["plan"]["items"].as_array().unwrap().len(), 8);
    let prompts = rig.ctx.planner.prompts.lock().unwrap().clone();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[0].contains("a man and a monster") && prompts[0].contains("phone"));
    assert!(prompts[1].contains("was refused"), "{}", prompts[1]);

    rig.ctx.planner.queue(&["nope".into(), "still nope".into()]);
    let (_, p) = call_json(
        &rig.app,
        Method::POST,
        "/design/productions",
        Some(json!({ "workspace_id": rig.ws_id, "description": "anything" })),
        false,
    )
    .await;
    let failed = rig.settle(p["id"].as_str().unwrap()).await;
    assert_eq!(failed["status"], "failed");
    assert!(
        failed["message"].as_str().unwrap().contains("wrong twice"),
        "{failed}"
    );

    let (st, _) = call_json(
        &rig.app,
        Method::POST,
        "/design/productions",
        Some(json!({ "workspace_id": rig.ws_id, "description": "x", "use": "tv" })),
        false,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

/// The models are made in plan order, one in its GPU steps at a time: the
/// next starts when one reaches its CPU step (Forge's game-ready pass) and
/// overlaps it; the character's skeleton and moves come after every model;
/// a failing item does not stop the others; voice comes from TTS, sound is
/// skipped; the scene is written from the made models.
#[tokio::test]
async fn models_overlap_one_gpu_step_at_a_time_and_skeletons_come_last() {
    let rig = Rig::new().await;
    let plan = json!({
        "title": "Sword", "summary": "A man and his sword.", "notes": [],
        "items": [
            {"id": "man", "kind": "character", "name": "Man", "description": "A man.", "enabled": true,
             "details": {"prompt": "man", "voice": "deep", "moves": ["idle"], "holds": [{"item": "chest", "socket": "back"}]}},
            {"id": "sword", "kind": "object", "name": "Sword", "description": "A sword.", "enabled": true, "details": {"prompt": "boom sword"}},
            {"id": "chest", "kind": "object", "name": "Chest", "description": "A chest.", "enabled": true, "details": {"prompt": "chest"}},
            {"id": "line", "kind": "voice_line", "name": "Man speaks", "description": "He talks.", "enabled": true,
             "details": {"speaker": "man", "text": "Ready."}},
            {"id": "clash", "kind": "sound", "name": "Clash", "description": "Metal.", "enabled": true, "details": {}},
            {"id": "rock", "kind": "object", "name": "Rock", "description": "A rock.", "enabled": false, "details": {"prompt": "rock"}}
        ],
        "layout": [{"item": "man", "place": "center"}, {"item": "chest", "place": "right"}]
    });
    let id = rig.ready(&plan).await;
    let started = rig
        .post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    assert_eq!(started["status"], "running");
    let done = rig.settle(&id).await;
    assert_eq!(done["status"], "done", "{done}");
    assert_eq!(done["message"], "Made 3 of 5; 1 could not be made.");

    let log = rig.log();
    let runs: Vec<&str> = log
        .iter()
        .filter_map(|l| l.strip_prefix("start "))
        .collect();
    assert_eq!(
        runs,
        [
            "generate man",
            "generate boom sword",
            "generate chest",
            "rig man",
            "animate man"
        ]
    );
    let at = |line: &str| log.iter().position(|l| l == line).expect(line);
    // The chest starts once the man reaches his CPU step and before he ends;
    // the sword fails at once (it has no CPU step).
    assert!(
        at("cpu generate man") < at("start generate boom sword"),
        "{log:?}"
    );
    assert!(
        at("start generate chest") < at("end generate man"),
        "{log:?}"
    );
    // Never two GPU steps at once: a model starts only after the one before
    // it reached its CPU step.
    assert!(
        at("start generate chest") > at("cpu generate man"),
        "{log:?}"
    );
    // The skeleton waits for every model; the moves follow the skeleton.
    assert!(at("start rig man") > at("end generate chest"), "{log:?}");
    assert!(at("start animate man") > at("end rig man"), "{log:?}");
    // Its moves were added (a manifest nesting its inputs' manifests is no failure).
    let man = item(&done, "man");
    assert_eq!(
        (man["status"].as_str(), man.get("message")),
        (Some("done"), None),
        "{man}"
    );
    assert_eq!(
        man["result"]["rig"],
        json!({"body_plan": "biped", "legs": 2, "forward": [0,0,1]}),
        "the rig's body plan"
    );
    let man_artifact = item(&done, "man")["result"]["artifact_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        item(&done, "sword")["message"],
        "The Sword could not be made: the model fell apart"
    );
    assert_eq!(item(&done, "chest")["status"], "done");
    let line = item(&done, "line");
    assert_eq!(line["status"], "done", "{line}");
    let clip = rig
        .ctx
        .media
        .store()
        .require_tts_clip(line["result"]["clip_id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(clip.clip.character_name.as_deref(), Some("Man"));
    assert_eq!(
        clip.clip.artifact_id.as_deref(),
        Some(man_artifact.as_str())
    );
    assert_eq!(item(&done, "clash")["status"], "skipped");
    assert!(item(&done, "clash")["message"]
        .as_str()
        .unwrap()
        .contains("sound engine"));
    assert_eq!(item(&done, "rock")["status"], "skipped");
    let notes = done["plan"]["notes"].to_string();
    assert!(!notes.contains("does not hold"), "{notes}");

    // One scene, with the two made models in it.
    let scene_id = done["scene_artifact_id"].as_str().unwrap();
    let a = rig
        .ctx
        .design()
        .store()
        .require_artifact(scene_id)
        .await
        .unwrap();
    assert_eq!(a.format, "scene3d");
    let (_, bytes) = rig.ctx.design().head_content(&a).await.unwrap();
    let doc: Value = serde_json::from_slice(&bytes).unwrap();
    let ids: Vec<&str> = doc["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["man", "chest", "ground"]);
}

#[tokio::test]
async fn cancel_stops_the_running_item_and_skips_the_rest() {
    let rig = Rig::new().await;
    let id = rig.ready(&objects(&["slow-rock", "chest"])).await;
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    for _ in 0..200 {
        if rig.log().iter().any(|l| l == "start generate slow-rock") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    rig.post(&format!("/design/productions/{id}/cancel"), json!({}))
        .await;
    let done = rig.settle(&id).await;
    assert_eq!(done["status"], "cancelled", "{done}");
    for it in ["slow-rock", "chest"] {
        assert_eq!(item(&done, it)["status"], "skipped", "{done}");
    }
    let jobs = rig
        .ctx
        .media
        .store()
        .list_3d_jobs(Some(&rig.ws_id), None, 10)
        .await
        .unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, "cancelled");
    // The chest was only looked up in the library (first, as every prop).
    assert!(!rig
        .log()
        .iter()
        .any(|l| l.starts_with("start") && l.contains("chest")));
}

/// A restart fails the item that was being made and starts the production
/// again in its old place in the line: it makes the rest (the failed item
/// again), the one started first before the one started after it.
#[tokio::test]
async fn after_a_restart_they_go_on_in_their_old_order() {
    let rig = Rig::new().await;
    let first = rig.ready(&objects(&["crate", "barrel"])).await;
    let second = rig.ready(&objects(&["rock"])).await;
    let now = chrono::Utc::now();
    for (id, ago) in [(&second, 5), (&first, 60)] {
        let mut p = rig.ctx.media.store().require_production(id).await.unwrap();
        p.status = "running".into();
        p.queued_at = Some(now - chrono::Duration::seconds(ago));
        p.plan.items[0].status = "working".into();
        rig.ctx.media.store().save_production(&p).await.unwrap();
    }

    rig.ctx.media.recover_interrupted().await.unwrap();
    let after = get_json(&rig.app, &format!("/design/productions/{second}")).await;
    assert_eq!(
        (after["status"].as_str(), after["queue_position"].as_i64()),
        (Some("running"), Some(1)),
        "{after}"
    );
    let done = rig.settle(&first).await;
    assert_eq!(done["status"], "done", "{done}");
    assert!(done["plan"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|i| i["status"] == "done"));
    assert_eq!(rig.settle(&second).await["status"], "done");
    let runs: Vec<String> = rig
        .log()
        .into_iter()
        .filter(|l| l.starts_with("start "))
        .collect();
    assert_eq!(
        runs,
        [
            "start generate crate",
            "start generate barrel",
            "start generate rock"
        ]
    );
}

/// Steering re-plans, keeps what did not change, makes only the changed and
/// added items, and a revert brings the old results back.
#[tokio::test]
async fn steering_remakes_only_what_changed_and_revert_restores_it() {
    let rig = Rig::new().await;
    let first = objects(&["crate", "barrel"]);
    let id = rig.ready(&first).await;
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let made = rig.settle(&id).await;
    let barrel_v1 = item(&made, "barrel")["result"].clone();
    let crate_v1 = item(&made, "crate")["result"].clone();

    let mut revised = objects(&["crate", "barrel", "coins"]);
    revised["items"][1]["details"]["prompt"] = json!("rusty barrel");
    rig.ctx.planner.queue(&[json!({
        "plan": revised,
        "revision": {"summary": "The barrel gets rusty and there are coins.", "changed": [], "added": [], "removed": []}
    })
    .to_string()]);
    let pins = json!([{"item_id": "barrel", "point": [0.1, 0.5, 0.0], "note": "make it rusty"}]);
    let steering = rig
        .post(
            &format!("/design/productions/{id}/steer"),
            json!({"comment": "rusty barrel and some coins", "pins": pins}),
        )
        .await;
    assert_eq!(steering["status"], "planning");
    let ready = rig.settle(&id).await;
    assert_eq!(ready["status"], "ready", "{ready}");
    let rev = &ready["revisions"][0];
    assert_eq!(rev["changed"], json!(["barrel"]));
    assert_eq!(rev["added"], json!(["coins"]));
    assert_eq!(rev["summary"], "The barrel gets rusty and there are coins.");
    assert!(rev.get("before").is_none(), "{rev}");
    assert_eq!(item(&ready, "barrel")["previous_result"], barrel_v1);
    assert_eq!(item(&ready, "crate")["result"], crate_v1);
    let prompts = rig.ctx.planner.prompts.lock().unwrap().clone();
    assert!(prompts.last().unwrap().contains("make it rusty"));

    let before_runs = rig.log().len();
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let remade = rig.settle(&id).await;
    assert_eq!(remade["status"], "done", "{remade}");
    let runs: Vec<String> = rig.log()[before_runs..]
        .iter()
        .filter(|l| l.starts_with("start "))
        .cloned()
        .collect();
    assert_eq!(
        runs,
        ["start generate rusty barrel", "start generate coins"]
    );
    assert_ne!(item(&remade, "barrel")["result"], barrel_v1);

    let reverted = rig
        .post(
            &format!("/design/productions/{id}/revert"),
            json!({"revision_id": rev["id"]}),
        )
        .await;
    assert_eq!(reverted["status"], "done", "{reverted}");
    assert_eq!(item(&reverted, "barrel")["result"], barrel_v1);
    assert!(reverted["plan"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|i| i["id"] != "coins"));
}

/// Create review v3, must-fix 2: "make it big" changes the piece's size in
/// the scene at once, without making the model again, and says so; asked to
/// make it again with nothing different, it would come back the same, so it
/// is not made and keeps its version (the screen says it came out the same).
#[tokio::test]
async fn a_size_change_is_the_scenes_and_a_same_remake_keeps_its_version() {
    let rig = Rig::new().await;
    let id = rig.ready(&objects(&["crate", "barrel"])).await;
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let made = rig.settle(&id).await;
    let barrel_v1 = item(&made, "barrel")["result"].clone();
    let small = barrel_scale(&rig, &made).await;

    let mut bigger = objects(&["crate", "barrel"]);
    bigger["items"][1]["details"]["size_m"] = json!(2.0);
    bigger["items"][1]["description"] = json!("A big barrel.");
    let runs = rig.log().len();
    rig.ctx.planner.queue(&[
        json!({"plan": bigger, "revision": {"summary": "A bigger barrel.", "changed": ["barrel"]}})
            .to_string(),
    ]);
    rig.post(
        &format!("/design/productions/{id}/steer"),
        json!({"comment": "make the barrel big"}),
    )
    .await;
    let resized = rig.settle(&id).await;
    assert_eq!(resized["status"], "done", "{resized}");
    assert_eq!(resized["revisions"][0]["changed"], json!(["barrel"]));
    assert_eq!(item(&resized, "barrel")["result"], barrel_v1);
    assert!(item(&resized, "barrel").get("previous_result").is_none());
    assert_eq!(rig.log().len(), runs, "nothing was made again");
    assert!(barrel_scale(&rig, &resized).await > small * 3.0);
    let done = resized["revisions"][0]["done"].as_array().unwrap();
    assert!(
        done.len() == 1 && done[0].as_str().unwrap().ends_with(" is bigger."),
        "{done:?}"
    );

    // "Make it again" with the same words: the same model, the same version.
    rig.ctx.planner.queue(&[
        json!({"plan": bigger, "revision": {"summary": "Again.", "changed": ["barrel"]}})
            .to_string(),
    ]);
    rig.post(
        &format!("/design/productions/{id}/steer"),
        json!({"comment": "make it again"}),
    )
    .await;
    let again = rig.settle(&id).await;
    assert_eq!(again["status"], "done", "{again}");
    assert_eq!(rig.log().len(), runs, "nothing was made again");
    assert_eq!(item(&again, "barrel")["result"], barrel_v1, "{again}");
    assert_eq!(item(&again, "barrel")["previous_result"], barrel_v1);
}

/// The barrel's scale in the production's scene.
async fn barrel_scale(rig: &Rig, p: &Value) -> f64 {
    let scene = p["scene_artifact_id"].as_str().unwrap();
    let a = rig
        .ctx
        .design()
        .store()
        .require_artifact(scene)
        .await
        .unwrap();
    let (_, bytes) = rig.ctx.design().head_content(&a).await.unwrap();
    let doc: Value = serde_json::from_slice(&bytes).unwrap();
    let barrel = doc["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == "barrel");
    barrel.unwrap()["scale"][0].as_f64().unwrap()
}

/// One steering round on the props plan with the barrel's prompt changed.
async fn steer_barrel(rig: &Rig, id: &str, prompt: &str) {
    let mut plan = objects(&["crate", "barrel"]);
    plan["items"][1]["details"]["prompt"] = json!(prompt);
    rig.ctx
        .planner
        .queue(&[json!({"plan": plan, "revision": {"summary": prompt}}).to_string()]);
    rig.post(
        &format!("/design/productions/{id}/steer"),
        json!({ "comment": prompt }),
    )
    .await;
    assert_eq!(rig.settle(id).await["status"], "ready");
}

/// Start, and stop it while `run` is being made.
async fn stop_during(rig: &Rig, id: &str, run: &str) -> Value {
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    for _ in 0..200 {
        if rig.log().iter().any(|l| l == &format!("start {run}")) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    rig.post(&format!("/design/productions/{id}/cancel"), json!({}))
        .await;
    let stopped = rig.settle(id).await;
    assert_eq!(stopped["status"], "cancelled", "{stopped}");
    stopped
}

/// Acceptance defects 7-8: a stopped production can be edited, steered and
/// reverted; a re-make is a new version of the artifact made before (also
/// after two steers in a row), and the scene embeds each item's own version,
/// so a revert shows the old model again.
#[tokio::test]
async fn a_stopped_production_can_be_steered_reverted_and_patched_and_remakes_are_versions() {
    let rig = Rig::new().await;
    let id = rig.ready(&objects(&["crate", "barrel"])).await;
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let made = rig.settle(&id).await;
    let barrel_v1 = item(&made, "barrel")["result"].clone();
    let artifact = barrel_v1["artifact_id"].as_str().unwrap().to_string();

    steer_barrel(&rig, &id, "slow barrel").await;
    let stopped = stop_during(&rig, &id, "generate slow barrel").await;
    assert_eq!(item(&stopped, "barrel")["status"], "skipped", "{stopped}");

    // Stopped: the plan can be edited and the steer reverted.
    let (st, patched) = call_json(
        &rig.app,
        Method::PATCH,
        &format!("/design/productions/{id}"),
        Some(json!({"items": [{"id": "crate", "description": "A crate."}]})),
        false,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{patched}");
    assert_eq!(patched["status"], "cancelled");
    let reverted = rig
        .post(
            &format!("/design/productions/{id}/revert"),
            json!({"revision_id": stopped["revisions"][0]["id"]}),
        )
        .await;
    assert_eq!(reverted["status"], "done", "{reverted}");
    assert_eq!(item(&reverted, "barrel")["result"], barrel_v1);

    // Steered twice around a stop, then made: a new version of the same
    // artifact, and only the barrel is made.
    steer_barrel(&rig, &id, "slow wide barrel").await;
    stop_during(&rig, &id, "generate slow wide barrel").await;
    steer_barrel(&rig, &id, "rusty barrel").await;
    let before = rig.log().len();
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let remade = rig.settle(&id).await;
    let runs: Vec<String> = rig.log()[before..]
        .iter()
        .filter(|l| l.starts_with("start "))
        .cloned()
        .collect();
    assert_eq!(runs, ["start generate rusty barrel"]);
    let barrel = &item(&remade, "barrel")["result"];
    assert_eq!(barrel["artifact_id"], artifact.as_str(), "{remade}");
    assert_ne!(barrel["version_id"], barrel_v1["version_id"]);

    // Reverting the second steer brings back the first barrel, and the scene
    // embeds that version, not the newest.
    let reverted = rig
        .post(
            &format!("/design/productions/{id}/revert"),
            json!({"revision_id": remade["revisions"][1]["id"]}),
        )
        .await;
    assert_eq!(item(&reverted, "barrel")["result"], barrel_v1, "{reverted}");
    let scene = rig
        .ctx
        .design()
        .store()
        .require_artifact(reverted["scene_artifact_id"].as_str().unwrap())
        .await
        .unwrap();
    let (_, bytes) = rig.ctx.design().head_content(&scene).await.unwrap();
    let doc: Value = serde_json::from_slice(&bytes).unwrap();
    let barrel = doc["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == "barrel")
        .unwrap();
    assert_eq!(barrel["src"], format!("otto://design/{artifact}@v1"));
}

/// A failed item being made again that is stopped stays failed (keeping
/// what it had), so the next Start tries it again.
#[tokio::test]
async fn a_stopped_retry_of_a_failed_item_stays_failed() {
    let rig = Rig::new().await;
    let id = rig.ready(&objects(&["crate", "slow-rock"])).await;
    let mut p = rig.ctx.media.store().require_production(&id).await.unwrap();
    p.plan.items[0].status = "done".into();
    p.plan.items[0].result = Some(crate::productions::ItemResult {
        clip_id: Some("c".into()),
        ..Default::default()
    });
    p.plan.items[1].status = "failed".into();
    p.plan.items[1].result = Some(crate::productions::ItemResult {
        clip_id: Some("partial".into()),
        ..Default::default()
    });
    rig.ctx.media.store().save_production(&p).await.unwrap();
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    for _ in 0..200 {
        if rig.log().iter().any(|l| l == "start generate slow-rock") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    rig.post(&format!("/design/productions/{id}/cancel"), json!({}))
        .await;
    let stopped = rig.settle(&id).await;
    let rock = item(&stopped, "slow-rock");
    assert_eq!(
        (rock["status"].as_str(), rock["message"].as_str()),
        (Some("failed"), Some("Stopped before it was made.")),
        "{stopped}"
    );
    assert_eq!(rock["result"]["clip_id"], "partial");
    assert!(
        !rig.log().iter().any(|l| l.contains("crate")),
        "a done item is kept"
    );
}

#[tokio::test]
async fn productions_take_the_workspace_role_their_route_needs() {
    let mut rig = Rig::new().await;
    let id = rig.ready(&objects(&["crate"])).await;
    rig.ctx.roles = Arc::new(ViewerOnly);
    rig.app = app(&rig.ctx);
    get_json(
        &rig.app,
        &format!("/design/productions?workspace_id={}", rig.ws_id),
    )
    .await;
    get_json(&rig.app, &format!("/design/productions/{id}")).await;
    for (method, uri, body) in [
        (
            Method::POST,
            "/design/productions".to_string(),
            json!({"workspace_id": rig.ws_id, "description": "x"}),
        ),
        (
            Method::PATCH,
            format!("/design/productions/{id}"),
            json!({"items": []}),
        ),
        (
            Method::POST,
            format!("/design/productions/{id}/start"),
            json!({}),
        ),
        (
            Method::POST,
            format!("/design/productions/{id}/cancel"),
            json!({}),
        ),
        (
            Method::POST,
            format!("/design/productions/{id}/steer"),
            json!({"comment": "more"}),
        ),
        (
            Method::POST,
            format!("/design/productions/{id}/revert"),
            json!({"revision_id": "r"}),
        ),
    ] {
        let (st, resp) = call_json(&rig.app, method.clone(), &uri, Some(body), false).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{method} {uri}: {resp}");
    }
}

#[tokio::test]
async fn a_ready_plan_can_be_edited_and_a_bad_edit_is_refused() {
    let rig = Rig::new().await;
    let id = rig.ready(&objects(&["crate", "barrel"])).await;
    let (st, p) = call_json(
        &rig.app,
        Method::PATCH,
        &format!("/design/productions/{id}"),
        Some(json!({"items": [{"id": "barrel", "enabled": false, "name": "Old barrel"}]})),
        false,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{p}");
    assert_eq!(item(&p, "barrel")["enabled"], false);
    assert_eq!(item(&p, "barrel")["name"], "Old barrel");
    let (st, _) = call_json(
        &rig.app,
        Method::PATCH,
        &format!("/design/productions/{id}"),
        Some(json!({"items": [{"id": "crate", "name": ""}]})),
        false,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let list = get_json(
        &rig.app,
        &format!("/design/productions?workspace_id={}", rig.ws_id),
    )
    .await;
    assert_eq!(list[0]["title"], "Props");
    assert!(list[0].get("plan").is_none());
}

/// Every MCP 3D / voice / Smart-mode tool's self-call (`route_for`) against
/// this router, and a job started through `otto.start_3d_job` on the fake
/// Forge followed to its result through `otto.get_3d_job`.
#[tokio::test]
async fn every_media_mcp_tool_reaches_its_route() {
    use otto_mcp::outward::{route_for, Method as M};
    let rig = Rig::new().await;
    let tool = |name: &'static str, args: Value| {
        let app = rig.app.clone();
        async move {
            let c = route_for(name, &args).unwrap();
            let method = match c.method {
                M::Get => Method::GET,
                M::Post => Method::POST,
                M::Put => Method::PUT,
                M::Patch => Method::PATCH,
                M::Delete => Method::DELETE,
            };
            let path = c.path.strip_prefix("/api/v1").unwrap().to_string();
            call_json(&app, method, &path, c.body, false).await
        }
    };
    let ws = rig.ws_id.clone();

    let (st, list) = tool("list_3d_providers", json!({})).await;
    assert_eq!(st, StatusCode::OK);
    assert!(list
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["id"] == BUILTIN_OTTO_FORGE_ID));
    let (st, job) = tool(
        "start_3d_job",
        json!({"workspace_id": ws, "prompt": "crate"}),
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED, "{job}");
    let jid = job["id"].as_str().unwrap().to_string();
    let mut done = Value::Null;
    for _ in 0..200 {
        let (st, j) = tool("get_3d_job", json!({"job_id": jid})).await;
        assert_eq!(st, StatusCode::OK);
        assert!(j["progress_pct"].as_i64().is_some() && j["stage"].is_string());
        if j["status"] == "succeeded" || j["status"] == "failed" {
            done = j;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert_eq!(done["status"], "succeeded", "{done}");
    assert_eq!(done["progress_pct"], 100);
    let aid = done["artifact_id"].as_str().unwrap().to_string();
    assert!(done["version_id"].is_string());
    let (st, jobs) = tool("list_3d_jobs", json!({"workspace_id": ws})).await;
    assert_eq!((st, jobs.as_array().unwrap().len()), (StatusCode::OK, 1));
    let (st, same) = tool("cancel_3d_job", json!({"job_id": jid})).await;
    assert_eq!(
        (st, same["status"].as_str()),
        (StatusCode::OK, Some("succeeded"))
    );
    let (st, exp) = tool(
        "export_3d_artifact",
        json!({"artifact_id": aid, "relative_path": "assets/crate.glb", "workspace_id": ws}),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{exp}");

    let (st, _) = tool("list_tts_providers", json!({})).await;
    assert_eq!(st, StatusCode::OK);
    let (st, voices) = tool(
        "list_tts_voices",
        json!({"provider_id": BUILTIN_MACOS_SAY_ID}),
    )
    .await;
    assert_eq!(
        (st, voices[0]["id"].as_str()),
        (StatusCode::OK, Some("Samantha"))
    );
    let (st, clip) = tool(
        "synthesize_voice",
        json!({"workspace_id": ws, "voice_id": "Samantha", "text": "Ready.", "character_name": "Man"}),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{clip}");
    assert!(
        clip["duration_ms"].is_number() && clip["visemes"].is_object(),
        "{clip}"
    );
    let (st, clips) = tool("list_voice_clips", json!({"workspace_id": ws})).await;
    assert_eq!((st, clips.as_array().unwrap().len()), (StatusCode::OK, 1));
    let (st, _) = tool("delete_voice_clip", json!({"clip_id": clip["id"]})).await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    rig.ctx.planner.queue(&[objects(&["rock"]).to_string()]);
    let (st, p) = tool(
        "create_production",
        json!({"workspace_id": ws, "description": "a rock"}),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{p}");
    let pid = p["id"].as_str().unwrap().to_string();
    assert_eq!(rig.settle(&pid).await["status"], "ready");
    let (st, got) = tool("get_production", json!({"production_id": pid})).await;
    assert_eq!(
        (st, got["plan"]["title"].as_str()),
        (StatusCode::OK, Some("Props"))
    );
    let (st, list) = tool("list_productions", json!({"workspace_id": ws})).await;
    assert_eq!((st, list.as_array().unwrap().len()), (StatusCode::OK, 1));
    let (st, _) = tool("start_production", json!({"production_id": pid})).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(rig.settle(&pid).await["status"], "done");
    // Change it like Create's change box, draw a piece again, undo, rebuild.
    let (st, _) = tool("rebuild_production_scene", json!({"production_id": pid})).await;
    assert_eq!(st, StatusCode::OK);
    let mut mossy = objects(&["rock"]);
    mossy["items"][0]["details"]["prompt"] = json!("mossy rock");
    rig.ctx
        .planner
        .queue(&[json!({"plan": mossy, "revision": {"summary": "A mossy rock."}}).to_string()]);
    let (st, s) = tool(
        "steer_production",
        json!({"production_id": pid, "comment": "make it mossy", "pins": [{"item_id": "rock", "note": "moss"}]}),
    )
    .await;
    assert_eq!(
        (st, s["status"].as_str()),
        (StatusCode::OK, Some("planning")),
        "{s}"
    );
    let ready = rig.settle(&pid).await;
    assert_eq!(ready["status"], "ready", "{ready}");
    let (st, r) = tool(
        "redraw_production_item",
        json!({"production_id": pid, "item_id": "rock"}),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    let rid = ready["revisions"][0]["id"].clone();
    let (st, back) = tool(
        "revert_production",
        json!({"production_id": pid, "revision_id": rid}),
    )
    .await;
    assert_eq!(
        (st, back["status"].as_str()),
        (StatusCode::OK, Some("done")),
        "{back}"
    );
    // A new scene in the same project.
    rig.ctx.planner.queue(&[objects(&["rock"]).to_string()]);
    let (st, p2) = tool(
        "create_production",
        json!({"workspace_id": ws, "description": "the rock again", "project_id": pid}),
    )
    .await;
    assert_eq!(
        (st, p2["project_id"].as_str()),
        (StatusCode::CREATED, Some(pid.as_str())),
        "{p2}"
    );
    let p2 = p2["id"].as_str().unwrap().to_string();
    rig.settle(&p2).await;
    let (_, list) = tool("list_productions", json!({"workspace_id": ws})).await;
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .all(|p| p["project_id"] == json!(pid)),
        "{list}"
    );
    let (st, _) = tool("cancel_production", json!({"production_id": pid})).await;
    assert_eq!(st, StatusCode::OK);

    // Sounds and music: start, follow through otto.get_audio_job, list, cancel.
    rig.add_audio();
    let (st, job) = tool(
        "start_music",
        json!({"workspace_id": ws, "prompt": "calm harp", "seconds": 20, "bpm": 80}),
    )
    .await;
    assert_eq!(
        (st, job["kind"].as_str()),
        (StatusCode::ACCEPTED, Some("music")),
        "{job}"
    );
    let jid = job["id"].as_str().unwrap().to_string();
    let mut done = Value::Null;
    for _ in 0..200 {
        let (st, j) = tool("get_audio_job", json!({"job_id": jid})).await;
        assert_eq!(st, StatusCode::OK);
        if j["status"] != "queued" && j["status"] != "running" {
            done = j;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert_eq!(done["status"], "succeeded", "{done}");
    assert_eq!(done["clip"]["kind"], "music");
    let (st, slow) = tool(
        "start_sound",
        json!({"workspace_id": ws, "prompt": "slow rain", "seconds": 5}),
    )
    .await;
    assert_eq!(
        (st, slow["kind"].as_str()),
        (StatusCode::ACCEPTED, Some("sound"))
    );
    let (st, jobs) = tool(
        "list_audio_jobs",
        json!({"workspace_id": ws, "kind": "music"}),
    )
    .await;
    assert_eq!((st, jobs.as_array().unwrap().len()), (StatusCode::OK, 1));
    let (st, stopped) = tool("cancel_audio_job", json!({"job_id": slow["id"]})).await;
    assert_eq!(
        (st, stopped["status"].as_str()),
        (StatusCode::OK, Some("cancelled"))
    );
}

/// `{clips}` carries a character's known moves to the CLI (once each);
/// a move the engine cannot make stays out of the argv and becomes a note.
#[tokio::test]
async fn moves_reach_clips_and_unknown_ones_become_a_note() {
    let rig = Rig::new().await;
    let (st, body) = call_json(
        &rig.app,
        Method::PATCH,
        &format!("/design/3d/providers/{BUILTIN_OTTO_FORGE_ID}"),
        Some(
            json!({"cli_args_template": ["{stage}", "{prompt}", "{mesh}", "{output}", "{image}", "{clips}"]}),
        ),
        false,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    let plan = json!({
        "title": "Man", "summary": "A man.", "notes": [],
        "items": [{"id": "man", "kind": "character", "name": "Man", "description": "A man.", "enabled": true,
                   "details": {"prompt": "man", "moves": ["walk", "jump", "walk", "idle"]}}]
    });
    let id = rig.ready(&plan).await;
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let done = rig.settle(&id).await;
    assert_eq!(item(&done, "man")["status"], "done", "{done}");
    let log = rig.log();
    assert!(log.contains(&"args walk,idle".to_string()), "{log:?}");
    assert!(!log.iter().any(|l| l.contains("jump")), "{log:?}");
    let notes = done["plan"]["notes"].to_string();
    assert!(notes.contains("The Man cannot jump yet"), "{notes}");
}

/// Generate gets each model's real size when the installed Forge takes it (a
/// character's height, an object's longest side: the plan's metres, else the
/// size rule), and nothing extra when an older Forge does not.
#[tokio::test]
async fn real_sizes_reach_forge_only_when_it_takes_them() {
    let plan = json!({
        "title": "Man", "summary": "A man.", "notes": [],
        "items": [
            {"id": "man", "kind": "character", "name": "Man", "description": "A man.", "enabled": true, "details": {"prompt": "man"}},
            {"id": "key", "kind": "object", "name": "Key", "description": "An iron key.", "enabled": true, "details": {"prompt": "iron key"}},
            {"id": "lamp", "kind": "object", "name": "Lamp", "description": "A lamp.", "enabled": true, "details": {"prompt": "lamp", "size_m": 0.4}}
        ]
    });
    for (help, want) in [
        (
            "  --height-m HEIGHT_M",
            vec![
                "args --height-m 1.8",
                "args --longest-side-m 0.2",
                "args --longest-side-m 0.4",
            ],
        ),
        ("usage: forge generate --prompt TEXT --output PATH", vec![]),
    ] {
        let rig = Rig::with_help(help).await;
        let id = rig.ready(&plan).await;
        rig.post(&format!("/design/productions/{id}/start"), json!({}))
            .await;
        let done = rig.settle(&id).await;
        assert_eq!(done["message"], Value::Null, "{done}");
        let args: Vec<String> = rig
            .log()
            .into_iter()
            .filter(|l| l.starts_with("args"))
            .collect();
        assert_eq!(args, want, "{help}");
        assert!(!rig.log().iter().any(|l| l.contains("--help")));
    }
}

#[tokio::test]
async fn swimming_fish_body_reaches_rig_and_pose_only_on_a_forge_that_accepts_it() {
    let plan = json!({
        "title": "Reef", "summary": "A swimmer.", "items": [
            {"id":"swimmer", "kind":"character", "name":"Swimmer", "description":"A striped swimmer.",
             "details":{"prompt":"striped swimmer", "body":"fish", "moves":["idle"],"pose":{"kind":"stand","look_at":"camera"}}}
        ]
    });
    for supports_fins in [true, false] {
        let mut help = FAST_HELP.replacen("library}", "library,pose}", 1);
        if supports_fins {
            help.push_str(" --swimming-fish");
        }
        let rig = Rig::with_help(&help).await;
        let id = rig.ready(&plan).await;
        rig.post(&format!("/design/productions/{id}/start"), json!({}))
            .await;
        let done = rig.settle(&id).await;
        assert_eq!(item(&done, "swimmer")["status"], "done", "{done}");
        let log = rig.log();
        let rig_end = log
            .iter()
            .position(|s| s == "end rig striped swimmer")
            .expect("swimmer rigged");
        assert_eq!(
            log.get(rig_end + 1)
                .is_some_and(|s| s == "args --swimming-fish"),
            supports_fins,
            "{log:?}"
        );
        let gaze = log
            .iter()
            .find(|s| s.starts_with("pose ") && s.contains("--look-at"))
            .expect("swimmer posed");
        assert_eq!(
            gaze.split_whitespace().any(|arg| arg == "--swimming-fish"),
            supports_fins
        );
        let manifest = &item(&done, "swimmer")["result"]["pose"]["manifest"];
        assert_eq!(
            manifest["forward"],
            if supports_fins {
                json!([-1, 0, 0])
            } else {
                Value::Null
            }
        );
    }
}

/// The built-in Otto Forge template passes `--clips {clips}`, and migration 0226
/// gives it to a row still on the old seed but leaves an edited one alone.
#[tokio::test]
async fn the_forge_template_passes_clips_and_the_migration_updates_only_the_old_seed() {
    let rig = Rig::new().await;
    let fresh: String =
        sqlx::query_scalar("SELECT cli_args_json FROM media_3d_providers WHERE id = 'otto-forge'")
            .fetch_one(&rig.ctx.pool)
            .await
            .unwrap();
    // The rig edited the template, so put the old seed back first.
    let old = r#"["{stage}","--mesh","{mesh}","--image","{image}","--prompt","{prompt}","--output","{output}","--poly-budget","{poly_budget}","--seed","{seed}"]"#;
    let migrate = include_str!("../../../otto-state/migrations/0226_otto_forge_clips.sql");
    for (start, want_clips) in [(old, true), (fresh.as_str(), false)] {
        sqlx::query("UPDATE media_3d_providers SET cli_args_json = ? WHERE id = 'otto-forge'")
            .bind(start)
            .execute(&rig.ctx.pool)
            .await
            .unwrap();
        sqlx::query(migrate).execute(&rig.ctx.pool).await.unwrap();
        let now: String = sqlx::query_scalar(
            "SELECT cli_args_json FROM media_3d_providers WHERE id = 'otto-forge'",
        )
        .fetch_one(&rig.ctx.pool)
        .await
        .unwrap();
        assert_eq!(
            now.ends_with(r#""--clips","{clips}"]"#),
            want_clips,
            "{start} -> {now}"
        );
        if !want_clips {
            assert_eq!(now, start, "an edited template is kept");
        }
    }
    // A fresh install seeds the new template.
    let (ctx, ..) = setup_test(true).await;
    let seeded: Vec<String> = ctx.media.list_3d_providers().await.unwrap()[0]
        .cli_args_template
        .clone();
    assert_eq!(seeded[seeded.len() - 2..], ["--clips", "{clips}"]);
}

/// Sound and music items are made through the sound engine in plan order,
/// one failing does not stop the next, a length past the engine's limit is
/// clamped and said, and a nonverbal line becomes a short sound.
#[tokio::test]
async fn sound_and_music_items_are_made_in_order_and_survive_one_failing() {
    let rig = Rig::new().await;
    let audio_log = rig.add_audio();
    let plan = json!({
        "title": "Cave", "summary": "A cave.", "notes": [],
        "items": [
            {"id": "man", "kind": "character", "name": "Man", "description": "A man.", "enabled": true, "details": {"prompt": "man", "voice": "deep"}},
            {"id": "drip", "kind": "sound", "name": "Drip", "description": "Water.", "enabled": true, "details": {"prompt": "boom drip", "seconds": 2}},
            {"id": "wind", "kind": "sound", "name": "Wind", "description": "Wind in a cave.", "enabled": true, "details": {"prompt": "cave wind", "seconds": 45, "loop": true}},
            {"id": "theme", "kind": "music", "name": "Theme", "description": "A dark theme.", "enabled": true, "details": {"prompt": "dark cello", "seconds": 600, "loop": true}},
            {"id": "grunt", "kind": "voice_line", "name": "Man grunts", "description": "A pained grunt.", "enabled": true, "details": {"speaker": "man", "nonverbal": true}}
        ]
    });
    let id = rig.ready(&plan).await;
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let done = rig.settle(&id).await;
    assert_eq!(
        done["message"], "Made 4 of 5; 1 could not be made.",
        "{done}"
    );
    let runs: Vec<String> = std::fs::read_to_string(&audio_log)
        .unwrap()
        .lines()
        .map(|l| l.split(" --seconds").next().unwrap().to_string())
        .collect();
    assert_eq!(
        runs,
        [
            "forge-sfx --prompt boom drip",
            "forge-sfx --prompt cave wind",
            "forge-music --prompt dark cello",
            "forge-sfx --prompt deep, A pained grunt.",
        ]
    );
    let drip = item(&done, "drip");
    assert_eq!(drip["status"], "failed");
    assert!(
        drip["message"]
            .as_str()
            .unwrap()
            .contains("the model ran out of memory"),
        "{drip}"
    );
    let wind = item(&done, "wind");
    assert_eq!(wind["status"], "done", "{wind}");
    assert_eq!(
        wind["message"],
        "Made 26.5 seconds long, not 45: a looping sound can be 0.5 to 26.5 seconds."
    );
    let theme = item(&done, "theme");
    assert!(
        theme["message"]
            .as_str()
            .unwrap()
            .contains("Made 500 seconds long"),
        "{theme}"
    );
    let full = std::fs::read_to_string(&audio_log).unwrap();
    assert!(
        full.contains("--seconds 26.5 --output") && full.contains("--seconds 500 --output"),
        "{full}"
    );
    for (it, kind) in [("wind", "sound"), ("theme", "music"), ("grunt", "sound")] {
        let cid = item(&done, it)["result"]["clip_id"]
            .as_str()
            .unwrap()
            .to_string();
        let clip = rig
            .ctx
            .media
            .store()
            .require_tts_clip(&cid)
            .await
            .unwrap()
            .clip;
        assert_eq!(clip.kind, kind, "{it}");
    }
}

/// Without the sound engine, sound and music are skipped plainly; a
/// nonverbal line with words is spoken, one without words is skipped.
#[tokio::test]
async fn without_the_sound_engine_audio_is_skipped_plainly() {
    let rig = Rig::new().await;
    let plan = json!({
        "title": "Cave", "summary": "A cave.", "notes": [],
        "items": [
            {"id": "man", "kind": "character", "name": "Man", "description": "A man.", "enabled": false, "details": {"prompt": "man"}},
            {"id": "theme", "kind": "music", "name": "Theme", "description": "A dark theme.", "enabled": true, "details": {}},
            {"id": "laugh", "kind": "voice_line", "name": "Man laughs", "description": "He laughs.", "enabled": true, "details": {"speaker": "man", "nonverbal": true, "text": "Ha ha!"}},
            {"id": "grunt", "kind": "voice_line", "name": "Man grunts", "description": "A grunt.", "enabled": true, "details": {"speaker": "man", "nonverbal": true}}
        ]
    });
    let id = rig.ready(&plan).await;
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let done = rig.settle(&id).await;
    assert_eq!(item(&done, "theme")["status"], "skipped");
    assert_eq!(
        item(&done, "theme")["message"],
        "The sound engine is not installed, so this was not made."
    );
    let laugh = item(&done, "laugh");
    assert_eq!(laugh["status"], "done", "{laugh}");
    let clip = rig
        .ctx
        .media
        .store()
        .require_tts_clip(laugh["result"]["clip_id"].as_str().unwrap())
        .await
        .unwrap()
        .clip;
    assert_eq!(
        (clip.kind.as_str(), clip.text.as_str()),
        ("voice", "Ha ha!")
    );
    assert_eq!(item(&done, "grunt")["status"], "skipped");
    assert!(item(&done, "grunt")["message"]
        .as_str()
        .unwrap()
        .contains("grunt, scream or laugh"));
}

/// A fake `forge-voice`: logs its argv, writes a small WAV, its mouth cues
/// and a manifest.
const FAKE_VOICE: &str = "#!/bin/sh\nDIR=$(dirname \"$0\")\nARGS=\"$*\"\necho \"$*\" >> \"$DIR/voice.log\"\n\
case \"$*\" in *Go!*--visemes*) echo '{\"error\": \"Failed to set up sub-word alignment\"}' >&2; exit 1;; esac\n\
while [ $# -gt 0 ]; do case \"$1\" in --output) OUT=\"$2\"; shift;; esac; shift; done\n\
printf 'RIFF\\044\\167\\001\\000WAVEfmt \\020\\000\\000\\000\\001\\000\\001\\000\\200\\273\\000\\000\\000\\167\\001\\000\\002\\000\\020\\000data\\000\\167\\001\\000' > \"$OUT\"\n\
head -c 96000 /dev/zero >> \"$OUT\"\n\
case \"$ARGS\" in *--visemes*) echo '{\"duration_sec\": 1.0, \"cues\": [{\"start\": 0.0, \"end\": 0.5, \"value\": \"A\"}]}' > \"$OUT.visemes.json\";; esac\n\
echo '{\"engine\": \"qwen\", \"voice\": \"designed\"}' > \"$OUT.manifest.json\"\n";

/// With Otto Forge's voice engine beside it, each character's lines are
/// designed from ONE description of that character (the same for all its
/// lines, another for another character) at the pace its words say, with
/// mouth cues; it speaks even with a Kokoro server turned on (the fallback).
#[tokio::test]
async fn forge_voice_designs_one_voice_per_character() {
    let rig = Rig::new().await;
    let bin = rig.log.parent().unwrap().join("bin");
    write_script(&bin, "forge-voice", FAKE_VOICE);
    let plan = json!({
        "title": "Talk", "summary": "Talk.", "notes": [],
        "items": [
            {"id": "man", "kind": "character", "name": "Old man", "description": "A man.", "enabled": false, "details": {"voice": "deep, tired, speaks slowly"}},
            {"id": "girl", "kind": "character", "name": "Girl", "description": "A girl.", "enabled": false, "details": {"voice": "bright and quick"}},
            {"id": "l1", "kind": "voice_line", "name": "One", "description": "He talks.", "enabled": true, "details": {"speaker": "man", "text": "Ready."}},
            {"id": "l2", "kind": "voice_line", "name": "Two", "description": "She talks.", "enabled": true, "details": {"speaker": "girl", "text": "Go!"}},
            {"id": "l3", "kind": "voice_line", "name": "Three", "description": "He talks.", "enabled": true, "details": {"speaker": "man", "text": "Never."}}
        ]
    });
    let id = rig.ready(&plan).await;
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let done = rig.settle(&id).await;
    let log = std::fs::read_to_string(bin.join("voice.log")).unwrap();
    // The girl's line fails to line up its mouth cues once (as the real
    // engine sometimes does) and is spoken again without them.
    assert_eq!(log.lines().count(), 4, "{log}");
    let runs: Vec<&str> = log
        .lines()
        .filter(|l| !(l.contains("Go!") && l.contains("--visemes")))
        .collect();
    let desc = |l: &str| {
        l.split("--description ")
            .nth(1)
            .unwrap()
            .split(" --speed")
            .next()
            .unwrap()
            .to_string()
    };
    assert_eq!(desc(runs[0]), "Old man. deep, tired, speaks slowly");
    assert_eq!(
        desc(runs[0]),
        desc(runs[2]),
        "the same voice for every line of his"
    );
    assert_eq!(desc(runs[1]), "Girl. bright and quick");
    assert!(
        runs[0].contains("--engine qwen --voice designed") && runs[0].contains("--speed 0.85"),
        "{log}"
    );
    assert!(
        runs[1].contains("--speed 1.1 ") && !runs[1].contains("--visemes"),
        "{log}"
    );
    for l in ["l1", "l2", "l3"] {
        let it = item(&done, l);
        assert_eq!(
            (it["status"].as_str(), it.get("message")),
            (Some("done"), None),
            "{it}"
        );
        let clip = rig
            .ctx
            .media
            .store()
            .require_tts_clip(it["result"]["clip_id"].as_str().unwrap())
            .await
            .unwrap()
            .clip;
        assert_eq!(clip.provider_kind, "otto_forge");
        assert_eq!(clip.visemes.is_some(), l != "l2", "{clip:?}");
        assert!(clip.manifest.is_some(), "{clip:?}");
    }
    // A Kokoro server the person turned on is only the fallback: the
    // designed voices still speak.
    let (st, body) = call_json(&rig.app, Method::POST, "/product/tts/providers",
        Some(json!({"name": "My Kokoro", "kind": "kokoro", "endpoint_url": "http://127.0.0.1:7811/", "allow_local_network": true, "enabled": true})), false).await;
    assert!(st.is_success(), "{st} {body}");
    let id = rig.ready(&plan).await;
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let again = rig.settle(&id).await;
    assert_eq!(item(&again, "l1")["status"], "done", "{again}");
    let log = std::fs::read_to_string(bin.join("voice.log")).unwrap();
    assert!(log.lines().count() >= 7, "{log}");
}

/// A line's own pace and mood set its speed (a whisper is slower, even from
/// a quick girl), and a line that still comes out faster than 0.28 s a word is
/// spoken once more, slowed to that floor (the real "Did you hear that?"
/// whisper came out at 0.21 s a word).
#[tokio::test]
async fn a_whisper_is_slow_and_no_line_is_too_fast() {
    let rig = Rig::new().await;
    let bin = rig.log.parent().unwrap().join("bin");
    write_script(&bin, "forge-voice", FAKE_VOICE);
    let plan = json!({
        "title": "Camp", "summary": "Camp.", "notes": [],
        "items": [
            {"id": "girl", "kind": "character", "name": "Girl", "description": "A girl.", "enabled": false, "details": {"voice": "bright, high, young girl, giggly, quick"}},
            {"id": "w", "kind": "voice_line", "name": "Whisper", "description": "She whispers.", "enabled": true,
             "details": {"speaker": "girl", "text": "Did you hear that out there?", "pace": "normal", "mood": "whispering, a little scared"}}
        ]
    });
    let id = rig.ready(&plan).await;
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let done = rig.settle(&id).await;
    let log = std::fs::read_to_string(bin.join("voice.log")).unwrap();
    let runs: Vec<&str> = log.lines().collect();
    // The fake engine speaks every line in 1 s: six words need 1.68 s.
    assert_eq!(runs.len(), 2, "{log}");
    assert!(
        runs[0].contains("--speed 0.85 "),
        "a whisper is slow: {log}"
    );
    assert!(
        runs[1].contains("--speed 0.7 "),
        "slowed to the floor: {log}"
    );
    let clip = rig
        .ctx
        .media
        .store()
        .require_tts_clip(item(&done, "w")["result"]["clip_id"].as_str().unwrap())
        .await
        .unwrap()
        .clip;
    assert!((clip.speed - 0.7).abs() < 1e-6, "{clip:?}");
}

/// The two drumsticks must aim at different skins in the actual Forge pose call.
#[tokio::test]
async fn a_two_handed_percussion_pose_sends_distinct_targets_to_forge() {
    let rig = Rig::with_help(POSE_HELP).await;
    let plan = json!({"title":"Drum rehearsal","summary":"A player at a kit.","notes":[],
        "items":[
            {"id":"kit","kind":"object","name":"Drum kit","description":"Two drums.","details":{"prompt":"drums","dims_m":[1.2,0.6,0.8]}},
            {"id":"player","kind":"character","name":"Drummer","description":"Standing.","details":{"prompt":"drummer","body":"two_legs","moves":["idle"]}},
            {"id":"l","kind":"object","name":"Left drumstick","description":"Slim stick.","details":{"prompt":"left drumstick"}},
            {"id":"r","kind":"object","name":"Right drumstick","description":"Slim stick.","details":{"prompt":"right drumstick"}}],
        "layout":[{"item":"kit","place":"center"},{"item":"player","facing":"kit","rel":{"to":"kit","how":"behind"}},
                  {"item":"l","rel":{"to":"player","how":"held","hand":"left_hand","toward":"kit"}},
                  {"item":"r","rel":{"to":"player","how":"held","hand":"right_hand","toward":"kit"}}]});
    let id = rig.ready(&plan).await;
    make(&rig, &id).await;
    let line = rig
        .log()
        .into_iter()
        .find(|s| s.starts_with("pose ") && s.contains("--other-hold-toward"))
        .expect("two playing hands");
    let arg = |flag| {
        line.split_whitespace()
            .skip_while(|s| *s != flag)
            .nth(1)
            .unwrap()
    };
    let x = |flag| arg(flag).split(',').next().unwrap().parse::<f64>().unwrap();
    let (left, right) = if arg("--hand") == "left" {
        ("--hold-toward", "--other-hold-toward")
    } else {
        ("--other-hold-toward", "--hold-toward")
    };
    assert!(
        x(left) > x(right) + 0.05,
        "both hands target the same head: {line}"
    );
}

/// Poses: a child who sits on its log and looks at the fire is posed once it
/// has its skeleton (the seat's height and the fire in its own frame), the
/// scene embeds this scene's posed model (the made character stays as it
/// was), a change of pose only poses again in seconds (nothing is made), a
/// pose the engine refuses is said, and an engine without `pose` leaves the
/// child standing in front of its log, said.
#[tokio::test]
async fn a_character_sits_on_its_seat_and_a_pose_change_only_poses_again() {
    let rig = Rig::with_help(POSE_HELP).await;
    let plan = |kind: &str| {
        json!({
            "title": "Camp", "summary": "A child by the fire.", "notes": [],
            "items": [
                {"id": "fire", "kind": "object", "name": "Campfire", "description": "A fire.", "enabled": true, "details": {"prompt": "campfire"}},
                {"id": "log", "kind": "object", "name": "Log seat", "description": "A log.", "enabled": true, "details": {"prompt": "log", "dims_m": [1.2, 0.4, 0.4]}},
                {"id": "kid", "kind": "character", "name": "Child", "description": "A child.", "enabled": true,
                 "details": {"prompt": "child", "moves": ["idle"], "pose": {"kind": kind, "on": "log", "look_at": "fire"},
                             "holds": [{"item": "stick", "socket": "right_hand"}]}},
                {"id": "stick", "kind": "object", "name": "Stick", "description": "A stick.", "enabled": true, "details": {"prompt": "stick"}}
            ],
            "layout": [{"item": "fire", "place": "center"}, {"item": "log", "place": "left"},
                       {"item": "kid", "rel": {"to": "log", "how": "on"}, "facing": "fire"},
                       {"item": "stick", "rel": {"to": "kid", "how": "held", "hand": "right_hand", "toward": "fire"}}]
        })
    };
    let id = rig.ready(&plan("sit")).await;
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let done = rig.settle(&id).await;
    assert_eq!(done["status"], "done", "{done}");
    let poses = || -> Vec<String> {
        rig.log()
            .into_iter()
            .filter(|l| l.starts_with("pose "))
            .collect()
    };
    let first = poses();
    assert_eq!(first.len(), 1, "{first:?}");
    // Holding the stick toward the fire: a fist round it, raised that way.
    for arg in [
        "--pose sit,hold",
        "--seat-height",
        "--look-at",
        "--hand right",
        "--hold-style raise",
        "--hold-toward",
    ] {
        assert!(first[0].contains(arg), "{arg}: {first:?}");
    }
    let kid = item(&done, "kid");
    assert_eq!(
        kid["result"]["pose"]["manifest"]["limb_tips"],
        json!([[0.1, 0.2, 0.3]])
    );
    let posed = kid["result"]["pose"]["artifact_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(kid["result"]["pose"]["spec"]["kind"], "sit", "{kid}");
    assert_ne!(
        Some(posed.as_str()),
        kid["result"]["artifact_id"].as_str(),
        "the made child stays as it was"
    );
    let scene = rig.scene(&done).await;
    let obj = scene["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == "kid")
        .unwrap()
        .clone();
    assert!(obj["src"].as_str().unwrap().contains(&posed), "{obj}");
    // The stick is in the fist, turned along the handle (not upright).
    let stick = scene["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == "stick")
        .unwrap();
    assert_ne!(stick["rotation"][0].as_f64().unwrap().abs(), 0.0, "{stick}");
    assert!(
        !done["plan"]["notes"].to_string().contains("in front of"),
        "{}",
        done["plan"]["notes"]
    );

    // Make it kneel instead: posed again, nothing made.
    let before = rig.log().len();
    rig.ctx.planner.queue(&[
        json!({"plan": plan("kneel"), "revision": {"summary": "The child kneels."}}).to_string(),
    ]);
    rig.post(
        &format!("/design/productions/{id}/steer"),
        json!({"comment": "make the child kneel"}),
    )
    .await;
    // Nothing to make: the steer's scene is written at once, posed again.
    let again = rig.settle(&id).await;
    assert_eq!(again["status"], "done", "{again}");
    let runs: Vec<String> = rig.log()[before..].to_vec();
    assert!(
        !runs.iter().any(|l| l.starts_with("start ")),
        "nothing made: {runs:?}"
    );
    assert!(
        runs.iter()
            .any(|l| l.starts_with("pose ") && l.contains("--pose kneel,hold")),
        "{runs:?}"
    );
    assert_eq!(
        item(&again, "kid")["result"]["pose"]["artifact_id"].as_str(),
        Some(posed.as_str()),
        "a new version of its posed model"
    );

    // A pose the engine refuses is said; the child stands.
    rig.ctx.planner.queue(&[
        json!({"plan": plan("curl"), "revision": {"summary": "The child curls up."}}).to_string(),
    ]);
    rig.post(
        &format!("/design/productions/{id}/steer"),
        json!({"comment": "curl up"}),
    )
    .await;
    let refused = rig.settle(&id).await;
    assert!(
        refused["plan"]["notes"]
            .to_string()
            .contains("could not pose Child"),
        "{}",
        refused["plan"]["notes"]
    );

    // A rebuilt scene is posed again with the engine as it is now.
    let before = rig.log().len();
    rig.post(&format!("/design/productions/{id}/scene"), json!({}))
        .await;
    assert!(
        rig.log()[before..].iter().any(|l| l.starts_with("pose ")),
        "{:?}",
        &rig.log()[before..]
    );

    // Sitting again: the refusal is no longer said.
    rig.ctx.planner.queue(&[
        json!({"plan": plan("sit"), "revision": {"summary": "The child sits."}}).to_string(),
    ]);
    rig.post(
        &format!("/design/productions/{id}/steer"),
        json!({"comment": "sit"}),
    )
    .await;
    let again = rig.settle(&id).await;
    assert!(
        !again["plan"]["notes"]
            .to_string()
            .contains("could not pose Child"),
        "{}",
        again["plan"]["notes"]
    );

    // An engine without `pose`: the child stands in front of its log, said.
    let old = Rig::new().await;
    let id = old.ready(&plan("sit")).await;
    old.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let done = old.settle(&id).await;
    assert!(!old.log().iter().any(|l| l.starts_with("pose ")));
    assert!(
        done["plan"]["notes"]
            .to_string()
            .contains("stands in front of the Log seat"),
        "{}",
        done["plan"]["notes"]
    );
}

/// Each character speaks in its own voice: never the novelty voice, never the
/// same one as another character while the engine has enough. Without a voice
/// engine the built-in voices speak, and the line says so.
#[tokio::test]
async fn each_speaker_gets_its_own_voice() {
    let rig = Rig::new().await;
    let plan = json!({
        "title": "Quest", "summary": "Talk.", "notes": [],
        "items": [
            {"id": "man", "kind": "character", "name": "Man", "description": "A man.", "enabled": false, "details": {"voice": "deep and broken"}},
            {"id": "monster", "kind": "character", "name": "Monster", "description": "A beast.", "enabled": false, "details": {"voice": "growling"}},
            {"id": "l1", "kind": "voice_line", "name": "Man", "description": "He talks.", "enabled": true, "details": {"speaker": "man", "text": "Ready."}},
            {"id": "l2", "kind": "voice_line", "name": "Monster", "description": "It roars.", "enabled": true, "details": {"speaker": "monster", "text": "Leave!"}},
            {"id": "l3", "kind": "voice_line", "name": "Man again", "description": "He talks.", "enabled": true, "details": {"speaker": "man", "text": "Never."}}
        ]
    });
    let id = rig.ready(&plan).await;
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let done = rig.settle(&id).await;
    let mut used = Vec::new();
    for line in ["l1", "l2", "l3"] {
        let cid = item(&done, line)["result"]["clip_id"]
            .as_str()
            .unwrap()
            .to_string();
        used.push(
            rig.ctx
                .media
                .store()
                .require_tts_clip(&cid)
                .await
                .unwrap()
                .clip
                .voice_id,
        );
    }
    assert_eq!(
        item(&done, "l1")["message"],
        "Spoken with the built-in voice because the voice engine is not installed."
    );
    let (man, monster, again) = (&used[0], &used[1], &used[2]);
    assert_eq!(
        (man.as_str(), monster.as_str()),
        ("Eddy (English (US))", "Fred")
    );
    assert_eq!(again, man);
}

/// Library first, before anything is generated: a prop the library has
/// and a Ranger come from it (no generate, no rig), marked as library pieces
/// that keep their size; a prop it lacks is generated; a plain man never
/// asks it. The plan and the run share one time estimate, counted by lanes.
#[tokio::test]
async fn the_library_comes_first_and_only_what_it_lacks_is_made() {
    let rig = Rig::new().await;
    let pack = rig
        ._dirs
        .2
        .path()
        .join("models/library/thumbnails/fantasy-props-megakit");
    std::fs::create_dir_all(&pack).unwrap();
    use base64::Engine;
    let png = base64::engine::general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8gEAAMFAQP/gGf4AAAAAElFTkSuQmCC").unwrap();
    std::fs::write(pack.join("chest-wood.png"), &png).unwrap();
    let plan = json!({
        "title": "Camp", "summary": "A camp.", "notes": [],
        "items": [
            {"id": "crate", "kind": "object", "name": "Crate", "description": "A crate.", "enabled": true, "details": {"prompt": "crate"}},
            {"id": "chest", "kind": "object", "name": "Old chest", "description": "A chest from the library.", "enabled": true, "details": {"prompt": "chest"}},
            {"id": "key", "kind": "object", "name": "Chest Key", "description": "An old key that hangs on the monster's neck.", "enabled": true, "details": {"prompt": "key"}},
            {"id": "ranger", "kind": "character", "name": "Ranger", "description": "A ranger.", "enabled": true, "details": {"prompt": "ranger", "moves": ["idle", "walk"]}},
            {"id": "man", "kind": "character", "name": "Man", "description": "A man.", "enabled": true, "details": {"prompt": "man", "moves": ["idle"]}}
        ],
        "layout": [{"item": "crate", "place": "left"}, {"item": "chest", "place": "right"}]
    });
    let id = rig.ready(&plan).await;
    let ready = get_json(&rig.app, &format!("/design/productions/{id}")).await;
    // One GPU job at a time: each model's library lookup, picture and shape
    // one after another (the man last: his own ready part), then the two
    // skeletons and the last moves (longer than the key's game-ready pass).
    // This Forge draws each picture inside its 3D run: no pictures estimate.
    let scene = 5 * 2 + 3 * (250 + 30) + 2 * (350 + 30) + 80;
    assert_eq!(ready["scene_eta_secs"], scene, "{ready}");
    assert_eq!(
        ready["eta_secs"],
        scene + 2 * 120 + 45,
        "the plan's estimate"
    );
    assert!(ready.get("pictures_eta_secs").is_none(), "{ready}");
    let started = rig
        .post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    assert_eq!(
        started["eta_secs"], ready["eta_secs"],
        "the run starts from the same one"
    );
    let done = rig.settle(&id).await;
    assert_eq!(done["status"], "done", "{done}");
    assert!(done.get("eta_secs").is_none());
    let log: Vec<String> = rig
        .log()
        .into_iter()
        .filter(|l| l.starts_with("library") || l.starts_with("start "))
        .collect();
    assert_eq!(
        log,
        [
            "library Crate. A crate. any ",
            "library chest. A chest from the library. any ",
            // A compound name asks by its head word first: "Chest Key" is a key.
            "library Key. An old key that hangs on the monster's neck. any ",
            "library Ranger. A ranger. character idle,walk",
            "start generate crate",
            "start generate man",
            "start rig man",
            "start animate man",
        ]
    );
    for it in ["chest", "key", "ranger"] {
        assert_eq!(
            item(&done, it)["message"],
            "Taken from the Otto Forge library."
        );
    }
    let chest = item(&done, "chest")["result"].clone();
    let design = rig.ctx.design();
    let a = design
        .store()
        .require_artifact(chest["artifact_id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(
        item(&done, "chest")["has_preview"],
        true,
        "library chest has its picture"
    );
    assert_eq!(
        rig.ctx
            .media()
            .production_preview(&id, "chest")
            .await
            .unwrap(),
        (png.clone(), "image/png")
    );
    // Older imports carry the model identity but have no saved thumbnail.
    sqlx::query("UPDATE design_artifacts SET thumb_blob = NULL WHERE id = ?")
        .bind(&a.id)
        .execute(&rig.ctx.pool)
        .await
        .unwrap();
    let old = get_json(&rig.app, &format!("/design/productions/{id}")).await;
    assert_eq!(
        item(&old, "chest")["has_preview"],
        true,
        "previously imported chest"
    );
    assert_eq!(
        rig.ctx
            .media()
            .production_preview(&id, "chest")
            .await
            .unwrap()
            .0,
        png
    );
    let (v, _) = design.head_content(&a).await.unwrap();
    assert_eq!(v.provenance["source"], "library");
    assert_eq!(v.provenance["library"]["licence"], "CC0");
    // In the scene the library chest keeps its size; the made crate is scaled.
    let scene = design
        .store()
        .require_artifact(done["scene_artifact_id"].as_str().unwrap())
        .await
        .unwrap();
    let (_, bytes) = design.head_content(&scene).await.unwrap();
    let doc: Value = serde_json::from_slice(&bytes).unwrap();
    let scale = |id: &str| {
        doc["objects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["id"] == id)
            .unwrap()["scale"]
            .clone()
    };
    assert_eq!(scale("chest"), json!([1.0, 1.0, 1.0]));
    assert_ne!(scale("crate"), json!([1.0, 1.0, 1.0]));
}

/// The agent and model picked on Create plan the production and every steer
/// of it (each planner the router asks the host for carries them); none
/// picked asks for the defaults; a pick no session could run is a 400 and
/// nothing is created.
#[tokio::test]
async fn the_picked_agent_and_model_plan_it_and_every_steer() {
    use otto_sessions::provider_models::{probe_provider_models_cached, ModelProbe};
    let rig = Rig::new().await;
    let catalog = probe_provider_models_cached(&ModelProbe::for_registry(&rig.ctx.providers)).await;
    let model = catalog["codex"][0].id.clone();
    let effort = catalog["codex"][0].supported_efforts[0].clone();
    let picked = crate::productions::AgentPick {
        provider: Some("codex".into()),
        model: Some(model.clone()),
        effort: Some(effort.clone()),
    };

    rig.ctx.planner.queue(&[objects(&["rock"]).to_string()]);
    let (st, p) = call_json(&rig.app, Method::POST, "/design/productions",
        Some(json!({ "workspace_id": rig.ws_id, "description": "a rock", "provider": "codex", "model": model, "effort": effort })), false).await;
    assert_eq!(st, StatusCode::CREATED, "{p}");
    assert_eq!(
        (p["provider"].as_str(), p["model"].as_str()),
        (Some("codex"), Some(model.as_str()))
    );
    assert_eq!(
        p["agent"],
        json!({"provider": "codex", "model": model, "effort": effort})
    );
    let id = p["id"].as_str().unwrap().to_string();
    assert_eq!(rig.settle(&id).await["status"], "ready");
    rig.ctx.planner.queue(&[json!({
        "plan": objects(&["rock", "tree"]),
        "revision": {"summary": "A tree too.", "changed": [], "added": ["tree"], "removed": []}
    })
    .to_string()]);
    rig.post(
        &format!("/design/productions/{id}/steer"),
        json!({"comment": "add a tree"}),
    )
    .await;
    let steered = rig.settle(&id).await;
    assert_eq!(
        steered["revisions"][0]["added"],
        json!(["tree"]),
        "{steered}"
    );
    assert_eq!(steered["model"].as_str(), Some(model.as_str()));
    assert_eq!(*rig.ctx.picks.lock().unwrap(), vec![picked.clone(), picked]);
    assert_eq!(rig.ctx.planner.prompts.lock().unwrap().len(), 2);

    rig.ctx.picks.lock().unwrap().clear();
    let id = rig.ready(&objects(&["rock"])).await;
    assert_eq!(*rig.ctx.picks.lock().unwrap(), vec![Default::default()]);
    let p = get_json(&rig.app, &format!("/design/productions/{id}")).await;
    assert!(
        p.get("provider").is_none() && p.get("model").is_none() && p.get("agent").is_none(),
        "{p}"
    );

    for (provider, model, effort, says) in [
        ("nope", None, None, "unknown provider"),
        ("claude", Some("no-such-model"), None, "no model"),
        (
            "codex",
            Some(model.as_str()),
            Some("turbo"),
            "no effort 'turbo'",
        ),
    ] {
        let (st, v) = call_json(&rig.app, Method::POST, "/design/productions",
            Some(json!({ "workspace_id": rig.ws_id, "description": "a rock", "provider": provider, "model": model, "effort": effort })), false).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
        assert!(v["message"].as_str().unwrap().contains(says), "{v}");
    }
    let list = get_json(
        &rig.app,
        &format!("/design/productions?workspace_id={}", rig.ws_id),
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 2);
}

/// Poll the production every 25 ms until `done` says so; every snapshot.
async fn watch(rig: &Rig, id: &str, done: impl Fn(&Value) -> bool) -> Vec<Value> {
    let mut seen = Vec::new();
    for _ in 0..800 {
        let p = get_json(&rig.app, &format!("/design/productions/{id}")).await;
        let stop = done(&p);
        seen.push(p);
        if stop {
            return seen;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("production {id}: never {:?}", seen.last());
}

/// What a working model shows: its steps in plain words (from the engine's
/// own lines), when it started, that it waits for the engine while Forge
/// waits for a heavy slot, and its concept picture as soon as it is drawn
/// (served by the preview route; 404 for an item without one).
#[tokio::test]
async fn a_working_model_shows_its_step_its_engine_wait_and_its_picture() {
    let rig = Rig::new().await;
    let id = rig.ready(&objects(&["queued-crate", "rock"])).await;
    let none = format!("/design/productions/{id}/items/rock/preview");
    let (st, _) = call_json(&rig.app, Method::GET, &none, None, false).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let seen = watch(&rig, &id, |p| p["status"] == "done").await;
    let crate_at = |p: &Value| item(p, "queued-crate").clone();
    let labels: Vec<String> = seen
        .iter()
        .filter_map(|p| Some(crate_at(p)["step"]["label"].as_str()?.to_string()))
        .fold(Vec::new(), |mut v, l| {
            if v.last() != Some(&l) {
                v.push(l);
            }
            v
        });
    assert_eq!(
        labels,
        [
            "Looking in the library",
            "Drawing the picture",
            "Building the 3D shape",
            "Getting it ready"
        ],
        "{labels:?}"
    );
    let working = seen
        .iter()
        .map(crate_at)
        .find(|i| i["step"]["label"] == "Drawing the picture")
        .unwrap();
    assert_eq!(working["step"]["of"], 3);
    assert!(working["working_since"].is_string(), "{working}");
    assert!(
        seen.iter().any(|p| crate_at(p)["waiting"] == "engine"),
        "the heavy-slot wait was never shown"
    );
    let drawn = seen
        .iter()
        .map(crate_at)
        .find(|i| i["has_preview"] == true)
        .expect("a picture while it was made");
    assert_eq!(
        drawn["status"], "working",
        "the picture comes before the model"
    );
    let done = seen.last().unwrap();
    let it = crate_at(done);
    assert_eq!(
        (it["preview_rev"].as_i64(), it.get("waiting")),
        (Some(1), None)
    );
    assert!(it.get("step").is_none(), "{it}");
    let uri = format!("/design/productions/{id}/items/queued-crate/preview");
    let resp = app(&rig.ctx)
        .oneshot(Request::get(&uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()["content-type"], "image/png");
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert!(bytes.starts_with(b"\x89PNG"), "{bytes:?}");
}

/// Repeating the same two anatomy-rejected seeds in the shape stage gives
/// the user the same drawings. A transient preview failure can keep its seed.
#[tokio::test]
async fn shape_uses_fresh_seed_after_concept_anatomy_rejections() {
    let rig = Rig::with_help(FAST_HELP).await;
    let bin = rig.log.parent().unwrap().join("bin");
    std::fs::rename(bin.join("forge"), bin.join("forge-test-backend")).unwrap();
    write_script(
        &bin,
        "forge",
        r#"#!/bin/sh
if [ "$1" = generate ] && [ "$2" = rejected ]; then
    echo '{"error":"Concept anatomy still mismatches the requested body after two drawings: seen four feet"}' >&2
    exit 1
fi
if [ "$1" = generate ] && [ "$2" = --prompt ]; then
    case "$3" in
        rejected) echo '{"error":"Concept anatomy still mismatches the requested body after two drawings: seen four feet"}' >&2; exit 1 ;;
        transient) echo '{"error":"concept server unavailable"}' >&2; exit 1 ;;
    esac
fi
exec "$(dirname "$0")/forge-test-backend" "$@"
"#,
    );
    let id = rig.ready(&json!({
        "title": "Walking models", "items": [
            {"id": "first", "kind": "object", "name": "Rejected model", "description": "rejected"},
            {"id": "second", "kind": "object", "name": "Transient model", "description": "transient"}
        ]
    })).await;
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let done = rig.settle(&id).await;
    assert_eq!(done["status"], "done", "{done}");
    let jobs = rig
        .ctx
        .media
        .store()
        .list_3d_jobs(Some(&rig.ws_id), None, 20)
        .await
        .unwrap();
    let generated_seed = |name| {
        jobs.iter()
            .find(|j| j.pipeline_stage == "generate" && j.title == name)
            .unwrap()
            .seed
    };
    assert_eq!(generated_seed("Rejected model"), Some(44));
    assert_eq!(generated_seed("Transient model"), None);
    assert_eq!(item(&done, "first")["status"], "failed");
    // The ordinary Start action retries only failed items. Here the next
    // picture succeeds, so its shape should use the next untried seed.
    std::fs::rename(bin.join("forge-test-backend"), bin.join("forge")).unwrap();
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let resumed = rig.settle(&id).await;
    assert_eq!(item(&resumed, "first")["status"], "done");
    assert_eq!(item(&resumed, "first")["seed"], 46);
    let jobs = rig
        .ctx
        .media
        .store()
        .list_3d_jobs(Some(&rig.ws_id), None, 20)
        .await
        .unwrap();
    let resumed_job = jobs
        .iter()
        .find(|j| j.title == "Rejected model" && j.pipeline_stage == "generate")
        .unwrap();
    assert_eq!(resumed_job.seed, Some(46));
    assert_eq!(
        jobs.iter()
            .filter(|j| j.title == "Transient model" && j.pipeline_stage == "generate")
            .count(),
        1
    );
}

/// Every piece's picture first, on one warm image server for the pass
/// (stopped after): every picture is drawn before any 3D shape is built, and
/// each shape is built from exactly its picture. An object's shape goes into
/// the scene as a first look and gets ready for games on the CPU while the
/// next shape is built (Pixal3D's own steps move its bar); never two GPU
/// jobs at once.
#[tokio::test]
async fn pictures_come_first_then_shapes_one_gpu_job_at_a_time() {
    let rig = Rig::with_help(&format!("{FAST_HELP} --body")).await;
    let root = rig.log.parent().unwrap().to_path_buf();
    let plan = json!({
        "title": "Camp", "summary": "A man, a crate and a lamp.", "notes": [],
        "items": [
            {"id": "man", "kind": "character", "name": "Man", "description": "A man.", "enabled": true, "details": {"prompt": "man", "body": "two_legs", "moves": ["idle"]}},
            {"id": "crate", "kind": "object", "name": "Crate", "description": "A crate.", "enabled": true, "details": {"prompt": "crate"}},
            {"id": "lamp", "kind": "object", "name": "Lamp", "description": "A lamp.", "enabled": true, "details": {"prompt": "lamp"}},
            {"id": "hi", "kind": "voice_line", "name": "Hi", "description": "He greets.", "enabled": true, "details": {"speaker": "man", "text": "Hello."}}
        ]
    });
    // Otto Forge's voice engine, logging into the same log as the fake Forge.
    let bin = root.join("bin");
    write_script(&bin, "forge-voice", FAKE_VOICE);
    std::os::unix::fs::symlink(&rig.log, bin.join("voice.log")).unwrap();
    let id = rig.ready(&plan).await;
    let ready = get_json(&rig.app, &format!("/design/productions/{id}")).await;
    assert_eq!(ready["pictures_eta_secs"], 3 * 2 + 3 * 30 + 10, "{ready}");
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let seen = watch(&rig, &id, |p| p["status"] == "done").await;
    let done = seen.last().unwrap();
    // The spoken line right after the pictures, before any shape (its voice
    // needs no model), and its clip joins the man's model once he is made.
    let log = rig.log();
    let spoke = log
        .iter()
        .position(|l| l.starts_with("speak "))
        .expect("spoken");
    assert!(
        at_end_of_pictures(&log) < spoke && spoke < first_shape_at(&log),
        "{log:?}"
    );
    let clip = item(done, "hi")["result"]["clip_id"]
        .as_str()
        .unwrap()
        .to_string();
    let clip = rig
        .ctx
        .media
        .store()
        .require_tts_clip(&clip)
        .await
        .unwrap()
        .clip;
    assert_eq!(
        clip.artifact_id.as_ref(),
        item(done, "man")["result"]["artifact_id"]
            .as_str()
            .map(String::from)
            .as_ref()
    );
    for it in ["man", "crate", "lamp"] {
        let it = item(done, it);
        assert_eq!(
            (it["status"].as_str(), it.get("made")),
            (Some("done"), None),
            "{it}"
        );
        assert_eq!(
            (&it["has_preview"], &it["preview_rev"]),
            (&json!(true), &json!(1)),
            "{it}"
        );
    }
    let log = rig.log();
    let at = |line: &str| log.iter().position(|l| l == line).expect(line);
    let first_shape = log
        .iter()
        .position(|l| l.starts_with("start generate"))
        .unwrap();
    let concepts: Vec<&String> = log
        .iter()
        .filter(|l| l.starts_with("start concept"))
        .collect();
    assert!(
        log.iter().any(|l| l == "body concept man humanoid"),
        "{log:?}"
    );
    assert!(
        log.iter().any(|l| l == "body generate man humanoid"),
        "{log:?}"
    );
    assert!(!log
        .iter()
        .any(|l| l.starts_with("body ") && (l.contains("crate") || l.contains("lamp"))));
    assert_eq!(concepts.len(), 3, "{log:?}");
    assert!(
        concepts.iter().all(|l| l.contains(" http://127.0.0.1:")),
        "drawn on the warm server: {log:?}"
    );
    assert!(
        at("end concept lamp") < first_shape,
        "every picture first: {log:?}"
    );
    // Each shape from exactly its picture; objects as a first look, readied after.
    for p in ["man", "crate", "lamp"] {
        assert!(log.contains(&format!("picture {p} {p}")), "{log:?}");
    }
    assert!(
        at("start gameready crate") < at("end generate lamp"),
        "{log:?}"
    );
    assert!(
        at("start generate lamp") < at("end gameready crate"),
        "{log:?}"
    );
    // The man too is a first look first, finished from his kept inputs
    // (the finishing folder travels with the draft) with the same budget,
    // then rigged and moved.
    assert!(log.contains(&"draft character man".to_string()), "{log:?}");
    assert!(at("finish character man") < at("start rig man"), "{log:?}");
    assert!(at("start rig man") < at("start animate man"), "{log:?}");
    let jobs = rig
        .ctx
        .media
        .store()
        .list_3d_jobs(Some(&rig.ws_id), None, 30)
        .await
        .unwrap();
    let budget = |stage: &str| {
        jobs.iter()
            .find(|j| j.pipeline_stage == stage && j.title == "Man")
            .map(|j| j.poly_budget)
            .unwrap()
    };
    assert_eq!(budget("generate"), budget("gameready"));
    assert!(budget("generate").is_some());
    // The warm image server for the pictures only, stopped before any shape.
    assert!(at("start server") < at("end concept man"), "{log:?}");
    assert!(at("server stopped") < first_shape, "{log:?}");
    // Never two GPU jobs at once (a picture, a shape until its CPU step, a skeleton).
    let mut gpu: Option<String> = None;
    for l in &log {
        let (what, rest) = l.split_once(' ').unwrap_or((l, ""));
        let (stage, name) = rest.split_once(' ').unwrap_or((rest, ""));
        let name = name.split(' ').next().unwrap_or_default().to_string();
        match (what, stage) {
            ("start", "concept" | "generate" | "rig") => {
                assert_eq!(gpu, None, "{l} while {gpu:?} works: {log:?}");
                gpu = Some(name);
            }
            ("end", "concept" | "generate" | "rig") | ("cpu", "generate")
                if gpu.as_deref() == Some(name.as_str()) =>
            {
                gpu = None;
            }
            _ => {}
        }
    }
    // The bar moves inside the 3D shape step (Pixal3D's part 3 of 6, flow 4/8).
    assert!(
        seen.iter().any(|p| p["plan"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["step"]["n"] == 2 && i["step"]["fraction"] == json!(0.2))),
        "no fraction seen"
    );
    // The crate's first look was in the scene before it was readied, as the same model.
    let crate_ = item(done, "crate");
    assert!(
        seen.iter().any(|p| item(p, "crate")["made"] == "draft"),
        "no first look seen"
    );
    let jobs = rig
        .ctx
        .media
        .store()
        .list_3d_jobs(Some(&rig.ws_id), None, 20)
        .await
        .unwrap();
    let readied = jobs
        .iter()
        .find(|j| j.pipeline_stage == "gameready" && j.title == "Crate")
        .unwrap();
    assert_eq!(
        crate_["result"]["version_id"].as_str(),
        readied.version_id.as_deref()
    );
    let drafted = jobs
        .iter()
        .find(|j| j.pipeline_stage == "generate" && j.title == "Crate")
        .unwrap();
    assert_eq!(
        drafted.artifact_id, readied.artifact_id,
        "a new version of the same model"
    );
    let kept = rig
        .ctx
        .media
        .data_dir()
        .join("media/productions")
        .join(&id)
        .join("man.finish");
    assert!(!kept.exists(), "the kept finishing inputs go once finished");
}

/// A Forge without `concept-server` (nor exact character drafts): the
/// pictures are drawn one by one without a warm server, objects are still
/// first looks, and a character is made in one run as before.
#[tokio::test]
async fn an_older_forge_makes_a_character_in_one_run() {
    let rig =
        Rig::with_help("  --height-m H  --longest-side-m L  --concept-only  --skip-gameready")
            .await;
    let plan = json!({
        "title": "Camp", "summary": "A man and a crate.", "notes": [],
        "items": [
            {"id": "man", "kind": "character", "name": "Man", "description": "A man.", "enabled": true, "details": {"prompt": "man"}},
            {"id": "crate", "kind": "object", "name": "Crate", "description": "A crate.", "enabled": true, "details": {"prompt": "crate"}}
        ]
    });
    let id = rig.ready(&plan).await;
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let done = rig.settle(&id).await;
    assert_eq!(done["status"], "done", "{done}");
    let log = rig.log();
    assert!(
        log.contains(&"start concept man".to_string()),
        "no server, no kind: {log:?}"
    );
    assert!(
        log.contains(&"cpu generate man".to_string()),
        "one run: {log:?}"
    );
    assert!(
        !log.iter()
            .any(|l| l.starts_with("draft character") || l == "start server"),
        "{log:?}"
    );
    assert!(
        log.contains(&"start gameready crate".to_string()),
        "{log:?}"
    );
}

/// A change while it makes reaches only a piece whose 3D shape is not
/// built yet and that is not being made right now: a steer on the crate and
/// a "not this one" on the lamp make each of them again from a new picture
/// at its turn; the rock being made goes on, nothing else starts over, and
/// what cannot be taken mid-run (a new barrel) waits, said plainly.
#[tokio::test]
async fn a_change_while_making_redraws_only_that_piece() {
    let rig = Rig::with_help(FAST_HELP).await;
    let mut plan = objects(&["rock", "crate", "lamp"]);
    plan["items"][0]["details"]["prompt"] = json!("slow rock");
    let id = rig.ready(&plan).await;
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    watch(&rig, &id, |p| item(p, "rock")["step"]["n"] == 2).await;
    // The rock is being made: it cannot be drawn again now.
    let (st, _) = call_json(
        &rig.app,
        Method::POST,
        &format!("/design/productions/{id}/items/rock/redraw"),
        None,
        false,
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    let p = rig
        .post(
            &format!("/design/productions/{id}/items/lamp/redraw"),
            json!({}),
        )
        .await;
    assert_eq!(p["status"], "running");
    assert_eq!(
        (&item(&p, "lamp")["seed"], item(&p, "lamp").get("made")),
        (&json!(43), None)
    );
    let mut revised = plan.clone();
    revised["items"][1]["details"]["prompt"] = json!("red crate");
    // Where and when it is changes no piece: it is taken at once.
    revised["setting"] = json!({"summary": "A yard at night.", "ground": "grass", "time": "night",
        "weather": "clear", "indoor": false});
    revised["items"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id": "barrel", "kind": "object", "name": "Barrel", "description": "A barrel.", "enabled": true, "details": {"prompt": "barrel"}}));
    rig.ctx.planner.queue(&[revised.to_string()]);
    let p = rig
        .post(
            &format!("/design/productions/{id}/steer"),
            json!({"comment": "A red crate and a barrel."}),
        )
        .await;
    assert_eq!(p["status"], "running");
    // The rock takes 20 s in the fake engine.
    let mut done = Value::Null;
    for _ in 0..2400 {
        done = get_json(&rig.app, &format!("/design/productions/{id}")).await;
        if done["status"] != "running" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert_eq!(done["status"], "done", "{done}");
    let log = rig.log();
    let starts: Vec<&str> = log
        .iter()
        .filter_map(|l| l.strip_prefix("start "))
        .filter(|l| *l != "server")
        .map(|l| l.split(" http").next().unwrap())
        .collect();
    // Each picture once, each shape once (nothing started over).
    let concepts = ["concept slow rock", "concept crate", "concept lamp"];
    assert_eq!(&starts[..3], concepts, "{log:?}");
    for shape in ["generate slow rock", "generate red crate", "generate lamp"] {
        let n = starts.iter().filter(|s| **s == shape).count();
        assert_eq!(n, 1, "{shape}: {log:?}");
    }
    assert_eq!(starts.len(), 9, "pictures, shapes, readied: {log:?}");
    // The rock from its own picture; the crate and the lamp drawn again in their own run.
    assert!(
        log.contains(&"picture slow rock slow rock".to_string()),
        "{log:?}"
    );
    assert!(
        !log.iter()
            .any(|l| l.starts_with("picture red crate") || l.starts_with("picture lamp")),
        "{log:?}"
    );
    assert_eq!(item(&done, "crate")["details"]["prompt"], "red crate");
    let ids: Vec<&Value> = done["plan"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| &i["id"])
        .collect();
    assert_eq!(ids, [&json!("rock"), &json!("crate"), &json!("lamp")]);
    let rev = done["revisions"].as_array().unwrap().last().unwrap();
    assert_eq!(rev["changed"], json!(["crate"]), "{done}");
    assert_eq!(done["plan"]["setting"]["time"], "night", "{done}");
    assert!(
        !rev["done"].as_array().unwrap().is_empty(),
        "the night is said: {rev}"
    );
    assert!(
        done["message"]
            .as_str()
            .unwrap_or_default()
            .contains("Barrel"),
        "the barrel waits: {done}"
    );
}

fn at_end_of_pictures(log: &[String]) -> usize {
    log.iter()
        .rposition(|l| l.starts_with("end concept"))
        .unwrap()
}

fn first_shape_at(log: &[String]) -> usize {
    log.iter()
        .position(|l| l.starts_with("start generate"))
        .unwrap()
}

/// A spoken line in a built-in voice (no GPU) is made beside a model; a
/// sound takes the GPU, so it waits until every piece has its shape: never
/// two GPU jobs at once. A spoken line waits only for its own speaker.
#[tokio::test]
async fn a_spoken_line_is_made_beside_a_model_and_a_sound_waits_for_the_gpu() {
    let rig = Rig::new().await;
    let audio = rig.add_audio();
    let plan = json!({
        "title": "Camp", "summary": "A camp.", "notes": [],
        "items": [
            {"id": "rock", "kind": "object", "name": "Rock", "description": "A rock.", "enabled": true, "details": {"prompt": "slow rock"}},
            {"id": "hello", "kind": "voice_line", "name": "Hello", "description": "Someone says hello.", "enabled": true, "details": {"text": "Hello."}},
            {"id": "wind", "kind": "sound", "name": "Wind", "description": "Wind.", "enabled": true, "details": {"prompt": "wind", "seconds": 2}}
        ]
    });
    let id = rig.ready(&plan).await;
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let seen = watch(&rig, &id, |p| item(p, "hello")["status"] == "done").await;
    let now = seen.last().unwrap();
    assert_eq!(item(now, "rock")["status"], "working", "{now}");
    assert_eq!(item(now, "wind")["status"], "waiting", "{now}");
    // A waiting item says what it waits for.
    assert!(
        item(now, "wind")["message"]
            .as_str()
            .is_some_and(|m| m.contains("sound engine")),
        "{now}"
    );
    assert!(!rig.log().iter().any(|l| l == "end generate slow rock"));
    rig.post(&format!("/design/productions/{id}/cancel"), json!({}))
        .await;
    let done = rig.settle(&id).await;
    assert_eq!(done["status"], "cancelled");
    assert_eq!(item(&done, "wind")["status"], "skipped", "{done}");
    assert!(!audio.exists(), "the sound engine never ran");
}

/// A character's skeleton waits until every model is in the scene: the
/// scene is ready to look at (with the standing character) while it gets
/// its skeleton. A failed skeleton leaves it standing, done, with a plain
/// note (never the engine's words), and a second Start tries only that
/// skeleton again.
#[tokio::test]
async fn the_scene_stands_before_skeletons_and_a_failed_one_leaves_it_standing() {
    let rig = Rig::new().await;
    let plan = json!({
        "title": "Woods", "summary": "A fox and a man.", "notes": [],
        "items": [
            {"id": "fox", "kind": "character", "name": "Little red fox", "description": "A small fox.", "enabled": true,
             "details": {"prompt": "little red fox", "moves": ["idle", "walk"]}},
            {"id": "man", "kind": "character", "name": "Man", "description": "A man.", "enabled": true,
             "details": {"prompt": "man", "moves": ["idle"]}},
            {"id": "log", "kind": "object", "name": "Log", "description": "A log.", "enabled": true, "details": {"prompt": "log"}}
        ]
    });
    let id = rig.ready(&plan).await;
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let seen = watch(&rig, &id, |p| {
        item(p, "man")["step"]["label"] == "Adding a skeleton" || p["status"] == "done"
    })
    .await;
    let rigging = seen.last().unwrap();
    assert_eq!(rigging["status"], "running", "{rigging}");
    assert_eq!(rigging["scene_ready"], true, "{rigging}");
    assert_eq!(
        item(rigging, "man")["step"],
        json!({"n": 4, "of": 5, "label": "Adding a skeleton"})
    );
    assert_eq!(
        scene_ids(&rig, rigging).await,
        ["fox", "ground", "log", "man"]
    );
    assert!(seen
        .iter()
        .all(|p| p["scene_ready"] != true || item(p, "log")["result"].is_object()));

    let done = rig.settle(&id).await;
    assert_eq!(done["status"], "done", "{done}");
    assert_eq!(done["message"], Value::Null, "nothing failed: {done}");
    let fox = item(&done, "fox");
    let note = "Otto could not add moves to the Little red fox this time, so it stands still.";
    assert_eq!(
        (fox["status"].as_str(), fox["message"].as_str()),
        (Some("done"), Some(note))
    );
    assert!(done["plan"]["notes"]
        .as_array()
        .unwrap()
        .contains(&json!(note)));
    // Its result is the model generate made (rig made no version).
    let jobs = rig
        .ctx
        .media
        .store()
        .list_3d_jobs(Some(&rig.ws_id), None, 20)
        .await
        .unwrap();
    let made = jobs
        .iter()
        .find(|j| j.pipeline_stage == "generate" && j.title == "Little red fox")
        .unwrap();
    assert_eq!(
        fox["result"]["version_id"].as_str(),
        made.version_id.as_deref()
    );
    let rig_job = jobs
        .iter()
        .find(|j| j.pipeline_stage == "rig" && j.title == "Little red fox")
        .unwrap();
    assert!(rig_job
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("no humanoid body"));
    assert!(scene_ids(&rig, &done).await.contains(&"fox".to_string()));
    assert_eq!(item(&done, "man")["status"], "done");

    let before = rig.log().len();
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let again = rig.settle(&id).await;
    assert_eq!(again["status"], "done");
    let runs: Vec<String> = rig.log()[before..]
        .iter()
        .filter(|l| l.starts_with("start "))
        .cloned()
        .collect();
    assert_eq!(
        runs,
        ["start rig little red fox"],
        "only its skeleton again"
    );
    assert_eq!(item(&again, "fox")["message"], note);
}

/// The scene document of a production (its scene artifact's head).
async fn scene_doc(rig: &Rig, p: &Value) -> Value {
    let a = rig
        .ctx
        .design()
        .store()
        .require_artifact(p["scene_artifact_id"].as_str().unwrap())
        .await
        .unwrap();
    let (_, bytes) = rig.ctx.design().head_content(&a).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// "Make it daytime, no fog" is the setting's: no item changes, nothing is
/// made again, the scene is rebuilt at once with the day's sky and sun, the
/// revision says it plainly, and a revert brings the foggy night back.
#[tokio::test]
async fn a_setting_steer_rebuilds_the_scene_without_making_anything() {
    let rig = Rig::new().await;
    let mut night = objects(&["pier", "lantern"]);
    night["setting"] = json!({"summary": "A pier over the sea at night, in fog.", "ground": "water",
        "time": "night", "weather": "fog", "lights": [{"item": "lantern"}]});
    let id = rig.ready(&night).await;
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let made = rig.settle(&id).await;
    assert_eq!(made["status"], "done", "{made}");
    assert_eq!(made["plan"]["setting"]["time"], "night");
    let doc = scene_doc(&rig, &made).await;
    assert_eq!(doc["environment"]["ground"]["kind"], "water");
    assert_eq!(doc["environment"]["preset"], "night");
    let fog = |d: &Value| d["environment"]["fog"]["density"].as_f64().unwrap();
    let lit = |d: &Value| {
        d["lights"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["id"] == "glow-lantern")
    };
    assert!(lit(&doc));
    let night_fog = fog(&doc);

    let mut day = night.clone();
    day["setting"]["time"] = json!("day");
    day["setting"]["weather"] = json!("clear");
    let runs = rig.log().len();
    rig.ctx.planner.queue(&[json!({"plan": day, "revision": {
        "summary": "It will be daytime with no fog.", "changed": [], "added": [], "removed": []}})
    .to_string()]);
    rig.post(
        &format!("/design/productions/{id}/steer"),
        json!({"comment": "make it daytime, no fog"}),
    )
    .await;
    let sunny = rig.settle(&id).await;
    assert_eq!(sunny["status"], "done", "{sunny}");
    assert_eq!(rig.log().len(), runs, "nothing was made again");
    let rev = &sunny["revisions"][0];
    assert_eq!(rev["changed"], json!([]));
    assert_eq!(
        rev["done"],
        json!(["It is daytime now.", "The fog is gone."])
    );
    assert!(rev.get("setting_before").is_none(), "stored, never sent");
    assert_eq!(
        item(&sunny, "pier")["result"],
        item(&made, "pier")["result"]
    );
    let doc = scene_doc(&rig, &sunny).await;
    assert_eq!(doc["environment"]["preset"], "studio-soft");
    assert!(fog(&doc) < night_fog / 3.0, "a clear day is only hazed");
    let sun = doc["lights"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == "sun")
        .unwrap()
        .clone();
    assert!(sun["intensity"].as_f64().unwrap() > 2.0, "{sun}");

    let back = rig
        .post(
            &format!("/design/productions/{id}/revert"),
            json!({"revision_id": rev["id"]}),
        )
        .await;
    assert_eq!(back["plan"]["setting"]["time"], "night");
    assert_eq!(
        scene_doc(&rig, &back).await["environment"]["preset"],
        "night"
    );
    assert_eq!(rig.log().len(), runs, "nothing was made for the revert");
    // The scene rebuilt with the current builder, in one call: a new version
    // of the same scene from the same made pieces, nothing made.
    let scene = back["scene_artifact_id"].as_str().unwrap().to_string();
    let seq = |a: otto_design::types::DesignArtifact| a.head_seq.unwrap_or_default();
    let before = seq(rig
        .ctx
        .design()
        .store()
        .require_artifact(&scene)
        .await
        .unwrap());
    let rebuilt = rig
        .post(&format!("/design/productions/{id}/scene"), json!({}))
        .await;
    assert_eq!(rebuilt["scene_artifact_id"], back["scene_artifact_id"]);
    let after = seq(rig
        .ctx
        .design()
        .store()
        .require_artifact(&scene)
        .await
        .unwrap());
    assert_eq!(after, before + 1, "a new version of the scene");
    assert_eq!(
        scene_doc(&rig, &rebuilt).await["environment"]["preset"],
        "night"
    );
    assert_eq!(rig.log().len(), runs, "nothing was made for the rebuild");
}

/// The ids of the objects in a production's scene, sorted.
async fn scene_ids(rig: &Rig, p: &Value) -> Vec<String> {
    let doc = scene_doc(rig, p).await;
    let mut ids: Vec<String> = doc["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_str().unwrap().to_string())
        .collect();
    ids.sort();
    ids
}

/// A character stopped while it gets its skeleton keeps its model: the next
/// Start goes on from that model (rig, moves) and never makes it again.
#[tokio::test]
async fn a_stopped_skeleton_goes_on_from_the_model_already_made() {
    let rig = Rig::new().await;
    let plan = json!({
        "title": "Man", "summary": "A man.", "notes": [],
        "items": [{"id": "man", "kind": "character", "name": "Man", "description": "A man.", "enabled": true,
                   "details": {"prompt": "stiff man", "moves": ["idle"]}}]
    });
    let id = rig.ready(&plan).await;
    let stopped = stop_during(&rig, &id, "rig stiff man").await;
    let man = item(&stopped, "man");
    assert_eq!(
        (man["status"].as_str(), man["made"].as_str()),
        (Some("failed"), Some("mesh")),
        "{man}"
    );
    let mesh = man["result"].clone();
    assert!(mesh["artifact_id"].is_string(), "{man}");
    let before = rig.log().len();
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let done = rig.settle(&id).await;
    assert_eq!(item(&done, "man")["status"], "done", "{done}");
    let runs: Vec<String> = rig.log()[before..]
        .iter()
        .filter(|l| l.starts_with("start "))
        .cloned()
        .collect();
    assert_eq!(runs, ["start rig stiff man", "start animate stiff man"]);
    assert_eq!(
        item(&done, "man")["result"]["artifact_id"],
        mesh["artifact_id"]
    );
}

/// Several productions: Start on a second one never fails; it waits its
/// turn with its `queue_position` and makes nothing until the one ahead is
/// done; a stopped one leaves the line; planning another is never held up.
#[tokio::test]
async fn productions_take_turns_in_start_order() {
    let rig = Rig::new().await;
    let a = rig.ready(&objects(&["slow-a"])).await;
    let b = rig.ready(&objects(&["crate"])).await;
    let c = rig.ready(&objects(&["barrel"])).await;
    for id in [&a, &b, &c] {
        let p = rig
            .post(&format!("/design/productions/{id}/start"), json!({}))
            .await;
        assert_eq!(p["status"], "running", "{p}");
    }
    let started = |line: &str| rig.log().iter().any(|l| l == line);
    for _ in 0..200 {
        if started("start generate slow-a") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let pos = |id: &String| {
        let app = rig.app.clone();
        let uri = format!("/design/productions/{id}");
        async move { get_json(&app, &uri).await["queue_position"].clone() }
    };
    assert_eq!(pos(&a).await, Value::Null, "it is the one being made");
    assert_eq!((pos(&b).await, pos(&c).await), (json!(1), json!(2)));
    let queued = get_json(&rig.app, &format!("/design/productions/{b}")).await;
    assert_eq!(item(&queued, "crate")["status"], "waiting", "{queued}");
    // Planning is never held up by the one being made.
    let d = rig.ready(&objects(&["rock"])).await;
    assert!(
        !started("end generate slow-a"),
        "{d} planned while slow-a was made"
    );
    // A stopped one leaves the line; the rest move up.
    rig.post(&format!("/design/productions/{b}/cancel"), json!({}))
        .await;
    assert_eq!(rig.settle(&b).await["status"], "cancelled");
    assert_eq!(pos(&c).await, json!(1));
    rig.post(&format!("/design/productions/{a}/cancel"), json!({}))
        .await;
    assert_eq!(rig.settle(&a).await["status"], "cancelled");
    assert_eq!(rig.settle(&c).await["status"], "done");
    let runs: Vec<String> = rig
        .log()
        .into_iter()
        .filter(|l| l.starts_with("start "))
        .collect();
    assert_eq!(runs, ["start generate slow-a", "start generate barrel"]);
}

/// Grants the Viewer role in these workspaces only.
struct ViewerOf(Vec<String>);
impl RoleChecker for ViewerOf {
    fn check<'a>(
        &'a self,
        _user: &'a User,
        workspace_id: &'a otto_core::Id,
        min: WorkspaceRole,
    ) -> BoxFuture<'a, Result<()>> {
        let ok = min <= WorkspaceRole::Viewer && self.0.contains(workspace_id);
        Box::pin(async move { ok.then_some(()).ok_or(Error::Forbidden("no".into())) })
    }
}

/// The Create shelf (`GET /design/creations`): every production, and every
/// 3D model and clip not inside one, across every workspace the caller can
/// see (never one he cannot), whatever workspace he is in.
#[tokio::test]
async fn the_shelf_spans_every_workspace_the_caller_sees() {
    let mut rig = Rig::new().await;
    let (d2, d3) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let u1 = "u1".to_string();
    let wss = &rig.ctx.workspaces;
    let second = wss
        .create("Second", d2.path().to_str().unwrap(), &u1)
        .await
        .unwrap();
    let hidden = wss
        .create("Hidden", d3.path().to_str().unwrap(), &u1)
        .await
        .unwrap();
    let all = [rig.ws_id.clone(), second.id.clone(), hidden.id.clone()];
    let (mut scenes, mut pieces) = (Vec::new(), Vec::new());
    for w in &all {
        // A production made (its piece and scene are inside it) ...
        rig.ctx.planner.queue(&[objects(&["rock"]).to_string()]);
        let (st, p) = call_json(
            &rig.app,
            Method::POST,
            "/design/productions",
            Some(json!({ "workspace_id": w, "description": "a rock" })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{p}");
        let id = p["id"].as_str().unwrap().to_string();
        rig.settle(&id).await;
        rig.post(&format!("/design/productions/{id}/start"), json!({}))
            .await;
        let done = rig.settle(&id).await;
        pieces.push(item(&done, "rock")["result"]["artifact_id"].clone());
        scenes.push(id);
        // ... a loose model and a loose clip.
        let (st, job) = call_json(
            &rig.app,
            Method::POST,
            "/design/3d/jobs",
            Some(json!({ "workspace_id": w, "prompt": "crate", "provider_id": BUILTIN_OTTO_FORGE_ID })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::ACCEPTED, "{job}");
        for _ in 0..200 {
            let j = get_json(
                &rig.app,
                &format!("/design/3d/jobs/{}", job["id"].as_str().unwrap()),
            )
            .await;
            if j["status"] == "succeeded" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        let (st, clip) = call_json(
            &rig.app,
            Method::POST,
            "/product/tts/synthesize",
            Some(json!({ "workspace_id": w, "voice_id": "Samantha", "text": "Hi." })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{clip}");
    }
    rig.ctx.roles = Arc::new(ViewerOf(vec![rig.ws_id.clone(), second.id.clone()]));
    rig.app = app(&rig.ctx);
    let shelf = get_json(&rig.app, "/design/creations").await;
    let ws_of = |k: &str| -> Vec<String> {
        shelf[k]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["workspace_id"].as_str().unwrap().to_string())
            .collect()
    };
    let seen = vec![second.id.clone(), rig.ws_id.clone()];
    assert_eq!(ws_of("productions"), seen, "newest first: {shelf}");
    assert_eq!(ws_of("models"), seen, "{shelf}");
    assert_eq!(ws_of("clips"), seen, "{shelf}");
    let p = &shelf["productions"][0];
    assert_eq!(
        (p["id"].as_str(), p["status"].as_str()),
        (Some(scenes[1].as_str()), Some("done"))
    );
    assert_eq!(
        (p["items_done"].as_i64(), p["items_total"].as_i64()),
        (Some(1), Some(1))
    );
    assert!(
        p["scene_artifact_id"].is_string() && p.get("pieces").is_none(),
        "{p}"
    );
    assert_eq!(p["workspace_name"], "Second");
    let models: Vec<&Value> = shelf["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| &m["id"])
        .collect();
    assert!(
        !pieces.iter().any(|piece| models.contains(&piece)),
        "a production's piece is not loose"
    );
    assert!(
        shelf["models"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["title"] == "crate"),
        "{shelf}"
    );
    // A piece of a workspace he cannot see is not reachable by its id either.
    let uri = format!("/design/productions/{}", scenes[2]);
    let (st, _) = call_json(&rig.app, Method::GET, &uri, None, false).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

/// A production in the project of `of` (one of its creations), planned as `plan`, ready.
async fn ready_in(rig: &Rig, of: Option<&str>, plan: &Value) -> String {
    rig.ctx.planner.queue(&[plan.to_string()]);
    let mut body = json!({ "workspace_id": rig.ws_id, "description": "a scene" });
    if let Some(of) = of {
        body["project_id"] = of.into();
    }
    let (st, p) = call_json(
        &rig.app,
        Method::POST,
        "/design/productions",
        Some(body),
        false,
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{p}");
    let id = p["id"].as_str().unwrap().to_string();
    assert_eq!(rig.settle(&id).await["status"], "ready");
    id
}

async fn make(rig: &Rig, id: &str) -> Value {
    rig.post(&format!("/design/productions/{id}/start"), json!({}))
        .await;
    let done = rig.settle(id).await;
    assert_eq!(done["status"], "done", "{done}");
    done
}

/// What an item of a made production is, to reuse it.
fn reuse_of(p: &Value, id: &str) -> Value {
    let r = &item(p, id)["result"];
    json!({ "artifact_id": r["artifact_id"], "version_id": r["version_id"] })
}

/// THE owner's case: a second scene of a project with the same cast and
/// props makes no shape of them. The planner is given the project's pieces
/// and place; the plan marks what it reuses structurally (never by name: the
/// grandmother is "Granny" now), the estimate counts only the new kettle and
/// line, Make builds only the kettle, every reused piece is in the scene at
/// once with its model, size and voice ("From …"), and a third scene with
/// nothing new makes nothing at all and finishes at once.
#[tokio::test]
async fn a_second_scene_of_a_project_makes_only_what_is_new() {
    let rig = Rig::new().await;
    let bin = rig.log.parent().unwrap().join("bin");
    write_script(&bin, "forge-voice", FAKE_VOICE);
    let first = json!({
        "title": "Breakfast", "summary": "Breakfast.", "notes": [],
        "items": [
            {"id": "grandma", "kind": "character", "name": "Grandmother", "description": "A kind old woman.", "enabled": true,
             "details": {"prompt": "old woman", "voice": "warm, old, slow", "moves": ["idle"], "size_m": 1.6, "body": "two_legs"}},
            {"id": "cat", "kind": "character", "name": "Cat", "description": "A grey cat.", "enabled": true,
             "details": {"prompt": "grey cat", "body": "four_legs", "size_m": 0.4}},
            {"id": "table", "kind": "object", "name": "Table", "description": "A wooden table.", "enabled": true,
             "details": {"prompt": "wooden table", "dims_m": [1.2, 0.75, 1.2]}},
            {"id": "teapot", "kind": "object", "name": "Teapot", "description": "A teapot.", "enabled": true, "details": {"prompt": "teapot"}},
            {"id": "hello", "kind": "voice_line", "name": "Grandmother speaks", "description": "She says good morning.", "enabled": true,
             "details": {"speaker": "grandma", "text": "Good morning."}}
        ],
        "layout": [{"item": "grandma", "place": "center"}, {"item": "cat", "place": "left"},
                   {"item": "table", "place": "front"}, {"item": "teapot", "rel": {"to": "table", "how": "on"}}],
        "setting": {"summary": "A warm kitchen in the morning.", "ground": "wood_floor", "time": "day", "weather": "clear", "indoor": true}
    });
    let one = ready_in(&rig, None, &first).await;
    let done1 = make(&rig, &one).await;
    assert!(
        get_json(&rig.app, &format!("/design/productions/{one}")).await["project_id"]
            == one.as_str()
    );
    let made_before = rig.log().len();
    let second = json!({
        "title": "Tea", "summary": "Tea.", "notes": [],
        "items": [
            {"id": "granny", "kind": "character", "name": "Granny", "description": "She pours tea.", "enabled": true,
             "details": {"reuse": reuse_of(&done1, "grandma"), "moves": ["idle", "walk"]}},
            {"id": "cat", "kind": "character", "name": "Cat", "description": "The cat waits.", "enabled": true, "details": {"reuse": reuse_of(&done1, "cat")}},
            {"id": "table", "kind": "object", "name": "Table", "description": "The table.", "enabled": true, "details": {"reuse": reuse_of(&done1, "table")}},
            {"id": "teapot", "kind": "object", "name": "Teapot", "description": "The teapot.", "enabled": true, "details": {"reuse": reuse_of(&done1, "teapot")}},
            {"id": "kettle", "kind": "object", "name": "Kettle", "description": "A kettle on the stove.", "enabled": true, "details": {"prompt": "kettle"}},
            {"id": "tea", "kind": "voice_line", "name": "Granny speaks", "description": "She offers tea.", "enabled": true,
             "details": {"speaker": "granny", "text": "Tea, little one?"}}
        ],
        "layout": [{"item": "granny", "place": "center"}, {"item": "cat", "place": "left"}, {"item": "table", "place": "front"},
                   {"item": "teapot", "rel": {"to": "table", "how": "on"}}, {"item": "kettle", "place": "back"}]
    });
    let two = ready_in(&rig, Some(&one), &second).await;
    let asked = rig
        .ctx
        .planner
        .prompts
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    let grandma_model = done1_model(&done1, "grandma");
    assert!(asked.contains("THIS SCENE IS PART OF A PROJECT"), "{asked}");
    assert!(
        asked.contains(&grandma_model) && asked.contains("\"wood_floor\""),
        "{asked}"
    );
    let ready = get_json(&rig.app, &format!("/design/productions/{two}")).await;
    assert_eq!(
        (ready["project_id"].as_str(), ready["project_name"].as_str()),
        (Some(one.as_str()), Some("Breakfast"))
    );
    assert_eq!(
        ready["project_pieces"].as_array().unwrap().len(),
        4,
        "{ready}"
    );
    let granny = &item(&ready, "granny")["details"];
    assert_eq!(granny["reuse"]["from"], "Breakfast");
    assert_eq!(
        (granny["prompt"].as_str(), granny["voice"].as_str()),
        (Some("old woman"), Some("warm, old, slow"))
    );
    assert_eq!(granny["moves"], json!(["idle"]), "it has the moves it has");
    assert!(
        ready["plan"]["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n == "Granny keeps the moves it has, so it does not walk here."),
        "{ready}"
    );
    // Only the kettle (its lookup, its shape with its picture, its game-ready
    // pass) and the line count.
    assert_eq!(ready["eta_secs"], 2 + 250 + 30 + 4 + 80, "{ready}");
    assert!(
        ready["eta_unreused_secs"].as_i64().unwrap() > 1_000,
        "{ready}"
    );
    let done2 = make(&rig, &two).await;
    let runs: Vec<String> = rig.log()[made_before..]
        .iter()
        .filter(|l| l.starts_with("start "))
        .cloned()
        .collect();
    assert_eq!(runs, ["start generate kettle"], "only the kettle is built");
    for (now, was) in [
        ("granny", "grandma"),
        ("cat", "cat"),
        ("table", "table"),
        ("teapot", "teapot"),
    ] {
        let it = item(&done2, now);
        assert_eq!(
            (it["status"].as_str(), it["message"].as_str()),
            (Some("done"), Some("From “Breakfast”.")),
            "{it}"
        );
        assert_eq!(
            it["result"]["artifact_id"],
            item(&done1, was)["result"]["artifact_id"]
        );
        assert_eq!(
            it["result"]["version_id"],
            item(&done1, was)["result"]["version_id"]
        );
    }
    assert_eq!(
        item(&done2, "granny")["result"]["rig"],
        item(&done1, "grandma")["result"]["rig"]
    );
    let mut ids = scene_ids(&rig, &done2).await;
    ids.retain(|i| i != "ground");
    ids.sort();
    assert_eq!(ids, ["cat", "granny", "kettle", "table", "teapot"]);
    // Her new line is designed from the same name and voice as her first.
    let voice = std::fs::read_to_string(bin.join("voice.log")).unwrap();
    let said: Vec<&str> = voice
        .lines()
        .map(|l| l.split(" --speed").next().unwrap())
        .collect();
    assert_eq!(said.len(), 2, "{voice}");
    assert_eq!(said[0], said[1]);
    assert!(
        said[1].ends_with("--description Grandmother. warm, old, slow"),
        "{voice}"
    );
    // A third scene with nothing new: nothing is made, it is done at once.
    let third = json!({
        "title": "Later", "summary": "Later.", "notes": [],
        "items": [
            {"id": "granny", "kind": "character", "name": "Granny", "description": "She rests.", "enabled": true, "details": {"reuse": reuse_of(&done2, "granny")}},
            {"id": "kettle", "kind": "object", "name": "Kettle", "description": "The kettle.", "enabled": true, "details": {"reuse": reuse_of(&done2, "kettle")}}
        ],
        "layout": [{"item": "granny", "place": "center"}, {"item": "kettle", "place": "back"}]
    });
    let three = ready_in(&rig, Some(&two), &third).await;
    let ready3 = get_json(&rig.app, &format!("/design/productions/{three}")).await;
    assert_eq!(
        ready3["project_id"],
        one.as_str(),
        "the project of the scene it was started from"
    );
    assert_eq!(
        item(&ready3, "granny")["details"]["reuse"]["from"],
        "Breakfast",
        "where it was first made"
    );
    assert_eq!(item(&ready3, "kettle")["details"]["reuse"]["from"], "Tea");
    assert_eq!(ready3["eta_secs"], 0, "{ready3}");
    let before = rig.log().len();
    let t = std::time::Instant::now();
    let done3 = make(&rig, &three).await;
    assert!(
        t.elapsed() < std::time::Duration::from_secs(3),
        "{:?}",
        t.elapsed()
    );
    assert_eq!(rig.log().len(), before, "nothing is made");
    let ids = scene_ids(&rig, &done3).await;
    assert_eq!(ids.iter().filter(|i| *i != "ground").count(), 2, "{ids:?}");
    // My creations says which project each is in.
    let shelf = get_json(&rig.app, "/design/creations").await;
    let names: Vec<(&str, Option<&str>)> = shelf["productions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["project_id"].as_str().unwrap(),
                p["project_name"].as_str(),
            )
        })
        .collect();
    let one = one.as_str();
    assert_eq!(
        names,
        [
            (one, Some("Breakfast")),
            (one, Some("Breakfast")),
            (one, None)
        ]
    );
}

fn done1_model(p: &Value, id: &str) -> String {
    item(p, id)["result"]["artifact_id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// A new project starts clean: nothing of another project is offered or
/// taken, even when the planner names one of its models. Inside a project a
/// piece can be switched to one of the project's (never another model); a
/// piece whose model was deleted meanwhile is made instead, and says so.
#[tokio::test]
async fn a_new_project_reuses_nothing_and_a_deleted_piece_is_made_again() {
    let rig = Rig::new().await;
    let one = ready_in(&rig, None, &objects(&["rock"])).await;
    let done1 = make(&rig, &one).await;
    let rock = reuse_of(&done1, "rock");
    let mut stray = objects(&["rock", "pebble"]);
    stray["items"][0]["details"]["reuse"] = rock.clone();
    let fresh = ready_in(&rig, None, &stray).await;
    let asked = rig
        .ctx
        .planner
        .prompts
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert!(
        !asked.contains("PART OF A PROJECT")
            && !asked.contains(rock["artifact_id"].as_str().unwrap())
    );
    let p = get_json(&rig.app, &format!("/design/productions/{fresh}")).await;
    assert_eq!(p["project_id"], fresh.as_str());
    assert!(item(&p, "rock")["details"].get("reuse").is_none(), "{p}");
    assert!(
        p.get("project_pieces").is_none() && p.get("eta_unreused_secs").is_none(),
        "{p}"
    );
    // In the project: the pebble is switched to the made rock, and back.
    let two = ready_in(&rig, Some(&one), &objects(&["rock", "pebble"])).await;
    let uri = format!("/design/productions/{two}");
    let switch = |reuse: Value| json!({"items": [{"id": "pebble", "details": {"prompt": "pebble", "reuse": reuse}}]});
    let (st, p) = call_json(
        &rig.app,
        Method::PATCH,
        &uri,
        Some(switch(rock.clone())),
        false,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{p}");
    assert_eq!(item(&p, "pebble")["details"]["reuse"]["from"], "Props");
    assert_eq!(p["eta_secs"], 2 + 250 + 30 + 80, "only the rock is to make");
    let (st, body) = call_json(
        &rig.app,
        Method::PATCH,
        &uri,
        Some(switch(json!({"artifact_id": "not-a-piece"}))),
        false,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{body}");
    // The rock's model is deleted before Make: the pebble is made instead.
    let aid = rock["artifact_id"].as_str().unwrap();
    rig.ctx.design().store().delete_artifact(aid).await.unwrap();
    let before = rig.log().len();
    let done2 = make(&rig, &two).await;
    let runs: Vec<String> = rig.log()[before..]
        .iter()
        .filter(|l| l.starts_with("start "))
        .cloned()
        .collect();
    assert_eq!(runs, ["start generate rock", "start generate pebble"]);
    let note = "The pebble from “Props” was deleted, so Otto makes it again.";
    assert!(
        done2["plan"]["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n == note),
        "{done2}"
    );
    assert_ne!(
        item(&done2, "pebble")["result"]["artifact_id"].as_str(),
        Some(aid)
    );
}

/// With the built-in voices a reused character speaks in the voice of its
/// earlier lines, even when a new character before it in the plan would
/// have taken that voice: the new one gets another.
#[tokio::test]
async fn a_reused_character_keeps_its_built_in_voice() {
    let rig = Rig::new().await;
    let line = |id: &str, who: &str| json!({"id": id, "kind": "voice_line", "name": id, "description": "Talk.", "enabled": true, "details": {"speaker": who, "text": "Ready."}});
    let man = json!({"id": "man", "kind": "character", "name": "Man", "description": "A man.", "enabled": false, "details": {"voice": "deep and broken"}});
    let first = json!({"title": "Quest", "summary": "Talk.", "notes": [], "items": [man, line("l1", "man")]});
    let one = ready_in(&rig, None, &first).await;
    let done1 = make(&rig, &one).await;
    let voice_of = |p: &Value, l: &str| {
        let clip = item(p, l)["result"]["clip_id"]
            .as_str()
            .unwrap()
            .to_string();
        let store = rig.ctx.media.store().clone();
        async move { store.require_tts_clip(&clip).await.unwrap().clip.voice_id }
    };
    let kept = voice_of(&done1, "l1").await;
    // The man was never made (turned off), so a made stand-in carries his line.
    let mut p = rig
        .ctx
        .media
        .store()
        .require_production(&one)
        .await
        .unwrap();
    p.plan.items[0].status = "done".into();
    p.plan.items[0].result = Some(crate::productions::ItemResult {
        artifact_id: Some("man-model".into()),
        ..Default::default()
    });
    rig.ctx.media.store().save_production(&p).await.unwrap();
    let boy = json!({"id": "boy", "kind": "character", "name": "Boy", "description": "A deep-voiced boy.", "enabled": false, "details": {"voice": "deep male"}});
    let reused = json!({"id": "man", "kind": "character", "name": "Man", "description": "The man.", "enabled": false, "details": {"reuse": {"artifact_id": "man-model"}}});
    let second = json!({"title": "Again", "summary": "Talk.", "notes": [], "items": [boy, reused, line("l2", "boy"), line("l3", "man")]});
    let two = ready_in(&rig, Some(&one), &second).await;
    let done2 = make(&rig, &two).await;
    assert_eq!(voice_of(&done2, "l3").await, kept);
    assert_ne!(voice_of(&done2, "l2").await, kept);
}
