//! Paste / drop attachments: save the bytes as a NEW file under the vault's
//! attachment folder and hand back the embed to insert at the cursor.
//!
//! Conventions follow the vault: an Obsidian vault's `.obsidian/app.json`
//! (`attachmentFolderPath`: `/` root, `./` beside the note, `./sub` a
//! subfolder beside the note, anything else a fixed folder; Obsidian's own
//! default is the root; `useMarkdownLinks` picks `![](…)` over `![[…]]`).
//! A vault without Obsidian settings uses `attachments/` and wikilink embeds.
//! The embed always carries the vault-relative path, so it resolves even
//! when the basename exists elsewhere (an ambiguous basename stays
//! unresolved by design).
//!
//! Safety: the name is reduced to one safe segment (no separators, no
//! hidden/dot names, no control, bidi or zero-width characters, no
//! link-breaking `[]#^|`), images are recognized by their bytes (never by
//! the extension alone), a file is created with `O_EXCL` (an existing file
//! is never overwritten — a numbered name is tried instead), and every
//! directory hop is opened `NOFOLLOW` relative to the vault root.

use std::path::Path;
use std::sync::Arc;

use otto_core::{Error, Result};
use rustix::fs::{Mode, OFlags};
use rustix::io::Errno;

use crate::engine::VaultEngine;
use crate::types::VaultAttachment;

/// Largest attachment accepted (decoded bytes).
pub const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
/// Folder used when the vault has no Obsidian attachment setting.
const DEFAULT_FOLDER: &str = "attachments";
/// Numbered-name attempts before giving up (`name 1.png` … `name 999.png`).
const MAX_SUFFIX: u32 = 999;
/// Stem budget in bytes (the extension and a ` 999` suffix come on top).
const MAX_STEM_BYTES: usize = 100;

/// Non-image types accepted by extension. Images are accepted only when the
/// bytes sniff as one; PDFs must start with `%PDF-`.
const PLAIN_EXTS: &[&str] = &[
    "txt", "csv", "json", "yaml", "yml", "log", "zip", "mp3", "m4a", "wav", "mp4", "mov", "webm",
    "docx", "xlsx", "pptx",
];
const IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "heic", "tif", "tiff",
];

/// The image type of `bytes`, by magic number.
pub(crate) fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.starts_with(b"BM") && bytes.len() >= 26 {
        Some("bmp")
    } else {
        None
    }
}

fn is_unsafe_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '[' | ']' | '#' | '^' | '%'
        )
        // Bidi controls and zero-width characters can disguise a name.
        || matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

