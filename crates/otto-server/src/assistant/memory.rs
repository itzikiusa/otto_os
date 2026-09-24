//! Assistant memory (plan §2.3, three layers):
//!
//! 1. **Profile** — `profile.md` in the assistant's cwd, owned by the user
//!    (versioned read/save like Personal Agent memory; the agent only reads it).
//! 2. **Memories** — atomic facts in `otto-memory`: collection `assistant`,
//!    workspace `scratch`, `visibility: private`, `created_by` = the user and
//!    `story_id = "user:<id>"` (the per-user partition — dedupe, list and FTS
//!    recall never cross users). Pending (memory approval / Hermes import) =
//!    lifecycle state `suggested`; forget is the soft forget with an undo token.
//! 3. Per-agent `memory/notes.md` of each Personal Agent — unchanged.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use otto_core::domain::SCRATCH_WORKSPACE_ID;
use otto_core::{Error, Result};
use otto_memory::types::{Memory, MemoryQuery, NewMemory, Scope};
use otto_state::memory::ListFilter;
use otto_state::MemoriesRepo;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::types::{AssistantMemory, MemorySource, ProfileDoc};
use super::{assistant_dir, emit_needs_you, emit_task, repo, system_turn};
use crate::state::ServerCtx;

/// The otto-memory collection the assistant writes.
pub const COLLECTION: &str = "assistant";
/// Max bytes of one memory.
pub const MAX_MEMORY_BYTES: usize = 4 * 1024;
/// Max bytes of `profile.md`.
pub const MAX_PROFILE_BYTES: usize = 256 * 1024;
/// How many matches one "forget X" may remove (all undoable).
pub const MAX_FORGET: usize = 10;

fn ws() -> String {
    SCRATCH_WORKSPACE_ID.to_string()
}

/// The per-user partition key.
pub fn story_of(user_id: &str) -> String {
    format!("user:{user_id}")
}

/// Is `m` one of `user_id`'s assistant memories?
pub fn owned_by(m: &Memory, user_id: &str) -> bool {
    m.collection == COLLECTION
        && m.created_by == user_id
        && m.story_id.as_deref() == Some(story_of(user_id).as_str())
}

/// Wire shape. `suggested` ⇒ `pending`; the `src:` tag carries the source.
pub fn to_wire(m: &Memory) -> AssistantMemory {
    let source_kind = match m.source_kind.as_str() {
        "agent" | "user" | "hermes" => m.source_kind.clone(),
        _ => "user".to_string(),
    };
    let (thread_id, file) = match source_kind.as_str() {
        "hermes" => (None, m.source_ref.clone()),
        _ => (m.source_ref.clone(), None),
    };
    AssistantMemory {
        id: m.id.clone(),
        text: m.body.clone(),
        kind: m.kind.clone(),
        tags: m.tags.clone(),
        state: if m.state == "suggested" {
            "pending".into()
        } else {
            "accepted".into()
        },
        source: MemorySource {
            kind: source_kind,
            thread_id,
            file,
        },
        created_at: m.created_at.clone(),
        updated_at: m.updated_at.clone(),
    }
}

/// A one-line title for the memory row (the body is the fact itself).
fn title_of(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    let t: String = line.chars().take(80).collect();
    if t.is_empty() {
        "memory".into()
    } else {
        t
    }
}

/// Save one memory for `user_id`. `pending` lands it `suggested` (review).
/// `source_kind` ∈ `agent | user | hermes`; `source_ref` = thread id / file.
#[allow(clippy::too_many_arguments)]
pub async fn save(
    ctx: &ServerCtx,
    user_id: &str,
    text: &str,
    kind: Option<&str>,
    tags: Vec<String>,
    source_kind: &str,
    source_ref: Option<String>,
    pending: bool,
) -> Result<Memory> {
    super::check_text("text", text, MAX_MEMORY_BYTES)?;
    let kind = kind
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .unwrap_or("fact")
        .to_string();
    let nm = NewMemory {
        collection: COLLECTION.into(),
        record_type: "item".into(),
        scope: Scope::Story,
        story_id: Some(story_of(user_id)),
        kind,
        title: title_of(text),
        body: text.trim().to_string(),
        entities: Vec::new(),
        tags: tags.into_iter().take(20).collect(),
        source_kind: source_kind.into(),
        source_ref,
        refs: Vec::new(),
        confidence: None,
        salience: None,
        visibility: "private".into(),
    };
    // save() returns the EXISTING row for an identical fact; only a brand-new
    // row may be parked for review (an already-accepted fact stays accepted).
    let existed = ctx
        .memory
        .repo()
        .find_by_hash(
            &ws(),
            COLLECTION,
            Scope::Story,
            nm.story_id.as_deref(),
            &MemoriesRepo::content_hash(&nm.body),
        )
        .await?
        .is_some();
    let saved = ctx
        .memory
        .save(&ws(), user_id, vec![nm])
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| Error::Internal("memory save returned nothing".into()))?;
    if pending && !existed && saved.state != "suggested" {
        return ctx.memory.set_state(&ws(), &saved.id, "suggested").await;
    }
    Ok(saved)
}

