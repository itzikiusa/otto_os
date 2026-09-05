//! Image extraction. Transcripts inline pasted screenshots as base64 PNG
//! (`{type:"image", source:{type:"base64", media_type, data}}` in Claude user
//! records and inside tool results). They are never sent inline over the API:
//! the fold writes each one ONCE to `<data>/transcripts/<sid>/img/<id>.<ext>`
//! and the block carries only the id (`GET …/transcript/images/{id}`).
//!
//! The id is the sha1 of the base64 TEXT, so a fold without a store (a unit
//! test, the history index) yields the same ids as one that writes files.

use std::path::{Path, PathBuf};

use base64::Engine;
use sha1::{Digest, Sha1};

/// Where extracted images for one transcript live.
#[derive(Debug, Clone)]
pub struct ImageStore {
    dir: PathBuf,
}

/// Extensions the store will ever write / the route will ever look up.
pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "gif", "webp"];

impl ImageStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Stable id for a base64 payload.
    pub fn id_for(data_b64: &str) -> String {
        let mut h = Sha1::new();
        h.update(data_b64.as_bytes());
        format!("{:x}", h.finalize())
    }

    /// File extension for a media type (unknown types are stored as `.png` —
    /// the route serves the media type from the block, not the extension).
    pub fn ext_for(media_type: &str) -> &'static str {
        match media_type {
            "image/jpeg" | "image/jpg" => "jpg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            _ => "png",
        }
    }

    /// Persist `data_b64` (if not already there) and return the id. A decode or
    /// write failure still returns the id — the block renders as a broken
    /// image rather than failing the whole fold.
    pub fn put(&self, media_type: &str, data_b64: &str) -> String {
        let id = Self::id_for(data_b64);
        let path = self.dir.join(format!("{id}.{}", Self::ext_for(media_type)));
        if path.is_file() {
            return id;
        }
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data_b64.trim()) else {
            tracing::debug!(%id, "transcript image: base64 decode failed");
            return id;
        };
        if let Err(e) = std::fs::create_dir_all(&self.dir).and_then(|_| {
            let tmp = self.dir.join(format!("{id}.tmp"));
            std::fs::write(&tmp, &bytes)?;
            std::fs::rename(&tmp, &path)
        }) {
            tracing::debug!(%id, "transcript image: write failed: {e}");
        }
        id
    }

    /// The on-disk path of an extracted image (any known extension), if present.
    /// `id` must be plain hex — anything else is refused so a client can never
    /// steer the lookup outside the store.
    pub fn find(&self, id: &str) -> Option<(PathBuf, &'static str)> {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        for ext in IMAGE_EXTS {
            let p = self.dir.join(format!("{id}.{ext}"));
            if p.is_file() {
                let mime = match *ext {
                    "jpg" => "image/jpeg",
                    "gif" => "image/gif",
                    "webp" => "image/webp",
                    _ => "image/png",
                };
                return Some((p, mime));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1×1 transparent PNG.
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

    #[test]
    fn put_writes_once_and_find_locates_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = ImageStore::new(dir.path().join("img"));
        let id = store.put("image/png", PNG);
        assert_eq!(id, ImageStore::id_for(PNG));
        let (p, mime) = store.find(&id).expect("found");
        assert_eq!(mime, "image/png");
        assert!(p.ends_with(format!("{id}.png")));
        let before = std::fs::metadata(&p).unwrap().modified().unwrap();
        store.put("image/png", PNG); // no rewrite
        assert_eq!(std::fs::metadata(&p).unwrap().modified().unwrap(), before);
        // Non-hex ids are refused outright.
        assert!(store.find("../etc/passwd").is_none());
        assert!(store.find("").is_none());
    }
}
