//! The normalized conversation model — the contract between the parser, the
//! REST/WS API and the UI (design §3; mirrored in `ui/src/lib/api/types.ts`
//! under `// ── Transcript`). Every field name here is on the wire verbatim.

use serde::{Deserialize, Serialize};

/// Which CLI wrote the transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Claude,
    Codex,
    Agy,
}

impl Provider {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "agy" => Some(Self::Agy),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Agy => "agy",
        }
    }
}

/// Why a session has no transcript to show (`turns: []`). Wire form is the
/// snake_case string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    NoProviderSessionId,
    TranscriptMissing,
    ProviderUnsupported,
    CodexRolloutUnresolved,
}

/// Aggregate counters for one transcript (or one page of it — `turns` and
/// `tool_calls` cover the whole file, never just the page).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    pub turns: u64,
    pub tool_calls: u64,
    pub cost_usd: Option<f64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub duration_ms: Option<u64>,
    /// Codex reasoning items — never recoverable, so counted, not rendered.
    pub reasoning_steps: u64,
    /// Claude `thinking` blocks — persisted empty, so a marker count only.
    pub thinking_steps: u64,
    /// Records the parser did not recognize (each also yields a `notice` block).
    pub unknown_records: u64,
}

/// One entry of the subagent tree, read from `subagents/<agent-id>.meta.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentMeta {
    pub agent_id: String,
    pub parent_agent_id: Option<String>,
    pub depth: u32,
    pub agent_type: String,
    pub description: String,
    pub model: Option<String>,
    /// The parent's `Agent` tool_use id that spawned it — the `subagent` block
    /// attaches to the turn owning that tool call.
    pub tool_use_id: Option<String>,
}

/// The page of a conversation the API returns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub session_id: Option<String>,
    pub provider: Provider,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub model: Option<String>,
    /// Opaque `"<record_index>"` of the FIRST record of the OLDEST returned turn;
    /// pass it back as `before` (exclusive) to page earlier. Records are
    /// append-only so the index is stable.
    pub cursor: String,
    pub has_earlier: bool,
    pub turns: Vec<Turn>,
    pub stats: Stats,
    /// The FULL subagent tree (not just the ones reachable from `Agent` results).
    pub subagents: Vec<SubagentMeta>,
    pub unavailable_reason: Option<UnavailableReason>,
}