/// Would saving `text` for `user_id` duplicate an existing live memory?
pub async fn is_duplicate(ctx: &ServerCtx, user_id: &str, text: &str) -> bool {
    ctx.memory
        .repo()
        .find_by_hash(
            &ws(),
            COLLECTION,
            Scope::Story,
            Some(story_of(user_id).as_str()),
            &MemoriesRepo::content_hash(text.trim()),
        )
        .await
        .ok()
        .flatten()
        .is_some()
}

/// The owner's memories: `(accepted, pending)`. `q` switches to FTS recall.
pub async fn list(
    ctx: &ServerCtx,
    user_id: &str,
    q: Option<&str>,
    limit: i64,
) -> Result<(Vec<Memory>, Vec<Memory>)> {
    let limit = limit.clamp(1, 500);
    let rows: Vec<Memory> = match q.map(str::trim).filter(|q| !q.is_empty()) {
        Some(text) => ctx
            .memory
            .search(
                &ws(),
                MemoryQuery {
                    text: Some(text.to_string()),
                    collection: Some(COLLECTION.into()),
                    story_id: Some(story_of(user_id)),
                    k: limit as usize,
                    viewer: Some(user_id.to_string()),
                    ..Default::default()
                },
            )
            .await?
            .into_iter()
            .map(|h| h.memory)
            .collect(),
        None => {
            ctx.memory
                .list(
                    &ws(),
                    ListFilter {
                        collection: Some(COLLECTION.into()),
                        story_id: Some(story_of(user_id)),
                        limit,
                        viewer: Some(user_id.to_string()),
                        ..Default::default()
                    },
                )
                .await?
        }
    };
    let (pending, accepted): (Vec<Memory>, Vec<Memory>) = rows
        .into_iter()
        .filter(|m| owned_by(m, user_id) && m.active)
        .partition(|m| m.state == "suggested");
    Ok((accepted, pending))
}

/// Load one of the owner's memories (another user's id is `NotFound`).
pub async fn get_owned(ctx: &ServerCtx, user_id: &str, id: &str) -> Result<Memory> {
    let m = ctx.memory.get(&ws(), id).await?;
    if !owned_by(&m, user_id) {
        return Err(Error::NotFound("memory".into()));
    }
    Ok(m)
}

/// Soft-forget one memory; returns its undo token.
pub async fn forget_one(ctx: &ServerCtx, user_id: &str, id: &str) -> Result<String> {
    get_owned(ctx, user_id, id).await?;
    Ok(ctx.memory.soft_forget(&ws(), id).await?.undo_token)
}

/// "Forget X": soft-forget up to [`MAX_FORGET`] of the owner's memories that
/// match `query` (FTS). Returns what went, with the undo tokens.
pub async fn forget_matching(
    ctx: &ServerCtx,
    user_id: &str,
    query: &str,
) -> Result<(Vec<Memory>, Vec<String>)> {
    super::check_text("query", query, 1024)?;
    let hits = ctx
        .memory
        .search(
            &ws(),
            MemoryQuery {
                text: Some(query.trim().to_string()),
                collection: Some(COLLECTION.into()),
                story_id: Some(story_of(user_id)),
                k: MAX_FORGET,
                viewer: Some(user_id.to_string()),
                ..Default::default()
            },
        )
        .await?;
    let mut gone = Vec::new();
    let mut tokens = Vec::new();
    for h in hits.into_iter().take(MAX_FORGET) {
        if !owned_by(&h.memory, user_id) || !h.memory.active {
            continue;
        }
        if let Ok(r) = ctx.memory.soft_forget(&ws(), &h.memory.id).await {
            tokens.push(r.undo_token);
            gone.push(h.memory);
        }
    }
    Ok((gone, tokens))
}

