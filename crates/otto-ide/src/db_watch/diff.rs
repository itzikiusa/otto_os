//! Structured tabular diff engine for IDE Database Watch (`Before -> After`).

use std::collections::{BTreeMap, HashSet};

use serde_json::Value;

use super::{
    IdeDbDiffKind, IdeDbFieldChange, IdeDbRowDiff, IdeDbWatchDiffSummary, IdeDbWatchSnapshotItem,
};

const MAX_DIFF_ROWS_EMITTED: usize = 150;

const PRIMARY_KEY_CANDIDATES: &[&str] = &[
    "id",
    "_id",
    "key",
    "refid",
    "ref_id",
    "uuid",
    "player_id",
    "playerid",
    "user_id",
    "account_id",
    "order_id",
    "transaction_id",
    "code",
    "slug",
    "name",
];

fn choose_key_columns(
    columns: &[String],
    prev_rows: &[Vec<Value>],
    curr_rows: &[Vec<Value>],
) -> Vec<usize> {
    if columns.is_empty() || (prev_rows.is_empty() && curr_rows.is_empty()) {
        return Vec::new();
    }
    for cand in PRIMARY_KEY_CANDIDATES {
        if let Some(idx) = columns.iter().position(|c| c.eq_ignore_ascii_case(cand)) {
            if column_is_unique(idx, prev_rows) && column_is_unique(idx, curr_rows) {
                return vec![idx];
            }
        }
    }
    let id_suffix_cols: Vec<usize> = columns
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let lower = c.to_ascii_lowercase();
            (lower.ends_with("_id") || lower.ends_with("id")).then_some(i)
        })
        .take(3)
        .collect();
    if !id_suffix_cols.is_empty()
        && composite_is_unique(&id_suffix_cols, prev_rows)
        && composite_is_unique(&id_suffix_cols, curr_rows)
    {
        return id_suffix_cols;
    }
    if column_is_unique(0, prev_rows) && column_is_unique(0, curr_rows) {
        return vec![0];
    }
    Vec::new()
}

fn column_is_unique(idx: usize, rows: &[Vec<Value>]) -> bool {
    if rows.is_empty() {
        return true;
    }
    let mut seen = HashSet::with_capacity(rows.len());
    for r in rows {
        let Some(val) = r.get(idx) else {
            return false;
        };
        if val.is_null() {
            return false;
        }
        if !seen.insert(compact_value_key(val)) {
            return false;
        }
    }
    true
}

fn composite_is_unique(indices: &[usize], rows: &[Vec<Value>]) -> bool {
    if rows.is_empty() {
        return true;
    }
    let mut seen = HashSet::with_capacity(rows.len());
    for r in rows {
        let mut parts = Vec::with_capacity(indices.len());
        for &idx in indices {
            let Some(val) = r.get(idx) else {
                return false;
            };
            if val.is_null() {
                return false;
            }
            parts.push(compact_value_key(val));
        }
        if !seen.insert(parts.join("|")) {
            return false;
        }
    }
    true
}

