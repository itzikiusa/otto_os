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

/// How long a FAILED snapshot build (unreachable server, no
/// `information_schema` access, a timeout) is remembered as an empty snapshot.
/// Without it every completion request — one per word typed — re-ran the pool
/// acquire + bulk introspection (or waited out a connect timeout) against a sick
/// connection. Short, so a recovered server completes again within seconds;
/// "Refresh schema" ([`CompletionCache::invalidate`]) clears it immediately.
pub const COMPLETION_NEGATIVE_TTL: Duration = Duration::from_secs(20);

/// Most completion items one response carries. A 5k-table × 50-column schema
/// otherwise ships ~250k items (tens of MB of JSON) per word typed, and the
/// editor re-sorts all of them. Past the cap the best-scored items win and the
/// response says `truncated`, so the editor re-asks on the next keystroke
/// instead of filtering a clipped list (see `finalize`).
pub const MAX_COMPLETION_ITEMS: usize = 1500;

/// The identifier being typed at the end of `prefix` — the segment after the
/// last `.` (a qualifier is not part of what the editor matches) — lowercased.
/// Empty right after a `.`, a space or an operator.
pub fn typed_word(prefix: &str) -> String {
    let tail_start = prefix
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '_' || *c == '$')
        .last()
        .map(|(i, _)| i)
        .unwrap_or(prefix.len());
    prefix[tail_start..].to_lowercase()
}

/// `true` when every char of `word` (already lowercased) appears in `label` in
/// order, case-insensitively. This is a SUPERSET of what CodeMirror's fuzzy
/// matcher shows for `word` or any longer word the user goes on to type, so a
/// server-side filter by it never hides an option the editor would have shown.
pub fn fuzzy_contains(label: &str, word: &str) -> bool {
    if word.is_empty() {
        return true;
    }
    let mut want = word.chars();
    let mut next = want.next();
    for c in label.chars().flat_map(char::to_lowercase) {
        match next {
            Some(w) if w == c => next = want.next(),
            Some(_) => {}
            None => break,
        }
    }
    next.is_none()
}

/// Bound a completion response for the word being typed at the end of `prefix`:
/// drop items that can't match it (see [`fuzzy_contains`]), then keep at most
/// [`MAX_COMPLETION_ITEMS`] — highest score first, a prefix match before a
/// scattered one, shorter labels first — and flag `truncated` when anything was
/// cut. Applied centrally by the service to every engine's answer.
pub fn finalize(resp: &mut crate::types::CompletionResponse, prefix: &str) {
    let word = typed_word(prefix);
    if !word.is_empty() {
        resp.items.retain(|it| fuzzy_contains(&it.label, &word));
    }
    if resp.items.len() <= MAX_COMPLETION_ITEMS {
        return;
    }
    let starts = |label: &str| {
        !word.is_empty()
            && label
                .get(..word.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(&word))
    };
    resp.items.sort_by(|a, b| {
        b.score
            .unwrap_or(0)
            .cmp(&a.score.unwrap_or(0))
            .then_with(|| starts(&b.label).cmp(&starts(&a.label)))
            .then_with(|| a.label.len().cmp(&b.label.len()))
    });
    resp.items.truncate(MAX_COMPLETION_ITEMS);
    resp.truncated = true;
}

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

