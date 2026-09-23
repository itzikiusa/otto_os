//! File-backed recovery records. Hidden directories are deliberately excluded
//! from the derived index. No retention cleanup silently removes user history.
use std::path::Path;
use std::sync::Arc;

use otto_core::{Error, Result};
use rustix::fs::{AtFlags, RenameFlags};
use sha2::{Digest, Sha256};

use crate::{types::*, VaultEngine};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 100
        || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err(Error::Invalid("invalid recovery entry id".into()));
    }
    Ok(())
}

impl VaultEngine {
    async fn recovery_write(root: &str, path: &str, bytes: &[u8]) -> Result<()> {
        let (parent, name) = Self::text_parent(root, path)?;
        // Recovery copies can contain private notes; only the owner may enter
        // per-revision directories and the trash manifest directory.
        rustix::fs::fchmod(&parent, rustix::fs::Mode::RWXU)
            .map_err(|e| Error::Internal(format!("protect recovery directory: {e}")))?;
        Self::atomic_replace_at(&parent, &name, bytes).await
    }

    async fn recovery_read(root: &str, path: &str) -> Result<Vec<u8>> {
        let (parent, name) = Self::text_parent(root, path)?;
        Self::text_file_bytes(&parent, &name)
            .await?
            .ok_or_else(|| Error::NotFound("recovery entry no longer exists".into()))
    }

    /// Directory capabilities reject symlinks on every hidden storage hop too.
    fn recovery_names(root: &str, dir: &str) -> Result<Vec<String>> {
        if !Path::new(root).join(dir).exists() {
            return Ok(Vec::new());
        }
        let _capability = Self::text_parent(root, &format!("{dir}/entry"))?;
        let mut names = std::fs::read_dir(Path::new(root).join(dir))
            .map_err(|e| Error::Internal(format!("read recovery directory: {e}")))?
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
            .filter(|name| valid_id(name.trim_end_matches(".json")).is_ok())
            .collect::<Vec<_>>();
        names.sort();
        names.reverse();
        Ok(names)
    }

    pub(crate) async fn prepare_revision(
        root: &str,
        path: &str,
        before: Option<&[u8]>,
        after: &[u8],
        reason: &str,
    ) -> Result<Option<VaultRevision>> {
        if before == Some(after) {
            return Ok(None);
        }
        let revision = VaultRevision {
            id: otto_core::new_id().to_string(),
            path: path.into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            before_hash: before.map(hash),
            after_hash: hash(after),
            reason: reason.into(),
            committed: false,
        };
        let dir = format!(".otto-history/{}", revision.id);
        if let Some(bytes) = before {
            Self::recovery_write(root, &format!("{dir}/before"), bytes).await?;
        }
        Self::recovery_write(root, &format!("{dir}/after"), after).await?;
        Self::recovery_write(
            root,
            &format!("{dir}/meta.json"),
            &serde_json::to_vec(&revision).unwrap(),
        )
        .await?;
        Ok(Some(revision))
    }

    pub(crate) async fn commit_revision(root: &str, revision: Option<VaultRevision>) -> Result<()> {
        if let Some(mut revision) = revision {
            revision.committed = true;
            Self::recovery_write(
                root,
                &format!(".otto-history/{}/meta.json", revision.id),
                &serde_json::to_vec(&revision).unwrap(),
            )
            .await?;
        }
        Ok(())
    }

    pub async fn revisions(
        &self,
        ws: &str,
        id: i64,
        path: Option<&str>,
    ) -> Result<Vec<VaultRevision>> {
        self.revisions_page(ws, id, path, None).await
    }

