//! Keyset pagination for an eligible MongoDB `find` (moved out of
//! `mongodb.rs`): the eligibility rule, the BSON-type-order "after the cursor"
//! clause, and the next page's cursor.

use super::*;

/// Decide whether a `find` can page by keyset, and if so build the page's
/// `(filter, sort)`. Eligible ⇔ no explicit `.limit(n)` (the auto-limiter's own
/// rule — an explicit limit is never paged), the sort is absent or exactly
/// `{_id: 1}`, and the filter carries no top-level `_id` (a user already pinning
/// `_id` gets the natural order they asked for). When eligible the sort is
/// `{_id: 1}` ALWAYS — page 1 included — so every page of the walk shares one
/// order; with a `cursor` (the previous page's last `_id`, Extended JSON) the
/// filter becomes `{$and: [<filter or {}>, <after cursor>]}` — `$and` rather
/// than a merged key, so a filter using `$or`/`$and` at the top stays intact.
/// `<after cursor>` is [`id_after`]: `$gt` alone is type-bracketed, so a
/// collection mixing `_id` types (string ids from an import, ObjectIds from
/// the app) would end its walk at the first type change.
/// `Ok(None)` ⇒ not eligible, use offset/`skip`. An undecodable cursor
/// is the caller's error (a 400), never a silent fall-back to `skip`.
pub(super) fn keyset_filter(
    filter: Option<&Document>,
    sort: Option<&Document>,
    explicit_limit: bool,
    cursor: Option<&Value>,
) -> Result<Option<(Document, Document)>> {
    if explicit_limit || !sort.is_none_or(sort_is_id_asc) {
        return Ok(None);
    }
    if filter.is_some_and(|f| f.contains_key("_id")) {
        return Ok(None);
    }
    let base = filter.cloned().unwrap_or_default();
    let filter = match cursor {
        Some(c) => {
            let last = json_to_bson(c)
                .map_err(|e| types::invalid(format!("invalid keyset cursor: {e}")))?;
            doc! { "$and": [base, id_after(last)] }
        }
        None => base,
    };
    Ok(Some((filter, doc! { "_id": 1 })))
}

/// The `$type` names an `_id` can take, in MongoDB's `{_id: 1}` sort order
/// (MinKey < null < numbers < symbol/string < object < binData < ObjectId <
/// bool < date < timestamp < regex < MaxKey; an `_id` is never an array). Each
/// entry is one comparison bracket: `$gt` only compares within a bracket.
const ID_TYPE_ORDER: &[&[&str]] = &[
    &["minKey"],
    &["null"],
    &["number"],
    &["symbol", "string"],
    &["object"],
    &["binData"],
    &["objectId"],
    &["bool"],
    &["date"],
    &["timestamp"],
    &["regex"],
    &["maxKey"],
];

/// The [`ID_TYPE_ORDER`] bracket of a cursor value; `None` for a type that is
/// not a valid / sortable `_id` (JavaScript code, DBPointer…).
fn id_type_rank(v: &Bson) -> Option<usize> {
    Some(match v {
        Bson::MinKey => 0,
        Bson::Null | Bson::Undefined => 1,
        Bson::Int32(_) | Bson::Int64(_) | Bson::Double(_) | Bson::Decimal128(_) => 2,
        Bson::Symbol(_) | Bson::String(_) => 3,
        Bson::Document(_) => 4,
        Bson::Binary(_) => 5,
        Bson::ObjectId(_) => 6,
        Bson::Boolean(_) => 7,
        Bson::DateTime(_) => 8,
        Bson::Timestamp(_) => 9,
        Bson::RegularExpression(_) => 10,
        Bson::MaxKey => 11,
        _ => return None,
    })
}

/// "Every `_id` after `last` in `{_id: 1}` order": the rest of `last`'s own
/// bracket (`$gt`) OR any `_id` of a later-sorting type. Without the second
/// arm a page boundary inside the strings returns only the remaining strings,
/// the short page reads as "no more rows" and every ObjectId-keyed document
/// is silently never shown (backend-correctness-6-03). Both arms use the
/// `_id` index. A cursor of an unrankable type keeps the plain `$gt`.
pub(super) fn id_after(last: Bson) -> Document {
    let later: Vec<Bson> = id_type_rank(&last)
        .map(|rank| {
            ID_TYPE_ORDER[rank + 1..]
                .iter()
                .flat_map(|names| names.iter().map(|n| Bson::String((*n).to_owned())))
                .collect()
        })
        .unwrap_or_default();
    if later.is_empty() {
        return doc! { "_id": { "$gt": last } };
    }
    doc! { "$or": [{ "_id": { "$gt": last } }, { "_id": { "$type": later } }] }
}

/// `{_id: 1}` exactly — one key, ascending — with the direction compared
/// numerically (the parser yields `Int64(1)`, a pasted EJSON sort may say
/// `Int32`/`Double`).
fn sort_is_id_asc(sort: &Document) -> bool {
    sort.len() == 1
        && sort.get("_id").is_some_and(|dir| match dir {
            Bson::Int32(n) => *n == 1,
            Bson::Int64(n) => *n == 1,
            Bson::Double(f) => *f == 1.0,
            _ => false,
        })
}

/// The `_id` cell of the LAST row (already the typed Extended-JSON projection
/// the wire carries, so it round-trips through [`json_to_bson`] unchanged) —
/// the keyset cursor for the next page. `None` when the result has no `_id`
/// column (a projection dropped it) or no rows, in which case the client keeps
/// paging by offset.
pub(super) fn last_row_id(r: &QueryResult) -> Option<Value> {
    let idx = r.columns.iter().position(|c| c.name == "_id")?;
    r.rows.last()?.get(idx).filter(|v| !v.is_null()).cloned()
}
