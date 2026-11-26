//! Characterization goldens on today's engine (test-plan Phase 0: §3.4
//! comparison 1 and the §3.5 chat goldens).
//!
//! Each scrubbed clone (tests/fixtures/workflows/compat/clones) runs in a
//! review-stubbed variant — the review_run node becomes a transform emitting
//! the review output its golden trace recorded — started from a chat
//! "Action: Workflow" block, through today's engine with the offline e2e agent
//! (OTTO_E2E=1, in a child process of this binary: the variable must not leak
//! into the other suites). What it did is written to
//! golden/<clone>/<scenario>.old.json (run and step states, step numbers,
//! every context file and its content, each agent step's exact prompt) and
//! golden/chat/<clone>-<scenario>.txt (every message, file and edit on the
//! recording channel adapter). The new engine must reproduce them after
//! normalization; ids, paths, times and durations are normalized here.
//!
//! Re-record (only on main's engine, or with a reason in the PR):
//! OTTO_WF_GOLDEN_WRITE=1 cargo test -p otto-server --test it wf_compat::

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use otto_channels::test_util::{install, Recorded, RecordingAdapter};
use otto_channels::workflow_trigger::WorkflowChatTrigger;
use otto_core::domain::Channel;
use otto_core::workflows::{RunStatus, Workflow, WorkflowGraph};
use otto_server::workflow_chat::WorkflowChatTriggerImpl;
use otto_server::ServerCtx;
use otto_state::{DbPool, WorkflowsRepo};
use serde_json::{json, Value};

const CHILD: &str = "OTTO_WF_COMPAT_CHILD";
const WS: &str = "ws-compat";
const CHAT: &str = "chat-compat";
const THREAD: &str = "thread-compat";
const USER: &str = "user-compat";

fn compat_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workflows/compat")
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
    )
    .unwrap()
}

/// One recorded scenario: a clone, and the golden trace whose review output
/// stands in for the review (None: the clone has no review step).
struct Scenario {
    clone: &'static str,
    name: &'static str,
    review_from: Option<&'static str>,
    jira_ticket: bool,
}

const SCENARIOS: &[Scenario] = &[
    Scenario {
        clone: "a",
        name: "passed",
        review_from: Some("success-median"),
        jira_ticket: true,
    },
    Scenario {
        clone: "a",
        name: "failed",
        review_from: Some("verdict-failed"),
        jira_ticket: true,
    },
    Scenario {
        clone: "b",
        name: "passed",
        review_from: Some("success-median"),
        jira_ticket: true,
    },
    Scenario {
        clone: "b",
        name: "failed",
        review_from: Some("verdict-failed"),
        jira_ticket: true,
    },
    Scenario {
        clone: "c",
        name: "passed",
        review_from: Some("success-median"),
        jira_ticket: true,
    },
    Scenario {
        clone: "c",
        name: "failed",
        review_from: Some("verdict-failed"),
        jira_ticket: true,
    },
    Scenario {
        clone: "d",
        name: "passed",
        review_from: Some("success-median"),
        jira_ticket: true,
    },
    Scenario {
        clone: "d",
        name: "failed",
        review_from: Some("verdict-failed"),
        jira_ticket: true,
    },
    Scenario {
        clone: "e",
        name: "no-ticket",
        review_from: None,
        jira_ticket: false,
    },
    Scenario {
        clone: "e",
        name: "ticket-unreadable",
        review_from: None,
        jira_ticket: true,
    },
];

/// Run \`test\` again in a child of this test binary with the offline e2e agent
/// on, a throwaway HOME (no skill or provider config is ever read from the
/// real one) and a PATH without agent CLIs. True inside the child.
fn in_offline_child(test: &str) -> bool {
    if std::env::var_os(CHILD).is_some() {
        return true;
    }
    let home = tempfile::tempdir().unwrap();
    #[allow(clippy::disallowed_methods)] // a sync re-exec of this binary, like workflow_node_driver
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("wf_compat::{test}"),
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .env("OTTO_E2E", "1")
        .env("HOME", home.path())
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success() && text.contains("1 passed"),
        "{test} in the offline child:\n{text}"
    );
    false
}