    pub async fn revisions_page(
        &self,
        ws: &str,
        id: i64,
        path: Option<&str>,
        before: Option<&str>,
    ) -> Result<Vec<VaultRevision>> {
        if let Some(cursor) = before {
            valid_id(cursor)?;
        }
        let v = self.get_scoped(ws, id).await?;
        if let Some(path) = path {
            Self::check_rel(path)?;
        }
        let mut out = Vec::new();
        for name in Self::recovery_names(&v.root_path, ".otto-history")? {
            if before.is_some_and(|cursor| name.as_str() >= cursor) {
                continue;
            }
            let bytes =
                match Self::recovery_read(&v.root_path, &format!(".otto-history/{name}/meta.json"))
                    .await
                {
                    Ok(bytes) => bytes,
                    Err(Error::NotFound(_)) => continue,
                    Err(e) => return Err(e),
                };
            let revision: VaultRevision = serde_json::from_slice(&bytes)
                .map_err(|e| Error::Invalid(format!("invalid revision record: {e}")))?;
            if path.is_none_or(|p| p == revision.path) {
                out.push(revision);
            }
            if out.len() == 200 {
                break;
            }
        }
        Ok(out)
    }

    pub async fn revision(&self, ws: &str, id: i64, entry: &str) -> Result<VaultRevisionDetail> {
        valid_id(entry)?;
        let v = self.get_scoped(ws, id).await?;
        let dir = format!(".otto-history/{entry}");
        let revision: VaultRevision = serde_json::from_slice(
            &Self::recovery_read(&v.root_path, &format!("{dir}/meta.json")).await?,
        )
        .map_err(|e| Error::Invalid(format!("invalid revision record: {e}")))?;
        let before = if revision.before_hash.is_some() {
            Some(
                String::from_utf8(
                    Self::recovery_read(&v.root_path, &format!("{dir}/before")).await?,
                )
                .map_err(|_| Error::Invalid("revision contains non-UTF-8 content".into()))?,
            )
        } else {
            None
        };
        let after =
            String::from_utf8(Self::recovery_read(&v.root_path, &format!("{dir}/after")).await?)
                .map_err(|_| Error::Invalid("revision contains non-UTF-8 content".into()))?;
        if before.as_deref().map(|raw| hash(raw.as_bytes())) != revision.before_hash
            || hash(after.as_bytes()) != revision.after_hash
        {
            return Err(Error::Conflict(
                "revision content failed its checksum; restore was refused".into(),
            ));
        }
        Ok(VaultRevisionDetail {
            revision,
            before,
            after,
        })
    }

    pub async fn restore_revision(
        self: &Arc<Self>,
        ws: &str,
        id: i64,
        entry: &str,
        version: &str,
        if_hash: &str,
    ) -> Result<()> {
        let detail = self.revision(ws, id, entry).await?;
        let content = match version {
            "before" => detail.before.as_deref().ok_or_else(|| {
                Error::Invalid("this revision created the file; no prior content exists".into())
            })?,
            "after" => &detail.after,
            _ => return Err(Error::Invalid("version must be before or after".into())),
        };
        // Restores go through normal guarded writes and create another revision.
        if detail.revision.path.to_ascii_lowercase().ends_with(".md") {
            self.write_note(ws, id, &detail.revision.path, content, Some(if_hash))
                .await?;
        } else {
            self.write_text_file(ws, id, &detail.revision.path, content, Some(if_hash))
                .await?;
        }
        Ok(())
    }

    pub(crate) async fn record_trash(root: &str, entry: &VaultTrashEntry) -> Result<()> {
        Self::recovery_write(
            root,
            &format!(".trash/.otto-index/{}.json", entry.id),
            &serde_json::to_vec(entry).unwrap(),
        )
        .await
    }

