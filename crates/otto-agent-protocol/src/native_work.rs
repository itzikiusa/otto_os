//! Work an agent runs outside its protocol stream, read from the agent's own
//! files for Threads' "Agents & tasks" drill-in. Blocking: the handlers run
//! these on the blocking pool.
//!
//! * Claude Code's background commands write their output to
//!   `<tmp>/claude-<uid>/<cwd>/<session>/tasks/<task_id>.output` (the launch
//!   tool result names it; verified live against 2.1.x) and send nothing
//!   while they run.
//! * agy's native swarm (verified live against 1.5.8) keeps its state under
//!   `~/.gemini/antigravity-cli/brain/<session>/.system_generated/swarm`:
//!   `members.json` (`{title, members: {name: {conversation_id, role,
//!   phase, killed, model, workspace}}}`) and `board.jsonl` (`{seq, agent,
//!   text, time}` posts). Each member's own steps are its conversation's
//!   `brain/<conversation>/.system_generated/logs/transcript.jsonl`
//!   (`{step_index, type, status, content, tool_calls: [{name, args}]}`).
//!   Over ACP agy reports only the coordinator's `swarm` tool calls.
//!
//! Every path is built from ids validated here and must resolve inside its
//! root (no links out); only bounded tails are read.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

/// A command output tail.
const OUTPUT_TAIL: u64 = 64 * 1024;
/// A member transcript tail / the board tail.
const TRANSCRIPT_TAIL: u64 = 256 * 1024;
const BOARD_TAIL: u64 = 128 * 1024;
const STEPS_SHOWN: usize = 80;
const STEP_TEXT: usize = 4_000;
const TOOL_INPUT: usize = 600;
const POSTS_SHOWN: usize = 40;

fn safe_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// `path` if it is a regular file whose real path is inside `root`.
fn contained_file(root: &Path, path: &Path) -> Option<PathBuf> {
    let root = root.canonicalize().ok()?;
    let real = path.canonicalize().ok()?;
    (real.starts_with(&root) && real.metadata().ok()?.is_file()).then_some(real)
}

/// The last `max` bytes of `path` as text (cut to whole lines when cut).
fn tail(path: &Path, max: u64) -> Option<(String, bool)> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(max);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::with_capacity((len - start) as usize);
    f.take(max).read_to_end(&mut buf).ok()?;
    let mut text = String::from_utf8_lossy(&buf).into_owned();
    if start > 0 {
        if let Some(n) = text.find('\n') {
            text.drain(..=n);
        }
    }
    Some((text, start > 0))
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[derive(Debug, Serialize, PartialEq)]
pub struct TaskOutput {
    pub text: String,
    /// Only the tail is shown.
    pub truncated: bool,
}

/// A Claude Code background command's output so far. `tmp` is Claude's temp
/// root (`$CLAUDE_CODE_TMPDIR`, else `/tmp`).
#[allow(clippy::disallowed_methods)] // runs on the blocking pool
pub fn claude_task_output(tmp: &Path, uid: u32, session: &str, task: &str) -> Option<TaskOutput> {
    if !safe_id(session) || !safe_id(task) {
        return None;
    }
    let root = tmp.join(format!("claude-{uid}"));
    // `<cwd>` is the project folder encoded; the session id is unique.
    for entry in std::fs::read_dir(&root).ok()?.flatten() {
        let candidate = entry
            .path()
            .join(session)
            .join("tasks")
            .join(format!("{task}.output"));
        if let Some(real) = contained_file(&root, &candidate) {
            let (text, truncated) = tail(&real, OUTPUT_TAIL)?;
            return Some(TaskOutput { text, truncated });
        }
    }
    None
}

