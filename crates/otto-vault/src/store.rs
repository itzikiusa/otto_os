//! SQLite persistence for the vault index. Every row here is DERIVED from the
//! files on disk and rebuildable by a rescan — the store never holds the only
//! copy of anything.

use chrono::Utc;
use otto_state::DbPool;
use sqlx::Row;

use otto_core::{Error, Result};

use crate::types::*;

fn dberr(op: &'static str) -> impl Fn(sqlx::Error) -> Error {
    move |e| Error::Internal(format!("{op}: {e}"))
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
}

/// A note row as the scanner writes it.
pub struct NoteRow {
    pub path: String,
    pub title: String,
    pub okf_type: Option<String>,
    pub description: Option<String>,
    pub frontmatter_json: String,
    pub tags_json: String,
    pub aliases_json: String,
    pub headings_json: String,
    pub word_count: i64,
    pub size: i64,
    pub mtime_ns: i64,
    pub hash: String,
    pub reserved: bool,
    pub has_frontmatter: bool,
    pub parse_error: bool,
    pub content_index_status: ContentIndexStatus,
}

#[cfg(test)]
type RegistrationPausePair = (
    tokio::sync::oneshot::Sender<()>,
    tokio::sync::oneshot::Receiver<()>,
);

#[derive(Clone)]
pub struct Store {
    pool: DbPool,
    #[cfg(test)]
    pub(crate) all_reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    #[cfg(test)]
    pub(crate) link_reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// Status aggregate COUNT round-trips — regression counter (F2).
    #[cfg(test)]
    pub(crate) status_count_reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// Note-index transactions committed — regression counter (F7).
    #[cfg(test)]
    pub(crate) index_commits: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    #[cfg(test)]
    pub(crate) registration_pause: std::sync::Arc<std::sync::Mutex<Option<RegistrationPausePair>>>,
    #[cfg(test)]
    pub(crate) fts_fail_once: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// The status aggregates that need a COUNT over links/tags/files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatusCounts {
    pub notes: i64,
    pub links: i64,
    pub unresolved: i64,
    pub tags: i64,
    pub attachments: i64,
}

#[derive(Clone, Copy)]
pub(crate) enum FtsIndexing {
    Enabled,
    Unavailable,
    Retry,
}