    pub async fn trash_entries(&self, ws: &str, id: i64) -> Result<Vec<VaultTrashEntry>> {
        let v = self.get_scoped(ws, id).await?;
        let mut out = Vec::new();
        for name in Self::recovery_names(&v.root_path, ".trash/.otto-index")? {
            let entry: VaultTrashEntry = serde_json::from_slice(
                &Self::recovery_read(&v.root_path, &format!(".trash/.otto-index/{name}")).await?,
            )
            .map_err(|e| Error::Invalid(format!("invalid trash record: {e}")))?;
            Self::check_rel(&entry.stored_path)?;
            if Path::new(&v.root_path)
                .join(".trash")
                .join(&entry.stored_path)
                .symlink_metadata()
                .is_ok()
            {
                out.push(entry);
            }
        }
        // Older versions kept files without manifests. Surface those too,
        // without mutating disk during a read. Folder records already cover
        // all their descendants, so do not list those files a second time.
        let trash_root = Path::new(&v.root_path).join(".trash");
        if trash_root.exists() {
            let _guard = Self::text_parent(&v.root_path, ".trash/entry")?;
            let walk = crate::scan::walk(&trash_root)
                .map_err(|e| Error::Internal(format!("list legacy trash: {e}")))?;
            for item in walk.notes.into_iter().chain(walk.files) {
                if out.iter().any(|e| {
                    item.rel == e.stored_path
                        || item.rel.starts_with(&format!("{}/", e.stored_path))
                }) {
                    continue;
                }
                out.push(VaultTrashEntry {
                    id: format!("legacy-{}", hash(item.rel.as_bytes())),
                    original_path: item.rel.clone(),
                    stored_path: item.rel,
                    deleted_at: chrono::DateTime::from_timestamp_nanos(item.mtime_ns).to_rfc3339(),
                    kind: "file".into(),
                });
            }
        }
        out.sort_by(|a, b| b.deleted_at.cmp(&a.deleted_at));
        Ok(out)
    }

    pub async fn restore_trash(
        self: &Arc<Self>,
        ws: &str,
        id: i64,
        entry: &str,
        destination: Option<&str>,
    ) -> Result<String> {
        valid_id(entry)?;
        let v = self.get_scoped(ws, id).await?;
        let lock = self.write_lock(id, "");
        let _guard = lock.lock().await;
        let manifest = format!(".trash/.otto-index/{entry}.json");
        let legacy = entry.starts_with("legacy-");
        let item: VaultTrashEntry = if legacy {
            self.trash_entries(ws, id)
                .await?
                .into_iter()
                .find(|item| item.id == entry)
                .ok_or_else(|| Error::NotFound("trash entry no longer exists".into()))?
        } else {
            serde_json::from_slice(&Self::recovery_read(&v.root_path, &manifest).await?)
                .map_err(|e| Error::Invalid(format!("invalid trash record: {e}")))?
        };
        Self::check_rel(&item.stored_path)?;
        let dest = Self::check_rel(destination.unwrap_or(&item.original_path))?;
        let state = self.index_state(id);
        state.ensure(self.store(), id).await?;
        let publication = state.publication.lock().await;
        state.mutated(&[&dest]);
        let (source_parent, source_name) =
            Self::text_parent(&v.root_path, &format!(".trash/{}", item.stored_path))?;
        let (target_parent, target_name) = Self::text_parent(&v.root_path, &dest)?;
        rustix::fs::renameat_with(
            &source_parent,
            source_name.as_str(),
            &target_parent,
            target_name.as_str(),
            RenameFlags::NOREPLACE,
        )
        .map_err(|e| match e {
            rustix::io::Errno::EXIST => {
                Error::Conflict(format!("target exists: {dest}; choose another destination"))
            }
            rustix::io::Errno::NOENT => Error::NotFound("trash entry no longer exists".into()),
            _ => Error::Internal(format!("restore trash: {e}")),
        })?;
        if !legacy {
            let (parent, name) = Self::text_parent(&v.root_path, &manifest)?;
            rustix::fs::unlinkat(parent, name, AtFlags::empty())
                .map_err(|e| Error::Internal(format!("remove restored trash marker: {e}")))?;
        }
        drop(publication);
        self.rescan_after_mutation(id).await;
        Ok(dest)
    }
}