/// Restore a forgotten memory (the chip's Undo / the Forget toast's Undo).
pub async fn undo(ctx: &ServerCtx, user_id: &str, token: &str) -> Result<Memory> {
    let m = ctx.memory.undo_forget(&ws(), token).await?;
    if !owned_by(&m, user_id) {
        // Tokens are random secrets, but never hand back another user's row.
        return Err(Error::NotFound("memory".into()));
    }
    Ok(m)
}

/// Accept a pending memory (and settle its review item, if any).
pub async fn accept(ctx: &ServerCtx, user_id: &str, id: &str) -> Result<Memory> {
    get_owned(ctx, user_id, id).await?;
    let m = ctx.memory.set_state(&ws(), id, "accepted").await?;
    settle_review(ctx, user_id, id, "accepted").await;
    Ok(m)
}

/// Close the open `memory_review` needs-you item for `memory_id`, if any.
pub async fn settle_review(ctx: &ServerCtx, user_id: &str, memory_id: &str, outcome: &str) {
    let Ok(open) = repo(ctx).needs_you(user_id).await else {
        return;
    };
    for t in open.into_iter().filter(|t| {
        t.kind == "memory_review"
            && t.needs_you
                .as_ref()
                .and_then(|n| n.get("memory_id"))
                .and_then(Value::as_str)
                == Some(memory_id)
    }) {
        if let Ok(done) = repo(ctx)
            .set_task_state(
                &t.id,
                "done",
                Some(Value::Null),
                Some(json!({"decision": outcome})),
            )
            .await
        {
            emit_task(ctx, &done);
            emit_needs_you(ctx, &done).await;
        }
    }
}

/// An agent `assistant_remember`: save (pending when memory approval is on),
/// post the memory chip (with Undo) into the thread, and — when pending —
/// open a `memory_review` needs-you item. Returns `(memory, pending)`.
pub async fn remember_from_agent(
    ctx: &ServerCtx,
    user_id: &str,
    thread_id: Option<&str>,
    text: &str,
    kind: Option<&str>,
    tags: Vec<String>,
) -> Result<(Memory, bool)> {
    let (settings, _) = super::threads::load_settings(ctx, user_id).await;
    let pending = settings.memory_approval;
    let m = save(
        ctx,
        user_id,
        text,
        kind,
        tags,
        "agent",
        thread_id.map(str::to_string),
        pending,
    )
    .await?;
    let is_pending = m.state == "suggested";
    if let Some(tid) = thread_id {
        let (label, undo) = if is_pending {
            ("Wants to remember", Value::Null)
        } else {
            ("Remembered", json!({"kind": "delete", "memory_id": m.id}))
        };
        system_turn(
            ctx,
            user_id,
            tid,
            "memory",
            &format!("{label}: {}", title_of(&m.body)),
            Some(json!({
                "action": if is_pending { "pending" } else { "remembered" },
                "memory_ids": [m.id],
                "undo": undo,
            })),
        )
        .await;
    }
    if is_pending {
        let task = repo(ctx)
            .create_task(otto_state::NewAssistantTask {
                owner_user_id: user_id.to_string(),
                thread_id: thread_id.map(str::to_string),
                kind: "memory_review".into(),
                state: "needs_you".into(),
                title: format!("Remember: {}", title_of(&m.body)),
                detail: m.body.clone(),
                origin: "thread".into(),
                needs_you: Some(json!({
                    "kind": "memory",
                    "prompt": format!("Otto wants to remember: \u{201c}{}\u{201d}", title_of(&m.body)),
                    "memory_id": m.id,
                })),
                ..Default::default()
            })
            .await?;
        emit_task(ctx, &task);
        emit_needs_you(ctx, &task).await;
    }
    Ok((m, is_pending))
}

