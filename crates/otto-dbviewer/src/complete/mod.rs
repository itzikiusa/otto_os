//! Smart completion — the engine-agnostic core behind context-aware, index-first
//! autocomplete in the DB query editor.
//!
//! Two independent concerns live here, deliberately kept PURE (no network, no
//! driver) so they're exhaustively unit-tested:
//!
//! - **Context analysis** ([`sql`], [`mongo`]) — given the text around the cursor,
//!   decide *what kind of identifier is expected* (a table after `FROM`, a column
//!   after `WHERE`, a Mongo field key inside `find({…})`, …) and *which object(s)*
//!   are in scope (resolved from the `FROM`/`JOIN` list or the `db.<coll>` prefix).
//! - **Assembly** — turn a cached [`SchemaSnapshot`] plus that context into a
//!   ranked `Vec<CompletionItem>`, where index columns/fields out-rank plain ones
//!   ("indexes first, then the rest of the schema").
//!
//! The drivers own a [`CompletionCache`] (one snapshot per `(connection, database)`,
//! refresh-driven) and call into [`sql::assemble`] / [`mongo::assemble`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub mod mongo;
pub mod sql;

/// How long a cached snapshot survives without an explicit refresh.
///
/// The cache is primarily *refresh-driven*: the UI's "Refresh schema" action
/// clears it (see `service::refresh_completion_cache`). This TTL is only a
/// safety net so a snapshot self-heals even if a refresh is missed — matching
/// "cached for the connection until it is refreshed" while never serving
/// arbitrarily stale schema.
pub const COMPLETION_TTL: Duration = Duration::from_secs(300);

/// Index membership of a column / field — the basis for "indexes first".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rank {
    /// Primary key (SQL) / the `_id` index (Mongo).
    Pk,
    /// Member of a UNIQUE index.
    Unique,
    /// Member of a non-unique / secondary index (incl. ClickHouse skip indexes
    /// and the primary/sorting key, and Mongo compound-index members).
    Index,
    /// Not part of any index.
    Plain,
}

impl Rank {
    /// A short human label for the completion `detail` line.
    pub fn label(self) -> Option<&'static str> {
        match self {
            Rank::Pk => Some("PRIMARY KEY"),
            Rank::Unique => Some("UNIQUE"),
            Rank::Index => Some("INDEX"),
            Rank::Plain => None,
        }
    }
}

/// Whether a snapshot object is a relational table/view or a Mongo collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjKind {
    Table,
    View,
    Collection,
}

/// One column (SQL) or field path (Mongo). For Mongo, `name` may be dotted
/// (e.g. `address.city`) so embedded fields complete after a `.`.
#[derive(Debug, Clone)]
pub struct FieldSnap {
    pub name: String,
    pub ty: Option<String>,
    pub rank: Rank,
}

impl FieldSnap {
    pub fn new(name: impl Into<String>, ty: Option<String>, rank: Rank) -> Self {
        Self {
            name: name.into(),
            ty,
            rank,
        }
    }
}

/// A table/view (SQL) or collection (Mongo) with its (pre-ordered) fields.
#[derive(Debug, Clone)]
pub struct ObjectSnap {
    pub name: String,
    pub kind: ObjKind,
    /// Columns (SQL — always populated, ordered index-first) or, for Mongo, the
    /// sampled field paths (filled lazily; see [`ObjectSnap::fields_ready`]).
    pub fields: Vec<FieldSnap>,
    /// `true` once `fields` reflects a real introspection/sampling. SQL builds
    /// fields eagerly (always `true`); Mongo samples a collection's fields only
    /// when it's the one in context (cheap), so its objects start `false`.
    pub fields_ready: bool,
}

/// A stored procedure / function (SQL) — powers routine-name completion after
/// `SHOW CREATE PROCEDURE`/`FUNCTION`, `CALL`, and `DROP PROCEDURE`/`FUNCTION`.
#[derive(Debug, Clone)]
pub struct RoutineSnap {
    pub name: String,
    pub is_function: bool,
}

