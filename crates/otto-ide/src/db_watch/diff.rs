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

/// Compute structured row & cell differences between `previous` and `current`.
pub fn compute_snapshot_diff(
    previous: &IdeDbWatchSnapshotItem,
    current: &IdeDbWatchSnapshotItem,
) -> IdeDbWatchDiffSummary {
    let columns = if !current.columns.is_empty() {
        &current.columns
    } else {
        &previous.columns
    };
    let key_indices = choose_key_columns(columns, &previous.rows, &current.rows);

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
            None => {
                added_count += 1;
                if diffs.len() < MAX_DIFF_ROWS_EMITTED {
                    diffs.push(IdeDbRowDiff {
                        key: k,
                        kind: IdeDbDiffKind::Added,
                        row: row.clone(),
                        previous_row: None,
                        changes: Vec::new(),
                    });
                }
            }
            Some(prev_row) if prev_row == row => {
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
                        if before != after {
                            let col_name = columns
                                .get(i)
                                .cloned()
                                .unwrap_or_else(|| format!("col_{i}"));
                            changes.push(IdeDbFieldChange {
                                column: col_name,
                                before,
                                after,
                            });
                        }
                    }
                    diffs.push(IdeDbRowDiff {
                        key: k,
                        kind: IdeDbDiffKind::Modified,
                        row: row.clone(),
                        previous_row: Some(prev_row.clone()),
                        changes,
                    });
                }
            }
        }
    }

    let mut removed_count = 0usize;
    for (k, prev_row) in prev_map {
        if !curr_keys_seen.contains(&k) {
            removed_count += 1;
            if diffs.len() < MAX_DIFF_ROWS_EMITTED {
                diffs.push(IdeDbRowDiff {
                    key: k,
                    kind: IdeDbDiffKind::Removed,
                    row: prev_row,
                    previous_row: None,
                    changes: Vec::new(),
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
        let diff = compute_snapshot_diff(&prev, &curr);
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
}