impl Transcript {
    /// The empty transcript a session without a resolvable file gets (HTTP 200).
    pub fn unavailable(provider: Provider, session_id: Option<String>, why: UnavailableReason) -> Self {
        Self {
            session_id,
            provider,
            title: None,
            cwd: None,
            model: None,
            cursor: "0".into(),
            has_earlier: false,
            turns: Vec::new(),
            stats: Stats::default(),
            subagents: Vec::new(),
            unavailable_reason: Some(why),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// One conversational turn. `id` is stable across re-parses (design §3): Claude
/// assistant → `requestId` (else `uuid`), user → `uuid`; Codex new era →
/// `<turn_id>:u|:a`, old era → `r<record_index of first record>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub id: String,
    pub role: Role,
    pub ts: Option<String>,
    pub blocks: Vec<Block>,
    pub duration_ms: Option<u64>,
    pub model: Option<String>,
    /// Attachments / reminders / hooks — collapsed into one chip by the UI.
    pub system: Vec<SystemNote>,
    /// Codex reasoning items in THIS turn (never recoverable, so a count; the
    /// UI renders "N reasoning steps (not recorded)" per response). Claude's
    /// equivalent is the `thinking` marker block. Additive to design §3.
    #[serde(default)]
    pub reasoning_steps: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Block {
    Text { md: String },
    /// Marker only — no thinking text is persisted on disk.
    Thinking { count: u64 },
    /// Served by `GET …/transcript/images/{id}`.
    Image {
        id: String,
        media_type: String,
        alt: Option<String>,
    },
    ToolCall {
        id: String,
        name: String,
        tool: ToolKind,
        title: String,
        input: serde_json::Value,
        result: Option<ToolResult>,
    },
    /// Children come from `Transcript::subagents`.
    Subagent {
        agent_id: String,
        description: String,
        agent_type: String,
        status: Option<SubagentStatus>,
    },
    /// The task-list state AFTER the call.
    Tasks { tasks: Vec<TaskItem> },
    /// Claude `queue-operation`; `injected` = the content is a
    /// `<task-notification>` / system payload rather than something typed.
    Queued {
        op: QueueOp,
        text: String,
        injected: bool,
    },
    Artifact { artifact: Artifact },
    Notice { note: SystemNote },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubagentStatus {
    Running,
    Done,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueueOp {
    Enqueue,
    Dequeue,
    Remove,
}

impl QueueOp {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "enqueue" => Some(Self::Enqueue),
            "dequeue" => Some(Self::Dequeue),
            "remove" => Some(Self::Remove),
            _ => None,
        }
    }
}

/// Coarse tool category (drives the UI icon). See `tool_kind_for_claude` /
/// `tool_kind_for_codex` for the name → kind tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolKind {
    Shell,
    Read,
    Edit,
    Write,
    Search,
    Agent,
    Mcp,
    Skill,
    Web,
    Ask,
    Task,
    Other,
}

/// Claude tool name → kind (design §3 table).
pub fn tool_kind_for_claude(name: &str) -> ToolKind {
    match name {
        "Bash" => ToolKind::Shell,
        "Read" | "NotebookRead" => ToolKind::Read,
        "Edit" | "MultiEdit" | "NotebookEdit" => ToolKind::Edit,
        "Write" => ToolKind::Write,
        "Grep" | "Glob" | "ToolSearch" | "LS" => ToolKind::Search,
        "Agent" | "Task" => ToolKind::Agent,
        "Skill" => ToolKind::Skill,
        "WebFetch" | "WebSearch" => ToolKind::Web,
        "AskUserQuestion" => ToolKind::Ask,
        "TodoWrite" | "TaskCreate" | "TaskUpdate" => ToolKind::Task,
        n if n.starts_with("mcp__") => ToolKind::Mcp,
        _ => ToolKind::Other,
    }
}

/// Codex `function_call` name → kind (items map by their own `item.type`).
pub fn tool_kind_for_codex_function(name: &str) -> ToolKind {
    match name {
        "shell" | "exec_command" | "exec" | "container.exec" | "shell_command" | "write_stdin" => {
            ToolKind::Shell
        }
        "apply_patch" => ToolKind::Edit,
        "update_plan" => ToolKind::Task,
        "view_image" | "read_file" => ToolKind::Read,
        "web_search" => ToolKind::Web,
        "request_user_input" => ToolKind::Ask,
        n if n.starts_with("mcp__") || n.contains("__") => ToolKind::Mcp,
        _ => ToolKind::Other,
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolResult {
    pub ok: bool,
    pub text: Option<String>,
    /// `text` was cut at [`crate::TOOL_TEXT_CAP`] bytes.
    pub truncated: bool,
    /// Size of the original result text in bytes.
    pub bytes: u64,
    pub image_ids: Vec<String>,
    /// Unified diff (Claude `structuredPatch`, Codex `unified_diff`).
    pub patch: Option<String>,
    pub file_path: Option<String>,
}

impl ToolResult {
    /// Apply the 64 KB caps to `text` and `patch` (idempotent).
    pub fn cap(&mut self) {
        if let Some(t) = self.text.take() {
            let (c, cut) = crate::util::cap_text(&t);
            self.truncated |= cut;
            self.text = Some(c);
        }
        if let Some(p) = self.patch.take() {
            let (c, cut) = crate::util::cap_text(&p);
            self.truncated |= cut;
            self.patch = Some(c);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskItemStatus {
    Pending,
    InProgress,
    Completed,
}

impl TaskItemStatus {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "in_progress" => Some(Self::InProgress),
            "completed" | "done" => Some(Self::Completed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskItem {
    pub ext_id: Option<String>,
    pub title: String,
    pub status: TaskItemStatus,
    pub active_form: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemNoteKind {
    SystemReminder,
    TaskNotification,
    Command,
    Hook,
    Attachment,
    Compaction,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemNote {
    pub kind: SystemNoteKind,
    pub title: String,
    pub body: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactKind {
    File,
    Pr,
    Image,
    Report,
    Url,
}

impl ArtifactKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Pr => "pr",
            Self::Image => "image",
            Self::Report => "report",
            Self::Url => "url",
        }
    }
}

/// Something the agent produced. `id` = `sha1(kind + ':' + (path ?? url))`,
/// opaque and stable; dedup is PER PATH and the last producing turn wins. It is
/// the ONLY handle the artifact file route accepts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub kind: ArtifactKind,
    pub label: String,
    pub path: Option<String>,
    pub url: Option<String>,
    pub mime: Option<String>,
    pub produced_at: Option<String>,
    pub turn_id: String,
}

impl Artifact {
    /// The opaque id for a `(kind, path-or-url)` pair.
    pub fn id_for(kind: ArtifactKind, path_or_url: &str) -> String {
        use sha1::{Digest, Sha1};
        let mut h = Sha1::new();
        h.update(kind.as_str().as_bytes());
        h.update(b":");
        h.update(path_or_url.as_bytes());
        format!("{:x}", h.finalize())
    }
}

/// One session listed by the History page (an Otto row, or a transcript found
/// on disk with no row — `status: on_disk`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub session_id: Option<String>,
    pub provider: Provider,
    pub title: Option<String>,
    pub first_prompt: Option<String>,
    pub cwd: String,
    pub repo_name: Option<String>,
    pub started_at: String,
    pub last_active_at: String,
    pub turns: Option<u64>,
    /// `running|idle|exited|reconnectable` from the session row, or `on_disk`.
    pub status: String,
    pub transcript_path: String,
    pub resumable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_wire_tags_are_snake_case_kind() {
        let b = Block::ToolCall {
            id: "t1".into(),
            name: "Bash".into(),
            tool: ToolKind::Shell,
            title: "ls".into(),
            input: serde_json::json!({"command": "ls"}),
            result: None,
        };
        let v = serde_json::to_value(&b).unwrap();
        assert_eq!(v["kind"], "tool_call");
        assert_eq!(v["tool"], "shell");
        assert!(v["result"].is_null());
        let t = serde_json::to_value(Block::Thinking { count: 2 }).unwrap();
        assert_eq!(t["kind"], "thinking");
        let n = serde_json::to_value(SystemNoteKind::SystemReminder).unwrap();
        assert_eq!(n, "system_reminder");
        let u = serde_json::to_value(UnavailableReason::NoProviderSessionId).unwrap();
        assert_eq!(u, "no_provider_session_id");
    }

    #[test]
    fn artifact_id_is_sha1_of_kind_and_path() {
        let a = Artifact::id_for(ArtifactKind::File, "/repo/x.md");
        assert_eq!(a.len(), 40);
        assert_eq!(a, Artifact::id_for(ArtifactKind::File, "/repo/x.md"));
        assert_ne!(a, Artifact::id_for(ArtifactKind::Image, "/repo/x.md"));
    }

    #[test]
    fn claude_tool_kinds_follow_the_table() {
        assert_eq!(tool_kind_for_claude("Bash"), ToolKind::Shell);
        assert_eq!(tool_kind_for_claude("MultiEdit"), ToolKind::Edit);
        assert_eq!(tool_kind_for_claude("mcp__otto__otto_vault_read"), ToolKind::Mcp);
        assert_eq!(tool_kind_for_claude("TaskUpdate"), ToolKind::Task);
        assert_eq!(tool_kind_for_claude("Frobnicate"), ToolKind::Other);
        assert_eq!(tool_kind_for_codex_function("apply_patch"), ToolKind::Edit);
        assert_eq!(tool_kind_for_codex_function("exec_command"), ToolKind::Shell);
    }
}