impl Store {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        let pool: DbPool = pool.into();
        Self {
            pool,
            #[cfg(test)]
            all_reads: Default::default(),
            #[cfg(test)]
            link_reads: Default::default(),
            #[cfg(test)]
            status_count_reads: Default::default(),
            #[cfg(test)]
            index_commits: Default::default(),
            #[cfg(test)]
            registration_pause: Default::default(),
            #[cfg(test)]
            fts_fail_once: Default::default(),
        }
    }

    pub fn pool(&self) -> &DbPool {
        &self.pool
    }

    // -- vaults -------------------------------------------------------------

    pub async fn create_vault(&self, ws: &str, name: &str, root: &str, okf: bool) -> Result<i64> {
        // Vaults are global — enforce root uniqueness ACROSS workspaces here
        // (the table's UNIQUE(ws_id, root_path) is a pre-global relic; a
        // migration can't retro-tighten it without risking existing rows).
        // BEGIN IMMEDIATE owns the writer before the duplicate lookup, so a
        // second workspace cannot pass the check before this insert commits.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("vault.create.begin"))?;
        let dup = sqlx::query("SELECT 1 FROM vaults WHERE root_path = ?")
            .bind(root)
            .fetch_optional(&mut *tx)
            .await
            .map_err(dberr("vault.create.dup"))?;
        if dup.is_some() {
            return Err(Error::Conflict(format!(
                "vault already registered at {root}"
            )));
        }
        #[cfg(test)]
        {
            let pause = self.registration_pause.lock().unwrap().take();
            if let Some((started, resume)) = pause {
                let _ = started.send(());
                let _ = resume.await;
            }
        }
        let r = sqlx::query(
            "INSERT INTO vaults (ws_id, name, root_path, okf, created_at) VALUES (?,?,?,?,?)",
        )
        .bind(ws)
        .bind(name)
        .bind(root)
        .bind(okf as i64)
        .bind(now())
        .execute(&mut *tx)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(ref d) if d.message().contains("UNIQUE") => {
                Error::Conflict(format!("vault already registered at {root}"))
            }
            other => Error::Internal(format!("vault.create: {other}")),
        })?;
        let id = r.last_insert_rowid();
        tx.commit().await.map_err(dberr("vault.create.commit"))?;
        Ok(id)
    }

    fn vault_from_row(r: &sqlx::sqlite::SqliteRow) -> VaultRec {
        VaultRec {
            id: r.get("id"),
            ws_id: r.get("ws_id"),
            name: r.get("name"),
            root_path: r.get("root_path"),
            okf: r.get::<i64, _>("okf") != 0,
            created_at: r.get("created_at"),
            last_scan_at: r.get("last_scan_at"),
            scan_state: r.get("scan_state"),
            notes: r.try_get("notes").unwrap_or(0),
            links: r.try_get("links").unwrap_or(0),
        }
    }

    const VAULT_COLS: &'static str = "v.id, v.ws_id, v.name, v.root_path, v.okf, v.created_at, \
         v.last_scan_at, v.scan_state, \
         (SELECT COUNT(*) FROM vault_notes n WHERE n.vault_id = v.id) AS notes, \
         (SELECT COUNT(*) FROM vault_links l WHERE l.vault_id = v.id) AS links";

    pub async fn list_vaults(&self) -> Result<Vec<VaultRec>> {
        // GLOBAL list — vaults are a cross-workspace library. Dedup by
        // root_path (lowest id wins) in case pre-global rows registered the
        // same folder from two workspaces.
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM vaults v \
             WHERE v.id IN (SELECT MIN(id) FROM vaults GROUP BY root_path) \
             ORDER BY v.id",
            Self::VAULT_COLS
        )))
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.list"))?;
        Ok(rows.iter().map(Self::vault_from_row).collect())
    }

    pub async fn get_vault_metadata(&self, id: i64) -> Result<VaultRec> {
        let row = sqlx::query("SELECT id, ws_id, name, root_path, okf, created_at, last_scan_at, scan_state FROM vaults WHERE id = ?")
            .bind(id).fetch_optional(&self.pool).await.map_err(dberr("vault.metadata"))?
            .ok_or_else(|| Error::NotFound("vault".into()))?;
        Ok(Self::vault_from_row(&row))
    }

    pub async fn get_vault(&self, id: i64) -> Result<VaultRec> {
        #[cfg(test)]
        self.status_count_reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM vaults v WHERE v.id = ?",
            Self::VAULT_COLS
        )))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("vault.get"))?
        .ok_or_else(|| Error::NotFound("vault".into()))?;
        Ok(Self::vault_from_row(&row))
    }

    pub async fn patch_vault(&self, id: i64, name: Option<&str>, okf: Option<bool>) -> Result<()> {
        if let Some(n) = name {
            sqlx::query("UPDATE vaults SET name = ? WHERE id = ?")
                .bind(n)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("vault.patch"))?;
        }
        if let Some(o) = okf {
            sqlx::query("UPDATE vaults SET okf = ? WHERE id = ?")
                .bind(o as i64)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("vault.patch"))?;
        }
        Ok(())
    }

    /// Unregister: removes the vault row + every derived index row. Never
    /// touches the files on disk.
    pub async fn delete_vault(&self, id: i64) -> Result<()> {
        for sql in [
            "DELETE FROM vault_links WHERE vault_id = ?",
            "DELETE FROM vault_tags WHERE vault_id = ?",
            "DELETE FROM vault_files WHERE vault_id = ?",
            "DELETE FROM vault_notes WHERE vault_id = ?",
            "DELETE FROM vaults WHERE id = ?",
        ] {
            sqlx::query(sql)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("vault.delete"))?;
        }
        // By rowid through the doc map; absent tables (no FTS5) are fine.
        let _ = self.fts_clear_vault(id).await;
        Ok(())
    }

    pub async fn set_scan_state(&self, id: i64, state: &str, touch_scan_time: bool) -> Result<()> {
        if touch_scan_time {
            sqlx::query("UPDATE vaults SET scan_state = ?, last_scan_at = ? WHERE id = ?")
                .bind(state)
                .bind(now())
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("vault.scan_state"))?;
        } else {
            sqlx::query("UPDATE vaults SET scan_state = ? WHERE id = ?")
                .bind(state)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("vault.scan_state"))?;
        }
        Ok(())
    }

    pub async fn status(&self, id: i64) -> Result<VaultStatus> {
        Ok(self.status_with(id, None).await?.0)
    }

    /// Status with the three aggregate COUNTs taken from `cached` when the
    /// caller knows nothing changed (F2); returns the counts it used.
    pub async fn status_with(
        &self,
        id: i64,
        cached: Option<StatusCounts>,
    ) -> Result<(VaultStatus, StatusCounts)> {
        let v = self.get_vault_metadata(id).await?;
        self.status_with_metadata(v, cached).await
    }

    pub async fn status_with_metadata(
        &self,
        v: VaultRec,
        cached: Option<StatusCounts>,
    ) -> Result<(VaultStatus, StatusCounts)> {
        let id = v.id;
        let counts = match cached {
            Some(c) => c,
            None => self.status_counts(id).await?,
        };
        #[cfg(test)]
        if cached.is_none() {
            self.status_count_reads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        let StatusCounts {
            notes,
            links,
            unresolved,
            tags,
            attachments,
        } = counts;
        Ok((
            VaultStatus {
                generation: None,
                graph_generation: None,
                id,
                scan_state: v.scan_state,
                last_scan_at: v.last_scan_at,
                notes,
                links,
                unresolved,
                tags,
                attachments,
                tracked_recovery: Vec::new(),
                search_index: None,
            },
            counts,
        ))
    }

    async fn status_counts(&self, id: i64) -> Result<StatusCounts> {
        let notes = sqlx::query_scalar("SELECT COUNT(*) FROM vault_notes WHERE vault_id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("vault.status.notes"))?;
        let links = sqlx::query_scalar("SELECT COUNT(*) FROM vault_links WHERE vault_id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("vault.status.links"))?;
        let unresolved: i64 = sqlx::query(
            "SELECT COUNT(*) AS c FROM vault_links WHERE vault_id = ? AND dst_path IS NULL",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("vault.status"))?
        .get("c");
        let tags: i64 =
            sqlx::query("SELECT COUNT(DISTINCT tag) AS c FROM vault_tags WHERE vault_id = ?")
                .bind(id)
                .fetch_one(&self.pool)
                .await
                .map_err(dberr("vault.status"))?
                .get("c");
        let attachments: i64 =
            sqlx::query("SELECT COUNT(*) AS c FROM vault_files WHERE vault_id = ?")
                .bind(id)
                .fetch_one(&self.pool)
                .await
                .map_err(dberr("vault.status"))?
                .get("c");
        Ok(StatusCounts {
            notes,
            links,
            unresolved,
            tags,
            attachments,
        })
    }

    /// `(size, mtime_ns)` of the given paths, notes and files alike — the
    /// watcher's targeted "did this really change?" probe (F2).
    pub async fn sigs_for<'a>(
        &self,
        vault: i64,
        paths: impl Iterator<Item = &'a str>,
    ) -> Result<std::collections::HashMap<String, (i64, i64)>> {
        let paths: Vec<&str> = paths.collect();
        let mut out = std::collections::HashMap::new();
        for chunk in paths.chunks(200) {
            let marks = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT path, size, mtime_ns FROM vault_notes WHERE vault_id = ? AND path IN ({marks}) \
                 UNION ALL SELECT path, size, mtime_ns FROM vault_files WHERE vault_id = ? AND path IN ({marks})"
            );
            // Only `?` placeholders are interpolated; paths are bound.
            let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.as_str())).bind(vault);
            for p in chunk {
                q = q.bind(*p);
            }
            q = q.bind(vault);
            for p in chunk {
                q = q.bind(*p);
            }
            for r in q
                .fetch_all(&self.pool)
                .await
                .map_err(dberr("vault.sigs_for"))?
            {
                out.insert(r.get("path"), (r.get("size"), r.get("mtime_ns")));
            }
        }
        Ok(out)
    }

    // -- notes ---------------------------------------------------------------

    /// `(path, size, mtime_ns)` of every indexed note — the incremental-scan diff basis.
    pub async fn note_sigs(&self, vault: i64) -> Result<Vec<(String, i64, i64)>> {
        let rows = sqlx::query("SELECT path, size, mtime_ns FROM vault_notes WHERE vault_id = ?")
            .bind(vault)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("vault.note_sigs"))?;
        Ok(rows
            .iter()
            .map(|r| (r.get("path"), r.get("size"), r.get("mtime_ns")))
            .collect())
    }

    pub async fn file_sigs(&self, vault: i64) -> Result<Vec<(String, i64, i64)>> {
        let rows = sqlx::query("SELECT path, size, mtime_ns FROM vault_files WHERE vault_id = ?")
            .bind(vault)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("vault.file_sigs"))?;
        Ok(rows
            .iter()
            .map(|r| (r.get("path"), r.get("size"), r.get("mtime_ns")))
            .collect())
    }

    async fn upsert_note_conn(
        conn: &mut sqlx::SqliteConnection,
        vault: i64,
        n: &NoteRow,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO vault_notes (vault_id, path, title, okf_type, description, \
             frontmatter_json, tags_json, aliases_json, headings_json, word_count, size, \
             mtime_ns, hash, reserved, has_frontmatter, parse_error, content_index_status) \
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) \
             ON CONFLICT(vault_id, path) DO UPDATE SET \
             title=excluded.title, okf_type=excluded.okf_type, description=excluded.description, \
             frontmatter_json=excluded.frontmatter_json, tags_json=excluded.tags_json, \
             aliases_json=excluded.aliases_json, headings_json=excluded.headings_json, \
             word_count=excluded.word_count, size=excluded.size, mtime_ns=excluded.mtime_ns, \
             hash=excluded.hash, reserved=excluded.reserved, \
             has_frontmatter=excluded.has_frontmatter, parse_error=excluded.parse_error, content_index_status=excluded.content_index_status",
        )
        .bind(vault)
        .bind(&n.path)
        .bind(&n.title)
        .bind(&n.okf_type)
        .bind(&n.description)
        .bind(&n.frontmatter_json)
        .bind(&n.tags_json)
        .bind(&n.aliases_json)
        .bind(&n.headings_json)
        .bind(n.word_count)
        .bind(n.size)
        .bind(n.mtime_ns)
        .bind(&n.hash)
        .bind(n.reserved as i64)
        .bind(n.has_frontmatter as i64)
        .bind(n.parse_error as i64)
        .bind(n.content_index_status.as_str())
        .execute(&mut *conn)
        .await
        .map_err(dberr("vault.upsert_note"))?;
        Ok(())
    }

    /// Publish one complete derived note atomically. No per-link auto-commits.
    pub(crate) async fn index_note(
        &self,
        vault: i64,
        note: &crate::prepare::PreparedNote,
        incoming: &[(i64, Option<String>)],
        fts: FtsIndexing,
    ) -> Result<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("vault.index.begin"))?;
        Self::index_note_conn(&mut tx, vault, note, incoming, fts).await?;
        tx.commit().await.map_err(dberr("vault.index.commit"))?;
        #[cfg(test)]
        self.index_commits
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    /// Index many scanned notes in ONE transaction (F7: a cold 10k-note scan
    /// commits ~50 times, not 10k). Scan-only: no incoming-link rewrites.
    pub(crate) async fn index_notes(
        &self,
        vault: i64,
        notes: &[crate::prepare::PreparedNote],
        fts: FtsIndexing,
    ) -> Result<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("vault.index.begin"))?;
        for note in notes {
            Self::index_note_conn(&mut tx, vault, note, &[], fts).await?;
        }
        tx.commit().await.map_err(dberr("vault.index.commit"))?;
        #[cfg(test)]
        self.index_commits
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    async fn index_note_conn(
        tx: &mut sqlx::SqliteConnection,
        vault: i64,
        note: &crate::prepare::PreparedNote,
        incoming: &[(i64, Option<String>)],
        fts: FtsIndexing,
    ) -> Result<()> {
        Self::upsert_note_conn(tx, vault, &note.row).await?;
        for sql in [
            "DELETE FROM vault_tags WHERE vault_id=? AND path=?",
            "DELETE FROM vault_links WHERE vault_id=? AND src_path=?",
        ] {
            sqlx::query(sql)
                .bind(vault)
                .bind(&note.row.path)
                .execute(&mut *tx)
                .await
                .map_err(dberr("vault.index.clear"))?;
        }
        let tags: Vec<String> = serde_json::from_str(&note.row.tags_json).unwrap_or_default();
        for tag in tags {
            sqlx::query("INSERT INTO vault_tags(vault_id,tag,path) VALUES(?,?,?)")
                .bind(vault)
                .bind(tag)
                .bind(&note.row.path)
                .execute(&mut *tx)
                .await
                .map_err(dberr("vault.index.tags"))?;
        }
        for (pos, link) in note.links.iter().enumerate() {
            sqlx::query("INSERT INTO vault_links(vault_id,src_path,raw_target,dst_path,kind,anchor,alias,pos) VALUES(?,?,?,?,?,?,?,?)").bind(vault).bind(&note.row.path).bind(&link.raw_target).bind(&link.dst_path).bind(&link.kind).bind(&link.anchor).bind(&link.alias).bind(pos as i64).execute(&mut *tx).await.map_err(dberr("vault.index.links"))?;
        }
        for (rowid, dst) in incoming {
            sqlx::query("UPDATE vault_links SET dst_path=? WHERE rowid=? AND vault_id=?")
                .bind(dst)
                .bind(rowid)
                .bind(vault)
                .execute(&mut *tx)
                .await
                .map_err(dberr("vault.index.incoming"))?;
        }
        if matches!(fts, FtsIndexing::Enabled) {
            fts_put_conn(tx, vault, &note.row.path, &note.row.title, &note.body)
                .await
                .map_err(dberr("vault.index.fts"))?;
        } else if matches!(fts, FtsIndexing::Retry) {
            // Persist the repair obligation across restart. A fallback-only
            // write must not look unchanged when full text becomes available.
            sqlx::query("UPDATE vault_notes SET mtime_ns=-1 WHERE vault_id=? AND path=?")
                .bind(vault)
                .bind(&note.row.path)
                .execute(&mut *tx)
                .await
                .map_err(dberr("vault.index.fts_pending"))?;
        }
        Ok(())
    }

    pub(crate) async fn update_link_destinations(
        &self,
        vault: i64,
        changed: &[(i64, Option<String>)],
    ) -> Result<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("vault.links.begin"))?;
        for (rowid, dst) in changed {
            sqlx::query("UPDATE vault_links SET dst_path=? WHERE rowid=? AND vault_id=?")
                .bind(dst)
                .bind(rowid)
                .bind(vault)
                .execute(&mut *tx)
                .await
                .map_err(dberr("vault.links.resolve"))?;
        }
        tx.commit().await.map_err(dberr("vault.links.commit"))?;
        Ok(())
    }

    /// Delete `paths`' notes with their links, tags and FTS rows in ONE
    /// transaction (a scan's pruned notes, batched).
    pub async fn remove_notes_batch(&self, vault: i64, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let json = serde_json::to_string(paths).map_err(|e| Error::Internal(e.to_string()))?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("vault.remove.begin"))?;
        for sql in [
            "DELETE FROM vault_notes WHERE vault_id = ? AND path IN (SELECT value FROM json_each(?))",
            "DELETE FROM vault_links WHERE vault_id = ? AND src_path IN (SELECT value FROM json_each(?))",
            "DELETE FROM vault_tags WHERE vault_id = ? AND path IN (SELECT value FROM json_each(?))",
        ] {
            sqlx::query(sql)
                .bind(vault)
                .bind(&json)
                .execute(&mut *tx)
                .await
                .map_err(dberr("vault.remove_note"))?;
        }
        let fts: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='vault_fts2_docs' AND type='table')",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(dberr("vault.remove.fts_check"))?;
        if fts {
            for path in paths {
                fts_del_conn(&mut tx, vault, path)
                    .await
                    .map_err(dberr("vault.remove.fts"))?;
            }
        }
        tx.commit().await.map_err(dberr("vault.remove.commit"))?;
        Ok(())
    }

    pub async fn upsert_file(
        &self,
        vault: i64,
        path: &str,
        size: i64,
        mtime_ns: i64,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO vault_files (vault_id, path, size, mtime_ns) VALUES (?,?,?,?) \
             ON CONFLICT(vault_id, path) DO UPDATE SET size=excluded.size, mtime_ns=excluded.mtime_ns",
        )
        .bind(vault)
        .bind(path)
        .bind(size)
        .bind(mtime_ns)
        .execute(&self.pool)
        .await
        .map_err(dberr("vault.upsert_file"))?;
        Ok(())
    }

    /// Upsert many `(path, size, mtime_ns)` file rows in ONE transaction (a
    /// scan's changed non-note files, batched instead of an autocommit each).
    pub async fn upsert_files_batch(&self, vault: i64, files: &[(String, i64, i64)]) -> Result<()> {
        if files.is_empty() {
            return Ok(());
        }
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("vault.upsert_files.begin"))?;
        for (path, size, mtime_ns) in files {
            sqlx::query(
                "INSERT INTO vault_files (vault_id, path, size, mtime_ns) VALUES (?,?,?,?) \
                 ON CONFLICT(vault_id, path) DO UPDATE SET size=excluded.size, mtime_ns=excluded.mtime_ns",
            )
            .bind(vault)
            .bind(path)
            .bind(size)
            .bind(mtime_ns)
            .execute(&mut *tx)
            .await
            .map_err(dberr("vault.upsert_file"))?;
        }
        tx.commit()
            .await
            .map_err(dberr("vault.upsert_files.commit"))?;
        Ok(())
    }

    /// Delete `paths`' file rows with one statement.
    pub async fn remove_files_batch(&self, vault: i64, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let json = serde_json::to_string(paths).map_err(|e| Error::Internal(e.to_string()))?;
        sqlx::query(
            "DELETE FROM vault_files WHERE vault_id = ? AND path IN (SELECT value FROM json_each(?))",
        )
        .bind(vault)
        .bind(&json)
        .execute(&self.pool)
        .await
        .map_err(dberr("vault.remove_file"))?;
        Ok(())
    }

    pub async fn note_meta(&self, vault: i64, path: &str) -> Result<NoteMeta> {
        let r = sqlx::query("SELECT * FROM vault_notes WHERE vault_id = ? AND path = ?")
            .bind(vault)
            .bind(path)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("vault.note_meta"))?
            .ok_or_else(|| Error::NotFound(format!("note {path}")))?;
        Ok(Self::meta_from_row(&r))
    }

    fn meta_from_row(r: &sqlx::sqlite::SqliteRow) -> NoteMeta {
        let parse = |s: String| serde_json::from_str(&s).unwrap_or(serde_json::Value::Null);
        let strs = |s: String| -> Vec<String> { serde_json::from_str(&s).unwrap_or_default() };
        NoteMeta {
            path: r.get("path"),
            title: r.get("title"),
            okf_type: r.get("okf_type"),
            description: r.get("description"),
            frontmatter: parse(r.get("frontmatter_json")),
            tags: strs(r.get("tags_json")),
            aliases: strs(r.get("aliases_json")),
            headings: serde_json::from_str(&r.get::<String, _>("headings_json"))
                .unwrap_or_default(),
            word_count: r.get("word_count"),
            size: r.get("size"),
            hash: r.get("hash"),
            reserved: r.get::<i64, _>("reserved") != 0,
            has_frontmatter: r.get::<i64, _>("has_frontmatter") != 0,
            parse_error: r.get::<i64, _>("parse_error") != 0,
            content_index_status: if r.get::<String, _>("content_index_status") == "size_limited" {
                ContentIndexStatus::SizeLimited
            } else {
                ContentIndexStatus::Full
            },
        }
    }

    // -- links / tags ---------------------------------------------------------

    pub async fn outgoing(&self, vault: i64, src: &str) -> Result<Vec<OutgoingLink>> {
        let rows = sqlx::query(
            "SELECT raw_target, dst_path, kind, anchor, alias FROM vault_links \
             WHERE vault_id = ? AND src_path = ? ORDER BY pos",
        )
        .bind(vault)
        .bind(src)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.outgoing"))?;
        Ok(rows
            .iter()
            .map(|r| OutgoingLink {
                raw_target: r.get("raw_target"),
                dst_path: r.get("dst_path"),
                kind: r.get("kind"),
                anchor: r.get("anchor"),
                alias: r.get("alias"),
            })
            .collect())
    }

    /// Sources that link TO `path` (backlinks), with the link kind.
    pub async fn backlinks(&self, vault: i64, path: &str) -> Result<Vec<(String, String, String)>> {
        let rows = sqlx::query(
            "SELECT DISTINCT l.src_path, n.title, l.kind FROM vault_links l \
             JOIN vault_notes n ON n.vault_id = l.vault_id AND n.path = l.src_path \
             WHERE l.vault_id = ? AND l.dst_path = ? ORDER BY l.src_path",
        )
        .bind(vault)
        .bind(path)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.backlinks"))?;
        Ok(rows
            .iter()
            .map(|r| (r.get("src_path"), r.get("title"), r.get("kind")))
            .collect())
    }

    /// One page of [`Self::backlinks`] plus each source note's indexed content
    /// hash (the backlink-context cache key) and the total row count. A hub
    /// note in a 10k-note vault has thousands of sources; only `limit` rows
    /// are read and sent (the count is one indexed aggregate).
    pub async fn backlinks_hashed_page(
        &self,
        vault: i64,
        path: &str,
        offset: usize,
        limit: usize,
    ) -> Result<(Vec<(String, String, String, String)>, u64)> {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM (SELECT DISTINCT l.src_path, l.kind FROM vault_links l \
             JOIN vault_notes n ON n.vault_id = l.vault_id AND n.path = l.src_path \
             WHERE l.vault_id = ? AND l.dst_path = ?)",
        )
        .bind(vault)
        .bind(path)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("vault.backlinks_count"))?;
        let rows = sqlx::query(
            "SELECT DISTINCT l.src_path, n.title, l.kind, n.hash FROM vault_links l \
             JOIN vault_notes n ON n.vault_id = l.vault_id AND n.path = l.src_path \
             WHERE l.vault_id = ? AND l.dst_path = ? ORDER BY l.src_path, l.kind \
             LIMIT ? OFFSET ?",
        )
        .bind(vault)
        .bind(path)
        .bind(i64::try_from(limit).unwrap_or(i64::MAX))
        .bind(i64::try_from(offset).unwrap_or(i64::MAX))
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.backlinks"))?;
        let rows = rows
            .iter()
            .map(|r| {
                (
                    r.get("src_path"),
                    r.get("title"),
                    r.get("kind"),
                    r.get("hash"),
                )
            })
            .collect();
        Ok((rows, total.max(0) as u64))
    }

    /// Every src_path that has at least one link whose dst is `path`.
    pub async fn linking_sources(&self, vault: i64, path: &str) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT DISTINCT src_path FROM vault_links WHERE vault_id = ? AND dst_path = ?",
        )
        .bind(vault)
        .bind(path)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.linking_sources"))?;
        Ok(rows.iter().map(|r| r.get("src_path")).collect())
    }

    /// Link rows whose lower-cased raw target contains `needle` (ASCII,
    /// lower-case) — the candidate set a newly created note can capture (F9).
    pub async fn links_mentioning(
        &self,
        vault: i64,
        needle: &str,
    ) -> Result<Vec<(i64, String, String, Option<String>)>> {
        let rows = sqlx::query(
            "SELECT rowid, src_path, raw_target, dst_path FROM vault_links \
             WHERE vault_id = ? AND instr(lower(raw_target), ?) > 0",
        )
        .bind(vault)
        .bind(needle)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.links_mentioning"))?;
        Ok(rows
            .iter()
            .map(|r| {
                (
                    r.get("rowid"),
                    r.get("src_path"),
                    r.get("raw_target"),
                    r.get("dst_path"),
                )
            })
            .collect())
    }

    /// Every link row `(rowid, src, raw, dst)` — the global re-resolve pass.
    pub async fn all_links_full(
        &self,
        vault: i64,
    ) -> Result<Vec<(i64, String, String, Option<String>)>> {
        #[cfg(test)]
        self.link_reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let rows = sqlx::query(
            "SELECT rowid, src_path, raw_target, dst_path FROM vault_links WHERE vault_id = ?",
        )
        .bind(vault)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.all_links_full"))?;
        Ok(rows
            .iter()
            .map(|r| {
                (
                    r.get("rowid"),
                    r.get("src_path"),
                    r.get("raw_target"),
                    r.get("dst_path"),
                )
            })
            .collect())
    }

    pub async fn tag_counts(&self, vault: i64) -> Result<Vec<TagCount>> {
        let rows = sqlx::query(
            "SELECT tag, COUNT(*) AS c FROM vault_tags WHERE vault_id = ? \
             GROUP BY tag ORDER BY c DESC, tag",
        )
        .bind(vault)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.tags"))?;
        Ok(rows
            .iter()
            .map(|r| TagCount {
                tag: r.get("tag"),
                count: r.get("c"),
            })
            .collect())
    }

    // -- listing / switcher / graph -------------------------------------------

    /// `(path, title, okf_type, reserved)` of every note (switcher + graph + tree).
    pub async fn all_notes(
        &self,
        vault: i64,
    ) -> Result<Vec<(String, String, Option<String>, bool)>> {
        #[cfg(test)]
        self.all_reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let rows = sqlx::query(
            "SELECT path, title, okf_type, reserved FROM vault_notes WHERE vault_id = ? ORDER BY path",
        )
        .bind(vault)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.all_notes"))?;
        Ok(rows
            .iter()
            .map(|r| {
                (
                    r.get("path"),
                    r.get("title"),
                    r.get("okf_type"),
                    r.get::<i64, _>("reserved") != 0,
                )
            })
            .collect())
    }

    pub async fn all_file_paths(&self, vault: i64) -> Result<Vec<String>> {
        #[cfg(test)]
        self.all_reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let rows = sqlx::query("SELECT path FROM vault_files WHERE vault_id = ?")
            .bind(vault)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("vault.all_files"))?;
        Ok(rows.iter().map(|r| r.get("path")).collect())
    }

    /// `(src, dst, kind)` of every RESOLVED link.
    pub async fn all_edges(&self, vault: i64) -> Result<Vec<(String, String, String)>> {
        let rows = sqlx::query(
            "SELECT src_path, dst_path, kind FROM vault_links \
             WHERE vault_id = ? AND dst_path IS NOT NULL",
        )
        .bind(vault)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.all_edges"))?;
        Ok(rows
            .iter()
            .map(|r| (r.get("src_path"), r.get("dst_path"), r.get("kind")))
            .collect())
    }

    /// Unresolved raw targets grouped: `(src, raw_target)`.
    pub async fn all_ghost_edges(&self, vault: i64) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query(
            "SELECT src_path, raw_target FROM vault_links \
             WHERE vault_id = ? AND dst_path IS NULL",
        )
        .bind(vault)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.ghost_edges"))?;
        Ok(rows
            .iter()
            .map(|r| (r.get("src_path"), r.get("raw_target")))
            .collect())
    }

    /// `(path → [tags])` for the graph's tag nodes.
    pub async fn all_note_tags(&self, vault: i64) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query("SELECT path, tag FROM vault_tags WHERE vault_id = ?")
            .bind(vault)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("vault.note_tags"))?;
        Ok(rows.iter().map(|r| (r.get("path"), r.get("tag"))).collect())
    }

    /// `path → (title, okf_type, reserved)` for the given paths; a path with
    /// no note row maps to `None` (a file or a stale target). Indexed lookups
    /// only, chunked under SQLite's bind-parameter limit.
    pub async fn notes_meta_for(
        &self,
        vault: i64,
        paths: &[String],
    ) -> Result<Vec<(String, Option<(String, Option<String>, bool)>)>> {
        let mut found: std::collections::HashMap<String, (String, Option<String>, bool)> =
            Default::default();
        for chunk in paths.chunks(IN_CHUNK) {
            let sql = format!(
                "SELECT path, title, okf_type, reserved FROM vault_notes \
                 WHERE vault_id = ? AND path IN ({})",
                placeholders(chunk.len())
            );
            let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.as_str())).bind(vault);
            for p in chunk {
                q = q.bind(p);
            }
            for r in q
                .fetch_all(&self.pool)
                .await
                .map_err(dberr("vault.notes_meta_for"))?
            {
                found.insert(
                    r.get("path"),
                    (
                        r.get("title"),
                        r.get("okf_type"),
                        r.get::<i64, _>("reserved") != 0,
                    ),
                );
            }
        }
        Ok(paths.iter().map(|p| (p.clone(), found.remove(p))).collect())
    }

    /// Resolved `(src, dst)` links touching any of `paths` in either
    /// direction — one local-graph BFS hop over `idx_vault_links_src`/`_dst`.
    pub async fn link_neighbors(
        &self,
        vault: i64,
        paths: &[String],
    ) -> Result<Vec<(String, String)>> {
        let mut out = Vec::new();
        for chunk in paths.chunks(IN_CHUNK) {
            let ph = placeholders(chunk.len());
            let sql = format!(
                "SELECT src_path, dst_path FROM vault_links \
                 WHERE vault_id = ? AND src_path IN ({ph}) AND dst_path IS NOT NULL \
                 UNION ALL \
                 SELECT src_path, dst_path FROM vault_links \
                 WHERE vault_id = ? AND dst_path IN ({ph})"
            );
            let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.as_str())).bind(vault);
            for p in chunk {
                q = q.bind(p);
            }
            q = q.bind(vault);
            for p in chunk {
                q = q.bind(p);
            }
            for r in q
                .fetch_all(&self.pool)
                .await
                .map_err(dberr("vault.link_neighbors"))?
            {
                out.push((r.get("src_path"), r.get("dst_path")));
            }
        }
        Ok(out)
    }

    /// `path → [tags]` for the given notes (indexed by `idx_vault_tags_path`).
    pub async fn note_tags_for(
        &self,
        vault: i64,
        paths: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<String>>> {
        let mut out: std::collections::HashMap<String, Vec<String>> = Default::default();
        for chunk in paths.chunks(IN_CHUNK) {
            let sql = format!(
                "SELECT path, tag FROM vault_tags WHERE vault_id = ? AND path IN ({})",
                placeholders(chunk.len())
            );
            let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.as_str())).bind(vault);
            for p in chunk {
                q = q.bind(p);
            }
            for r in q
                .fetch_all(&self.pool)
                .await
                .map_err(dberr("vault.note_tags_for"))?
            {
                out.entry(r.get("path")).or_default().push(r.get("tag"));
            }
        }
        Ok(out)
    }

    /// Search with the `tag:` / `path:` / `type:` filters applied IN SQL, before
    /// the LIMIT — a filtered match ranked below the first page is still found,
    /// and no whole-table read happens per search. Returns
    /// `(path, title, snippet, score, reserved)`.
    pub(crate) async fn search_filtered(
        &self,
        vault: i64,
        mode: SearchMode<'_>,
        f: &SearchFilters<'_>,
        limit: i64,
    ) -> Result<Vec<(String, String, String, f32, bool)>> {
        let mut filter_sql = String::new();
        if f.path_prefix.is_some() {
            filter_sql.push_str(" AND substr(n.path, 1, length(?)) = ?");
        }
        if f.okf_type.is_some() {
            filter_sql.push_str(" AND n.okf_type = ? COLLATE NOCASE");
        }
        if f.tag.is_some() {
            filter_sql.push_str(
                " AND EXISTS (SELECT 1 FROM vault_tags t WHERE t.vault_id = n.vault_id \
                 AND t.path = n.path AND (t.tag = ? OR substr(t.tag, 1, length(?) + 1) = ? || '/'))",
            );
        }
        let sql = match mode {
            // Contentless index: no stored body, so the snippet is computed
            // afterwards for this page only (`search_index::fill_snippets`).
            // CROSS JOIN pins the MATCH as the outer loop (never one MATCH
            // per doc row of the vault).
            SearchMode::Fts(_) => format!(
                "SELECT n.path AS path, n.title AS title, n.reserved AS reserved, \
                 '' AS snip, bm25(vault_fts2) AS rank FROM vault_fts2 \
                 CROSS JOIN vault_fts2_docs d ON d.rid = vault_fts2.rowid \
                 CROSS JOIN vault_notes n ON n.vault_id = ? AND n.path = d.path \
                 WHERE d.vault_id = ? AND vault_fts2 MATCH ?{filter_sql} \
                 ORDER BY rank LIMIT ?"
            ),
            SearchMode::Like(_) => format!(
                "SELECT n.path AS path, n.title AS title, n.reserved AS reserved, \
                 n.title AS snip, -0.1 AS rank FROM vault_notes n WHERE n.vault_id = ? AND \
                 (n.title LIKE ? ESCAPE '\\' OR n.path LIKE ? ESCAPE '\\' OR n.description LIKE ? ESCAPE '\\')\
                 {filter_sql} ORDER BY n.path LIMIT ?"
            ),
            SearchMode::FilterOnly => format!(
                "SELECT n.path AS path, n.title AS title, n.reserved AS reserved, \
                 n.title AS snip, 0.0 AS rank FROM vault_notes n WHERE n.vault_id = ?\
                 {filter_sql} ORDER BY n.path LIMIT ?"
            ),
        };
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.as_str())).bind(vault);
        match mode {
            SearchMode::Fts(expr) => q = q.bind(vault).bind(expr.to_string()),
            SearchMode::Like(needle) => {
                let pat = format!("%{}%", like_escape(needle));
                q = q.bind(pat.clone()).bind(pat.clone()).bind(pat);
            }
            SearchMode::FilterOnly => {}
        }
        if let Some(pp) = f.path_prefix {
            q = q.bind(pp.to_string()).bind(pp.to_string());
        }
        if let Some(ty) = f.okf_type {
            q = q.bind(ty.to_string());
        }
        if let Some(t) = f.tag {
            q = q
                .bind(t.to_string())
                .bind(t.to_string())
                .bind(t.to_string());
        }
        let rows = q
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("vault.search_filtered"))?;
        Ok(rows
            .iter()
            .map(|r| {
                let rank: f64 = r.get("rank");
                (
                    r.get("path"),
                    r.get("title"),
                    r.get("snip"),
                    -rank as f32,
                    r.get::<i64, _>("reserved") != 0,
                )
            })
            .collect())
    }

    // -- FTS -------------------------------------------------------------------

    /// Create the full-text index if the linked SQLite supports FTS5 (see
    /// `search_index.rs` for the layout): a CONTENTLESS `vault_fts2` (token
    /// index only — the v1 `vault_fts` stored a second full copy of every
    /// body, ~78 MB at 10k notes) plus `vault_fts2_docs`, the
    /// `(vault_id, path) ↔ rowid` map every write and search goes through.
    /// A v1 table still present means this database was indexed (or, after
    /// a rollback, last written) by an older build: v2 rows are untrusted,
    /// so both are dropped and the background backfill rebuilds v2 from the
    /// notes on disk.
    pub async fn ensure_fts(&self) -> bool {
        self.try_ensure_fts().await.unwrap_or(false)
    }

    /// Only an unavailable SQLite module is permanent; busy/I/O/schema
    /// initialization failures must be retried by the engine.
    pub(crate) async fn try_ensure_fts(&self) -> std::result::Result<bool, sqlx::Error> {
        #[cfg(test)]
        if self
            .fts_fail_once
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            return Err(sqlx::Error::PoolTimedOut);
        }
        let created = sqlx::query(
            "CREATE VIRTUAL TABLE IF NOT EXISTS vault_fts2 USING fts5(\
             title, body, content='', contentless_delete=1, \
             tokenize='unicode61 remove_diacritics 2')",
        )
        .execute(&self.pool)
        .await;
        if let Err(e) = created {
            if matches!(&e, sqlx::Error::Database(d) if d.message().contains("no such module: fts5"))
            {
                return Ok(false);
            }
            return Err(e);
        }
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS vault_fts2_docs (rid INTEGER PRIMARY KEY, \
             vault_id INTEGER NOT NULL, path TEXT NOT NULL, UNIQUE (vault_id, path))",
        )
        .execute(&self.pool)
        .await?;
        self.retire_legacy_fts().await?;
        Ok(true)
    }

    /// Drop the v1 table (and its rowid map) and empty v2 in one transaction.
    /// Nothing is migrated in place: the index is derived, and the backfill
    /// rebuilds it from the files. Rollback-safe: an older build recreates
    /// its own `vault_fts` and re-indexes every note it finds missing.
    async fn retire_legacy_fts(&self) -> std::result::Result<(), sqlx::Error> {
        let legacy: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='vault_fts' AND type='table')",
        )
        .fetch_one(&self.pool)
        .await?;
        if !legacy {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        let notes: i64 = sqlx::query_scalar("SELECT count(*) FROM vault_notes")
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO vault_fts2(vault_fts2) VALUES('delete-all')")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM vault_fts2_docs")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DROP TABLE vault_fts")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DROP TABLE IF EXISTS vault_fts_ids")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        tracing::info!(
            notes,
            "vault search index: retired the v1 table that stored every body twice; \
             the v2 index is rebuilt from the notes in the background"
        );
        Ok(())
    }

    pub async fn fts_index(&self, vault: i64, path: &str, title: &str, body: &str) {
        let Ok(mut tx) = self.pool.begin().await else {
            return;
        };
        if fts_put_conn(&mut tx, vault, path, title, body)
            .await
            .is_ok()
        {
            let _ = tx.commit().await;
        }
    }

    pub async fn fts_remove(&self, vault: i64, path: &str) {
        let Ok(mut tx) = self.pool.begin().await else {
            return;
        };
        if fts_del_conn(&mut tx, vault, path).await.is_ok() {
            let _ = tx.commit().await;
        }
    }

    /// Notes for OKF validation: everything the DB knows, one pass.
    pub async fn okf_rows(&self, vault: i64) -> Result<Vec<OkfNoteRow>> {
        let rows = sqlx::query(
            "SELECT path, okf_type, frontmatter_json, headings_json, reserved, \
             has_frontmatter, parse_error FROM vault_notes WHERE vault_id = ? ORDER BY path",
        )
        .bind(vault)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.okf_rows"))?;
        Ok(rows
            .iter()
            .map(|r| OkfNoteRow {
                path: r.get("path"),
                okf_type: r.get("okf_type"),
                frontmatter_json: r.get("frontmatter_json"),
                headings_json: r.get("headings_json"),
                reserved: r.get::<i64, _>("reserved") != 0,
                has_frontmatter: r.get::<i64, _>("has_frontmatter") != 0,
                parse_error: r.get::<i64, _>("parse_error") != 0,
            })
            .collect())
    }

    /// `(path, title, description)` for index.md generation, one directory.
    pub async fn dir_notes(
        &self,
        vault: i64,
        dir: &str,
    ) -> Result<Vec<(String, String, Option<String>)>> {
        let (pat, depth_from) = if dir.is_empty() {
            ("%".to_string(), 0usize)
        } else {
            (format!("{}/%", like_escape(dir)), dir.len() + 1)
        };
        let rows = sqlx::query(
            "SELECT path, title, description FROM vault_notes \
             WHERE vault_id = ? AND path LIKE ? ESCAPE '\\' AND reserved = 0 ORDER BY path",
        )
        .bind(vault)
        .bind(&pat)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.dir_notes"))?;
        Ok(rows
            .iter()
            .filter(|r| {
                let p: String = r.get("path");
                !p[depth_from..].contains('/')
            })
            .map(|r| (r.get("path"), r.get("title"), r.get("description")))
            .collect())
    }
}