/// Reduce a client-supplied name to `(stem, extension)`: one segment, safe
/// characters, no leading dot, bounded length. The extension is lower-case
/// ASCII alphanumerics (≤ 10) or empty.
pub(crate) fn split_safe_name(raw: &str) -> (String, String) {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = last
        .chars()
        .map(|c| if is_unsafe_char(c) { '-' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches(['.', '-', ' ']);
    let (stem, ext) = match cleaned.rsplit_once('.') {
        Some((s, e))
            if !s.is_empty()
                && !e.is_empty()
                && e.len() <= 10
                && e.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            (s, e.to_ascii_lowercase())
        }
        _ => (cleaned, String::new()),
    };
    let mut stem = stem.trim().trim_end_matches(['.', ' ']).to_string();
    if stem.len() > MAX_STEM_BYTES {
        let mut cut = MAX_STEM_BYTES;
        while !stem.is_char_boundary(cut) {
            cut -= 1;
        }
        stem.truncate(cut);
        stem = stem.trim_end_matches(['.', ' ']).to_string();
    }
    (stem, ext)
}

/// Decide the stored `(stem, ext)` from the name and the bytes. Errors are
/// user-facing: they explain what is accepted.
pub(crate) fn classify(raw_name: &str, bytes: &[u8], stamp: &str) -> Result<(String, String)> {
    if bytes.is_empty() {
        return Err(Error::Invalid("The file is empty.".into()));
    }
    if bytes.len() > MAX_ATTACHMENT_BYTES {
        return Err(Error::PayloadTooLarge(format!(
            "The file is {:.1} MB; attachments can be at most {} MB.",
            bytes.len() as f64 / (1024.0 * 1024.0),
            MAX_ATTACHMENT_BYTES / (1024 * 1024)
        )));
    }
    let (stem, ext) = split_safe_name(raw_name);
    let ext = if let Some(kind) = sniff_image(bytes) {
        // The bytes decide: a PNG named `.jpg` is stored as `.png`.
        kind.to_string()
    } else if IMAGE_EXTS.contains(&ext.as_str()) {
        return Err(Error::UnsupportedMedia(format!(
            "This doesn't look like a PNG, JPEG, GIF, WebP or BMP image (named .{ext}); \
             it was not saved."
        )));
    } else if ext == "pdf" {
        if !bytes.starts_with(b"%PDF-") {
            return Err(Error::UnsupportedMedia(
                "This file is named .pdf but isn't a PDF; it was not saved.".into(),
            ));
        }
        ext
    } else if PLAIN_EXTS.contains(&ext.as_str()) {
        ext
    } else {
        return Err(Error::UnsupportedMedia(format!(
            "{} files can't be attached. Attach an image, a PDF, or a {} file.",
            if ext.is_empty() {
                "Unnamed".to_string()
            } else {
                format!(".{ext}")
            },
            PLAIN_EXTS.join(" / ")
        )));
    };
    let stem = if stem.is_empty() {
        if sniff_image(bytes).is_some() {
            format!("Pasted image {stamp}")
        } else {
            format!("Attachment {stamp}")
        }
    } else {
        stem
    };
    Ok((stem, ext))
}

/// Where attachments go and how they are linked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttachmentStyle {
    /// Vault-relative folder ("" = vault root).
    pub folder: String,
    pub markdown_links: bool,
}

