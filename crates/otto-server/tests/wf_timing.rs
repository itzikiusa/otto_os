//! Timing replay T1 (test plan §4.1): each clone's committed median golden
//! trace runs through the real engine on the in-memory fake agents
//! (otto_agent_protocol::scripted), every fake sleep scaled by SCALE and the
//! engine's own costs at full size. Each agent step sleeps its measured work
//! (the trace's duration minus what the old engine added — the admission
//! gate's holds and the 20 s idle confirm, compat/timing.json); the review's
//! scope, reviewers and summarizer split the review's work.
//!
//! Per run it measures every step's engine overhead,
//! `(settled − ready) − scripted work`, and projects the wall-clock at full
//! size along the schedule the engine really ran. Each clone runs twice:
//! serial (`wf_max_parallel_steps = 1`, plus the old engine's waits added
//! back — this models today and is checked against the export's own
//! duration) and as the engine runs now. Pass lines: DIRECTION's numbers as
//! corrected in DECISIONS.md.
//!
//! The full local replay (every exported run) is
//! `scripts/workflows/timing-replay.py full`; it runs [`replay_every_trace`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use otto_agent_protocol::{scripted, AgentRuntime, RuntimeConfig, SessionManagerHost};
use otto_core::workflows::{NodeStatus, RunScope, RunStatus, WorkflowGraph, WorkflowRun};
use otto_server::ServerCtx;
use otto_state::WorkflowsRepo;
use serde_json::{json, Value};

/// A 10-minute step sleeps 0.6 s.
const SCALE: f64 = 0.001;
const WS: &str = "ws-timing";
/// How a review's work splits when the scope assessment ran (runs.md §4.1,
/// era B medians 144 / 327 / 226 s) and when it did not.
const SPLIT_SCOPE: [f64; 3] = [0.21, 0.47, 0.32];
const SPLIT_PLAIN: [f64; 3] = [0.0, 0.6, 0.4];

fn compat_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workflows/compat")
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
    )
    .unwrap()
}

