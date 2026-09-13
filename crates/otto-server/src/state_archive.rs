//! Complete saved-state archives and additive, previewed restore. Database rows
//! remain in one transaction; filesystem publication exclusively creates files.
mod files;
mod schema;
use crate::{
    error::{ApiError, ApiResult},
    state::ServerCtx,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use otto_core::Error;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqliteConnection};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

pub const MAX_ARCHIVE_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
pub type ArchiveRow = BTreeMap<String, Value>;
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SnapshotOptions {
    #[serde(default)]
    pub portable: bool,
    #[serde(default = "yes")]
    pub include_files: bool,
}
impl Default for SnapshotOptions {
    fn default() -> Self {
        Self {
            portable: false,
            include_files: true,
        }
    }
}
fn yes() -> bool {
    true
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ArchiveRoot {
    pub id: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ArchiveFile {
    pub root: String,
    pub path: String,
    pub sha256: String,
    pub content_base64: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StateArchive {
    pub archive_format: u32,
    pub schema_version: i64,
    pub daemon_version: String,
    pub snapshot_at: String,
    pub records: BTreeMap<String, Vec<ArchiveRow>>,
    pub roots: Vec<ArchiveRoot>,
    pub files: Vec<ArchiveFile>,
    pub excluded: Vec<String>,
    pub reconnect: Vec<String>,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    #[default]
    Abort,
    SkipExisting,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RestoreOptions {
    pub conflicts: ConflictPolicy,
    pub preview_token: String,
    pub confirm: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RestoreConflict {
    pub kind: String,
    pub location: String,
    pub reason: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RestorePreview {
    pub preview_token: String,
    pub can_restore: bool,
    pub record_count: usize,
    pub file_count: usize,
    pub table_counts: BTreeMap<String, usize>,
    pub conflicts: Vec<RestoreConflict>,
    pub excluded: Vec<String>,
    pub reconnect: Vec<String>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RestoreResult {
    pub records_inserted: usize,
    pub records_skipped: usize,
    pub files_restored: usize,
    pub files_skipped: usize,
    pub restore_root: String,
    pub reconnect: Vec<String>,
}
fn invalid(message: &str) -> ApiError {
    ApiError(Error::Invalid(message.into()))
}
fn db_error(error: sqlx::Error) -> ApiError {
    ApiError(Error::Internal(format!("Archive database: {error}")))
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn archive_bytes(archive: &StateArchive) -> ApiResult<Vec<u8>> {
    let bytes = serde_json::to_vec(archive).map_err(|e| invalid(&e.to_string()))?;
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(invalid(
            "Archive exceeds 256 MiB; use a smaller portable selection",
        ));
    }
    Ok(bytes)
}
async fn version(conn: &mut SqliteConnection) -> ApiResult<i64> {
    sqlx::query_scalar("SELECT COALESCE(MAX(version),0) FROM _sqlx_migrations WHERE success=1")
        .fetch_one(conn)
        .await
        .map_err(db_error)
}
fn restore_id(archive: &StateArchive) -> ApiResult<String> {
    Ok(digest(&archive_bytes(archive)?)[..24].to_string())
}

pub async fn build_snapshot(ctx: &ServerCtx, options: SnapshotOptions) -> ApiResult<StateArchive> {
    build_snapshot_parts(&ctx.pool, &ctx.data_dir, &ctx.version, options).await
}

pub(crate) async fn build_snapshot_parts(
    pool: &sqlx::SqlitePool,
    data_dir: &std::path::Path,
    daemon_version: &str,
    options: SnapshotOptions,
) -> ApiResult<StateArchive> {
    let mut tx = pool.begin().await.map_err(db_error)?;
    let tables = schema::schema(&mut tx).await?;
    let mut excluded=vec![".otto-sync Git snapshot output directories".into(),"Authentication sessions, password hashes, Keychain contents, secret references and active permission bindings/allowlists".into(),"Live processes, sockets, PTY state, external repository working trees and database-engine data".into(),"Derived FTS/Vault indexes, provider catalogs and disposable caches".into(),"External provider transcript directories and workflow/agent worktrees (references retained)".into()];
    let mut reconnect = BTreeSet::new();
    let mut records = BTreeMap::new();
    let mut total = 0;
    for (name, spec) in &tables {
        if schema::excluded_table(name, options.portable) {
            excluded.push(format!("table {name}"));
            continue;
        }
        let mut rows = schema::read_rows(&mut tx, name).await?;
        if name == "settings" {
            rows.retain(|row| {
                !row.get("key")
                    .and_then(Value::as_str)
                    .is_some_and(schema::secret_key)
            });
        }
        for row in &mut rows {
            schema::sanitize(name, spec, row, &mut reconnect);
        }
        total += serde_json::to_vec(&rows)
            .map_err(|e| invalid(&e.to_string()))?
            .len();
        if total > MAX_ARCHIVE_BYTES {
            return Err(invalid(
                "Saved records exceed 256 MiB; no partial archive was exported",
            ));
        }
        records.insert(name.clone(), rows);
    }
    let schema_version = version(&mut tx).await?;
    tx.commit().await.map_err(db_error)?;
    let mut archive = StateArchive {
        archive_format: 2,
        schema_version,
        daemon_version: daemon_version.into(),
        snapshot_at: chrono::Utc::now().to_rfc3339(),
        records,
        roots: vec![],
        files: vec![],
        excluded,
        reconnect: reconnect.into_iter().collect(),
    };
    if options.include_files {
        let mut sources = Vec::new();
        for directory in files::DATA_ROOTS {
            if options.portable && !matches!(*directory, "library" | "canvas" | "product") {
                continue;
            }
            let path = data_dir.join(directory);
            if path.exists() {
                sources.push((
                    ArchiveRoot {
                        id: format!("data-{directory}"),
                        kind: "data".into(),
                        owner_id: Some((*directory).into()),
                    },
                    path,
                ));
            }
        }
        for row in archive.records.get("vaults").into_iter().flatten() {
            let id = row
                .get("id")
                .and_then(Value::as_i64)
                .ok_or_else(|| invalid("Invalid saved vault id"))?;
            let path = row
                .get("root_path")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("Invalid saved vault path"))?;
            // Missing registered roots are an explicit error, never a silently partial backup.
            sources.push((
                ArchiveRoot {
                    id: format!("vault-{id}"),
                    kind: "vault".into(),
                    owner_id: Some(id.to_string()),
                },
                PathBuf::from(path),
            ));
        }
        for (root, path) in sources {
            files::source_files(
                &root,
                &path,
                &mut archive.files,
                &mut archive.excluded,
                &mut total,
            )?;
            archive.roots.push(root);
        }
    } else {
        archive
            .excluded
            .push("File assets excluded by export selection".into());
    }
    if options.portable {
        archive
            .excluded
            .push("Portable selection excludes runtime history asset directories".into());
    }
    archive.excluded.sort();
    archive.excluded.dedup();
    archive_bytes(&archive)?;
    Ok(archive)
}

async fn validate_archive(
    conn: &mut SqliteConnection,
    archive: &StateArchive,
) -> ApiResult<BTreeMap<String, schema::TableSpec>> {
    archive_bytes(archive)?;
    if archive.archive_format != 2 {
        return Err(invalid("Expected archive_format 2"));
    }
    if archive.schema_version != version(conn).await? {
        return Err(invalid("Archive schema version differs from this daemon; restore using the matching Otto version"));
    }
    let tables = schema::schema(conn).await?;
    for (name, rows) in &archive.records {
        let spec = tables
            .get(name)
            .ok_or_else(|| invalid(&format!("Unknown archive table {name}")))?;
        if schema::excluded_table(name, false) {
            return Err(invalid(&format!(
                "Excluded runtime/credential table {name} cannot be imported"
            )));
        }
        if rows.iter().any(|row| {
            row.is_empty()
                || spec
                    .columns
                    .iter()
                    .filter(|c| c.pk > 0)
                    .any(|c| row.get(&c.name).is_none_or(Value::is_null))
                || row
                    .keys()
                    .any(|k| !spec.columns.iter().any(|c| &c.name == k))
        }) {
            return Err(invalid(&format!("Invalid columns for table {name}")));
        }
    }
    files::validate_files(archive)?;
    for root in &archive.roots {
        if root.kind == "vault"
            && !archive
                .records
                .get("vaults")
                .into_iter()
                .flatten()
                .any(|r| {
                    r.get("id").and_then(Value::as_i64).map(|id| id.to_string()) == root.owner_id
                })
        {
            return Err(invalid("Vault asset root has no owning saved vault record"));
        }
    }
    Ok(tables)
}
fn row_key(row: &ArchiveRow, spec: &schema::TableSpec) -> String {
    let mut keys: Vec<_> = spec.columns.iter().filter(|c| c.pk > 0).collect();
    keys.sort_by_key(|c| c.pk);
    if keys.is_empty() {
        digest(serde_json::to_string(row).unwrap_or_default().as_bytes())
    } else {
        keys.iter()
            .map(|c| format!("{}={}", c.name, row.get(&c.name).unwrap_or(&Value::Null)))
            .collect::<Vec<_>>()
            .join(",")
    }
}
fn remap_value(value: &mut Value, users: &BTreeMap<String, String>) {
    match value {
        Value::String(s) => {
            if let Some(id) = users.get(s) {
                *s = id.clone();
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|v| remap_value(v, users)),
        Value::Object(m) => m.values_mut().for_each(|v| remap_value(v, users)),
        _ => {}
    }
}

struct Applied {
    inserted: usize,
    skipped: usize,
    conflicts: Vec<RestoreConflict>,
    skipped_vaults: BTreeSet<String>,
    skipped_owners: BTreeSet<(String, String)>,
    target_signature: String,
    warnings: Vec<String>,
}
async fn apply_rows(
    conn: &mut SqliteConnection,
    data_dir: &std::path::Path,
    archive: &StateArchive,
    tables: &BTreeMap<String, schema::TableSpec>,
    restore_id: &str,
) -> ApiResult<Applied> {
    let mut signature = Sha256::new();
    let mut mapped_users = BTreeMap::new();
    let mut warnings = vec![];
    let users = schema::read_rows(conn, "users").await?;
    for imported in archive.records.get("users").into_iter().flatten() {
        if let Some(existing) = users
            .iter()
            .find(|u| u.get("username") == imported.get("username"))
        {
            if let (Some(from), Some(to)) = (
                imported.get("id").and_then(Value::as_str),
                existing.get("id").and_then(Value::as_str),
            ) {
                if from != to {
                    mapped_users.insert(from.into(), to.into());
                    warnings.push(format!("User {} maps to the existing local identity; existing credentials and permissions remain unchanged",imported.get("username").unwrap_or(&Value::Null)));
                }
            }
        }
    }
    signature.update(serde_json::to_vec(&mapped_users).map_err(|e| invalid(&e.to_string()))?);
    let mut result = Applied {
        inserted: 0,
        skipped: 0,
        conflicts: vec![],
        skipped_vaults: BTreeSet::new(),
        skipped_owners: BTreeSet::new(),
        target_signature: String::new(),
        warnings,
    };
    sqlx::query("PRAGMA defer_foreign_keys=ON")
        .execute(&mut *conn)
        .await
        .map_err(db_error)?;
    // FK constraints are deferred, so cycles and table ordering do not force a partial restore.
    for (table, rows) in &archive.records {
        let spec = &tables[table];
        for original in rows {
            let mut row = original.clone();
            if table == "users"
                && row
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| mapped_users.contains_key(id))
            {
                result.skipped += 1;
                continue;
            }
            if table == "settings"
                && row
                    .get("key")
                    .and_then(Value::as_str)
                    .is_some_and(schema::secret_key)
            {
                result.skipped += 1;
                continue;
            }
            let mut reconnect = BTreeSet::new();
            schema::sanitize(table, spec, &mut row, &mut reconnect);
            for (column, value) in &mut row {
                if spec.user_columns.contains(column)
                    || matches!(
                        column.as_str(),
                        "user_id"
                            | "created_by"
                            | "author_id"
                            | "actor_id"
                            | "actor_user_id"
                            | "effective_user_id"
                            | "real_author_id"
                            | "owner_id"
                            | "approved_by"
                            | "decided_by"
                            | "requested_by"
                            | "executor_id"
                            | "human_rater"
                            | "promoted_by"
                            | "waived_by"
                            | "started_by_id"
                            | "caller_user_id"
                            | "real_actor_id"
                            | "imported_by"
                    )
                {
                    remap_value(value, &mapped_users);
                }
                if let Value::String(text) = value {
                    if text.starts_with('/') {
                        for root in archive.roots.iter().filter(|r| r.kind == "data") {
                            if let Some(directory) = root.owner_id.as_deref() {
                                let marker = format!("/{directory}/");
                                if let Some((_, path)) = text.rsplit_once(&marker) {
                                    if archive
                                        .files
                                        .iter()
                                        .any(|f| f.root == root.id && f.path == path)
                                    {
                                        *text = data_dir
                                            .join(directory)
                                            .join(path)
                                            .to_string_lossy()
                                            .into_owned();
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                // Rewrite absolute references to the old managed root only when
                // an exported root supplies the actual file. External repos stay references.
                if column == "root_path" && table == "vaults" {
                    let id = original
                        .get("id")
                        .and_then(Value::as_i64)
                        .ok_or_else(|| invalid("Invalid vault id"))?;
                    let root = archive.roots.iter().find(|r| r.id == format!("vault-{id}"));
                    if let Some(root) = root {
                        *value = json!(data_dir
                            .join(files::root_relative(root, restore_id)?)
                            .to_string_lossy());
                    }
                }
            }
            let inserted = schema::insert_row(conn, table, spec, &row).await?;
            signature.update(table.as_bytes());
            signature.update(row_key(&row, spec).as_bytes());
            signature.update([u8::from(inserted)]);
            if inserted {
                result.inserted += 1;
            } else {
                if let Some(id) = original.get("id").and_then(Value::as_str) {
                    result.skipped_owners.insert((table.clone(), id.into()));
                }
                result.skipped += 1;
                result.conflicts.push(RestoreConflict {
                    kind: "record".into(),
                    location: format!("{table}/{}", row_key(&row, spec)),
                    reason: "Existing primary or unique key; existing record will be kept".into(),
                });
                if table == "vaults" {
                    if let Some(id) = original.get("id").and_then(Value::as_i64) {
                        result.skipped_vaults.insert(id.to_string());
                    }
                }
            }
        }
    }
    let broken = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(conn)
        .await
        .map_err(db_error)?;
    if !broken.is_empty() {
        let names: BTreeSet<String> = broken.iter().map(|r| r.get::<String, _>("table")).collect();
        return Err(invalid(&format!("Restore would leave missing references in: {}. Include the related saved records or restore into a compatible profile.",names.into_iter().collect::<Vec<_>>().join(", "))));
    }
    result.target_signature = format!("{:x}", signature.finalize());
    Ok(result)
}
type PlannedFiles = (Vec<(String, usize)>, Vec<RestoreConflict>, String);

fn plan_files(
    ctx: &ServerCtx,
    archive: &StateArchive,
    restore_id: &str,
    skipped_vaults: &BTreeSet<String>,
    skipped_owners: &BTreeSet<(String, String)>,
) -> ApiResult<PlannedFiles> {
    let mut pending = vec![];
    let mut conflicts = vec![];
    let mut signature = Sha256::new();
    for (index, file) in archive.files.iter().enumerate() {
        let root = archive
            .roots
            .iter()
            .find(|r| r.id == file.root)
            .ok_or_else(|| invalid("Unknown file root"))?;
        let relative = format!("{}/{}", files::root_relative(root, restore_id)?, file.path);
        let mut owner_skipped = root.kind == "vault"
            && root
                .owner_id
                .as_ref()
                .is_some_and(|id| skipped_vaults.contains(id));
        let parts: Vec<_> = file.path.split('/').collect();
        let owner = match root.id.as_str() {
            "data-canvas" => Some(("canvas_scenes", parts[0])),
            "data-transcripts" => Some(("sessions", parts[0])),
            "data-product" if parts.len() > 1 => match parts[0] {
                "attachments" => Some(("product_stories", parts[1])),
                "mockup_assist" => Some(("product_attachments", parts[1])),
                _ => None,
            },
            _ => None,
        };
        if let Some((table, id)) = owner {
            owner_skipped |= skipped_owners.contains(&(table.into(), id.into()));
        }
        let exists = files::existing(&ctx.data_dir, &relative)?;
        signature.update(relative.as_bytes());
        signature.update([u8::from(exists), u8::from(owner_skipped)]);
        if exists || owner_skipped {
            conflicts.push(RestoreConflict {
                kind: "file".into(),
                location: format!("{}/{}", file.root, file.path),
                reason: if owner_skipped {
                    "Owning saved record already exists; its assets will be kept"
                } else {
                    "Destination already exists; existing file will be kept"
                }
                .into(),
            });
        } else {
            pending.push((relative, index));
        }
    }
    Ok((pending, conflicts, format!("{:x}", signature.finalize())))
}
fn preview_token(
    archive: &StateArchive,
    policy: ConflictPolicy,
    rows: &str,
    files: &str,
) -> ApiResult<String> {
    let mut hash = Sha256::new();
    hash.update(archive_bytes(archive)?);
    hash.update(format!("{policy:?}:{rows}:{files}"));
    Ok(format!("{:x}", hash.finalize()))
}

pub async fn preview_restore(
    ctx: &ServerCtx,
    archive: &StateArchive,
    conflicts: ConflictPolicy,
) -> ApiResult<RestorePreview> {
    let mut tx = ctx.pool.begin().await.map_err(db_error)?;
    let tables = validate_archive(&mut tx, archive).await?;
    let restore_id = restore_id(archive)?;
    let applied = apply_rows(&mut tx, &ctx.data_dir, archive, &tables, &restore_id).await;
    let mut preview=RestorePreview{preview_token:String::new(),can_restore:false,record_count:archive.records.values().map(Vec::len).sum(),file_count:archive.files.len(),table_counts:archive.records.iter().map(|(t,r)|(t.clone(),r.len())).collect(),conflicts:vec![],excluded:archive.excluded.clone(),reconnect:archive.reconnect.clone(),warnings:vec!["Existing rows and files are never overwritten. Imported schedules/services stay disabled; process histories are not resumed.".into(),"External repository, connection and provider paths remain references; reconnect or remap them on this machine.".into(),"Archives contain private free-form documents and history. Structured credentials are excluded; review user-written content before sharing.".into(),"Database rows share one snapshot; files are read individually with change detection. Save edits before exporting for cross-file consistency.".into()]};
    match applied {
        Ok(mut rows) => {
            let (_, file_conflicts, file_signature) = plan_files(
                ctx,
                archive,
                &restore_id,
                &rows.skipped_vaults,
                &rows.skipped_owners,
            )?;
            preview.preview_token =
                preview_token(archive, conflicts, &rows.target_signature, &file_signature)?;
            rows.conflicts.extend(file_conflicts);
            preview.can_restore =
                conflicts == ConflictPolicy::SkipExisting || rows.conflicts.is_empty();
            preview.conflicts = rows.conflicts;
            preview.warnings.extend(rows.warnings);
        }
        Err(error) => preview.warnings.push(error.0.to_string()),
    }
    tx.rollback().await.map_err(db_error)?;
    Ok(preview)
}

pub async fn restore_snapshot(
    ctx: &ServerCtx,
    archive: &StateArchive,
    options: RestoreOptions,
) -> ApiResult<RestoreResult> {
    // Own the transaction/publication even if the initiating HTTP client disconnects.
    let ctx = ctx.clone();
    let archive = archive.clone();
    tokio::spawn(async move { restore_owned(&ctx, &archive, options).await })
        .await
        .map_err(|e| invalid(&format!("Restore worker: {e}")))?
}
async fn restore_owned(
    ctx: &ServerCtx,
    archive: &StateArchive,
    options: RestoreOptions,
) -> ApiResult<RestoreResult> {
    if !options.confirm || options.preview_token.is_empty() {
        return Err(invalid("Review a preview and confirm before restoring"));
    }
    let mut tx = ctx.pool.begin().await.map_err(db_error)?;
    let tables = validate_archive(&mut tx, archive).await?;
    let id = restore_id(archive)?;
    let rows = apply_rows(&mut tx, &ctx.data_dir, archive, &tables, &id).await?;
    let (pending, file_conflicts, file_signature) = plan_files(
        ctx,
        archive,
        &id,
        &rows.skipped_vaults,
        &rows.skipped_owners,
    )?;
    if preview_token(
        archive,
        options.conflicts,
        &rows.target_signature,
        &file_signature,
    )? != options.preview_token
    {
        return Err(ApiError(Error::Conflict(
            "Restore target changed after preview; preview again".into(),
        )));
    }
    if options.conflicts == ConflictPolicy::Abort
        && (!rows.conflicts.is_empty() || !file_conflicts.is_empty())
    {
        return Err(ApiError(Error::Conflict(
            "Restore has conflicts; choose skip existing or use another profile".into(),
        )));
    }
    // Decode and stage every file before any canonical asset is published.
    let stage = tempfile::Builder::new()
        .prefix("otto-restore-")
        .tempdir_in(&ctx.data_dir)
        .map_err(|e| invalid(&e.to_string()))?;
    for (_, index) in &pending {
        let bytes = STANDARD
            .decode(&archive.files[*index].content_base64)
            .map_err(|_| invalid("Invalid asset encoding"))?;
        std::fs::write(stage.path().join(index.to_string()), bytes)
            .map_err(|e| invalid(&e.to_string()))?;
    }
    let mut published: Vec<(String, String)> = vec![];
    let mut skipped = file_conflicts.len();
    for (relative, index) in &pending {
        let bytes = match std::fs::read(stage.path().join(index.to_string())) {
            Ok(bytes) => bytes,
            Err(error) => {
                rollback_files(ctx, &published);
                return Err(invalid(&error.to_string()));
            }
        };
        match files::publish(&ctx.data_dir, relative, &bytes) {
            Ok(true) => published.push((relative.clone(), archive.files[*index].sha256.clone())),
            Ok(false) if options.conflicts == ConflictPolicy::SkipExisting => skipped += 1,
            Ok(false) => {
                rollback_files(ctx, &published);
                return Err(ApiError(Error::Conflict(
                    "A destination appeared during restore; preview again".into(),
                )));
            }
            Err(error) => {
                rollback_files(ctx, &published);
                return Err(error);
            }
        }
    }
    if let Err(error) = tx.commit().await {
        rollback_files(ctx, &published);
        return Err(db_error(error));
    }
    // Rebuild derived Vault indexes from the relocated file roots. This never
    // mutates document bytes and makes restored notes immediately discoverable.
    for root in &archive.roots {
        if root.kind == "vault" {
            if let Some(id) = root.owner_id.as_deref().and_then(|s| s.parse::<i64>().ok()) {
                if !rows.skipped_vaults.contains(&id.to_string()) {
                    let _ = ctx.vault.scan(id).await;
                }
            }
        }
    }
    Ok(RestoreResult {
        records_inserted: rows.inserted,
        records_skipped: rows.skipped,
        files_restored: published.len(),
        files_skipped: skipped,
        restore_root: ctx
            .data_dir
            .join("restored")
            .join(id)
            .to_string_lossy()
            .into_owned(),
        reconnect: archive.reconnect.clone(),
    })
}
/// Remove only unchanged bytes this restore created; never remove modified or
/// replaced user files during rollback. Empty created directories may remain.
fn rollback_files(ctx: &ServerCtx, files: &[(String, String)]) {
    for (relative, hash) in files.iter().rev() {
        let _ = files::remove_matching(&ctx.data_dir, relative, hash);
    }
}

#[cfg(test)]
mod tests;