/// Read the vault's attachment convention. `note` is the vault-relative
/// note the attachment is for (its folder anchors `./` settings).
pub(crate) fn style_for(root: &str, note: Option<&str>) -> AttachmentStyle {
    let obsidian = Path::new(root).join(".obsidian");
    let settings: Option<serde_json::Value> = std::fs::symlink_metadata(&obsidian)
        .ok()
        .filter(|m| m.is_dir())
        .and_then(|_| {
            let app = obsidian.join("app.json");
            let meta = std::fs::symlink_metadata(&app).ok()?;
            if !meta.is_file() || meta.len() > 256 * 1024 {
                return None;
            }
            serde_json::from_slice(&std::fs::read(app).ok()?).ok()
        });
    let is_obsidian = obsidian.is_dir();
    let configured = settings
        .as_ref()
        .and_then(|s| s.get("attachmentFolderPath"))
        .and_then(|v| v.as_str())
        .map(str::trim);
    let markdown_links = settings
        .as_ref()
        .and_then(|s| s.get("useMarkdownLinks"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let note_dir = note
        .and_then(|n| n.rsplit_once('/').map(|(d, _)| d.to_string()))
        .unwrap_or_default();
    let join = |a: &str, b: &str| match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_string(),
        (_, true) => a.to_string(),
        _ => format!("{a}/{b}"),
    };
    let folder = match configured {
        Some("/") | Some("") => String::new(),
        Some(p) if p == "./" || p == "." => note_dir,
        Some(p) if p.starts_with("./") => join(&note_dir, p[2..].trim_matches('/')),
        Some(p) => p.trim_matches('/').to_string(),
        // An Obsidian vault without the key uses Obsidian's default: root.
        None if is_obsidian => String::new(),
        None => DEFAULT_FOLDER.to_string(),
    };
    AttachmentStyle {
        folder,
        markdown_links,
    }
}

/// The embed for a saved file.
pub(crate) fn embed_for(rel: &str, markdown_links: bool) -> String {
    if markdown_links {
        let encoded: String = rel
            .chars()
            .map(|c| match c {
                ' ' => "%20".to_string(),
                '(' => "%28".to_string(),
                ')' => "%29".to_string(),
                c => c.to_string(),
            })
            .collect();
        format!("![]({encoded})")
    } else {
        format!("![[{rel}]]")
    }
}

/// Candidate file name for attempt `n` (0 = the plain name).
fn candidate(stem: &str, ext: &str, n: u32) -> String {
    let stem = if n == 0 {
        stem.to_string()
    } else {
        format!("{stem} {n}")
    };
    if ext.is_empty() {
        stem
    } else {
        format!("{stem}.{ext}")
    }
}

/// Create `folder/<unique name>` with `O_EXCL` relative to the held parent
/// directory and write `bytes`. Returns the name used. A partial file is
/// removed on a failed write (it is ours: `O_EXCL` created it).
fn create_unique(
    parent: &rustix::fd::OwnedFd,
    stem: &str,
    ext: &str,
    bytes: &[u8],
) -> Result<String> {
    use std::io::Write;
    for n in 0..=MAX_SUFFIX {
        let name = candidate(stem, ext, n);
        let fd = match rustix::fs::openat(
            parent,
            name.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR | Mode::RGRP | Mode::ROTH,
        ) {
            Ok(fd) => fd,
            Err(Errno::EXIST) => continue,
            Err(e) => return Err(Error::Internal(format!("create attachment {name}: {e}"))),
        };
        let mut file = std::fs::File::from(fd);
        let written = file.write_all(bytes).and_then(|()| file.sync_all());
        if let Err(e) = written {
            drop(file);
            let _ = rustix::fs::unlinkat(parent, name.as_str(), rustix::fs::AtFlags::empty());
            return Err(Error::Internal(format!("write attachment {name}: {e}")));
        }
        return Ok(name);
    }
    Err(Error::Conflict(format!(
        "{} names like \"{}\" already exist; rename the file and retry.",
        MAX_SUFFIX + 1,
        candidate(stem, ext, 0)
    )))
}

impl VaultEngine {
    /// `POST …/attachment?note=&name=` — save pasted/dropped bytes as a new
    /// file and return its path and embed. Never overwrites a file.
    pub async fn save_attachment(
        self: &Arc<Self>,
        ws: &str,
        id: i64,
        note: Option<&str>,
        name: &str,
        bytes: &[u8],
    ) -> Result<VaultAttachment> {
        let v = self.get_scoped(ws, id).await?;
        let note = note
            .filter(|n| !n.trim().is_empty())
            .map(Self::check_rel)
            .transpose()?;
        let stamp = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
        let (stem, ext) = classify(name, bytes, &stamp)?;
        let root = v.root_path.clone();
        let style = {
            let root = root.clone();
            let note = note.clone();
            tokio::task::spawn_blocking(move || style_for(&root, note.as_deref()))
                .await
                .map_err(|e| Error::Internal(format!("attachment settings: {e}")))?
        };
        // Validate the folder like any client path (no `..`, no hidden
        // segments) with a placeholder leaf, then vet protected locations.
        let probe = if style.folder.is_empty() {
            candidate(&stem, &ext, 0)
        } else {
            format!("{}/{}", style.folder, candidate(&stem, &ext, 0))
        };
        let probe = Self::check_rel(&probe)?;
        Self::abs_guarded(&root, &probe)?;
        let lock = self.write_lock(&root);
        let _guard = lock.lock().await;
        let state = self.index_state(id);
        state.ensure(self.store(), id).await?;
        let publication = state.publication.lock().await;
        let (parent, _) = Self::text_parent(&root, &probe)?;
        let bytes = bytes.to_vec();
        let (stem2, ext2) = (stem.clone(), ext.clone());
        let saved =
            tokio::task::spawn_blocking(move || create_unique(&parent, &stem2, &ext2, &bytes))
                .await
                .map_err(|e| Error::Internal(format!("attachment write: {e}")))??;
        let rel = if style.folder.is_empty() {
            saved
        } else {
            format!("{}/{saved}", style.folder)
        };
        state.mutated(&[&rel]);
        drop(publication);
        // Index it now so the embed resolves on the next render.
        self.rescan_after_mutation(id).await;
        let size = std::fs::metadata(Path::new(&root).join(&rel))
            .map(|m| m.len() as i64)
            .unwrap_or_default();
        Ok(VaultAttachment {
            embed: embed_for(&rel, style.markdown_links),
            path: rel,
            size,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

    #[test]
    fn names_are_one_safe_segment() {
        for (raw, stem, ext) in [
            ("../../etc/passwd", "passwd", ""),
            ("/abs/path/shot.PNG", "shot", "png"),
            ("C:\\Users\\me\\evil.txt", "evil", "txt"),
            ("..hidden.txt", "hidden", "txt"),
            (".env", "env", ""),
            ("a[b]#c^d|e.png", "a-b--c-d-e", "png"),
            ("rtl\u{202E}gnp.exe", "rtl-gnp", "exe"),
            ("zero\u{200B}width.csv", "zero-width", "csv"),
            ("ctrl\u{0007}bell.txt", "ctrl-bell", "txt"),
            ("  spaced  .txt", "spaced", "txt"),
            ("Résumé 日本語.pdf", "Résumé 日本語", "pdf"),
            ("noext", "noext", ""),
            ("weird.ext!", "weird.ext!", ""),
            ("", "", ""),
        ] {
            assert_eq!(
                split_safe_name(raw),
                (stem.to_string(), ext.to_string()),
                "{raw:?}"
            );
        }
        // Long multi-byte names are cut on a character boundary.
        let long = "é".repeat(200) + ".png";
        let (stem, ext) = split_safe_name(&long);
        assert!(stem.len() <= MAX_STEM_BYTES && stem.chars().all(|c| c == 'é'));
        assert_eq!(ext, "png");
    }

    #[test]
    fn images_are_typed_by_content_not_name() {
        let (stem, ext) = classify("photo.jpg", PNG, "20261112").unwrap();
        assert_eq!((stem.as_str(), ext.as_str()), ("photo", "png"));
        let (stem, ext) = classify("image.png", PNG, "20261112").unwrap();
        assert_eq!((stem.as_str(), ext.as_str()), ("image", "png"));
        // A clipboard paste with no name.
        let (stem, _) = classify("", PNG, "20261112").unwrap();
        assert_eq!(stem, "Pasted image 20261112");
        // Not an image, though named like one.
        assert!(matches!(
            classify("x.png", b"<script>alert(1)</script>", "s"),
            Err(Error::UnsupportedMedia(_))
        ));
        assert!(matches!(
            classify("x.svg", b"<svg onload=alert(1)>", "s"),
            Err(Error::UnsupportedMedia(_))
        ));
        assert!(matches!(
            classify("x.html", b"<html>", "s"),
            Err(Error::UnsupportedMedia(_))
        ));
        assert!(matches!(
            classify("x.pdf", b"not a pdf", "s"),
            Err(Error::UnsupportedMedia(_))
        ));
        assert!(classify("x.pdf", b"%PDF-1.7 ...", "s").is_ok());
        assert!(matches!(
            classify("x.txt", b"", "s"),
            Err(Error::Invalid(_))
        ));
        let big = vec![b'a'; MAX_ATTACHMENT_BYTES + 1];
        assert!(matches!(
            classify("x.txt", &big, "s"),
            Err(Error::PayloadTooLarge(_))
        ));
    }

    #[test]
    fn style_follows_obsidian_settings() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        // Plain folder: the default attachments folder, wikilinks.
        assert_eq!(
            style_for(root, Some("notes/a.md")),
            AttachmentStyle {
                folder: "attachments".into(),
                markdown_links: false
            }
        );
        // Obsidian vault without the key: Obsidian's default, the root.
        std::fs::create_dir(dir.path().join(".obsidian")).unwrap();
        assert_eq!(style_for(root, Some("notes/a.md")).folder, "");
        for (setting, want) in [
            ("/", ""),
            ("./", "notes"),
            ("./assets", "notes/assets"),
            ("Files/Media/", "Files/Media"),
        ] {
            std::fs::write(
                dir.path().join(".obsidian/app.json"),
                format!(r#"{{"attachmentFolderPath":"{setting}","useMarkdownLinks":true}}"#),
            )
            .unwrap();
            let s = style_for(root, Some("notes/a.md"));
            assert_eq!(s.folder, want, "{setting}");
            assert!(s.markdown_links);
        }
        assert_eq!(
            embed_for("a b/c (1).png", true),
            "![](a%20b/c%20%281%29.png)"
        );
        assert_eq!(
            embed_for("attachments/x.png", false),
            "![[attachments/x.png]]"
        );
    }

    async fn vault() -> (Arc<VaultEngine>, tempfile::TempDir, i64) {
        let e = Arc::new(VaultEngine::new(otto_state::db::test_pool().await));
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.md"), "# Note").unwrap();
        let id = e
            .store()
            .create_vault("ws", "Att", dir.path().to_str().unwrap(), true)
            .await
            .unwrap();
        e.scan(id).await.unwrap();
        (e, dir, id)
    }

    /// Never overwrites: the same name twice yields `name 1.ext`, and an
    /// existing user file keeps its bytes. The embed resolves after save.
    #[tokio::test]
    async fn saving_never_collides_or_overwrites() {
        let (e, dir, id) = vault().await;
        std::fs::create_dir(dir.path().join("attachments")).unwrap();
        std::fs::write(dir.path().join("attachments/shot.png"), b"user bytes").unwrap();
        let a = e
            .save_attachment("ws", id, Some("note.md"), "shot.png", PNG)
            .await
            .unwrap();
        assert_eq!(a.path, "attachments/shot 1.png");
        assert_eq!(a.embed, "![[attachments/shot 1.png]]");
        let b = e
            .save_attachment("ws", id, Some("note.md"), "shot.png", PNG)
            .await
            .unwrap();
        assert_eq!(b.path, "attachments/shot 2.png");
        assert_eq!(
            std::fs::read(dir.path().join("attachments/shot.png")).unwrap(),
            b"user bytes"
        );
        assert_eq!(std::fs::read(dir.path().join(&a.path)).unwrap(), PNG);
        assert_eq!(a.size, PNG.len() as i64);
        assert!(e
            .store()
            .all_file_paths(id)
            .await
            .unwrap()
            .contains(&a.path));
    }

    /// Hostile names and folders cannot leave the attachment folder.
    #[tokio::test]
    async fn saving_cannot_escape_the_attachments_folder() {
        let (e, dir, id) = vault().await;
        for name in ["../../escape.png", "/tmp/escape.png", "..\\..\\escape.png"] {
            let a = e.save_attachment("ws", id, None, name, PNG).await.unwrap();
            assert_eq!(
                Path::new(&a.path).parent().unwrap(),
                Path::new("attachments"),
                "{name}"
            );
            assert!(dir.path().join(&a.path).is_file());
        }
        assert!(!dir.path().parent().unwrap().join("escape.png").exists());
        // A traversal in the note path is refused outright.
        assert!(matches!(
            e.save_attachment("ws", id, Some("../x.md"), "a.png", PNG)
                .await,
            Err(Error::Invalid(_))
        ));
        // A hidden folder from settings is refused, not created.
        std::fs::create_dir(dir.path().join(".obsidian")).unwrap();
        std::fs::write(
            dir.path().join(".obsidian/app.json"),
            r#"{"attachmentFolderPath":"../outside"}"#,
        )
        .unwrap();
        assert!(e
            .save_attachment("ws", id, None, "a.png", PNG)
            .await
            .is_err());
        assert!(!dir.path().parent().unwrap().join("outside").exists());
        // A symlinked attachment folder pointing outside is refused.
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".obsidian/app.json"),
            r#"{"attachmentFolderPath":"linked"}"#,
        )
        .unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("linked")).unwrap();
        assert!(e
            .save_attachment("ws", id, None, "a.png", PNG)
            .await
            .is_err());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }
}