fn compact_value_key(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".into(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn row_key(
    row: &[Value],
    columns: &[String],
    key_indices: &[usize],
    occurrence: &mut BTreeMap<String, usize>,
) -> String {
    let base = if !key_indices.is_empty() {
        key_indices
            .iter()
            .map(|&idx| {
                let col = columns.get(idx).map(String::as_str).unwrap_or("key");
                let val = row.get(idx).map(compact_value_key).unwrap_or_default();
                if key_indices.len() == 1 {
                    val
                } else {
                    format!("{col}={val}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        serde_json::to_string(row).unwrap_or_default()
    };
    let seq = occurrence.entry(base.clone()).or_insert(0);
    *seq += 1;
    if *seq == 1 {
        base
    } else {
        format!("{base}#{seq}")
    }
}

/// How [`diff_snapshots`] keys and compares rows.
#[derive(Debug, Clone, Default)]
pub struct DiffOpts {
    /// The table's primary key (column names); empty = guess a unique column.
    pub primary_key: Vec<String>,
    /// `current` is a full window (`ORDER BY pk DESC LIMIT n` returned n
    /// rows): a previous row older than the window's oldest key left the
    /// window, it was not deleted.
    pub window_full: bool,
    /// `previous` was a full window: a row older than ITS oldest key entered
    /// the window (a newer row was deleted), it was not added.
    pub previous_full: bool,
}

/// A Redis `ttl` counts down on its own: only a TTL that appears, goes away
/// or goes UP (re-set) is a change (`-1` / `-2` / null = no TTL).
fn ttl_changed(before: &Value, after: &Value) -> bool {
    let ttl = |v: &Value| v.as_i64().filter(|n| *n >= 0);
    match (ttl(before), ttl(after)) {
        (Some(b), Some(a)) => a > b,
        (b, a) => b.is_some() != a.is_some(),
    }
}

fn cell_changed(ttl_col: Option<usize>, i: usize, before: &Value, after: &Value) -> bool {
    if Some(i) == ttl_col {
        ttl_changed(before, after)
    } else {
        before != after
    }
}

/// Order two key values: numbers numerically, everything else as text.
/// Order two key values: integers exactly (64-bit ids), other numbers as
/// f64, everything else as text.
fn cmp_key(a: &Value, b: &Value) -> std::cmp::Ordering {
    if let (Some(x), Some(y)) = (a.as_i64(), b.as_i64()) {
        return x.cmp(&y);
    }
    if let (Some(x), Some(y)) = (a.as_u64(), b.as_u64()) {
        return x.cmp(&y);
    }
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        _ => compact_value_key(a).cmp(&compact_value_key(b)),
    }
}

/// Order two key tuples column by column (`ORDER BY c1 DESC, c2 DESC`).
fn cmp_tuple(a: &[Value], b: &[Value]) -> std::cmp::Ordering {
    a.iter()
        .zip(b)
        .map(|(x, y)| cmp_key(x, y))
        .find(|o| o.is_ne())
        .unwrap_or(std::cmp::Ordering::Equal)
}

/// Structured row & cell differences between `previous` and `current`,
/// keyed by the real primary key when given.
pub fn diff_snapshots(
    previous: &IdeDbWatchSnapshotItem,
    current: &IdeDbWatchSnapshotItem,
    opts: &DiffOpts,
) -> IdeDbWatchDiffSummary {
    let columns = if !current.columns.is_empty() {
        &current.columns
    } else {
        &previous.columns
    };
    let pk: Vec<usize> = opts
        .primary_key
        .iter()
        .filter_map(|k| columns.iter().position(|c| c == k))
        .collect();
    let pk_ok = !pk.is_empty() && pk.len() == opts.primary_key.len();
    let key_indices = if pk_ok {
        pk.clone()
    } else {
        choose_key_columns(columns, &previous.rows, &current.rows)
    };
    let is_redis = columns.len() == 4 && columns[0] == "key" && columns[3] == "ttl";
    let ttl_col = is_redis.then_some(3);
    let same = |a: &[Value], b: &[Value]| {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .enumerate()
                .all(|(i, (x, y))| !cell_changed(ttl_col, i, x, y))
    };
    // The oldest key tuple still inside a full `ORDER BY pk DESC` window.
    // Only with the real primary key: a guessed key or a Redis SCAN has no order.
    let pk_tuple = |r: &[Value]| -> Vec<Value> {
        pk.iter()
            .map(|&i| r.get(i).cloned().unwrap_or(Value::Null))
            .collect()
    };
    let window_floor = (opts.window_full && !is_redis && pk_ok)
        .then(|| {
            current
                .rows
                .iter()
                .map(|r| pk_tuple(r))
                .min_by(|a, b| cmp_tuple(a, b))
        })
        .flatten();

    // The same for the previous read: a row older than ITS oldest key entered
    // the window (a newer row was deleted), it was not added.
    let prev_floor = (opts.previous_full && !is_redis && pk_ok)
        .then(|| {
            previous
                .rows
                .iter()
                .map(|r| pk_tuple(r))
                .min_by(|a, b| cmp_tuple(a, b))
        })
        .flatten();
    let entered_window = |row: &[Value]| {
        prev_floor
            .as_ref()
            .is_some_and(|floor| cmp_tuple(&pk_tuple(row), floor).is_lt())
    };

    let mut prev_occ = BTreeMap::new();
    let mut prev_map: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for row in &previous.rows {
        let k = row_key(row, columns, &key_indices, &mut prev_occ);
        prev_map.insert(k, row.clone());
    }

    let mut curr_occ = BTreeMap::new();
    let mut curr_keys_seen = HashSet::with_capacity(current.rows.len());
    let mut added_count = 0usize;
    let mut modified_count = 0usize;
    let mut unchanged_count = 0usize;
    let mut diffs = Vec::new();

    for row in &current.rows {
        let k = row_key(row, columns, &key_indices, &mut curr_occ);
        curr_keys_seen.insert(k.clone());
        match prev_map.get(&k) {
            None if entered_window(row) => {}
            None => {
                added_count += 1;
                if diffs.len() < MAX_DIFF_ROWS_EMITTED {
                    diffs.push(IdeDbRowDiff {
                        key: k,
                        kind: IdeDbDiffKind::Added,
                        row: row.clone(),
                        previous_row: None,
                        changes: Vec::new(),
                        masked: false,
                    });
                }
            }
            Some(prev_row) if same(prev_row, row) => {
                unchanged_count += 1;
            }
            Some(prev_row) => {
                modified_count += 1;
                if diffs.len() < MAX_DIFF_ROWS_EMITTED {
                    let max_cols = prev_row.len().max(row.len()).max(columns.len());
                    let mut changes = Vec::new();
                    for i in 0..max_cols {
                        let before = prev_row.get(i).cloned().unwrap_or(Value::Null);
                        let after = row.get(i).cloned().unwrap_or(Value::Null);
                        if cell_changed(ttl_col, i, &before, &after) {
                            let col_name = columns
                                .get(i)
                                .cloned()
                                .unwrap_or_else(|| format!("col_{i}"));
                            changes.push(IdeDbFieldChange {
                                column: col_name,
                                before,
                                after,
                                masked: false,
                            });
                        }
                    }
                    diffs.push(IdeDbRowDiff {
                        key: k,
                        kind: IdeDbDiffKind::Modified,
                        row: row.clone(),
                        previous_row: Some(prev_row.clone()),
                        changes,
                        masked: false,
                    });
                }
            }
        }
    }

    let mut removed_count = 0usize;
    for (k, prev_row) in prev_map {
        if !curr_keys_seen.contains(&k) {
            let left_window = window_floor
                .as_ref()
                .is_some_and(|floor| cmp_tuple(&pk_tuple(&prev_row), floor).is_lt());
            if left_window {
                continue;
            }
            removed_count += 1;
            if diffs.len() < MAX_DIFF_ROWS_EMITTED {
                diffs.push(IdeDbRowDiff {
                    key: k,
                    kind: IdeDbDiffKind::Removed,
                    row: prev_row,
                    previous_row: None,
                    changes: Vec::new(),
                    masked: false,
                });
            }
        }
    }

    IdeDbWatchDiffSummary {
        added_count,
        modified_count,
        removed_count,
        unchanged_count,
        diffs,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn computes_added_modified_and_removed_rows_by_primary_key() {
        let prev = IdeDbWatchSnapshotItem {
            columns: vec!["id".into(), "balance".into(), "status".into()],
            rows: vec![
                vec![json!(1), json!("100.00"), json!("active")],
                vec![json!(2), json!("50.00"), json!("locked")],
            ],
        };
        let curr = IdeDbWatchSnapshotItem {
            columns: vec!["id".into(), "balance".into(), "status".into()],
            rows: vec![
                vec![json!(1), json!("175.50"), json!("active")],
                vec![json!(3), json!("10.00"), json!("active")],
            ],
        };
        let diff = diff_snapshots(&prev, &curr, &DiffOpts::default());
        assert_eq!(diff.added_count, 1);
        assert_eq!(diff.modified_count, 1);
        assert_eq!(diff.removed_count, 1);
        assert_eq!(diff.unchanged_count, 0);
        let modified = diff
            .diffs
            .iter()
            .find(|d| d.kind == IdeDbDiffKind::Modified)
            .expect("modified row");
        assert_eq!(modified.key, "1");
        assert_eq!(modified.changes.len(), 1);
        assert_eq!(modified.changes[0].column, "balance");
        assert_eq!(modified.changes[0].before, json!("100.00"));
        assert_eq!(modified.changes[0].after, json!("175.50"));
    }

    fn snap(columns: &[&str], rows: Vec<Vec<Value>>) -> IdeDbWatchSnapshotItem {
        IdeDbWatchSnapshotItem {
            columns: columns.iter().map(|c| c.to_string()).collect(),
            rows,
        }
    }

    #[test]
    fn a_redis_ttl_counting_down_is_not_a_change_but_a_reset_is() {
        let cols = ["key", "type", "value", "ttl"];
        let prev = snap(
            &cols,
            vec![
                vec![json!("s:1"), json!("string"), json!("a"), json!(300)],
                vec![json!("s:2"), json!("string"), json!("b"), json!(300)],
                vec![json!("s:3"), json!("string"), json!("c"), json!(-1)],
            ],
        );
        let curr = snap(
            &cols,
            vec![
                vec![json!("s:1"), json!("string"), json!("a"), json!(297)],
                vec![json!("s:2"), json!("string"), json!("b"), json!(600)],
                vec![json!("s:3"), json!("string"), json!("c"), json!(60)],
            ],
        );
        let d = diff_snapshots(
            &prev,
            &curr,
            &DiffOpts {
                primary_key: vec!["key".into()],
                window_full: false,
                previous_full: false,
            },
        );
        assert_eq!(
            (d.unchanged_count, d.modified_count),
            (1, 2),
            "{:?}",
            d.diffs
        );
        assert!(d.diffs.iter().all(|r| r.key != "s:1"));
    }

    #[test]
    fn a_row_pushed_out_of_a_full_window_is_not_a_delete() {
        // ORDER BY id DESC LIMIT 3: an INSERT of id 4 pushes id 1 out.
        let prev = snap(
            &["id", "v"],
            vec![
                vec![json!(3), json!("c")],
                vec![json!(2), json!("b")],
                vec![json!(1), json!("a")],
            ],
        );
        let curr = snap(
            &["id", "v"],
            vec![
                vec![json!(4), json!("d")],
                vec![json!(3), json!("c")],
                vec![json!(2), json!("b")],
            ],
        );
        let opts = DiffOpts {
            primary_key: vec!["id".into()],
            window_full: true,
            previous_full: false,
        };
        let d = diff_snapshots(&prev, &curr, &opts);
        assert_eq!(
            (d.added_count, d.removed_count, d.unchanged_count),
            (1, 0, 2)
        );
        // A delete pulls an older row into a full window: it is not new.
        let five = snap(
            &["id", "v"],
            vec![
                vec![json!(5), json!("e")],
                vec![json!(4), json!("d")],
                vec![json!(3), json!("c")],
            ],
        );
        let after_delete = snap(
            &["id", "v"],
            vec![
                vec![json!(5), json!("e")],
                vec![json!(3), json!("c")],
                vec![json!(2), json!("b")],
            ],
        );
        let both_full = DiffOpts {
            previous_full: true,
            ..opts.clone()
        };
        let d = diff_snapshots(&five, &after_delete, &both_full);
        assert_eq!(
            (d.added_count, d.removed_count, d.unchanged_count),
            (0, 1, 2)
        );
        // A Redis read is an unordered SCAN: a new key that sorts first is new.
        let keys = |ks: &[&str]| {
            snap(
                &["key", "type", "value", "ttl"],
                ks.iter()
                    .map(|k| vec![json!(k), json!("string"), json!("v"), json!(-1)])
                    .collect(),
            )
        };
        let redis = DiffOpts {
            primary_key: vec!["key".into()],
            window_full: true,
            previous_full: true,
        };
        let d = diff_snapshots(
            &keys(&["s:5", "s:4"]),
            &keys(&["s:5", "s:4", "s:0"]),
            &redis,
        );
        assert_eq!(d.added_count, 1);
        // A real delete inside the window still counts.
        let curr = snap(
            &["id", "v"],
            vec![
                vec![json!(4), json!("d")],
                vec![json!(3), json!("c")],
                vec![json!(1), json!("a")],
            ],
        );
        let d = diff_snapshots(&prev, &curr, &opts);
        assert_eq!(d.removed_count, 1);
        assert_eq!(
            d.diffs
                .iter()
                .find(|r| r.kind == IdeDbDiffKind::Removed)
                .unwrap()
                .key,
            "2"
        );
    }

    #[test]
    fn the_real_composite_primary_key_keys_the_rows() {
        let prev = snap(
            &["game_id", "account_id", "status"],
            vec![
                vec![json!(1), json!(7), json!("open")],
                vec![json!(1), json!(8), json!("open")],
            ],
        );
        let curr = snap(
            &["game_id", "account_id", "status"],
            vec![
                vec![json!(1), json!(7), json!("closed")],
                vec![json!(1), json!(8), json!("open")],
            ],
        );
        let opts = DiffOpts {
            primary_key: vec!["game_id".into(), "account_id".into()],
            window_full: false,
            previous_full: false,
        };
        let d = diff_snapshots(&prev, &curr, &opts);
        assert_eq!(d.modified_count, 1);
        assert_eq!(d.diffs[0].key, "game_id=1, account_id=7");
        assert_eq!(d.diffs[0].changes.len(), 1);
        assert_eq!(d.diffs[0].changes[0].column, "status");
    }

    #[test]
    fn a_primary_key_missing_from_the_columns_does_not_panic_a_full_window() {
        let prev = snap(&["ID", "v"], vec![vec![json!(1), json!("a")]]);
        let curr = snap(&["ID", "v"], vec![vec![json!(2), json!("b")]]);
        let opts = DiffOpts {
            primary_key: vec!["id".into()],
            window_full: true,
            previous_full: true,
        };
        let d = diff_snapshots(&prev, &curr, &opts);
        assert_eq!(d.added_count + d.removed_count, 2);
    }

    #[test]
    fn a_deleted_redis_key_is_a_delete_even_in_a_full_scan_window() {
        let cols = ["key", "type", "value", "ttl"];
        let row = |k: &str| vec![json!(k), json!("string"), json!("v"), json!(-1)];
        let prev = snap(&cols, vec![row("s:1"), row("s:5"), row("s:9")]);
        let curr = snap(&cols, vec![row("s:5"), row("s:9")]);
        let opts = DiffOpts {
            primary_key: vec!["key".into()],
            window_full: true,
            previous_full: true,
        };
        assert_eq!(
            diff_snapshots(&prev, &curr, &opts).removed_count,
            1,
            "SCAN has no order"
        );
    }

    #[test]
    fn a_composite_key_row_pushed_out_of_the_window_is_not_a_delete() {
        // ORDER BY game_id DESC, account_id DESC LIMIT 3; INSERT (1, 10).
        let r = |a: i64| vec![json!(1), json!(a)];
        let prev = snap(&["game_id", "account_id"], vec![r(9), r(8), r(7)]);
        let curr = snap(&["game_id", "account_id"], vec![r(10), r(9), r(8)]);
        let opts = DiffOpts {
            primary_key: vec!["game_id".into(), "account_id".into()],
            window_full: true,
            previous_full: true,
        };
        let d = diff_snapshots(&prev, &curr, &opts);
        assert_eq!((d.added_count, d.removed_count), (1, 0));
    }

    #[test]
    fn sixty_four_bit_keys_compare_exactly() {
        // 2^53 and 2^53 + 1 are the same f64.
        let (lo, hi) = (9_007_199_254_740_992_i64, 9_007_199_254_740_993_i64);
        let prev = snap(&["id"], vec![vec![json!(hi)], vec![json!(lo)]]);
        let curr = snap(&["id"], vec![vec![json!(hi + 1)], vec![json!(hi)]]);
        let opts = DiffOpts {
            primary_key: vec!["id".into()],
            window_full: true,
            previous_full: true,
        };
        let d = diff_snapshots(&prev, &curr, &opts);
        assert_eq!((d.added_count, d.removed_count), (1, 0), "{:?}", d.diffs);
    }
}