/// The cheap, cached shape of a database: its sibling databases + the list of
/// tables/collections (with SQL columns already attached) + its routines.
#[derive(Debug, Clone, Default)]
pub struct SchemaSnapshot {
    pub databases: Vec<String>,
    pub objects: Vec<ObjectSnap>,
    /// Stored procedures/functions in the scoped database (SQL only).
    pub routines: Vec<RoutineSnap>,
}

impl SchemaSnapshot {
    /// Case-insensitive object lookup (SQL identifiers fold case on most servers;
    /// Mongo collection names are case-sensitive but unique enough in practice).
    pub fn object(&self, name: &str) -> Option<&ObjectSnap> {
        self.objects
            .iter()
            .find(|o| o.name.eq_ignore_ascii_case(name))
    }
}

/// Relative ranking hints mapped to CodeMirror's `boost`. Higher sorts earlier
/// among equally-matching options. Tuned so: in-scope index columns lead, then
/// in-scope plain columns, then everything else; tables lead in a table slot.
pub mod score {
    pub const PK: i32 = 95;
    pub const UNIQUE: i32 = 85;
    pub const INDEX: i32 = 75;
    pub const PLAIN_COL: i32 = 55;
    pub const TABLE: i32 = 60;
    /// A column from a table NOT in the `FROM` scope — offered only as a weak
    /// fallback so a mistyped/unparsed scope still completes something.
    pub const OUT_OF_SCOPE_COL: i32 = 10;
    pub const KEYWORD: i32 = 0;
    pub const FUNCTION: i32 = -5;
    pub const DATABASE: i32 = -10;
    pub const DATABASE_IN_CTX: i32 = 60;

    // Mongo
    pub const MONGO_INDEX_FIELD: i32 = 95;
    pub const MONGO_FIELD: i32 = 40;
    pub const MONGO_COLLECTION: i32 = 60;
    pub const MONGO_METHOD: i32 = 55;
    pub const MONGO_OPERATOR: i32 = 0;
}

/// Relative strength of a rank (for "keep the strongest" when a column belongs
/// to several indexes): Pk > Unique > Index > Plain.
pub fn rank_strength(rank: Rank) -> u8 {
    match rank {
        Rank::Pk => 3,
        Rank::Unique => 2,
        Rank::Index => 1,
        Rank::Plain => 0,
    }
}

/// Score for a column/field by its index rank.
pub fn rank_score(rank: Rank) -> i32 {
    match rank {
        Rank::Pk => score::PK,
        Rank::Unique => score::UNIQUE,
        Rank::Index => score::INDEX,
        Rank::Plain => score::PLAIN_COL,
    }
}

// --- Per-connection cache ----------------------------------------------------

/// Key for a cached snapshot: the connection's `cache_key()` plus the scoped
/// database (empty string when the engine has no database level).
type SnapKey = (String, String);
/// Key for a lazily-sampled Mongo collection's fields.
type FieldKey = (String, String, String);

#[derive(Clone)]
struct Cached<T> {
    value: Arc<T>,
    built_at: Instant,
}

/// A driver-owned cache of completion snapshots, keyed by connection + database.
///
/// Shared across connections that share a `cache_key` (same endpoint+db+user) —
/// exactly the reuse the connection pool already assumes. Invalidated wholesale
/// for a connection by [`CompletionCache::invalidate`] (the refresh action).
#[derive(Default)]
pub struct CompletionCache {
    snapshots: Mutex<HashMap<SnapKey, Cached<SchemaSnapshot>>>,
    /// Mongo only: a collection's sampled field paths, cached independently of
    /// the (cheap) collection list so we sample only what's actually in context.
    fields: Mutex<HashMap<FieldKey, Cached<Vec<FieldSnap>>>>,
}