fn git(dir: &Path, args: &[&str]) {
    #[allow(clippy::disallowed_methods)] // a test fixture's git checkout
    let out = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=Timing",
            "-c",
            "user.email=timing@example.invalid",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// One step's scripted work at full size, seconds.
#[derive(Default, Clone, Copy)]
struct Work {
    agent: f64,
    /// scope · slowest reviewer · summarizer (review steps).
    review: [f64; 3],
    /// What the old engine added besides the work (gate holds + idle confirm).
    old_extra: f64,
}

impl Work {
    fn total(&self) -> f64 {
        self.agent + self.review.iter().sum::<f64>()
    }
}

/// Each executed node's scripted work, from the trace and the old engine's waits.
fn work_of(trace: &Value, waits: &Value) -> BTreeMap<String, Work> {
    let mut out = BTreeMap::new();
    for n in trace["nodes"].as_array().unwrap() {
        let id = n["id"].as_str().unwrap();
        let dur = n["duration_s"].as_f64().unwrap_or(0.0);
        let w = &waits["nodes"][id];
        let extra = w[0].as_f64().unwrap_or(0.0) + w[1].as_f64().unwrap_or(0.0);
        let work = (dur - extra).max(1.0);
        let entry = match n["kind"].as_str().unwrap() {
            "agent_prompt" => Work {
                agent: work,
                old_extra: extra,
                ..Work::default()
            },
            "review_run" => {
                let split = if n["review"]["scope_ran"] == true {
                    SPLIT_SCOPE
                } else {
                    SPLIT_PLAIN
                };
                Work {
                    review: split.map(|s| s * work),
                    old_extra: extra,
                    ..Work::default()
                }
            }
            _ => continue,
        };
        out.insert(id.to_string(), entry);
    }
    out
}

fn ms(s: f64) -> u64 {
    (s * 1000.0).round() as u64
}

struct Rig {
    _tmp: tempfile::TempDir,
    ctx: ServerCtx,
    repo: PathBuf,
}

/// An isolated context on the in-memory fakes, the clone's step scripts and
/// review persona files written, and a checkout with a feature commit to review.
async fn rig(clone: &Value, work: &BTreeMap<String, Work>, slots: Option<u64>) -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let (data, repo, fakes, scripts) = (
        root.join("data"),
        root.join("repo"),
        root.join("review"),
        root.join("scripts"),
    );
    for d in [&data, &repo, &fakes, &scripts] {
        std::fs::create_dir_all(d).unwrap();
    }
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "timing\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "base"]);
    git(&repo, &["checkout", "-qb", "feature"]);
    std::fs::write(
        repo.join("lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a - b }\n",
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "feature"]);

    let mut review = [0.0f64; 3];
    for n in clone["graph"]["nodes"].as_array().unwrap() {
        let Some(w) = work.get(n["id"].as_str().unwrap()) else {
            continue;
        };
        if n["kind"] == "agent_prompt" {
            let slug = otto_server::workflow_context::slug(n["name"].as_str().unwrap());
            let script = format!(
                "sleep {}\nwrite $HANDOFF @filler:400\nreply done\n",
                ms(w.agent)
            );
            std::fs::write(scripts.join(format!("{slug}.txt")), script).unwrap();
        } else {
            review = w.review;
        }
    }
    // The scope agent keeps the declared lenses; reviewers and summarizer find nothing.
    let lenses: Vec<Value> = clone["graph"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["kind"] == "review_run")
        .flat_map(|n| {
            n["params"]["reviewers"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .map(|r| json!({"name": r["lens"], "reason": "touched", "instructions": "look"}))
        .collect();
    let plan =
        json!({"summary": "small change", "related_modules": [], "risks": [], "lenses": lenses});
    std::fs::write(
        fakes.join("scope.json"),
        json!([{"severity": "info", "body": plan.to_string()}]).to_string(),
    )
    .unwrap();
    // An orchestrator reviewer's merge must name every lens it ran (one
    // advisory each), and the final verifier then verifies them; separate
    // reviewers find nothing.
    let orchestrated = clone["graph"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["kind"] == "review_run" && n["params"]["mode"] == "orchestrator");
    if orchestrated {
        let notes: Vec<Value> = lenses
            .iter()
            .map(|l| json!({"path": "lib.rs", "line": 1, "severity": "info", "body": "fine", "lens": l["name"]}))
            .collect();
        let ids: Vec<String> = (0..notes.len())
            .map(|i| otto_review::engine::raw_claim_id(0, i))
            .collect();
        let draft = json!([{"source_ids": ids, "path": "lib.rs", "line": 1, "severity": "info", "body": "fine",
            "category": "correctness", "disposition": "verified", "evidence": "lib.rs:1"}]);
        std::fs::write(fakes.join("reviewer.json"), Value::Array(notes).to_string()).unwrap();
        std::fs::write(fakes.join("summarizer.json"), draft.to_string()).unwrap();
    }
    for (role, s) in ["scope", "reviewer", "summarizer"].iter().zip(review) {
        std::fs::write(
            fakes.join(format!("{role}.pre")),
            format!("sleep {}\n", ms(s)),
        )
        .unwrap();
    }

    let pool = otto_state::open(&data.join("otto.db")).await.unwrap();
    let now = "2026-01-01T00:00:00Z";
    sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES ('root', 'root', 'x', 'Root', 1, ?)")
        .bind(now).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at) VALUES (?, 'Timing', ?, '{}', 0, ?)")
        .bind(WS).bind(root.to_string_lossy()).bind(now).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES (?, 'root', 'admin')",
    )
    .bind(WS)
    .execute(&pool)
    .await
    .unwrap();
    let mut ctx = ServerCtx::for_tests(&pool, data.clone()).await;
    let host = SessionManagerHost {
        manager: Arc::clone(&ctx.manager),
        workspaces: otto_state::WorkspacesRepo::new(pool.clone()),
        publish: Box::new(|_| {}),
        notices: otto_agent_protocol::attention::AttentionNotices::new(
            otto_state::NotificationsRepo::new(pool.clone()),
            Arc::new(|_| {}),
        ),
    };
    let env: Vec<(String, String)> = [
        ("OTTO_FAKE_TIME_SCALE", SCALE.to_string()),
        ("OTTO_FAKE_SCRIPTS", scripts.to_string_lossy().into_owned()),
        ("OTTO_FAKE_REVIEW_DIR", fakes.to_string_lossy().into_owned()),
        (
            "OTTO_FAKE_STATE_DIR",
            root.join("fake-state").to_string_lossy().into_owned(),
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    ctx.agent_runtime = AgentRuntime::new(
        otto_state::AgentRuntimeRepo::new(pool.clone()),
        Arc::new(host),
        scripted::drivers(),
        RuntimeConfig {
            attachments_dir: root.join("attachments"),
            transport: Some(scripted::transport(env)),
            ..RuntimeConfig::default()
        },
    );
    let settings = otto_state::SettingsRepo::new(pool.clone());
    // The test machine's own load must not hold an agent (the busy-machine
    // rule is admission.rs's and the load test's).
    settings
        .put("agent_admission_enabled", &json!(false))
        .await
        .unwrap();
    if let Some(n) = slots {
        settings
            .put("wf_max_parallel_steps", &json!(n))
            .await
            .unwrap();
    }
    // Notify steps post to a recording channel adapter.
    ctx.outbound = otto_channels::test_util::RecordingAdapter::new(
        otto_core::domain::Channel::Slack,
        WS,
        "chat-timing",
        None,
    )
    .factory();
    let key = format!("chan-bot-{WS}-slack");
    ctx.secrets.put(&key, "timing-token").unwrap();
    ctx.integrations_store
        .upsert(
            &WS.into(),
            otto_core::domain::Channel::Slack,
            true,
            Some(key),
            None,
            "user-timing",
            false,
            "",
            "chat-timing",
            "",
        )
        .await
        .unwrap();
    ctx.git_store
        .create_repo(otto_state::git::NewRepo {
            workspace_id: WS.into(),
            name: "repo".into(),
            path: repo.to_string_lossy().into(),
            remote_url: None,
            provider: None,
            git_account_id: None,
        })
        .await
        .unwrap();
    Rig {
        _tmp: tmp,
        ctx,
        repo,
    }
}

/// Run the clone to its end on the rig.
async fn run(rig: &Rig, clone: &Value) -> WorkflowRun {
    // Timing only: the file contract each prompt names (its `References:`
    // line, which the fake checks) is wf_compat's, and a step may name a file
    // it writes itself.
    let mut graph: WorkflowGraph = serde_json::from_value(clone["graph"].clone()).unwrap();
    for n in &mut graph.nodes {
        if let Some(p) = n.params.get_mut("prompt") {
            let kept: Vec<&str> = p
                .as_str()
                .unwrap_or_default()
                .lines()
                .filter(|l| !l.starts_with("References:"))
                .collect();
            *p = Value::String(kept.join("\n"));
        }
    }
    let repo = WorkflowsRepo::new(rig.ctx.pool.clone());
    let wf = repo
        .create(
            &WS.into(),
            clone["name"].as_str().unwrap(),
            "",
            "",
            &graph,
            &"root".into(),
        )
        .await
        .unwrap();
    let input = json!({
        "working_directory": rig.repo.to_string_lossy(), "base": "main", "pr_branch": "feature",
        "msg": "lorem ipsum", "goals": ["consectetur adipiscing"],
    });
    let run = repo
        .create_run(&wf.id, &WS.into(), &input, Some(&"root".into()))
        .await
        .unwrap();
    let ws = rig.ctx.workspaces.get(&WS.into()).await.unwrap();
    tokio::time::timeout(
        Duration::from_secs(180),
        otto_server::workflow_engine::run_workflow(
            rig.ctx.clone(),
            ws,
            wf,
            run.id.clone(),
            input,
            RunScope::default(),
            None,
        ),
    )
    .await
    .expect("the replay settles");
    repo.get_run(&run.id).await.unwrap()
}

/// What one replay measured, seconds.
#[derive(Debug, Clone, serde::Serialize)]
struct Measured {
    run: String,
    slots: Option<u64>,
    status: String,
    /// The export's own duration of this run.
    today_s: f64,
    /// Full-size wall-clock along the schedule the engine ran.
    projected_s: f64,
    /// `projected_s` plus the old engine's waits (the serial replay's model of today).
    old_model_s: f64,
    wall_s: f64,
    /// Per agent step, `(settled − ready) − scripted work`.
    agent_overheads_s: Vec<f64>,
    review_overhead_s: Option<f64>,
}

fn end(n: &otto_core::workflows::NodeRunState) -> Option<DateTime<Utc>> {
    Some(n.started_at? + chrono::Duration::milliseconds(n.duration_ms? as i64))
}

fn secs(d: chrono::Duration) -> f64 {
    d.num_microseconds().unwrap_or(0) as f64 / 1e6
}

/// Overheads and the full-size projection of one settled replay.
fn measure(
    name: &str,
    slots: Option<u64>,
    trace: &Value,
    clone: &Value,
    work: &BTreeMap<String, Work>,
    run: &WorkflowRun,
) -> Measured {
    let graph: WorkflowGraph = serde_json::from_value(clone["graph"].clone()).unwrap();
    let ran: Vec<_> = run
        .nodes
        .iter()
        .filter(|n| n.started_at.is_some() && n.duration_ms.is_some())
        .collect();
    let t0 = ran.iter().filter_map(|n| n.started_at).min().unwrap();
    let t1 = ran.iter().filter_map(|n| end(n)).max().unwrap();
    let by_id: BTreeMap<&str, _> = ran.iter().map(|n| (n.node_id.as_str(), *n)).collect();
    let scripted = |id: &str| work.get(id).map_or(0.0, Work::total);
    let (mut agent, mut review) = (Vec::new(), None);
    for n in &ran {
        let ready = graph
            .edges
            .iter()
            .filter(|e| e.target == n.node_id)
            .filter_map(|e| by_id.get(e.source.as_str()).and_then(|p| end(p)))
            .max()
            .unwrap_or(t0);
        let ovh = secs(end(n).unwrap() - ready) - SCALE * scripted(&n.node_id);
        match graph
            .nodes
            .iter()
            .find(|g| g.id == n.node_id)
            .map(|g| g.kind.as_str())
        {
            Some("agent_prompt") => agent.push(ovh),
            Some("review_run") => review = Some(ovh),
            _ => {}
        }
    }
    // Longest path at full size: a node follows every node that ended before
    // it started (the schedule the engine ran); its own time is its work at
    // full size plus what the engine spent around it at real size.
    let mut order = ran.clone();
    order.sort_by_key(|n| (n.started_at, end(n)));
    let mut proj_end: BTreeMap<&str, f64> = BTreeMap::new();
    for n in &order {
        let start = n.started_at.unwrap();
        // (an instant node that ended as this one started was ordered before it)
        let before = order.iter().filter(|p| end(p).unwrap() <= start);
        let (base, last_end) = before
            .filter_map(|p| Some((*proj_end.get(p.node_id.as_str())?, end(p)?)))
            .fold((0.0f64, t0), |(b, l), (pe, e)| (b.max(pe), l.max(e)));
        let own =
            secs(end(n).unwrap() - start) - SCALE * scripted(&n.node_id) + scripted(&n.node_id);
        proj_end.insert(n.node_id.as_str(), base + secs(start - last_end) + own);
    }
    let projected = proj_end.values().cloned().fold(0.0, f64::max);
    let old_extra: f64 = work
        .iter()
        .filter(|(id, _)| by_id.contains_key(id.as_str()))
        .map(|(_, w)| w.old_extra)
        .sum();
    Measured {
        run: name.to_string(),
        slots,
        status: format!("{:?}", run.status),
        today_s: trace["run"]["duration_s"].as_f64().unwrap_or(0.0),
        projected_s: projected,
        old_model_s: projected + old_extra,
        wall_s: secs(t1 - t0),
        agent_overheads_s: agent,
        review_overhead_s: review,
    }
}

async fn replay(
    name: &str,
    clone: &Value,
    trace: &Value,
    waits: &Value,
    slots: Option<u64>,
) -> Measured {
    let work = work_of(trace, waits);
    let rig = rig(clone, &work, slots).await;
    let run = run(&rig, clone).await;
    let failed: Vec<_> = run
        .nodes
        .iter()
        .filter(|n| n.status == NodeStatus::Error)
        .map(|n| (&n.node_id, &n.error))
        .collect();
    assert_eq!(
        run.status,
        RunStatus::Success,
        "{name}: {:?} {failed:?}",
        run.error
    );
    // Every scripted step really slept its work: the fake ran it, not a shortcut.
    for (id, w) in &work {
        let n = run.nodes.iter().find(|n| &n.node_id == id).unwrap();
        if n.status == NodeStatus::Success {
            let took = n.duration_ms.unwrap() as f64 / 1000.0;
            assert!(
                took >= SCALE * w.total() * 0.95,
                "{name}/{id}: {took}s < scripted {}",
                SCALE * w.total()
            );
        }
    }
    measure(name, slots, trace, clone, &work, &run)
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() as f64 - 1.0) * p).round() as usize]
}