/// Post the "Forgot N" chip (Undo restores them all).
pub async fn forgot_chip(
    ctx: &ServerCtx,
    user_id: &str,
    thread_id: &str,
    gone: &[Memory],
    tokens: &[String],
) {
    let text = match gone.len() {
        0 => "Nothing to forget — no matching memories.".to_string(),
        1 => format!("Forgot: {}", title_of(&gone[0].body)),
        n => format!("Forgot {n} memories"),
    };
    let ids: Vec<&str> = gone.iter().map(|m| m.id.as_str()).collect();
    let undo = if tokens.is_empty() {
        Value::Null
    } else {
        json!({"kind": "restore", "undo_tokens": tokens})
    };
    system_turn(
        ctx,
        user_id,
        thread_id,
        "memory",
        &text,
        Some(json!({"action": "forgot", "memory_ids": ids, "undo": undo})),
    )
    .await;
}

// ---------------------------------------------------------------------------
// profile.md
// ---------------------------------------------------------------------------

fn profile_path(dir: &Path) -> Result<PathBuf> {
    let path = dir.join("profile.md");
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            Err(Error::Forbidden("profile.md cannot be a symlink".into()))
        }
        Ok(meta) if !meta.is_file() => Err(Error::Invalid("profile.md must be a file".into())),
        _ => Ok(path),
    }
}

fn version_of(content: Option<&[u8]>) -> String {
    let mut h = Sha256::new();
    h.update([u8::from(content.is_some())]);
    if let Some(b) = content {
        h.update(b);
    }
    format!("{:x}", h.finalize())
}

fn read_profile_sync(dir: &Path) -> Result<ProfileDoc> {
    let path = profile_path(dir)?;
    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ProfileDoc {
                content: String::new(),
                version: version_of(None),
                exists: false,
            })
        }
        Err(e) => return Err(Error::Internal(format!("profile: {e}"))),
    };
    let mut bytes = Vec::new();
    file.take(MAX_PROFILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::Internal(format!("profile: {e}")))?;
    if bytes.len() > MAX_PROFILE_BYTES {
        return Err(Error::Invalid("profile.md exceeds 256 KiB".into()));
    }
    let version = version_of(Some(&bytes));
    let content =
        String::from_utf8(bytes).map_err(|_| Error::Invalid("profile.md must be UTF-8".into()))?;
    Ok(ProfileDoc {
        content,
        version,
        exists: true,
    })
}

/// Read `profile.md` (missing ⇒ empty, `exists: false`; never provisions).
pub async fn read_profile(ctx: &ServerCtx, user_id: &str) -> Result<ProfileDoc> {
    let dir = assistant_dir(ctx, user_id);
    tokio::task::spawn_blocking(move || read_profile_sync(&dir))
        .await
        .map_err(|e| Error::Internal(format!("profile: {e}")))?
}

/// Save `profile.md` with compare-and-swap on `version` (409 when stale),
/// atomically via a sibling temp file.
pub async fn save_profile(
    ctx: &ServerCtx,
    user_id: &str,
    version: &str,
    content: &str,
) -> Result<ProfileDoc> {
    if content.len() > MAX_PROFILE_BYTES {
        return Err(Error::Invalid("profile.md exceeds 256 KiB".into()));
    }
    let dir = assistant_dir(ctx, user_id);
    let (version, content) = (version.to_string(), content.to_string());
    tokio::task::spawn_blocking(move || save_profile_sync(&dir, &version, &content))
        .await
        .map_err(|e| Error::Internal(format!("profile: {e}")))?
}