/// A completion snapshot from an access-scoped [`crate::types::SchemaGraph`] —
/// what the enforced path may see. Primary-key columns lead each object (the
/// assembler's fields are index-first); FK / other index membership isn't in the
/// graph, so the rest rank as plain columns. `databases` is just the one
/// authorized schema.
pub fn snapshot_from_graph(graph: &crate::types::SchemaGraph) -> SchemaSnapshot {
    use crate::types::NodeKind;
    let objects = graph
        .tables
        .iter()
        .map(|t| {
            let (mut pk, mut rest): (Vec<FieldSnap>, Vec<FieldSnap>) = (Vec::new(), Vec::new());
            for c in &t.columns {
                let ty = (!c.data_type.is_empty()).then(|| c.data_type.clone());
                if c.primary_key {
                    pk.push(FieldSnap::new(c.name.clone(), ty, Rank::Pk));
                } else {
                    rest.push(FieldSnap::new(c.name.clone(), ty, Rank::Plain));
                }
            }
            pk.append(&mut rest);
            ObjectSnap {
                name: t.name.clone(),
                kind: match t.kind {
                    NodeKind::View => ObjKind::View,
                    NodeKind::Collection => ObjKind::Collection,
                    _ => ObjKind::Table,
                },
                fields: pk,
                fields_ready: true,
            }
        })
        .collect();
    SchemaSnapshot {
        databases: vec![graph.schema.clone()],
        objects,
        routines: Vec::new(),
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
    /// [`COMPLETION_TTL`], or [`COMPLETION_NEGATIVE_TTL`] for a failed build.
    ttl: Duration,
}

impl<T> Cached<T> {
    fn new(value: Arc<T>, ttl: Duration) -> Self {
        Self {
            value,
            built_at: Instant::now(),
            ttl,
        }
    }

    fn fresh(&self) -> bool {
        self.built_at.elapsed() < self.ttl
    }
}

/// A driver-owned cache of completion snapshots, keyed by connection + database.
///
/// Shared across connections that share a `cache_key` (same endpoint+db+user) —
/// exactly the reuse the connection pool already assumes. Invalidated wholesale
/// for a connection by [`CompletionCache::invalidate`] (the refresh action).
pub struct CompletionCache {
    /// Lifetime of a successful snapshot ([`COMPLETION_TTL`] by default).
    ttl: Duration,
    /// Optional snapshot/field-map entry ceiling for the enforced service cache.
    capacity: Option<usize>,
    snapshots: Mutex<HashMap<SnapKey, Cached<SchemaSnapshot>>>,
    /// Mongo only: a collection's sampled field paths, cached independently of
    /// the (cheap) collection list so we sample only what's actually in context.
    fields: Mutex<HashMap<FieldKey, Cached<Vec<FieldSnap>>>>,
    /// Single-flight gates for snapshot builds, one per key while a build runs:
    /// concurrent misses (several words typed before the first introspection
    /// lands, or the TTL expiring mid-typing) wait for the ONE build instead of
    /// each running the same remote introspection. Removed when the build ends.
    building: Mutex<HashMap<SnapKey, Arc<tokio::sync::Mutex<()>>>>,
    /// Single-flight gates for Mongo field builds, keyed like `fields` — a cold
    /// collection typed against from several keystrokes samples ONCE.
    fields_building: Mutex<HashMap<FieldKey, Arc<tokio::sync::Mutex<()>>>>,
    /// Mongo only: the first user database per connection, used to seed
    /// collection completion before a database is chosen (`None` = the server
    /// was unreachable — negatively cached like a failed snapshot).
    first_db: Mutex<HashMap<String, Cached<Option<String>>>>,
}

impl Default for CompletionCache {
    fn default() -> Self {
        Self::with_ttl(COMPLETION_TTL)
    }
}

impl CompletionCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// A cache whose successful snapshots live `ttl` instead of
    /// [`COMPLETION_TTL`] — the access-enforced path keeps them short so a
    /// narrowed grant stops completing hidden names quickly.
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            ttl,
            capacity: None,
            snapshots: Mutex::default(),
            fields: Mutex::default(),
            building: Mutex::default(),
            fields_building: Mutex::default(),
            first_db: Mutex::default(),
        }
    }

    pub(crate) fn with_ttl_and_capacity(ttl: Duration, capacity: usize) -> Self {
        let mut cache = Self::with_ttl(ttl);
        cache.capacity = Some(capacity.max(1));
        cache
    }

    /// A fresh-enough cached snapshot, or `None` (the caller builds + `put`s).
    pub fn get_snapshot(&self, cache_key: &str, db: &str) -> Option<Arc<SchemaSnapshot>> {
        let map = self.snapshots.lock().unwrap();
        map.get(&(cache_key.to_string(), db.to_string()))
            .filter(|c| c.fresh())
            .map(|c| c.value.clone())
    }

    /// The cached snapshot for `(cache_key, db)`, else the result of `build` —
    /// run by at most ONE caller per key at a time (the others wait, then read
    /// what it cached). A `None` build is remembered as an empty snapshot for
    /// [`COMPLETION_NEGATIVE_TTL`] so a failing connection isn't re-introspected
    /// on every completion request.
    pub async fn snapshot_or_build<F, Fut>(
        &self,
        cache_key: &str,
        db: &str,
        build: F,
    ) -> Arc<SchemaSnapshot>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Option<SchemaSnapshot>>,
    {
        if let Some(s) = self.get_snapshot(cache_key, db) {
            return s;
        }
        let key = (cache_key.to_string(), db.to_string());
        let gate = self
            .building
            .lock()
            .unwrap()
            .entry(key.clone())
            .or_default()
            .clone();
        let guard = gate.lock().await;
        // Another caller may have finished the build while we waited.
        if let Some(s) = self.get_snapshot(cache_key, db) {
            return s;
        }
        let out = match build().await {
            Some(snap) => self.put_snapshot(cache_key, db, snap),
            None => self.put_snapshot_ttl(
                cache_key,
                db,
                SchemaSnapshot::default(),
                COMPLETION_NEGATIVE_TTL,
            ),
        };
        drop(guard);
        // Waiters hold their own clone of the gate; later callers hit the cache.
        self.building.lock().unwrap().remove(&key);
        out
    }

    pub fn put_snapshot(
        &self,
        cache_key: &str,
        db: &str,
        snap: SchemaSnapshot,
    ) -> Arc<SchemaSnapshot> {
        self.put_snapshot_ttl(cache_key, db, snap, self.ttl)
    }

    fn put_snapshot_ttl(
        &self,
        cache_key: &str,
        db: &str,
        snap: SchemaSnapshot,
        ttl: Duration,
    ) -> Arc<SchemaSnapshot> {
        let value = Arc::new(snap);
        let key = (cache_key.to_string(), db.to_string());
        let mut snapshots = self.snapshots.lock().unwrap();
        prune_for_insert(&mut snapshots, &key, self.capacity);
        snapshots.insert(key, Cached::new(value.clone(), ttl));
        value
    }

    /// Mongo: the cached first-user-database pick for a connection. The outer
    /// `None` is a miss; `Some(None)` is a remembered failure.
    pub fn get_first_db(&self, cache_key: &str) -> Option<Option<String>> {
        let map = self.first_db.lock().unwrap();
        map.get(cache_key)
            .filter(|c| c.fresh())
            .map(|c| (*c.value).clone())
    }

    pub fn put_first_db(&self, cache_key: &str, db: Option<String>) {
        let ttl = if db.is_some() {
            COMPLETION_TTL
        } else {
            COMPLETION_NEGATIVE_TTL
        };
        self.first_db
            .lock()
            .unwrap()
            .insert(cache_key.to_string(), Cached::new(Arc::new(db), ttl));
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
            .filter(|c| c.fresh())
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
        let key = (cache_key.to_string(), db.to_string(), object.to_string());
        let mut fields = self.fields.lock().unwrap();
        prune_for_insert(&mut fields, &key, self.capacity);
        fields.insert(key, Cached::new(value.clone(), self.ttl));
        value
    }

    /// A collection's cached field paths, else the result of `build` — run by at
    /// most ONE caller per `(cache_key, db, object)` at a time (the others wait,
    /// then read what it cached). Mirrors [`Self::snapshot_or_build`]: an empty
    /// build (unreachable server, no privileges) is remembered for
    /// [`COMPLETION_NEGATIVE_TTL`] so each keystroke doesn't re-sample.
    pub async fn fields_or_build<F, Fut>(
        &self,
        cache_key: &str,
        db: &str,
        object: &str,
        build: F,
    ) -> Arc<Vec<FieldSnap>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Vec<FieldSnap>>,
    {
        if let Some(f) = self.get_fields(cache_key, db, object) {
            return f;
        }
        let key = (cache_key.to_string(), db.to_string(), object.to_string());
        let gate = self
            .fields_building
            .lock()
            .unwrap()
            .entry(key.clone())
            .or_default()
            .clone();
        let guard = gate.lock().await;
        if let Some(f) = self.get_fields(cache_key, db, object) {
            return f;
        }
        let fields = build().await;
        let ttl = if fields.is_empty() {
            COMPLETION_NEGATIVE_TTL
        } else {
            self.ttl
        };
        let value = Arc::new(fields);
        {
            let mut fields = self.fields.lock().unwrap();
            prune_for_insert(&mut fields, &key, self.capacity);
            fields.insert(key.clone(), Cached::new(value.clone(), ttl));
        }
        drop(guard);
        self.fields_building.lock().unwrap().remove(&key);
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
        self.first_db.lock().unwrap().remove(cache_key);
    }

    /// Drop every EXPIRED entry (snapshots, Mongo field lists, first-db
    /// probes) — a `get` only filters them, it never removes them, so a
    /// connection typed against once kept its snapshot resident until the same
    /// key was rebuilt. Called from the service's 5-minute reaper (DB2-07).
    /// Returns how many entries were dropped.
    pub fn sweep(&self) -> usize {
        let mut dropped = 0;
        {
            let mut m = self.snapshots.lock().unwrap();
            let before = m.len();
            m.retain(|_, c| c.fresh());
            dropped += before - m.len();
        }
        {
            let mut m = self.fields.lock().unwrap();
            let before = m.len();
            m.retain(|_, c| c.fresh());
            dropped += before - m.len();
        }
        {
            let mut m = self.first_db.lock().unwrap();
            let before = m.len();
            m.retain(|_, c| c.fresh());
            dropped += before - m.len();
        }
        dropped
    }

    /// Total cached snapshot entries — for the refresh endpoint's warm summary.
    pub fn snapshot_count(&self) -> usize {
        self.snapshots.lock().unwrap().len()
    }
}

