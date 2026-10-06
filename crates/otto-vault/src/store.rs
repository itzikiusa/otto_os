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
        let dup = sqlx::query("SELECT 1 FROM vaults WHERE root_path = ?")
            .bind(root)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("vault.create.dup"))?;
        if dup.is_some() {
            return Err(Error::Conflict(format!(
                "vault already registered at {root}"
            )));
        }
        let r = sqlx::query(
            "INSERT INTO vaults (ws_id, name, root_path, okf, created_at) VALUES (?,?,?,?,?)",
        )
        .bind(ws)
        .bind(name)
        .bind(root)
        .bind(okf as i64)
        .bind(now())
        .execute(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(ref d) if d.message().contains("UNIQUE") => {
                Error::Conflict(format!("vault already registered at {root}"))
            }
            other => Error::Internal(format!("vault.create: {other}")),
        })?;
        Ok(r.last_insert_rowid())
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
        // By rowid through the map (the UNINDEXED `vault_id` would scan the
        // whole index); a DB whose map was never built falls back to the scan.
        let by_rowid = sqlx::query(
            "DELETE FROM vault_fts WHERE rowid IN (SELECT rid FROM vault_fts_ids WHERE vault_id = ?)",
        )
        .bind(id)
        .execute(&self.pool)
        .await;
        if by_rowid.is_ok() {
            let _ = sqlx::query("DELETE FROM vault_fts_ids WHERE vault_id = ?")
                .bind(id)
                .execute(&self.pool)
                .await;
        } else {
            let _ = sqlx::query("DELETE FROM vault_fts WHERE vault_id = ?")
                .bind(id)
                .execute(&self.pool)
                .await;
        }
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

    pub async fn upsert_note(&self, vault: i64, n: &NoteRow) -> Result<()> {
        let mut conn = self
            .pool
            .acquire()
            .await
            .map_err(dberr("vault.note.acquire"))?;
        Self::upsert_note_conn(&mut conn, vault, n).await
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
        fts: bool,
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
        fts: bool,
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
        fts: bool,
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
        if fts {
            fts_put_conn(tx, vault, &note.row.path, &note.row.title, &note.body)
                .await
                .map_err(dberr("vault.index.fts"))?;
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

    pub async fn remove_note(&self, vault: i64, path: &str) -> Result<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("vault.remove.begin"))?;
        for sql in [
            "DELETE FROM vault_notes WHERE vault_id = ? AND path = ?",
            "DELETE FROM vault_links WHERE vault_id = ? AND src_path = ?",
            "DELETE FROM vault_tags WHERE vault_id = ? AND path = ?",
        ] {
            sqlx::query(sql)
                .bind(vault)
                .bind(path)
                .execute(&mut *tx)
                .await
                .map_err(dberr("vault.remove_note"))?;
        }
        let (fts, map): (bool, bool) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='vault_fts' AND type='table'), \
                    EXISTS(SELECT 1 FROM sqlite_master WHERE name='vault_fts_ids' AND type='table')",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(dberr("vault.remove.fts_check"))?;
        if fts && map {
            fts_del_conn(&mut tx, vault, path)
                .await
                .map_err(dberr("vault.remove.fts"))?;
        } else if fts {
            // Map not built yet (FTS never ensured by this daemon): the old scan.
            sqlx::query("DELETE FROM vault_fts WHERE vault_id=? AND path=?")
                .bind(vault)
                .bind(path)
                .execute(&mut *tx)
                .await
                .map_err(dberr("vault.remove.fts"))?;
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

    pub async fn remove_file(&self, vault: i64, path: &str) -> Result<()> {
        sqlx::query("DELETE FROM vault_files WHERE vault_id = ? AND path = ?")
            .bind(vault)
            .bind(path)
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

    pub async fn replace_links(&self, vault: i64, src: &str, links: &[OutgoingLink]) -> Result<()> {
        sqlx::query("DELETE FROM vault_links WHERE vault_id = ? AND src_path = ?")
            .bind(vault)
            .bind(src)
            .execute(&self.pool)
            .await
            .map_err(dberr("vault.replace_links"))?;
        for (i, l) in links.iter().enumerate() {
            sqlx::query(
                "INSERT INTO vault_links (vault_id, src_path, raw_target, dst_path, kind, anchor, alias, pos) \
                 VALUES (?,?,?,?,?,?,?,?)",
            )
            .bind(vault)
            .bind(src)
            .bind(&l.raw_target)
            .bind(&l.dst_path)
            .bind(&l.kind)
            .bind(&l.anchor)
            .bind(&l.alias)
            .bind(i as i64)
            .execute(&self.pool)
            .await
            .map_err(dberr("vault.replace_links"))?;
        }
        Ok(())
    }

    pub async fn replace_tags(&self, vault: i64, path: &str, tags: &[String]) -> Result<()> {
        sqlx::query("DELETE FROM vault_tags WHERE vault_id = ? AND path = ?")
            .bind(vault)
            .bind(path)
            .execute(&self.pool)
            .await
            .map_err(dberr("vault.replace_tags"))?;
        for t in tags {
            sqlx::query("INSERT INTO vault_tags (vault_id, tag, path) VALUES (?,?,?)")
                .bind(vault)
                .bind(t)
                .bind(path)
                .execute(&self.pool)
                .await
                .map_err(dberr("vault.replace_tags"))?;
        }
        Ok(())
    }

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

    /// [`Self::backlinks`] plus each source note's indexed content hash (the
    /// backlink-context cache key).
    pub async fn backlinks_hashed(
        &self,
        vault: i64,
        path: &str,
    ) -> Result<Vec<(String, String, String, String)>> {
        let rows = sqlx::query(
            "SELECT DISTINCT l.src_path, n.title, l.kind, n.hash FROM vault_links l \
             JOIN vault_notes n ON n.vault_id = l.vault_id AND n.path = l.src_path \
             WHERE l.vault_id = ? AND l.dst_path = ? ORDER BY l.src_path, l.kind",
        )
        .bind(vault)
        .bind(path)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.backlinks"))?;
        Ok(rows
            .iter()
            .map(|r| {
                (
                    r.get("src_path"),
                    r.get("title"),
                    r.get("kind"),
                    r.get("hash"),
                )
            })
            .collect())
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

    /// Paths of notes with at least one UNRESOLVED link (rename may have made
    /// a previously-broken target valid, or vice versa).
    pub async fn unresolved_sources(&self, vault: i64) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT DISTINCT src_path FROM vault_links WHERE vault_id = ? AND dst_path IS NULL",
        )
        .bind(vault)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.unresolved_sources"))?;
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

    pub async fn update_link_dst(&self, rowid: i64, dst: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE vault_links SET dst_path = ? WHERE rowid = ?")
            .bind(dst)
            .bind(rowid)
            .execute(&self.pool)
            .await
            .map_err(dberr("vault.update_link_dst"))?;
        Ok(())
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

    /// `(path, aliases_json)` for notes that have aliases.
    pub async fn all_aliases(&self, vault: i64) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query(
            "SELECT path, aliases_json FROM vault_notes WHERE vault_id = ? AND aliases_json != '[]'",
        )
        .bind(vault)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.aliases"))?;
        Ok(rows
            .iter()
            .map(|r| (r.get("path"), r.get("aliases_json")))
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
            SearchMode::Fts(_) => format!(
                "SELECT n.path AS path, n.title AS title, n.reserved AS reserved, \
                 snippet(vault_fts, 3, '\u{2039}', '\u{203a}', '…', 14) AS snip, \
                 bm25(vault_fts) AS rank FROM vault_fts \
                 JOIN vault_notes n ON n.vault_id = ? AND n.path = vault_fts.path \
                 WHERE vault_fts.vault_id = ? AND vault_fts MATCH ?{filter_sql} \
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
                let pat = format!("%{}%", needle.replace('%', "\\%").replace('_', "\\_"));
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

    /// Create the FTS5 index if the linked SQLite supports it, plus the
    /// `(vault_id, path) → rowid` map every write goes through.
    ///
    /// Why the map (perf r3-01-04): `vault_id`/`path` are UNINDEXED FTS columns,
    /// so `DELETE … WHERE vault_id=? AND path=?` scanned the whole index —
    /// ~20 ms per changed note at 10k notes, inside the write transaction
    /// (a full rebuild ≈ 200 s of write lock). Through the map a replace is an
    /// indexed lookup plus a rowid update. The map is derived data like the FTS
    /// table itself; a DB indexed before it existed gets it built here once
    /// (one scan), and duplicate FTS rows for one note are dropped.
    pub async fn ensure_fts(&self) -> bool {
        let created = sqlx::query(
            "CREATE VIRTUAL TABLE IF NOT EXISTS vault_fts USING fts5(\
             vault_id UNINDEXED, path UNINDEXED, title, body, tokenize='unicode61 remove_diacritics 2')",
        )
        .execute(&self.pool)
        .await
        .is_ok();
        if !created {
            return false;
        }
        self.ensure_fts_map().await.is_ok()
    }

    async fn ensure_fts_map(&self) -> std::result::Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='vault_fts_ids' AND type='table')",
        )
        .fetch_one(&mut *tx)
        .await?;
        if !exists {
            sqlx::query(
                "CREATE TABLE vault_fts_ids (vault_id INTEGER NOT NULL, path TEXT NOT NULL, \
                 rid INTEGER NOT NULL, PRIMARY KEY (vault_id, path))",
            )
            .execute(&mut *tx)
            .await?;
            // Later rows win (the newest index of a note), then orphans go.
            sqlx::query(
                "INSERT OR REPLACE INTO vault_fts_ids (vault_id, path, rid) \
                 SELECT vault_id, path, rowid FROM vault_fts ORDER BY rowid",
            )
            .execute(&mut *tx)
            .await?;
            sqlx::query("DELETE FROM vault_fts WHERE rowid NOT IN (SELECT rid FROM vault_fts_ids)")
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await
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

    /// bm25-ranked FTS search → `(path, snippet, score)`. `match_expr` must be a
    /// sanitized FTS5 MATCH expression.
    pub async fn fts_search(
        &self,
        vault: i64,
        match_expr: &str,
        limit: i64,
    ) -> Result<Vec<(String, String, f32)>> {
        let rows = sqlx::query(
            "SELECT path, snippet(vault_fts, 3, '\u{2039}', '\u{203a}', '…', 14) AS snip, \
             bm25(vault_fts) AS rank FROM vault_fts \
             WHERE vault_id = ? AND vault_fts MATCH ? ORDER BY rank LIMIT ?",
        )
        .bind(vault)
        .bind(match_expr)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.fts_search"))?;
        Ok(rows
            .iter()
            .map(|r| {
                let rank: f64 = r.get("rank");
                (r.get("path"), r.get("snip"), -rank as f32)
            })
            .collect())
    }

    /// LIKE fallback when FTS5 is unavailable or the query has no FTS tokens.
    pub async fn like_search(
        &self,
        vault: i64,
        needle: &str,
        limit: i64,
    ) -> Result<Vec<(String, String, f32)>> {
        let pat = format!("%{}%", needle.replace('%', "\\%").replace('_', "\\_"));
        let rows = sqlx::query(
            "SELECT path, title FROM vault_notes WHERE vault_id = ? AND \
             (title LIKE ? ESCAPE '\\' OR path LIKE ? ESCAPE '\\' OR description LIKE ? ESCAPE '\\') \
             ORDER BY path LIMIT ?",
        )
        .bind(vault)
        .bind(&pat)
        .bind(&pat)
        .bind(&pat)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.like_search"))?;
        Ok(rows
            .iter()
            .map(|r| (r.get("path"), r.get::<String, _>("title"), 0.1f32))
            .collect())
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

pub fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Replace one note's FTS row through the rowid map (see
/// [`Store::ensure_fts`]): an UPDATE by rowid when the note is indexed,
/// else an INSERT + map row. Never scans the index.
async fn fts_put_conn(
    conn: &mut sqlx::SqliteConnection,
    vault: i64,
    path: &str,
    title: &str,
    body: &str,
) -> std::result::Result<(), sqlx::Error> {
    let rid: Option<i64> =
        sqlx::query_scalar("SELECT rid FROM vault_fts_ids WHERE vault_id = ? AND path = ?")
            .bind(vault)
            .bind(path)
            .fetch_optional(&mut *conn)
            .await?;
    if let Some(rid) = rid {
        let done = sqlx::query("UPDATE vault_fts SET title = ?, body = ? WHERE rowid = ?")
            .bind(title)
            .bind(body)
            .bind(rid)
            .execute(&mut *conn)
            .await?;
        if done.rows_affected() > 0 {
            return Ok(());
        }
    }
    let inserted =
        sqlx::query("INSERT INTO vault_fts (vault_id, path, title, body) VALUES (?,?,?,?)")
            .bind(vault)
            .bind(path)
            .bind(title)
            .bind(body)
            .execute(&mut *conn)
            .await?;
    sqlx::query("INSERT OR REPLACE INTO vault_fts_ids (vault_id, path, rid) VALUES (?,?,?)")
        .bind(vault)
        .bind(path)
        .bind(inserted.last_insert_rowid())
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
        sqlx::query_scalar("SELECT rid FROM vault_fts_ids WHERE vault_id = ? AND path = ?")
            .bind(vault)
            .bind(path)
            .fetch_optional(&mut *conn)
            .await?;
    if let Some(rid) = rid {
        sqlx::query("DELETE FROM vault_fts WHERE rowid = ?")
            .bind(rid)
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM vault_fts_ids WHERE vault_id = ? AND path = ?")
            .bind(vault)
            .bind(path)
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

    async fn fts_rows(s: &Store, vault: i64) -> Vec<(String, String)> {
        sqlx::query_as("SELECT path, body FROM vault_fts WHERE vault_id = ? ORDER BY path")
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
        assert_eq!(
            fts_rows(&s, 1).await,
            vec![
                ("a.md".to_string(), "alpha second".to_string()),
                ("b.md".to_string(), "beta".to_string())
            ]
        );
        let hits = s.fts_search(1, "\"first\"", 10).await.unwrap();
        assert!(hits.is_empty(), "the old body is gone from the index");
        s.fts_remove(1, "a.md").await;
        assert_eq!(fts_rows(&s, 1).await.len(), 1);
        assert_eq!(fts_rows(&s, 2).await.len(), 1, "other vault untouched");

        let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
            "EXPLAIN QUERY PLAN SELECT rid FROM vault_fts_ids WHERE vault_id = 1 AND path = 'a.md'",
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

    /// A DB indexed before the map existed: `ensure_fts` builds it once and
    /// drops duplicate rows for one note (the newest wins).
    #[tokio::test]
    async fn legacy_index_gets_its_map_built_and_deduplicated() {
        let s = Store::new(otto_state::db::test_pool().await);
        sqlx::query(
            "CREATE VIRTUAL TABLE vault_fts USING fts5(\
             vault_id UNINDEXED, path UNINDEXED, title, body, tokenize='unicode61 remove_diacritics 2')",
        )
        .execute(s.pool())
        .await
        .unwrap();
        for body in ["old", "new"] {
            sqlx::query(
                "INSERT INTO vault_fts (vault_id, path, title, body) VALUES (7, 'n.md', 'N', ?)",
            )
            .bind(body)
            .execute(s.pool())
            .await
            .unwrap();
        }
        assert!(s.ensure_fts().await);
        assert_eq!(
            fts_rows(&s, 7).await,
            vec![("n.md".to_string(), "new".to_string())]
        );
        s.fts_index(7, "n.md", "N", "newer").await;
        assert_eq!(
            fts_rows(&s, 7).await,
            vec![("n.md".to_string(), "newer".to_string())]
        );
        // Idempotent on the next start.
        assert!(s.ensure_fts().await);
        assert_eq!(fts_rows(&s, 7).await.len(), 1);
        // Unregistering the vault drops its FTS rows through the map.
        s.delete_vault(7).await.unwrap();
        assert!(fts_rows(&s, 7).await.is_empty());
    }
}