fn save_profile_sync(dir: &Path, version: &str, content: &str) -> Result<ProfileDoc> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if read_profile_sync(dir)?.version != version {
        return Err(Error::Conflict(
            "Your profile changed since you opened it. Reload and reconcile your edits.".into(),
        ));
    }
    std::fs::create_dir_all(dir).map_err(|e| Error::Internal(format!("profile: {e}")))?;
    let path = profile_path(dir)?;
    let mut staged = tempfile::NamedTempFile::new_in(dir)
        .map_err(|e| Error::Internal(format!("profile: {e}")))?;
    staged
        .write_all(content.as_bytes())
        .map_err(|e| Error::Internal(format!("profile: {e}")))?;
    staged
        .persist(&path)
        .map_err(|e| Error::Internal(format!("profile: {e}")))?;
    read_profile_sync(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(created_by: &str, story: Option<&str>, collection: &str, state: &str) -> Memory {
        Memory {
            id: "m1".into(),
            workspace_id: "scratch".into(),
            collection: collection.into(),
            record_type: "item".into(),
            scope: Scope::Story,
            story_id: story.map(str::to_string),
            kind: "fact".into(),
            title: "t".into(),
            body: "Prefers aisle seats".into(),
            entities: vec![],
            tags: vec!["hermes".into()],
            source_kind: "hermes".into(),
            source_ref: Some("USER.md".into()),
            refs: vec![],
            confidence: 1.0,
            salience: 0.5,
            content_hash: "h".into(),
            active: true,
            superseded_by: None,
            version: 1,
            created_by: created_by.into(),
            created_at: "c".into(),
            updated_at: "u".into(),
            last_accessed_at: None,
            access_count: 0,
            expires_at: None,
            visibility: "private".into(),
            state: state.into(),
            provenance_json: None,
            forgotten_at: None,
            undo_token: None,
        }
    }

    #[test]
    fn ownership_needs_collection_creator_and_partition() {
        assert!(owned_by(
            &mem("u1", Some("user:u1"), COLLECTION, "accepted"),
            "u1"
        ));
        assert!(!owned_by(
            &mem("u1", Some("user:u1"), COLLECTION, "accepted"),
            "u2"
        ));
        assert!(!owned_by(
            &mem("u1", Some("user:u2"), COLLECTION, "accepted"),
            "u1"
        ));
        assert!(!owned_by(
            &mem("u1", Some("user:u1"), "product", "accepted"),
            "u1"
        ));
        assert!(!owned_by(&mem("u1", None, COLLECTION, "accepted"), "u1"));
    }

    #[test]
    fn wire_shape_maps_state_and_source() {
        let w = to_wire(&mem("u1", Some("user:u1"), COLLECTION, "suggested"));
        assert_eq!(w.state, "pending");
        assert_eq!(w.source.kind, "hermes");
        assert_eq!(w.source.file.as_deref(), Some("USER.md"));
        assert_eq!(w.source.thread_id, None);
        assert_eq!(w.text, "Prefers aisle seats");
        let mut agent = mem("u1", Some("user:u1"), COLLECTION, "accepted");
        agent.source_kind = "agent".into();
        agent.source_ref = Some("thread-1".into());
        let w = to_wire(&agent);
        assert_eq!(w.state, "accepted");
        assert_eq!(w.source.thread_id.as_deref(), Some("thread-1"));
        let v = serde_json::to_value(&w).unwrap();
        for k in [
            "id",
            "text",
            "kind",
            "tags",
            "state",
            "source",
            "created_at",
            "updated_at",
        ] {
            assert!(v.get(k).is_some(), "{k}");
        }
    }

    #[test]
    fn title_is_the_first_line_capped() {
        assert_eq!(title_of("Likes tea\nand biscuits"), "Likes tea");
        assert_eq!(title_of(&"z".repeat(200)).chars().count(), 80);
        assert_eq!(title_of("   "), "memory");
    }

    #[test]
    fn profile_saves_with_compare_and_swap() {
        let dir = tempfile::tempdir().unwrap();
        let first = read_profile_sync(dir.path()).unwrap();
        assert!(!first.exists);
        assert!(
            !dir.path().join("profile.md").exists(),
            "reads never provision"
        );
        let saved = save_profile_sync(dir.path(), &first.version, "Name: Itzik\n").unwrap();
        assert!(saved.exists);
        assert_eq!(saved.content, "Name: Itzik\n");
        // A stale version is refused and leaves the file alone.
        assert!(matches!(
            save_profile_sync(dir.path(), &first.version, "stale"),
            Err(Error::Conflict(_))
        ));
        assert_eq!(
            read_profile_sync(dir.path()).unwrap().content,
            "Name: Itzik\n"
        );
    }

    #[test]
    fn a_symlinked_profile_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("x.md"), "secret").unwrap();
        std::os::unix::fs::symlink(outside.path().join("x.md"), dir.path().join("profile.md"))
            .unwrap();
        assert!(matches!(
            read_profile_sync(dir.path()),
            Err(Error::Forbidden(_))
        ));
    }
}
