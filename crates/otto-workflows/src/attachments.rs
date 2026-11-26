//! Files that came with the chat message that starts (or steers) a run
//! (addendum-chat-optimize.md §1.2; DR-109, DR-201). They are copied into the
//! run's context dir when the run is created — before it can queue — and
//! recorded in `<context>/attachments.json`, so a queued run, a restart, a
//! Retry step or a re-review days later (after the listener's 24 h temp
//! sweep) still finds them. Only the listener's structured list is copied;
//! nothing is ever read back out of message text.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use otto_channels::adapter::InboundAttachment;
use otto_core::workflows::{AttachmentRef, AttachmentSource};

/// Under the run's context dir.
pub const ATTACHMENTS_DIR: &str = "attachments";
pub const ATTACHMENTS_FILE: &str = "attachments.json";
/// Per run, start and steer together.
pub const MAX_FILES: usize = 10;
pub const MAX_TOTAL_BYTES: u64 = 50 << 20;
const OVER_LIMIT: &str = "over the per-run attachment limit";

/// `<data_dir>/workflow-context/<run_id>` — the dir
/// `context::RunContextFiles::create` makes for the run.
pub fn context_dir(data_dir: &Path, run_id: &str) -> PathBuf {
    data_dir.join("workflow-context").join(run_id)
}

/// Copy `files` into `<context>/attachments/` and append them to
/// `attachments.json`; returns the new refs. A file that was not received,
/// is not one the listener saved in the temp dir, or is past the per-run cap
/// is listed with its `error` and no `path`. Blocking IO: run it on the
/// blocking pool.
pub fn copy_into(
    context: &Path,
    files: &[InboundAttachment],
    source: AttachmentSource,
) -> std::io::Result<Vec<AttachmentRef>> {
    let dir = context.join(ATTACHMENTS_DIR);
    std::fs::create_dir_all(&dir)?;
    if std::fs::symlink_metadata(&dir)?.file_type().is_symlink() {
        return Err(std::io::Error::other("attachments dir is a symlink"));
    }
    let tmp = std::env::temp_dir().canonicalize()?;
    let mut all = read(context);
    let mut used: u64 = all.iter().filter_map(|a| a.size_bytes).sum();
    let mut copied = all.iter().filter(|a| a.path.is_some()).count();
    let mut out = Vec::with_capacity(files.len());
    for f in files {
        let mut r = AttachmentRef {
            name: f.name.clone(),
            mime: f.mime.clone(),
            size_bytes: None,
            path: None,
            error: f.error.clone(),
            source,
        };
        if r.error.is_none() {
            match copy_one(&dir, &tmp, f, &mut used, copied) {
                Ok((name, size)) => {
                    copied += 1;
                    r.size_bytes = Some(size);
                    r.path = Some(format!("{ATTACHMENTS_DIR}/{name}"));
                }
                Err(e) => r.error = Some(e),
            }
        }
        out.push(r);
    }
    all.extend(out.iter().cloned());
    let json = serde_json::to_vec_pretty(&all).map_err(std::io::Error::other)?;
    let part = context.join(format!("{ATTACHMENTS_FILE}.part"));
    std::fs::write(&part, json)?;
    std::fs::rename(&part, context.join(ATTACHMENTS_FILE))?;
    Ok(out)
}

/// One file: checked, then copied to a fresh, sanitised name.
fn copy_one(
    dir: &Path,
    tmp: &Path,
    f: &InboundAttachment,
    used: &mut u64,
    copied: usize,
) -> Result<(String, u64), String> {
    let not_ours = || "not a file Otto received for this message".to_string();
    let src = f
        .path
        .as_ref()
        .and_then(|p| p.canonicalize().ok())
        .ok_or_else(not_ours)?;
    let meta = std::fs::metadata(&src).map_err(|_| not_ours())?;
    if src.parent() != Some(tmp) || !meta.is_file() {
        return Err(not_ours());
    }
    if copied >= MAX_FILES || *used + meta.len() > MAX_TOTAL_BYTES {
        return Err(OVER_LIMIT.into());
    }
    let safe = safe_name(&f.name);
    let (stem, ext) = split_ext(&safe);
    for n in 1.. {
        let name = if n == 1 {
            format!("{stem}{ext}")
        } else {
            format!("{stem}-{n}{ext}")
        };
        let dest = otto_core::paths::confine_join(dir, &name).ok_or_else(not_ours)?;
        // create_new = O_CREAT|O_EXCL: never follows or reuses what is there.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dest)
        {
            Ok(mut out) => {
                let mut input = std::fs::File::open(&src).map_err(|e| e.to_string())?;
                let size = std::io::copy(&mut input, &mut out).map_err(|e| e.to_string())?;
                *used += size;
                return Ok((name, size));
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("could not copy: {e}")),
        }
    }
    unreachable!("1.. is unbounded")
}

/// The run's attachments, start and steer, in arrival order.
pub fn read(context: &Path) -> Vec<AttachmentRef> {
    std::fs::read(context.join(ATTACHMENTS_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// The `run-brief.md` lines naming each file for the steps.
pub fn brief_lines(refs: &[AttachmentRef]) -> Vec<String> {
    refs.iter()
        .map(|a| match (&a.path, &a.error) {
            (Some(path), _) => format!(
                "Attachment from the requester: {path} ({}, {})",
                a.mime,
                size_words(a.size_bytes.unwrap_or(0))
            ),
            (None, error) => format!(
                "Attachment not received: {} — {}",
                a.name,
                error.as_deref().unwrap_or("unknown error")
            ),
        })
        .collect()
}

/// `[A-Za-z0-9._-]` only; anything else becomes `_`; empty or dots-only → `file`.
fn safe_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.chars().all(|c| c == '.') {
        "file".into()
    } else {
        s
    }
}

/// `a.tar.gz` → (`a.tar`, `.gz`); a leading dot is not an extension.
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => name.split_at(i),
        _ => (name, ""),
    }
}

fn size_words(bytes: u64) -> String {
    match bytes {
        0..=1023 => format!("{bytes} B"),
        1024..=1_048_575 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

#[cfg(test)]
#[path = "attachments_tests.rs"]
mod tests;