fn table(rows: &[Measured]) -> String {
    let mut s = String::from("| run | slots | today min | old model min | projected min | wall s | agent ovh p50 / p95 / max s | review ovh s |\n|---|---|---|---|---|---|---|---|\n");
    for r in rows {
        let mut o = r.agent_overheads_s.clone();
        let (p50, p95, max) = if o.is_empty() {
            (0.0, 0.0, 0.0)
        } else {
            (pct(&mut o, 0.5), pct(&mut o, 0.95), pct(&mut o, 1.0))
        };
        s += &format!(
            "| {} | {} | {:.1} | {:.1} | {:.1} | {:.2} | {p50:.3} / {p95:.3} / {max:.3} | {} |\n",
            r.run,
            r.slots.map_or("default".into(), |n| n.to_string()),
            r.today_s / 60.0,
            r.old_model_s / 60.0,
            r.projected_s / 60.0,
            r.wall_s,
            r.review_overhead_s
                .map_or("-".into(), |v| format!("{v:.3}")),
        );
    }
    s
}

fn report(rows: &[Measured]) {
    let text = table(rows);
    println!("{text}");
    if let Some(path) = std::env::var_os("OTTO_WF_TIMING_REPORT") {
        std::fs::write(&path, serde_json::to_string_pretty(rows).unwrap()).unwrap();
        std::fs::write(Path::new(&path).with_extension("md"), text).unwrap();
    }
}