fn prune_for_insert<K: Eq + std::hash::Hash + Clone, V>(
    entries: &mut HashMap<K, Cached<V>>,
    key: &K,
    capacity: Option<usize>,
) {
    let Some(capacity) = capacity else { return };
    entries.retain(|_, value| value.fresh());
    if !entries.contains_key(key) {
        while entries.len() >= capacity {
            let oldest = entries
                .iter()
                .min_by_key(|(_, value)| value.built_at)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                entries.remove(&oldest);
            } else {
                break;
            }
        }
    }
}

/// A small TTL cache whose misses are built by at most ONE caller per key (the
/// others wait on the key's gate, then read what it stored). Drivers use it for
/// expensive per-object reads several surfaces share — e.g. Mongo's collection
/// sample, which the tree, the structure tab and field completion all derive
/// from. The builder picks each entry's lifetime, so a failure can be cached
/// briefly (negative TTL) and a success longer.
pub struct SingleFlight<K, V> {
    entries: Mutex<HashMap<K, Cached<V>>>,
    gates: Mutex<HashMap<K, Arc<tokio::sync::Mutex<()>>>>,
}

impl<K, V> Default for SingleFlight<K, V> {
    fn default() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            gates: Mutex::new(HashMap::new()),
        }
    }
}

impl<K: std::hash::Hash + Eq + Clone, V> SingleFlight<K, V> {
    /// A fresh cached value, or `None`.
    pub fn get(&self, key: &K) -> Option<Arc<V>> {
        let map = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        map.get(key).filter(|c| c.fresh()).map(|c| c.value.clone())
    }

