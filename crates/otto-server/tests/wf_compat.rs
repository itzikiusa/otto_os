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

use otto_channels::admission::Admission;
use otto_channels::test_util::{Recorded, RecordingAdapter};
use otto_channels::Inbound;
use otto_core::domain::Channel;
use otto_core::workflows::{NodeStatus, RunStatus, Workflow, WorkflowGraph};
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

/// One recorded scenario: a clone, the golden trace whose review output
/// stands in for the review (None: the clone has no review step), whether
/// the message names a ticket, and whether it carries Slack file attachments
/// (downloaded from a wiremock Slack through the listener's real path).
struct Scenario {
    clone: &'static str,
    name: &'static str,
    review_from: Option<&'static str>,
    jira_ticket: bool,
    slack_files: bool,
}

const fn sc(
    clone: &'static str,
    name: &'static str,
    review_from: Option<&'static str>,
    jira_ticket: bool,
) -> Scenario {
    Scenario {
        clone,
        name,
        review_from,
        jira_ticket,
        slack_files: false,
    }
}

const SCENARIOS: &[Scenario] = &[
    sc("a", "passed", Some("success-median"), true),
    sc("a", "failed", Some("verdict-failed"), true),
    sc("b", "passed", Some("success-median"), true),
    sc("b", "failed", Some("verdict-failed"), true),
    sc("c", "passed", Some("success-median"), true),
    sc("c", "failed", Some("verdict-failed"), true),
    Scenario {
        slack_files: true,
        ..sc("c", "passed-with-files", Some("success-median"), true)
    },
    sc("d", "passed", Some("success-median"), true),
    sc("d", "failed", Some("verdict-failed"), true),
    sc("e", "no-ticket", None, false),
    sc("e", "ticket-unreadable", None, true),
];

/// Run \`test\` again in a child of this test binary with the offline e2e agent
/// on, a throwaway HOME (no skill or provider config is ever read from the
/// real one) and a PATH without agent CLIs. True inside the child.
fn in_offline_child(test: &str) -> bool {
    if std::env::var_os(CHILD).is_some() {
        return true;
    }
    let home = tempfile::tempdir().unwrap();
    // Attachments the Slack listener downloads land here, not in the real temp dir.
    let tmpdir = home.path().join("tmp");
    std::fs::create_dir_all(&tmpdir).unwrap();
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
        .env("TMPDIR", &tmpdir)
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
    rec: Arc<RecordingAdapter>,
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
    // Every outbound workflow message lands on the recording adapter.
    let rec = RecordingAdapter::new(Channel::Slack, WS, CHAT, Some(THREAD));
    let mut ctx = ServerCtx::for_tests(&pool, data.clone()).await;
    ctx.outbound = rec.factory();
    let key = format!("chan-bot-{WS}-slack");
    ctx.secrets.put(&key, "compat-token").unwrap();
    ctx.integrations_store
        .upsert(
            &WS.into(),
            Channel::Slack,
            true,
            Some(key),
            None,
            USER,
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
        rec,
    }
}

/// What one scenario left behind, before normalization.
struct Outcome {
    golden: Value,
    chat: Vec<Recorded>,
    /// The wiremock Slack's base URL, when the message carried files.
    slack: Option<String>,
    run: otto_core::workflows::WorkflowRun,
}