/// The CI-sized replay: one median trace per clone, serial and now.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn median_traces_replay_inside_the_acceptance_numbers() {
    let dir = compat_dir();
    let timing = read_json(&dir.join("timing.json"));
    let mut rows = Vec::new();
    for c in ["a", "b", "c", "d", "e"] {
        let clone = read_json(&dir.join("clones").join(format!("{c}.json")));
        let trace = read_json(&dir.join("traces").join(c).join("success-median.json"));
        let waits = &timing[format!("{c}/success-median")];
        for slots in [Some(1), None] {
            rows.push(replay(&format!("{c}/success-median"), &clone, &trace, waits, slots).await);
        }
    }
    report(&rows);
    let now = |c: &str| {
        rows.iter()
            .find(|r| r.run.starts_with(c) && r.slots.is_none())
            .unwrap()
    };
    let serial = |c: &str| {
        rows.iter()
            .find(|r| r.run.starts_with(c) && r.slots == Some(1))
            .unwrap()
    };
    // The serial replay with the old waits added back is today's run (model check).
    for c in ["a", "b", "c", "d", "e"] {
        let r = serial(c);
        let off = (r.old_model_s - r.today_s).abs() / r.today_s;
        assert!(
            off < 0.10,
            "{c}: the old-engine model {:.0}s vs the export's {:.0}s",
            r.old_model_s,
            r.today_s
        );
    }
    // Test plan §4.3's lines, for the clones whose median trace is from era B
    // (the gate's 300 s holds): a = PR Reviewer. The era-A medians (b, d, e and
    // c = Quick Review) had no gate to remove; they must only not be slower.
    assert_eq!(timing["a/success-median"]["era"], "B");
    assert!(
        now("a").projected_s <= 20.0 * 60.0,
        "PR Reviewer unchanged: {:.1} min",
        now("a").projected_s / 60.0
    );
    assert!(
        now("c").projected_s <= 14.0 * 60.0,
        "Quick unchanged: {:.1} min",
        now("c").projected_s / 60.0
    );
    for c in ["a", "b", "c", "d", "e"] {
        assert!(
            now(c).projected_s <= serial(c).old_model_s,
            "{c}: slower than today's model"
        );
    }
    // The fork (e) runs its two sweeps side by side: faster than serial.
    assert!(
        now("e").projected_s < serial("e").projected_s - 60.0,
        "e: the fork did not overlap"
    );
    let mut all: Vec<f64> = rows
        .iter()
        .flat_map(|r| r.agent_overheads_s.clone())
        .collect();
    let (p95, max) = (pct(&mut all, 0.95), pct(&mut all, 1.0));
    assert!(
        p95 <= 1.0 && max <= 2.0,
        "engine overhead per agent step p95 {p95:.3}s, max {max:.3}s"
    );
}

/// The full replay over a directory the local script writes (every exported
/// run, scrubbed, outside the repository): `OTTO_WF_TIMING_DIR`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "local: scripts/workflows/timing-replay.py full"]
async fn replay_every_trace() {
    let dir = PathBuf::from(std::env::var("OTTO_WF_TIMING_DIR").expect("OTTO_WF_TIMING_DIR"));
    let timing = read_json(&dir.join("timing.json"));
    let mut rows = Vec::new();
    for (key, waits) in timing.as_object().unwrap() {
        let (c, name) = key.split_once('/').unwrap();
        let clone = read_json(&dir.join("clones").join(format!("{c}.json")));
        let trace = read_json(&dir.join("traces").join(c).join(format!("{name}.json")));
        rows.push(replay(key, &clone, &trace, waits, None).await);
    }
    report(&rows);
}