impl CompletionCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// A fresh-enough cached snapshot, or `None` (the caller builds + `put`s).
    pub fn get_snapshot(&self, cache_key: &str, db: &str) -> Option<Arc<SchemaSnapshot>> {
        let map = self.snapshots.lock().unwrap();
        map.get(&(cache_key.to_string(), db.to_string()))
            .filter(|c| c.built_at.elapsed() < COMPLETION_TTL)
            .map(|c| c.value.clone())
    }

    pub fn put_snapshot(
        &self,
        cache_key: &str,
        db: &str,
        snap: SchemaSnapshot,
    ) -> Arc<SchemaSnapshot> {
        let value = Arc::new(snap);
        self.snapshots.lock().unwrap().insert(
            (cache_key.to_string(), db.to_string()),
            Cached {
                value: value.clone(),
                built_at: Instant::now(),
            },
        );
        value
    }

    /// A collection's cached field paths (Mongo), or `None`.
    pub fn get_fields(
        &self,
        cache_key: &str,
        db: &str,
        object: &str,
    ) -> Option<Arc<Vec<FieldSnap>>> {
        let map = self.fields.lock().unwrap();
        map.get(&(cache_key.to_string(), db.to_string(), object.to_string()))
            .filter(|c| c.built_at.elapsed() < COMPLETION_TTL)
            .map(|c| c.value.clone())
    }

    pub fn put_fields(
        &self,
        cache_key: &str,
        db: &str,
        object: &str,
        fields: Vec<FieldSnap>,
    ) -> Arc<Vec<FieldSnap>> {
        let value = Arc::new(fields);
        self.fields.lock().unwrap().insert(
            (cache_key.to_string(), db.to_string(), object.to_string()),
            Cached {
                value: value.clone(),
                built_at: Instant::now(),
            },
        );
        value
    }

    /// Drop every cached entry for a connection (its `cache_key`). Called when
    /// the user refreshes the connection so the next completion re-introspects.
    pub fn invalidate(&self, cache_key: &str) {
        self.snapshots
            .lock()
            .unwrap()
            .retain(|(k, _), _| k != cache_key);
        self.fields
            .lock()
            .unwrap()
            .retain(|(k, _, _), _| k != cache_key);
    }

    /// Total cached snapshot entries — for the refresh endpoint's warm summary.
    pub fn snapshot_count(&self) -> usize {
        self.snapshots.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> SchemaSnapshot {
        SchemaSnapshot {
            databases: vec!["a".into()],
            objects: vec![ObjectSnap {
                name: "users".into(),
                kind: ObjKind::Table,
                fields: vec![FieldSnap::new("id", Some("int".into()), Rank::Pk)],
                fields_ready: true,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn cache_roundtrip_and_invalidate() {
        let c = CompletionCache::new();
        assert!(c.get_snapshot("ck", "db").is_none());
        c.put_snapshot("ck", "db", snap());
        assert!(c.get_snapshot("ck", "db").is_some());
        assert_eq!(c.snapshot_count(), 1);
        // A different db is a different entry.
        assert!(c.get_snapshot("ck", "other").is_none());
        c.invalidate("ck");
        assert!(c.get_snapshot("ck", "db").is_none());
        assert_eq!(c.snapshot_count(), 0);
    }

    #[test]
    fn invalidate_only_targets_one_connection() {
        let c = CompletionCache::new();
        c.put_snapshot("conn-a", "db", snap());
        c.put_snapshot("conn-b", "db", snap());
        c.put_fields("conn-a", "db", "users", vec![]);
        c.invalidate("conn-a");
        assert!(c.get_snapshot("conn-a", "db").is_none());
        assert!(c.get_fields("conn-a", "db", "users").is_none());
        assert!(c.get_snapshot("conn-b", "db").is_some(), "conn-b untouched");
    }

    #[test]
    fn case_insensitive_object_lookup() {
        let s = snap();
        assert!(s.object("USERS").is_some());
        assert!(s.object("users").is_some());
        assert!(s.object("nope").is_none());
    }

    #[test]
    fn rank_scores_are_index_first() {
        assert!(rank_score(Rank::Pk) > rank_score(Rank::Unique));
        assert!(rank_score(Rank::Unique) > rank_score(Rank::Index));
        assert!(rank_score(Rank::Index) > rank_score(Rank::Plain));
    }
}