fn git(dir: &Path, args: &[&str]) {
    #[allow(clippy::disallowed_methods)]
    let out = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=Compat",
            "-c",
            "user.email=compat@example.invalid",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The clone with its review step replaced by a transform emitting the
/// recorded review output (scope agent, reviewers and summarizer cannot run
/// offline).
fn stubbed(clone: &Value, review: Option<&Value>) -> Value {
    let mut wf = clone.clone();
    for node in wf["graph"]["nodes"].as_array_mut().unwrap() {
        if node["kind"] == "review_run" {
            let r = review.expect("a review clone needs a trace");
            let threshold = node["params"]["threshold"].clone();
            node["kind"] = json!("transform");
            node["params"] = json!({"json": {
                "review_id": "review-1", "status": "done", "outcome": if r["passed"] == true { "passed" } else { "failed" },
                "score": r["score"], "passed": r["passed"], "threshold": threshold,
                "total": r["total"], "blocking": r["blocking"], "advisory": r["advisory"],
                "severity": r["severity"], "findings": [],
            }});
        }
    }
    wf
}

struct Harness {
    _tmp: tempfile::TempDir,
    data: PathBuf,
    repo: PathBuf,
    ctx: ServerCtx,
    pool: DbPool,
}

async fn harness() -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let (data, repo) = (root.join("data"), root.join("repo"));
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "compat\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", "compat"]);
    let pool = otto_state::open(&data.join("otto.db")).await.unwrap();
    let now = "2026-01-01T00:00:00Z";
    sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES ('root', 'root', 'x', 'Root', 1, ?)")
        .bind(now).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at) VALUES (?, 'Compat', ?, '{}', 0, ?)")
        .bind(WS).bind(root.to_string_lossy()).bind(now).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES (?, 'root', 'admin')",
    )
    .bind(WS)
    .execute(&pool)
    .await
    .unwrap();
    let ctx = ServerCtx::for_tests(&pool, data.clone()).await;
    let key = format!("chan-bot-{WS}-slack");
    ctx.secrets.put(&key, "compat-token").unwrap();
    ctx.integrations_store
        .upsert(
            &WS.into(),
            Channel::Slack,
            true,
            Some(key),
            None,
            "",
            false,
            "",
            CHAT,
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
    Harness {
        _tmp: tmp,
        data,
        repo,
        ctx,
        pool,
    }
}

/// What one scenario left behind, before normalization.
struct Outcome {
    golden: Value,
    chat: Vec<Recorded>,
}