pub struct OkfNoteRow {
    pub path: String,
    pub okf_type: Option<String>,
    pub frontmatter_json: String,
    pub headings_json: String,
    pub reserved: bool,
    pub has_frontmatter: bool,
    pub parse_error: bool,
}

/// Escape a LIKE pattern for `ESCAPE '\\'` — the one shared helper (backslash
/// first, then `%` and `_`), re-exported so callers here keep their path.
pub use otto_state::like_escape;

/// Replace one note's FTS row through the doc map (see
/// [`Store::ensure_fts`]): the map row keeps its rowid, the contentless row
/// is deleted and re-inserted under it. Never scans the index.
pub(crate) async fn fts_put_conn(
    conn: &mut sqlx::SqliteConnection,
    vault: i64,
    path: &str,
    title: &str,
    body: &str,
) -> std::result::Result<(), sqlx::Error> {
    let rid: Option<i64> =
        sqlx::query_scalar("SELECT rid FROM vault_fts2_docs WHERE vault_id = ? AND path = ?")
            .bind(vault)
            .bind(path)
            .fetch_optional(&mut *conn)
            .await?;
    let rid = match rid {
        Some(rid) => {
            sqlx::query("DELETE FROM vault_fts2 WHERE rowid = ?")
                .bind(rid)
                .execute(&mut *conn)
                .await?;
            rid
        }
        None => sqlx::query("INSERT INTO vault_fts2_docs (vault_id, path) VALUES (?, ?)")
            .bind(vault)
            .bind(path)
            .execute(&mut *conn)
            .await?
            .last_insert_rowid(),
    };
    sqlx::query("INSERT INTO vault_fts2 (rowid, title, body) VALUES (?, ?, ?)")
        .bind(rid)
        .bind(title)
        .bind(body)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Drop one note's FTS row by rowid (and its map row).
async fn fts_del_conn(
    conn: &mut sqlx::SqliteConnection,
    vault: i64,
    path: &str,
) -> std::result::Result<(), sqlx::Error> {
    let rid: Option<i64> =
        sqlx::query_scalar("SELECT rid FROM vault_fts2_docs WHERE vault_id = ? AND path = ?")
            .bind(vault)
            .bind(path)
            .fetch_optional(&mut *conn)
            .await?;
    if let Some(rid) = rid {
        sqlx::query("DELETE FROM vault_fts2 WHERE rowid = ?")
            .bind(rid)
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM vault_fts2_docs WHERE rid = ?")
            .bind(rid)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Bind parameters per `IN (…)` chunk (SQLite's default limit is 32 766; a
/// small chunk keeps each statement cheap to prepare).
const IN_CHUNK: usize = 400;

/// `?,?,…` for an `IN (…)` list. The dynamic SQL built in this file only ever
/// interpolates these placeholders and fixed clause text (audited for
/// `AssertSqlSafe`); every value is bound.
fn placeholders(n: usize) -> String {
    let mut s = String::with_capacity(n * 2);
    for i in 0..n {
        if i > 0 {
            s.push(',');
        }
        s.push('?');
    }
    s
}

/// How [`Store::search_filtered`] matches text.
#[derive(Clone, Copy, Debug)]
pub(crate) enum SearchMode<'a> {
    /// A sanitized FTS5 MATCH expression (bm25-ranked).
    Fts(&'a str),
    /// LIKE over title/path/description (no FTS5, or FTS found nothing).
    Like(&'a str),
    /// No text: only the filters select notes.
    FilterOnly,
}

/// `tag:` / `path:` / `type:` operators for [`Store::search_filtered`].
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SearchFilters<'a> {
    pub tag: Option<&'a str>,
    pub path_prefix: Option<&'a str>,
    pub okf_type: Option<&'a str>,
}

#[cfg(test)]
mod fts_map_tests {
    use super::*;

    /// backend-performance-4-02: the batch writers touch exactly the listed
    /// paths of exactly one vault (json_each binds, no per-row autocommit).
    #[tokio::test]
    async fn batched_file_upserts_and_removals() {
        let s = Store::new(otto_state::db::test_pool().await);
        let files: Vec<(String, i64, i64)> =
            (0..3).map(|i| (format!("f{i}.bin"), i, i * 10)).collect();
        s.upsert_files_batch(1, &files).await.unwrap();
        s.upsert_files_batch(2, &files[..1]).await.unwrap();
        s.upsert_files_batch(1, &[("f0.bin".into(), 7, 70)])
            .await
            .unwrap();
        let mut sigs = s.file_sigs(1).await.unwrap();
        sigs.sort();
        assert_eq!(
            sigs[0],
            ("f0.bin".to_string(), 7, 70),
            "conflict updates in place"
        );
        assert_eq!(sigs.len(), 3);
        s.remove_files_batch(1, &["f0.bin".into(), "f2.bin".into(), "it's\"odd".into()])
            .await
            .unwrap();
        assert_eq!(
            s.file_sigs(1).await.unwrap(),
            vec![("f1.bin".to_string(), 1, 10)]
        );
        assert_eq!(
            s.file_sigs(2).await.unwrap().len(),
            1,
            "other vault untouched"
        );
        s.remove_notes_batch(1, &[]).await.unwrap();
    }

    /// Test-only: the paths whose v2 index row matches `match_expr`.
    async fn fts_hits(s: &Store, vault: i64, match_expr: &str) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT d.path FROM vault_fts2 CROSS JOIN vault_fts2_docs d \
             ON d.rid = vault_fts2.rowid WHERE d.vault_id = ? AND vault_fts2 MATCH ? \
             ORDER BY d.path",
        )
        .bind(vault)
        .bind(match_expr)
        .fetch_all(s.pool())
        .await
        .unwrap()
    }

    async fn fts_rows(s: &Store, vault: i64) -> Vec<String> {
        sqlx::query_scalar("SELECT path FROM vault_fts2_docs WHERE vault_id = ? ORDER BY path")
            .bind(vault)
            .fetch_all(s.pool())
            .await
            .unwrap()
    }

    /// r3-01-04: a re-index replaces the note's row in place (no whole-index
    /// scan, no duplicate), removal deletes it, and the lookup is indexed.
    #[tokio::test]
    async fn replace_and_remove_go_through_the_rowid_map() {
        let s = Store::new(otto_state::db::test_pool().await);
        assert!(s.ensure_fts().await);
        s.fts_index(1, "a.md", "A", "alpha first").await;
        s.fts_index(1, "b.md", "B", "beta").await;
        s.fts_index(1, "a.md", "A", "alpha second").await;
        s.fts_index(2, "a.md", "A", "other vault").await;
        assert_eq!(fts_rows(&s, 1).await, vec!["a.md", "b.md"]);
        assert!(
            fts_hits(&s, 1, "\"first\"").await.is_empty(),
            "the old body is gone from the index"
        );
        assert_eq!(fts_hits(&s, 1, "\"second\"").await, vec!["a.md"]);
        s.fts_remove(1, "a.md").await;
        assert_eq!(fts_rows(&s, 1).await.len(), 1);
        assert!(fts_hits(&s, 1, "\"alpha\"").await.is_empty());
        assert_eq!(fts_rows(&s, 2).await.len(), 1, "other vault untouched");

        let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
            "EXPLAIN QUERY PLAN SELECT rid FROM vault_fts2_docs WHERE vault_id = 1 AND path = 'a.md'",
        )
        .fetch_all(s.pool())
        .await
        .unwrap();
        assert!(
            plan.iter()
                .any(|(.., d)| d.contains("PRIMARY KEY") || d.contains("INDEX")),
            "indexed lookup, got {plan:?}"
        );
    }

    /// The v2 table stores no body: the size win this layout exists for.
    #[tokio::test]
    async fn v2_index_keeps_no_copy_of_the_body() {
        let s = Store::new(otto_state::db::test_pool().await);
        assert!(s.ensure_fts().await);
        s.fts_index(1, "a.md", "A", "distinctivebodyword").await;
        let content_tables: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master WHERE name = 'vault_fts2_content'",
        )
        .fetch_one(s.pool())
        .await
        .unwrap();
        assert_eq!(content_tables, 0, "contentless: no %_content shadow table");
        let body: Option<String> =
            sqlx::query_scalar("SELECT body FROM vault_fts2 WHERE rowid = 1")
                .fetch_optional(s.pool())
                .await
                .unwrap()
                .flatten();
        assert_eq!(body, None, "a contentless row returns no stored text");
        assert_eq!(
            fts_hits(&s, 1, "\"distinctivebodyword\"").await,
            vec!["a.md"]
        );
    }

    /// A DB indexed by the v1 layout (a `vault_fts` holding every body): the
    /// first probe drops it and its map, empties v2 (it may be stale after a
    /// rollback), and is idempotent on the next start.
    #[tokio::test]
    async fn legacy_table_is_retired_and_v2_starts_empty() {
        let s = Store::new(otto_state::db::test_pool().await);
        assert!(s.ensure_fts().await);
        s.fts_index(7, "stale.md", "S", "written before a rollback")
            .await;
        for sql in [
            "CREATE VIRTUAL TABLE vault_fts USING fts5(\
             vault_id UNINDEXED, path UNINDEXED, title, body, tokenize='unicode61 remove_diacritics 2')",
            "CREATE TABLE vault_fts_ids (vault_id INTEGER NOT NULL, path TEXT NOT NULL, \
             rid INTEGER NOT NULL, PRIMARY KEY (vault_id, path))",
            "INSERT INTO vault_fts (vault_id, path, title, body) VALUES (7, 'n.md', 'N', 'old')",
        ] {
            sqlx::query(sqlx::AssertSqlSafe(sql)).execute(s.pool()).await.unwrap();
        }
        assert!(s.ensure_fts().await);
        let left: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master WHERE name IN ('vault_fts', 'vault_fts_ids')",
        )
        .fetch_one(s.pool())
        .await
        .unwrap();
        assert_eq!(left, 0, "v1 table and map dropped");
        assert!(
            fts_rows(&s, 7).await.is_empty(),
            "untrusted v2 rows cleared"
        );
        assert!(fts_hits(&s, 7, "\"rollback\"").await.is_empty());
        s.fts_index(7, "n.md", "N", "newer").await;
        // Idempotent on the next start: v2 rows survive.
        assert!(s.ensure_fts().await);
        assert_eq!(fts_rows(&s, 7).await, vec!["n.md"]);
        // Unregistering the vault drops its FTS rows through the map.
        s.delete_vault(7).await.unwrap();
        assert!(fts_rows(&s, 7).await.is_empty());
        assert!(fts_hits(&s, 7, "\"newer\"").await.is_empty());
    }
}