#[derive(Debug, Serialize, PartialEq)]
pub struct SwarmPost {
    pub seq: u64,
    pub agent: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct SwarmStep {
    pub index: u64,
    /// `PLANNER_RESPONSE`, `GENERIC` (a tool result), `USER_INPUT`, …
    pub r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// `name args` per tool call.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct SwarmMember {
    pub name: String,
    pub role: String,
    /// agy's phase (`RUNNING`, …); `killed` overrides it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    pub killed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Its own board posts and the board posts naming it, newest last.
    pub posts: Vec<SwarmPost>,
    /// Its own steps (only for the member asked for), newest last.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<SwarmStep>,
    pub truncated: bool,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Swarm {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub members: Vec<SwarmMember>,
}

/// agy's swarm of `session` (the coordinator's conversation), with `member`'s
/// steps. `None` when the session has no swarm. `brain` is
/// `~/.gemini/antigravity-cli/brain`.
pub fn agy_swarm(brain: &Path, session: &str, member: Option<&str>) -> Option<Swarm> {
    if !safe_id(session) {
        return None;
    }
    let gen = brain.join(session).join(".system_generated");
    let members_file = contained_file(brain, &gen.join("swarm").join("members.json"))?;
    let doc: Value = serde_json::from_slice(&std::fs::read(members_file).ok()?).ok()?;
    let posts: Vec<SwarmPost> = contained_file(brain, &gen.join("swarm").join("board.jsonl"))
        .and_then(|p| tail(&p, BOARD_TAIL))
        .map(|(t, _)| {
            t.lines()
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .filter_map(|v| {
                    Some(SwarmPost {
                        seq: v.get("seq")?.as_u64()?,
                        agent: v.get("agent")?.as_str()?.to_string(),
                        text: clip(v.get("text")?.as_str()?, STEP_TEXT),
                        time: v.get("time").and_then(Value::as_str).map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let s = |m: &Value, k: &str| m.get(k).and_then(Value::as_str).map(str::to_string);
    let mut members: Vec<SwarmMember> = doc
        .get("members")?
        .as_object()?
        .iter()
        .filter(|(name, m)| {
            *name != "coordinator" && s(m, "role").as_deref() != Some("coordinator")
        })
        .map(|(name, m)| {
            let mention = format!("@{name}");
            let mut mine: Vec<&SwarmPost> = posts
                .iter()
                .filter(|p| {
                    p.agent == *name || p.text.contains(&mention) || p.text.contains("@all")
                })
                .collect();
            mine.drain(..mine.len().saturating_sub(POSTS_SHOWN));
            let (steps, truncated) = if member == Some(name.as_str()) {
                s(m, "conversation_id")
                    .filter(|c| safe_id(c))
                    .and_then(|c| {
                        let path = brain
                            .join(c)
                            .join(".system_generated")
                            .join("logs")
                            .join("transcript.jsonl");
                        tail(&contained_file(brain, &path)?, TRANSCRIPT_TAIL)
                    })
                    .map(|(t, cut)| {
                        let mut steps = transcript_steps(&t);
                        let more = steps.len() > STEPS_SHOWN;
                        steps.drain(..steps.len().saturating_sub(STEPS_SHOWN));
                        (steps, cut || more)
                    })
                    .unwrap_or_default()
            } else {
                (Vec::new(), false)
            };
            SwarmMember {
                name: name.clone(),
                role: s(m, "role").unwrap_or_else(|| "worker".into()),
                phase: s(m, "phase"),
                killed: m.get("killed").and_then(Value::as_bool).unwrap_or(false),
                model: s(m, "model"),
                workspace: s(m, "workspace"),
                posts: mine
                    .into_iter()
                    .map(|p| SwarmPost {
                        seq: p.seq,
                        agent: p.agent.clone(),
                        text: p.text.clone(),
                        time: p.time.clone(),
                    })
                    .collect(),
                steps,
                truncated,
            }
        })
        .collect();
    members.sort_by(|a, b| a.name.cmp(&b.name));
    Some(Swarm {
        title: s(&doc, "title"),
        members,
    })
}

/// A member's transcript lines as steps; the opening system envelope and
/// agy's "Created At / Completed At" result headers are dropped.
fn transcript_steps(text: &str) -> Vec<SwarmStep> {
    text.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| {
            let ty = v.get("type")?.as_str()?.to_string();
            if ty == "SYSTEM_MESSAGE" {
                return None;
            }
            let body = v
                .get("content")
                .or_else(|| v.get("error"))
                .and_then(Value::as_str)
                .map(|c| {
                    c.lines()
                        .filter(|l| {
                            !l.starts_with("Created At: ") && !l.starts_with("Completed At: ")
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                        .trim()
                        .to_string()
                })
                .filter(|c| !c.is_empty());
            let tools: Vec<String> = v
                .get("tool_calls")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|c| {
                            let name = c.get("name")?.as_str()?;
                            let args = c.get("args").map(|a| a.to_string()).unwrap_or_default();
                            Some(clip(&format!("{name} {args}"), TOOL_INPUT))
                        })
                        .collect()
                })
                .unwrap_or_default();
            if body.is_none() && tools.is_empty() {
                return None;
            }
            Some(SwarmStep {
                index: v.get("step_index").and_then(Value::as_u64).unwrap_or(0),
                r#type: ty,
                status: v.get("status").and_then(Value::as_str).map(str::to_string),
                text: body.map(|b| clip(&b, STEP_TEXT)),
                tools,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_task_output_reads_only_its_own_tail() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("claude-501/-w-proj/sess-1/tasks");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bb1.output"), "tick 1\ntick 2\n").unwrap();
        let out = claude_task_output(tmp.path(), 501, "sess-1", "bb1").unwrap();
        assert_eq!(
            out,
            TaskOutput {
                text: "tick 1\ntick 2\n".into(),
                truncated: false
            }
        );
        assert!(claude_task_output(tmp.path(), 501, "sess-1", "../x").is_none());
        assert!(claude_task_output(tmp.path(), 501, "other", "bb1").is_none());
        // A link out of the temp root is not followed.
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "s").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), dir.join("bb2.output")).unwrap();
        assert!(claude_task_output(tmp.path(), 501, "sess-1", "bb2").is_none());
    }

    #[test]
    fn agy_swarm_reads_members_board_and_one_transcript() {
        let brain = tempfile::tempdir().unwrap();
        let sw = brain.path().join("root-1/.system_generated/swarm");
        std::fs::create_dir_all(&sw).unwrap();
        std::fs::write(
            sw.join("members.json"),
            r#"{"title":"probe","members":{"coordinator":{"conversation_id":"root-1","role":"coordinator","phase":"RUNNING","killed":false},"echoer":{"conversation_id":"kid-1","role":"worker","phase":"RUNNING","killed":false,"model":"gemini-4-carbon-medium"}}}"#,
        )
        .unwrap();
        std::fs::write(
            sw.join("board.jsonl"),
            "{\"seq\":1,\"agent\":\"coordinator\",\"text\":\"@echoer run echo\",\"kind\":\"brief\"}\n{\"seq\":2,\"agent\":\"echoer\",\"text\":\"@coordinator hello\"}\n{\"seq\":3,\"agent\":\"coordinator\",\"text\":\"@other x\"}\n",
        )
        .unwrap();
        let logs = brain.path().join("kid-1/.system_generated/logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(
            logs.join("transcript.jsonl"),
            "{\"step_index\":0,\"type\":\"SYSTEM_MESSAGE\",\"content\":\"brief\"}\n{\"step_index\":1,\"type\":\"PLANNER_RESPONSE\",\"status\":\"DONE\",\"tool_calls\":[{\"name\":\"run_command\",\"args\":{\"CommandLine\":\"echo hi\"}}]}\n{\"step_index\":2,\"type\":\"GENERIC\",\"status\":\"DONE\",\"content\":\"Created At: x\\nCompleted At: y\\nhi\"}\n",
        )
        .unwrap();
        let swarm = agy_swarm(brain.path(), "root-1", Some("echoer")).unwrap();
        assert_eq!(swarm.title.as_deref(), Some("probe"));
        assert_eq!(
            swarm.members.len(),
            1,
            "the coordinator is the thread itself"
        );
        let m = &swarm.members[0];
        assert_eq!(
            (m.name.as_str(), m.phase.as_deref(), m.killed),
            ("echoer", Some("RUNNING"), false)
        );
        assert_eq!(m.posts.iter().map(|p| p.seq).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(m.steps.len(), 2);
        assert_eq!(
            m.steps[0].tools,
            [r#"run_command {"CommandLine":"echo hi"}"#]
        );
        assert_eq!(m.steps[1].text.as_deref(), Some("hi"));
        // Without `member`, no transcript is read.
        assert!(agy_swarm(brain.path(), "root-1", None).unwrap().members[0]
            .steps
            .is_empty());
        assert!(agy_swarm(brain.path(), "nope", None).is_none());
        assert!(agy_swarm(brain.path(), "../root-1", None).is_none());
    }
}