async fn run_scenario(s: &Scenario) -> (Harness, Outcome) {
    let h = harness().await;
    let rec = RecordingAdapter::new(Channel::Slack, WS, CHAT, Some(THREAD));
    install(&rec);
    let dir = compat_dir();
    let clone = read_json(&dir.join("clones").join(format!("{}.json", s.clone)));
    let review = s.review_from.map(|t| {
        let trace = read_json(&dir.join("traces").join(s.clone).join(format!("{t}.json")));
        let node = trace["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["kind"] == "review_run")
            .cloned()
            .unwrap();
        node["review"].clone()
    });
    let wf = stubbed(&clone, review.as_ref());
    let graph: WorkflowGraph = serde_json::from_value(wf["graph"].clone()).unwrap();
    let repo = WorkflowsRepo::new(h.pool.clone());
    let created = repo
        .create(
            &WS.into(),
            wf["name"].as_str().unwrap(),
            wf["description"].as_str().unwrap(),
            wf["instructions"].as_str().unwrap(),
            &graph,
            &"root".into(),
        )
        .await
        .unwrap();

    // The message a person posts in the thread; the bridge replies with the ack.
    let mut text = format!(
        "Action: Workflow\nName: {}\nMsg: lorem ipsum dolor sit amet\nWorking Directory: {}\nBranch: main\nGoals:\n  - consectetur adipiscing\n",
        created.name,
        h.repo.display()
    );
    if s.jira_ticket {
        text.push_str("Jira ticket: COMPAT-1\n");
    }
    let msg = rec.inbound(USER, &text);
    let trigger = WorkflowChatTriggerImpl { ctx: h.ctx.clone() };
    let ack = trigger
        .try_start(
            &msg.workspace_id,
            "slack",
            &msg.chat,
            msg.thread.as_deref(),
            &msg.user,
            &msg.text,
        )
        .await
        .expect("an Action: Workflow block starts a run");
    let adapter: Arc<dyn otto_channels::adapter::Adapter> = rec.clone();
    adapter
        .send_formatted(&msg.chat, msg.thread.as_deref(), &ack.reply)
        .await
        .unwrap();

    let (run_id,): (String,) = sqlx::query_as("SELECT id FROM workflow_runs WHERE workflow_id = ?")
        .bind(&created.id)
        .fetch_one(&h.pool)
        .await
        .unwrap();
    let run_id: otto_core::Id = run_id;
    let deadline = Instant::now() + Duration::from_secs(120);
    let run = loop {
        let run = repo.get_run(&run_id).await.unwrap();
        let settled = matches!(
            run.status,
            RunStatus::Success | RunStatus::Error | RunStatus::Canceled
        );
        if settled && !otto_server::workflow_engine::driver_alive(&run_id) {
            break run;
        }
        assert!(
            Instant::now() < deadline,
            "{}/{} did not settle: {:?}",
            s.clone,
            s.name,
            run.status
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let golden = describe(&h, &created, &run, s);
    (
        h,
        Outcome {
            golden,
            chat: rec.recorded(),
        },
    )
}

/// The run as a person and the next step see it: states, numbers, files, prompts.
fn describe(
    h: &Harness,
    wf: &Workflow,
    run: &otto_core::workflows::WorkflowRun,
    s: &Scenario,
) -> Value {
    let ctx_dir = h.data.join("workflow-context").join(&run.id);
    let mut files = BTreeMap::new();
    if let Ok(entries) = std::fs::read_dir(&ctx_dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            files.insert(
                name,
                Value::String(std::fs::read_to_string(e.path()).unwrap_or_default()),
            );
        }
    }
    let step_no: BTreeMap<String, Value> = wf
        .graph
        .nodes
        .iter()
        .map(|n| {
            let slug = otto_server::workflow_context::slug(&n.name);
            let no = files.keys().find_map(|f| {
                let rest = f
                    .strip_prefix("step")?
                    .strip_suffix(&format!("-{slug}.md"))?;
                rest.parse::<u64>().ok()
            });
            (n.id.clone(), no.map_or(Value::Null, Value::from))
        })
        .collect();
    let prompts: Vec<Value> = std::env::var_os("OTTO_E2E_PROMPT_LOG")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .map(|p| {
            let mut text = p.as_str().unwrap().to_string();
            // Each stored prompt is the clone's; the golden pins what the engine adds around it.
            for n in &wf.graph.nodes {
                if let Some(stored) = n.params.get("prompt").and_then(Value::as_str) {
                    text = text.replace(stored, &format!("<stored prompt of {}>", n.id));
                }
                for skill in n
                    .params
                    .get("skills")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let name = skill.as_str().unwrap();
                    if let Some(body) = otto_skills::bundled_body(name) {
                        text = text.replace(&body, &format!("<bundled skill {name}>"));
                    }
                }
            }
            Value::String(text)
        })
        .collect();
    let nodes: Vec<Value> = run
        .nodes
        .iter()
        .map(|n| {
            json!({
                "id": n.node_id, "status": n.status, "error": n.error, "attempts": n.attempts,
                "sessions": n.sessions.len(), "logs": n.logs, "output": n.output,
            })
        })
        .collect();
    json!({
        "scenario": format!("{}/{}", s.clone, s.name),
        "run": {"status": run.status, "error": run.error},
        "step_numbers": step_no,
        "nodes": nodes,
        "context_files": files,
        "prompts": prompts,
    })
}

/// Replace what differs between two runs of the same scenario: paths, ids,
/// times, durations, commit hashes.
struct Normalizer {
    paths: Vec<(String, &'static str)>,
    ids: Vec<String>,
}

impl Normalizer {
    fn new(h: &Harness) -> Self {
        let mut paths = vec![];
        for (p, to) in [(&h.repo, "<repo>"), (&h.data, "<data>")] {
            paths.push((p.to_string_lossy().into_owned(), to));
            if let Some(raw) = p.to_str().and_then(|s| s.strip_prefix("/private")) {
                paths.push((raw.to_string(), to));
            }
        }
        Self { paths, ids: vec![] }
    }

    fn text(&mut self, s: &str) -> String {
        let mut out = s.to_string();
        for (p, to) in &self.paths {
            out = out.replace(p.as_str(), to);
        }
        let ulid = regex::Regex::new(r"\b[0-9A-HJKMNP-TV-Z]{26}\b").unwrap();
        for m in ulid.find_iter(&out.clone()) {
            if !self.ids.iter().any(|i| i == m.as_str()) {
                self.ids.push(m.as_str().to_string());
            }
        }
        for (i, id) in self.ids.iter().enumerate() {
            out = out.replace(id.as_str(), &format!("id-{}", i + 1));
            let short = &id[id.len() - 6..];
            out = regex::Regex::new(&format!(r"\b{short}\b"))
                .unwrap()
                .replace_all(&out, format!("short-{}", i + 1).as_str())
                .into_owned();
        }
        let rules: [(&str, &str); 4] = [
            (
                r"\b\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?(Z|[+-]\d\d:\d\d)?",
                "<time>",
            ),
            (r"\b[0-9a-f]{40}\b", "<sha>"),
            (r"\b\d+(\.\d+)?(ms|s)\b", "<dur>"),
            (r"\b\d+m \d+s\b", "<dur>"),
        ];
        for (re, to) in rules {
            out = regex::Regex::new(re)
                .unwrap()
                .replace_all(&out, to)
                .into_owned();
        }
        out
    }

    fn value(&mut self, v: &Value) -> Value {
        match v {
            Value::String(s) => Value::String(self.text(s)),
            Value::Array(a) => Value::Array(a.iter().map(|x| self.value(x)).collect()),
            Value::Object(o) => Value::Object(
                o.iter()
                    .map(|(k, x)| match k.as_str() {
                        "duration_ms" | "started_at" | "finished_at" | "updated_at"
                        | "created_at" => (k.clone(), Value::from("<time>")),
                        _ => (self.text(k), self.value(x)),
                    })
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    fn chat(&mut self, log: &[Recorded]) -> String {
        let mut out = String::new();
        for r in log {
            let (head, body) = match r {
                Recorded::Send {
                    id,
                    chat,
                    thread,
                    text,
                    formatted,
                } => (
                    format!(
                        "send {id} to {chat} {} {}",
                        thread.as_deref().unwrap_or("(no thread)"),
                        if *formatted { "formatted" } else { "plain" }
                    ),
                    text.clone(),
                ),
                Recorded::Edit {
                    chat,
                    message_id,
                    text,
                } => (format!("edit {message_id} in {chat}"), text.clone()),
                Recorded::Upload {
                    chat,
                    thread,
                    name,
                    content,
                } => (
                    format!(
                        "upload {name} to {chat} {}",
                        thread.as_deref().unwrap_or("(no thread)")
                    ),
                    String::from_utf8_lossy(content).into_owned(),
                ),
            };
            out.push_str(&format!("=== {head}\n{}\n", self.text(&body)));
        }
        out
    }
}

/// Compare with (or, with OTTO_WF_GOLDEN_WRITE=1, write) one golden file.
fn check_golden(path: &Path, got: &str, diffs: &mut Vec<String>) {
    if std::env::var_os("OTTO_WF_GOLDEN_WRITE").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(path).unwrap_or_default();
    if want != got {
        let line = want
            .lines()
            .zip(got.lines())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| want.lines().count().min(got.lines().count()));
        diffs.push(format!(
            "{} differs at line {}:\n  golden: {:?}\n  now:    {:?}",
            path.display(),
            line + 1,
            want.lines().nth(line).unwrap_or("<end>"),
            got.lines().nth(line).unwrap_or("<end>")
        ));
    }
}

/// proves: X1.a — the five clones load as definitions with every field and
/// pass today's validation unchanged.
/// proves: X1.b — they use exactly the seven kinds the exported workflows use.
#[test]
fn clones_import_and_validate() {
    let mut kinds = std::collections::BTreeSet::new();
    for c in ["a", "b", "c", "d", "e"] {
        let v = read_json(&compat_dir().join("clones").join(format!("{c}.json")));
        let wf: Workflow =
            serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("clone {c}: {e}"));
        let kept = |g: &Value| -> Vec<Value> {
            let nodes = g["nodes"].as_array().unwrap().iter();
            nodes
                .map(|n| json!([n["id"], n["kind"], n["name"], n["params"]]))
                .collect()
        };
        let back = serde_json::to_value(&wf.graph).unwrap();
        assert!(
            kept(&back) == kept(&v["graph"]) && back["edges"] == v["graph"]["edges"],
            "clone {c} does not round-trip"
        );
        let issues = otto_workflows::validation::validate(&wf.graph);
        assert!(issues.is_empty(), "clone {c}: {issues:?}");
        kinds.extend(wf.graph.nodes.iter().map(|n| n.kind.clone()));
    }
    let want = [
        "agent_prompt",
        "channel_notify",
        "condition",
        "log",
        "manual_trigger",
        "prepare_context",
        "review_run",
    ];
    assert_eq!(kinds.into_iter().collect::<Vec<_>>(), want);
}

/// proves: C1, C2, X1.f, X1.g — and part of X1.h and X7 (the bundled
/// templates, MCP tool names and chat control words are not covered here).
/// On today's engine, each clone's
/// review-stubbed run from a chat block produces exactly the golden step
/// numbers (both sides of the PASSED/FAILED join, the Test Cases fork),
/// context files and their contents, step-prompt bytes (preamble, input
/// excerpt, completion rules), notify renderings, routing, and old-style chat
/// transcript (ack, per-step lines and files, final brief).
#[test]
fn characterization_goldens_on_todays_engine() {
    if !in_offline_child("characterization_goldens_on_todays_engine") {
        return;
    }
    let log_dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut diffs = vec![];
    for s in SCENARIOS {
        let log = log_dir
            .path()
            .join(format!("{}-{}.prompts", s.clone, s.name));
        // The offline child is its own process running only this test.
        std::env::set_var("OTTO_E2E_PROMPT_LOG", &log);
        let (h, out) = rt.block_on(run_scenario(s));
        let mut norm = Normalizer::new(&h);
        let golden = norm.value(&out.golden);
        let chat = norm.chat(&out.chat);
        let dir = compat_dir().join("golden");
        check_golden(
            &dir.join(s.clone).join(format!("{}.old.json", s.name)),
            &(serde_json::to_string_pretty(&golden).unwrap() + "\n"),
            &mut diffs,
        );
        check_golden(
            &dir.join("chat").join(format!("{}-{}.txt", s.clone, s.name)),
            &chat,
            &mut diffs,
        );
    }
    assert!(
        diffs.is_empty(),
        "today's engine no longer matches its goldens:\n{}",
        diffs.join("\n")
    );
}