    /// The cached value for `key`, else `build()`'s — single-flight. `build`
    /// returns the value and how long to keep it.
    pub async fn get_or_build<F, Fut>(&self, key: K, build: F) -> Arc<V>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = (V, Duration)>,
    {
        if let Some(v) = self.get(&key) {
            return v;
        }
        let gate = self
            .gates
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(key.clone())
            .or_default()
            .clone();
        let guard = gate.lock().await;
        if let Some(v) = self.get(&key) {
            return v;
        }
        let (value, ttl) = build().await;
        let value = Arc::new(value);
        {
            let mut map = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            // Sweep expired entries on insert so a long session browsing many
            // collections doesn't keep every stale sample resident.
            map.retain(|_, c| c.fresh());
            map.insert(key.clone(), Cached::new(value.clone(), ttl));
        }
        drop(guard);
        self.gates
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&key);
        value
    }

    /// Drop every entry whose key matches `pred` (connection refresh / close).
    /// Drop every expired entry (see [`CompletionCache::sweep`]).
    pub fn sweep(&self) -> usize {
        let mut map = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let before = map.len();
        map.retain(|_, c| c.fresh());
        before - map.len()
    }

    pub fn invalidate_where(&self, pred: impl Fn(&K) -> bool) {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|k, _| !pred(k));
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
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

    #[tokio::test]
    async fn bounded_cache_evicts_oldest_without_crossing_field_scopes() {
        let c = CompletionCache::with_ttl_and_capacity(Duration::from_secs(60), 2);
        c.put_snapshot("conn", "old", snap());
        c.snapshots
            .lock()
            .unwrap()
            .get_mut(&("conn".into(), "old".into()))
            .unwrap()
            .built_at = Instant::now() - Duration::from_secs(1);
        c.put_snapshot("conn", "new", snap());
        c.put_snapshot("conn", "newest", snap());
        assert_eq!(c.snapshot_count(), 2);
        assert!(c.get_snapshot("conn", "old").is_none());
        assert!(c.get_snapshot("conn", "newest").is_some());

        let field = || vec![FieldSnap::new("visible", None, Rank::Plain)];
        c.put_fields("conn", "user-a/policy-1", "profiles", field());
        assert!(c
            .get_fields("conn", "user-b/policy-1", "profiles")
            .is_none());
        assert!(c
            .get_fields("conn", "user-a/policy-2", "profiles")
            .is_none());
        c.fields
            .lock()
            .unwrap()
            .get_mut(&("conn".into(), "user-a/policy-1".into(), "profiles".into()))
            .unwrap()
            .built_at = Instant::now() - Duration::from_secs(1);
        c.fields_or_build("conn", "user-b/policy-1", "profiles", || async { field() })
            .await;
        c.fields_or_build("conn", "user-a/policy-2", "profiles", || async { field() })
            .await;
        assert_eq!(c.fields.lock().unwrap().len(), 2);
        assert!(c
            .get_fields("conn", "user-a/policy-1", "profiles")
            .is_none());
        assert!(c
            .get_fields("conn", "user-a/policy-2", "profiles")
            .is_some());
        c.invalidate("conn");
        assert_eq!(c.snapshot_count(), 0);
        assert!(c.fields.lock().unwrap().is_empty());
    }

    /// DB2-07: the reaper's sweep drops expired entries of every map (a `get`
    /// only filters them) and keeps fresh ones.
    #[test]
    fn sweep_drops_only_expired_entries() {
        let expired = CompletionCache::with_ttl(Duration::ZERO);
        expired.put_snapshot("ck", "db", snap());
        expired.put_snapshot("ck", "db2", snap());
        assert_eq!(
            expired.snapshot_count(),
            2,
            "get filters, the map keeps them"
        );
        assert_eq!(expired.sweep(), 2);
        assert_eq!(expired.snapshot_count(), 0);

        let live = CompletionCache::new();
        live.put_snapshot("ck", "db", snap());
        live.put_fields("ck", "db", "users", vec![]);
        live.put_first_db("ck", Some("db".into()));
        assert_eq!(live.sweep(), 0);
        assert_eq!(live.snapshot_count(), 1);
        assert!(live.get_fields("ck", "db", "users").is_some());
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

    #[tokio::test]
    async fn failed_build_is_negatively_cached_until_invalidated() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let c = CompletionCache::new();
        let builds = AtomicUsize::new(0);
        for _ in 0..3 {
            let s = c
                .snapshot_or_build("ck", "db", || async {
                    builds.fetch_add(1, Ordering::SeqCst);
                    None
                })
                .await;
            assert!(s.objects.is_empty());
        }
        assert_eq!(builds.load(Ordering::SeqCst), 1, "a failure is remembered");
        c.invalidate("ck");
        let s = c
            .snapshot_or_build("ck", "db", || async {
                builds.fetch_add(1, Ordering::SeqCst);
                Some(snap())
            })
            .await;
        assert_eq!(
            builds.load(Ordering::SeqCst),
            2,
            "refresh clears the negative entry"
        );
        assert_eq!(s.objects.len(), 1);
    }

    #[tokio::test]
    async fn concurrent_misses_share_one_build() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let c = Arc::new(CompletionCache::new());
        let builds = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let (c, builds) = (c.clone(), builds.clone());
            tasks.push(tokio::spawn(async move {
                c.snapshot_or_build("ck", "db", || async move {
                    builds.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    Some(snap())
                })
                .await
                .objects
                .len()
            }));
        }
        for t in tasks {
            assert_eq!(t.await.unwrap(), 1);
        }
        assert_eq!(builds.load(Ordering::SeqCst), 1, "single-flight");
    }

    #[test]
    fn first_db_cache_roundtrip_and_invalidate() {
        let c = CompletionCache::new();
        assert_eq!(c.get_first_db("ck"), None);
        c.put_first_db("ck", Some("app".into()));
        assert_eq!(c.get_first_db("ck"), Some(Some("app".into())));
        c.put_first_db("other", None);
        assert_eq!(c.get_first_db("other"), Some(None), "failure remembered");
        c.invalidate("ck");
        assert_eq!(c.get_first_db("ck"), None);
    }

    #[test]
    fn case_insensitive_object_lookup() {
        let s = snap();
        assert!(s.object("USERS").is_some());
        assert!(s.object("users").is_some());
        assert!(s.object("nope").is_none());
    }

    #[test]
    fn typed_word_is_the_segment_after_the_qualifier() {
        assert_eq!(typed_word("SELECT * FROM us"), "us");
        assert_eq!(typed_word("SELECT u.Na"), "na");
        assert_eq!(typed_word("SELECT u."), "");
        assert_eq!(typed_word("WHERE a = "), "");
        assert_eq!(typed_word("db.orders.fi"), "fi");
        assert_eq!(typed_word("$ma"), "$ma");
        assert_eq!(typed_word(""), "");
    }

    #[test]
    fn fuzzy_contains_is_an_in_order_case_insensitive_subsequence() {
        assert!(fuzzy_contains("user_id", "uid"));
        assert!(fuzzy_contains("USER_ID", "id"));
        assert!(fuzzy_contains("anything", ""));
        assert!(!fuzzy_contains("user_id", "diu"), "order matters");
        assert!(!fuzzy_contains("orders", "ordersx"));
    }

    fn items(n: usize, score: i32, stem: &str) -> Vec<crate::types::CompletionItem> {
        (0..n)
            .map(|i| {
                crate::types::CompletionItem::new(
                    format!("{stem}{i}"),
                    crate::types::CompletionKind::Column,
                )
                .scored(score)
            })
            .collect()
    }

    #[test]
    fn finalize_filters_by_the_typed_word_without_truncating_small_lists() {
        let mut resp = crate::types::CompletionResponse {
            items: [items(3, 55, "user_"), items(3, 55, "order_")].concat(),
            ..Default::default()
        };
        finalize(&mut resp, "SELECT * FROM t WHERE usr");
        assert_eq!(resp.items.len(), 3, "only labels that can match `usr` stay");
        assert!(resp.items.iter().all(|i| i.label.starts_with("user_")));
        assert!(!resp.truncated);
    }

    #[test]
    fn finalize_caps_huge_lists_keeping_the_best_scored_and_flags_truncated() {
        // 5k tables × 50 cols worth of out-of-scope columns + a few PK columns.
        let mut all = items(250_000, score::OUT_OF_SCOPE_COL, "col_");
        all.extend(items(5, score::PK, "id_"));
        let mut resp = crate::types::CompletionResponse {
            items: all,
            ..Default::default()
        };
        finalize(&mut resp, "SELECT * FROM t WHERE ");
        assert_eq!(resp.items.len(), MAX_COMPLETION_ITEMS);
        assert!(resp.truncated, "a clipped list must make the editor re-ask");
        assert!(
            resp.items[..5].iter().all(|i| i.score == Some(score::PK)),
            "the strongest items survive the cap, first"
        );
    }

    #[test]
    fn finalize_prefers_prefix_matches_when_capping() {
        let mut all = items(MAX_COMPLETION_ITEMS + 10, 55, "xa_");
        all.push(
            crate::types::CompletionItem::new("amount", crate::types::CompletionKind::Column)
                .scored(55),
        );
        let mut resp = crate::types::CompletionResponse {
            items: all,
            ..Default::default()
        };
        finalize(&mut resp, "WHERE a");
        assert!(resp.truncated);
        assert_eq!(resp.items[0].label, "amount");
    }

    #[test]
    fn graph_snapshot_ranks_pk_first_for_the_enforced_path() {
        use crate::types::{GraphColumn, GraphTable, NodeKind, SchemaGraph};
        let col = |name: &str, pk: bool| GraphColumn {
            name: name.into(),
            data_type: "int".into(),
            nullable: !pk,
            primary_key: pk,
            foreign_key: false,
        };
        let graph = SchemaGraph {
            schema: "shop".into(),
            tables: vec![GraphTable {
                id: "db:shop/table:customers".into(),
                schema: "shop".into(),
                name: "customers".into(),
                kind: NodeKind::Table,
                columns: vec![col("country", false), col("id", true), col("name", false)],
            }],
            edges: Vec::new(),
            relationships: true,
            truncated: false,
        };
        let snap = snapshot_from_graph(&graph);
        assert_eq!(snap.databases, vec!["shop".to_string()]);
        let names: Vec<&str> = snap.objects[0]
            .fields
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(names, ["id", "country", "name"]);

        // Context-aware like the driver path: in a WHERE over `customers`, the
        // PK column out-ranks the plain ones, and no out-of-scope noise leads.
        let ctx = sql::analyze("SELECT * FROM customers WHERE ", "");
        let items = sql::assemble(&ctx, &snap, &["SELECT"], &[]);
        let best = items
            .iter()
            .filter(|i| i.kind == crate::types::CompletionKind::Column)
            .max_by_key(|i| i.score.unwrap_or(0))
            .unwrap();
        assert_eq!(best.label, "id");
        assert_eq!(best.score, Some(score::PK));
        // In a table slot the table itself is offered, not its columns.
        let ctx = sql::analyze("SELECT * FROM cu", "");
        let items = sql::assemble(&ctx, &snap, &[], &[]);
        assert!(items.iter().any(|i| i.label == "customers"));
        assert!(items
            .iter()
            .all(|i| i.kind != crate::types::CompletionKind::Column));
    }

    #[test]
    fn rank_scores_are_index_first() {
        assert!(rank_score(Rank::Pk) > rank_score(Rank::Unique));
        assert!(rank_score(Rank::Unique) > rank_score(Rank::Index));
        assert!(rank_score(Rank::Index) > rank_score(Rank::Plain));
    }

    /// DB-05 / DB-10 guard: ten concurrent cold field requests for one
    /// collection run the (expensive, sampling) builder exactly once.
    #[tokio::test]
    async fn fields_or_build_is_single_flight() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = Arc::new(CompletionCache::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..10 {
            let cache = cache.clone();
            let calls = calls.clone();
            tasks.push(tokio::spawn(async move {
                cache
                    .fields_or_build("k", "db", "coll", || async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(30)).await;
                        vec![FieldSnap::new("a", Some("int32".into()), Rank::Plain)]
                    })
                    .await
                    .len()
            }));
        }
        for t in tasks {
            assert_eq!(t.await.unwrap(), 1);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // A refresh clears it, so the next request re-builds.
        cache.invalidate("k");
        assert!(cache.get_fields("k", "db", "coll").is_none());
    }

    #[tokio::test]
    async fn fields_or_build_negatively_caches_an_empty_build() {
        let cache = CompletionCache::new();
        let first = cache
            .fields_or_build("k", "db", "c", || async { Vec::new() })
            .await;
        assert!(first.is_empty());
        // Within the negative TTL the empty result is served, not rebuilt.
        let again = cache
            .fields_or_build("k", "db", "c", || async {
                panic!("must not rebuild inside the negative TTL")
            })
            .await;
        assert!(again.is_empty());
    }

    #[tokio::test]
    async fn single_flight_builds_once_and_honours_ttl() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let sf: Arc<SingleFlight<String, u32>> = Arc::new(SingleFlight::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..10 {
            let sf = sf.clone();
            let calls = calls.clone();
            tasks.push(tokio::spawn(async move {
                *sf.get_or_build("x".to_string(), || async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(30)).await;
                    (7, Duration::from_secs(60))
                })
                .await
            }));
        }
        for t in tasks {
            assert_eq!(t.await.unwrap(), 7);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // A zero TTL entry is never served from cache.
        let v = sf
            .get_or_build("y".to_string(), || async { (1, Duration::ZERO) })
            .await;
        assert_eq!(*v, 1);
        assert!(sf.get(&"y".to_string()).is_none());
        sf.invalidate_where(|k| k == "x");
        assert!(sf.get(&"x".to_string()).is_none());
        assert!(sf.len() <= 1);
    }
}