/// The person's message as the Slack listener hands it to the bridge: plain,
/// or with two attachments (one downloads, one is gone) fetched from a
/// wiremock Slack through the listener's real download path.
async fn inbound(h: &Harness, text: &str, files: bool) -> (Inbound, Option<wiremock::MockServer>) {
    if !files {
        return (h.rec.inbound(USER, text), None);
    }
    use wiremock::matchers::{method, path};
    let slack = wiremock::MockServer::start().await;
    wiremock::Mock::given(method("GET"))
        .and(path("/files/lorem.txt"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("lorem ipsum\n"))
        .mount(&slack)
        .await;
    let event = json!({
        "type": "message", "channel": CHAT, "user": USER, "ts": "1700000000.000200", "thread_ts": THREAD,
        "text": text,
        "files": [
            {"id": "F1", "name": "lorem.txt", "mimetype": "text/plain",
             "url_private_download": format!("{}/files/lorem.txt", slack.uri())},
            {"id": "F2", "name": "ipsum.pdf", "mimetype": "application/pdf",
             "url_private_download": format!("{}/files/ipsum.pdf", slack.uri())},
        ],
    });
    let integ = h
        .ctx
        .integrations_store
        .get(&WS.into(), Channel::Slack)
        .await
        .unwrap()
        .unwrap();
    let msg = otto_channels::test_util::slack_inbound(&event, &integ, "compat-token")
        .await
        .expect("a message with text");
    let saved = std::fs::read(std::env::temp_dir().join("otto-slack-F1-lorem.txt")).unwrap();
    assert_eq!(
        saved, b"lorem ipsum\n",
        "the listener saved the file's bytes"
    );
    (msg, Some(slack))
}

/// Run one scenario; `slots` sets `wf_max_parallel_steps` (None: the
/// default), `edit` changes the stubbed clone before it is created.
async fn run_scenario(
    s: &Scenario,
    slots: Option<u64>,
    edit: fn(&mut Value),
) -> (Harness, Outcome) {
    let h = harness().await;
    if let Some(n) = slots {
        otto_state::SettingsRepo::new(h.pool.clone())
            .put("wf_max_parallel_steps", &json!(n))
            .await
            .unwrap();
    }
    let dir = compat_dir();
    let clone = read_json(&dir.join("clones").join(format!("{}.json", s.clone)));
    let review = s.review_from.map(|t| {
        let trace = read_json(&dir.join("traces").join(s.clone).join(format!("{t}.json")));
        let nodes = trace["nodes"].as_array().unwrap();
        nodes.iter().find(|n| n["kind"] == "review_run").unwrap()["review"].clone()
    });
    let mut wf = stubbed(&clone, review.as_ref());
    edit(&mut wf);
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

    // The message a person posts in the thread, through the real bridge
    // (which replies with the ack on the same adapter).
    let mut text = format!(
        "Action: Workflow\nName: {}\nMsg: lorem ipsum dolor sit amet\nWorking Directory: {}\nBranch: main\nGoals:\n  - consectetur adipiscing\n",
        created.name,
        h.repo.display()
    );
    if s.jira_ticket {
        text.push_str("Jira ticket: COMPAT-1\n");
    }
    let (msg, slack) = inbound(&h, &text, s.slack_files).await;
    let bridge = otto_channels::Bridge::new_with_workflow_trigger(
        h.ctx.manager.clone(),
        h.ctx.workspaces.clone(),
        otto_state::SettingsRepo::new(h.pool.clone()),
        otto_channels::Mirror::new(h.ctx.manager.clone()),
        "root".into(),
        Some(Arc::new(WorkflowChatTriggerImpl { ctx: h.ctx.clone() })),
        Default::default(),
    );
    let integ = h
        .ctx
        .integrations_store
        .get(&WS.into(), Channel::Slack)
        .await
        .unwrap()
        .unwrap();
    let Admission::Work(permit) = bridge
        .admission
        .admit("slack", &msg.chat, msg.thread.as_deref())
    else {
        panic!("the bridge admits one message");
    };
    bridge.handle(&integ, h.rec.clone(), msg, permit).await;

    let (run_id,): (String,) = sqlx::query_as("SELECT id FROM workflow_runs WHERE workflow_id = ?")
        .bind(&created.id)
        .fetch_one(&h.pool)
        .await
        .expect("the bridge started a run");
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
    let chat = h.rec.recorded();
    let slack = slack.map(|m| m.uri());
    (
        h,
        Outcome {
            golden,
            chat,
            slack,
            run,
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
    // New beside today's files (DR-201): the starting message's files are
    // copied into the run's context dir. Asserted here, left out of the
    // Phase-0 listing.
    let attached = otto_workflows::attachments::read(&ctx_dir);
    if s.slack_files {
        let got: Vec<(&str, Option<&str>, bool)> = attached
            .iter()
            .map(|a| (a.name.as_str(), a.path.as_deref(), a.error.is_some()))
            .collect();
        assert_eq!(
            got,
            [
                ("lorem.txt", Some("attachments/lorem.txt"), false),
                ("ipsum.pdf", None, true),
            ]
        );
        assert_eq!(
            std::fs::read(ctx_dir.join("attachments/lorem.txt")).unwrap(),
            b"lorem ipsum\n",
            "the run keeps its own copy of the file"
        );
    } else {
        assert!(attached.is_empty(), "{attached:?}");
    }
    let mut files = BTreeMap::new();
    if let Ok(entries) = std::fs::read_dir(&ctx_dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if matches!(
                name.as_str(),
                otto_workflows::attachments::ATTACHMENTS_DIR
                    | otto_workflows::attachments::ATTACHMENTS_FILE
            ) {
                continue;
            }
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
    fn new(h: &Harness, slack: Option<&str>) -> Self {
        let mut paths = vec![];
        let tmp = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        for (p, to) in [(&h.repo, "<repo>"), (&h.data, "<data>"), (&tmp, "<tmp>")] {
            let p = p.to_string_lossy().trim_end_matches('/').to_string();
            let raw = p.strip_prefix("/private").map(str::to_string);
            paths.push((p, to));
            paths.extend(raw.map(|raw| (raw, to)));
        }
        paths.extend(slack.map(|s| (s.to_string(), "<slack>")));
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
                    formatted,
                } => (
                    format!(
                        "edit {message_id} in {chat}{}",
                        if *formatted { " formatted" } else { "" }
                    ),
                    text.clone(),
                ),
                Recorded::Upload {
                    chat,
                    thread,
                    name,
                    content,
                    caption,
                } => (
                    format!(
                        "upload {name} to {chat} {}{}",
                        thread.as_deref().unwrap_or("(no thread)"),
                        caption
                            .as_deref()
                            .map(|c| format!(" caption {c:?}"))
                            .unwrap_or_default()
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

/// Test plan X1.a: the five clones load as definitions with every field and
/// pass today's validation unchanged.
/// X1.b: they use exactly the seven kinds the exported workflows use.
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

/// On today's engine, each clone's review-stubbed runs from a chat block
/// produce exactly the golden step numbers (both sides of the PASSED/FAILED
/// join, the Test Cases fork), context files and their contents, step-prompt
/// bytes (preamble, input excerpt, completion rules), notify renderings,
/// routing, and old-style chat transcript (ack, per-step lines and files,
/// final brief). One test per clone, so each stays well inside CI's per-test
/// time limit.
fn goldens_of(clone: &str) {
    if !in_offline_child(&format!("golden_{clone}")) {
        return;
    }
    let log_dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut diffs = vec![];
    for s in SCENARIOS.iter().filter(|s| s.clone == clone) {
        let log = log_dir.path().join(format!("{}.prompts", s.name));
        // The offline child is its own process running only this test.
        std::env::set_var("OTTO_E2E_PROMPT_LOG", &log);
        // Clone E's fork runs one sweep at a time here, so its per-step chat
        // lines keep their recorded order; side by side is proven below.
        let slots = (s.clone == "e").then_some(1);
        let (h, out) = rt.block_on(run_scenario(s, slots, |_| {}));
        let mut norm = Normalizer::new(&h, out.slack.as_deref());
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

/// Test plan C1, C2, X1.f, X1.g, and part of X1.h and X7 (the bundled
/// templates, MCP tool names and chat control words are not covered here):
/// clone A (12-node review), passed and failed verdicts.
#[test]
fn golden_a() {
    goldens_of("a");
}

/// Clone B (the 12-node review's twin), passed and failed verdicts.
#[test]
fn golden_b() {
    goldens_of("b");
}

/// Clone C (10-node quick review), passed and failed verdicts.
#[test]
fn golden_c() {
    goldens_of("c");
}

/// Clone D (feature review), passed and failed verdicts.
#[test]
fn golden_d() {
    goldens_of("d");
}

/// Clone E (11-node document workflow with the fork): no ticket, and a
/// required ticket that cannot be read.
#[test]
fn golden_e() {
    goldens_of("e");
}

/// Engine parallelism (engine.md §3, §5.3): at the default step slots clone
/// E's two sweeps (n10, n11) run at the same time, under the numbers the
/// serial engine gave them (6:n5 7:n10 8:n11 9:n6). Each sweep holds for
/// 1.5 s, so they overlap only when they really run side by side.
#[test]
fn fork_sweeps_run_side_by_side() {
    if !in_offline_child("fork_sweeps_run_side_by_side") {
        return;
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let s = SCENARIOS
        .iter()
        .find(|s| s.clone == "e" && s.name == "no-ticket")
        .unwrap();
    let (_h, out) = rt.block_on(run_scenario(s, None, |wf| {
        for node in wf["graph"]["nodes"].as_array_mut().unwrap() {
            if node["id"] == "n10" || node["id"] == "n11" {
                let prompt = node["params"]["prompt"].as_str().unwrap_or("").to_string();
                node["params"]["prompt"] =
                    json!(format!("{prompt}\nOTTO_E2E_SUBAGENTS: 1 hold_ms=1500"));
            }
        }
    }));
    let state = |id: &str| out.run.nodes.iter().find(|n| n.node_id == id).unwrap();
    let numbers: Vec<_> = ["n5", "n10", "n11", "n6"]
        .iter()
        .map(|id| state(id).step_no)
        .collect();
    assert_eq!(numbers, [Some(6), Some(7), Some(8), Some(9)]);
    let span = |id: &str| {
        let st = state(id);
        assert_eq!(st.status, NodeStatus::Success, "{id}: {:?}", st.error);
        let start = st.started_at.unwrap();
        let ms = st.duration_ms.unwrap() as i64;
        assert!(ms >= 1_400, "{id} held for its sweep: {ms} ms");
        (start, start + chrono::Duration::milliseconds(ms))
    };
    let ((a0, a1), (b0, b1)) = (span("n10"), span("n11"));
    assert!(
        a0 < b1 && b0 < a1,
        "the sweeps ran one after the other: n10 {a0}..{a1}, n11 {b0}..{b1}"
    );
}

/// Chat live style (engine.md §9.2, DR-113) through the real bridge and
/// engine: the run's thread gets the ack, ONE status message edited in place
/// (formatted), every step file captioned, no per-step text lines, and a
/// final post that says the run passed. The Phase-0 goldens above prove the
/// per_step style is byte for byte today's.
#[test]
fn live_style_keeps_one_status_message_and_captions_every_file() {
    if !in_offline_child("live_style_keeps_one_status_message_and_captions_every_file") {
        return;
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let s = SCENARIOS
        .iter()
        .find(|s| s.clone == "c" && s.name == "passed")
        .unwrap();
    let (_h, out) = rt.block_on(run_scenario(s, None, |wf| {
        for node in wf["graph"]["nodes"].as_array_mut().unwrap() {
            if node["kind"] == "manual_trigger" {
                node["params"]["chat_updates"] = json!("live");
            }
        }
    }));
    assert_eq!(out.run.status, RunStatus::Success, "{:?}", out.run.error);
    let sends: Vec<&str> = out
        .chat
        .iter()
        .filter_map(|r| match r {
            Recorded::Send { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let card = sends
        .iter()
        .position(|t| t.starts_with("*Compat C*"))
        .unwrap_or_else(|| panic!("a status message: {sends:#?}"));
    assert!(
        !sends
            .iter()
            .any(|t| t.contains("▶ *") || t.contains(" done (")),
        "no per-step lines in live: {sends:#?}"
    );
    assert!(
        sends.last().unwrap().starts_with("✅ *Compat C*"),
        "the final post: {sends:#?}"
    );
    let card_id = format!("m{}", card + 1);
    let edits: Vec<_> = out
        .chat
        .iter()
        .filter_map(|r| match r {
            Recorded::Edit {
                message_id,
                formatted,
                ..
            } => Some((message_id.clone(), *formatted)),
            _ => None,
        })
        .collect();
    assert!(!edits.is_empty(), "the card is edited in place");
    assert!(
        edits.iter().all(|(id, f)| *id == card_id && *f),
        "only the card, formatted: {edits:?} (card {card_id})"
    );
    let step_files: Vec<_> = out
        .chat
        .iter()
        .filter_map(|r| match r {
            Recorded::Upload { name, caption, .. } if name.starts_with("step") => Some(caption),
            _ => None,
        })
        .collect();
    assert!(!step_files.is_empty());
    assert!(
        step_files
            .iter()
            .all(|c| c.as_deref().is_some_and(|c| c.starts_with('✅'))),
        "every step file captioned: {step_files:?}"
    );
}

/// Regression: `channel_notify` sends directly over the outbound adapter while
/// `▶ *step* started` is queued on the background progress pump. Even when
/// formatted progress posts are delayed (simulating chat API latency or CI
/// scheduling lag), `channel_notify` must flush the pump first so its plain
/// message never lands ahead of `▶ *step* started`.
#[tokio::test]
async fn channel_notify_waits_for_progress_pump() {
    let h = harness().await;
    h.rec.delay_formatted(Duration::from_millis(30));
    // Build the edge with variables rather than a JSON source literal so
    // `otto-core`'s background-session-source scanner does not mistake the
    // node id for a session `meta.source` value.
    let (from, to) = ("n1", "n2");
    let graph: WorkflowGraph = serde_json::from_value(json!({
        "nodes": [
            {
                "id": from,
                "kind": "manual_trigger",
                "name": "Start 1",
                "params": { "chat_updates": "per_step" }
            },
            {
                "id": to,
                "kind": "channel_notify",
                "name": "Notify 9",
                "params": { "message": "notify body" }
            }
        ],
        "edges": [
            { "id": "e1", "source": from, "target": to }
        ]
    }))
    .unwrap();
    let repo = WorkflowsRepo::new(h.pool.clone());
    let created = repo
        .create(&WS.into(), "Notify Order", "", "", &graph, &"root".into())
        .await
        .unwrap();
    let text = format!(
        "Action: Workflow\nName: {}\nMsg: test\nWorking Directory: {}\nBranch: main\n",
        created.name,
        h.repo.display()
    );
    let msg = h.rec.inbound(USER, &text);
    let bridge = otto_channels::Bridge::new_with_workflow_trigger(
        h.ctx.manager.clone(),
        h.ctx.workspaces.clone(),
        otto_state::SettingsRepo::new(h.pool.clone()),
        otto_channels::Mirror::new(h.ctx.manager.clone()),
        "root".into(),
        Some(Arc::new(WorkflowChatTriggerImpl { ctx: h.ctx.clone() })),
        Default::default(),
    );
    let integ = h
        .ctx
        .integrations_store
        .get(&WS.into(), Channel::Slack)
        .await
        .unwrap()
        .unwrap();
    let Admission::Work(permit) = bridge
        .admission
        .admit("slack", &msg.chat, msg.thread.as_deref())
    else {
        panic!("the bridge admits one message");
    };
    bridge.handle(&integ, h.rec.clone(), msg, permit).await;
    let (run_id,): (String,) = sqlx::query_as("SELECT id FROM workflow_runs WHERE workflow_id = ?")
        .bind(&created.id)
        .fetch_one(&h.pool)
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let run = loop {
        let run = repo.get_run(&run_id).await.unwrap();
        if matches!(
            run.status,
            RunStatus::Success | RunStatus::Error | RunStatus::Canceled
        ) && !otto_server::workflow_engine::driver_alive(&run_id)
        {
            break run;
        }
        assert!(Instant::now() < deadline, "run did not settle");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(run.status, RunStatus::Success, "{:?}", run.error);
    let sends: Vec<(bool, String)> = h
        .rec
        .recorded()
        .into_iter()
        .filter_map(|r| match r {
            Recorded::Send {
                formatted, text, ..
            } => Some((formatted, text)),
            _ => None,
        })
        .collect();
    // Assert relative order rather than exact send count: when sibling workflow
    // tests occupy global run slots, a queued/dequeued announcement can add an
    // extra send before the run starts.
    let pos = |pred: &dyn Fn(&(bool, String)) -> bool, label: &str| {
        sends
            .iter()
            .position(pred)
            .unwrap_or_else(|| panic!("missing {label} in {sends:#?}"))
    };
    let ack = pos(
        &|(f, t)| *f && t.starts_with("🚀 Started workflow"),
        "workflow ack",
    );
    let run_started = pos(
        &|(f, t)| *f && t.starts_with("🚀 *Notify Order* started"),
        "run started",
    );
    let step_started = pos(&|(f, t)| *f && t == "▶ *Notify 9* started", "step started");
    let notify = pos(&|(f, t)| !*f && t == "notify body", "plain notify body");
    let step_done = pos(
        &|(f, t)| *f && t.starts_with("✅ *Notify 9* done"),
        "step done",
    );
    let run_done = pos(
        &|(f, t)| *f && t.starts_with("✅ *Notify Order* — success"),
        "run done",
    );
    assert!(
        ack < run_started
            && run_started < step_started
            && step_started < notify
            && notify < step_done
            && step_done < run_done,
        "out-of-order sends: ack={ack}, run_started={run_started}, step_started={step_started}, notify={notify}, step_done={step_done}, run_done={run_done}\n{sends:#?}"
    );
}
