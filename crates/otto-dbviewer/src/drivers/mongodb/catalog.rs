//! Aggregation operator/stage and SQL-on-Mongo completion catalogs.

/// Resolve which collection a SQL column slot refers to, for field completion.
/// Resolution order, mirroring how `mongo_sql` resolves field paths:
///   1. an explicit `qualifier.` that names a FROM table or alias → that collection;
///   2. otherwise the base (first) FROM collection — this also handles an embedded
///      dotted path like `WHERE address.city`, where `address` is NOT a table but
///      a field path on the base collection (the field list already carries the
///      dotted paths, which the editor filters against the typed prefix);
///   3. failing any FROM table, the collection selected in the schema tree.
pub(crate) fn resolve_sql_collection(
    sctx: &crate::complete::sql::SqlCtx,
    node_coll: Option<&str>,
) -> Option<String> {
    use crate::complete::sql::SqlExpect;
    if let SqlExpect::Column { qualifier: Some(q) } = &sctx.expect {
        if let Some(t) = sctx.tables.iter().find(|t| {
            t.alias
                .as_deref()
                .is_some_and(|a| a.eq_ignore_ascii_case(q))
                || t.name.eq_ignore_ascii_case(q)
        }) {
            return Some(t.name.clone());
        }
        // Unknown qualifier → fall through to the base collection (embedded path).
    }
    sctx.tables
        .first()
        .map(|t| t.name.clone())
        .or_else(|| node_coll.map(str::to_string))
}

/// SQL keywords offered when completing the SQL dialect Mongo accepts — kept to
/// the subset `mongo_sql` actually translates (single-word tokens so they match
/// the editor's `[\w$.]` completion boundary).
pub(super) const MONGO_SQL_KEYWORDS: &[&str] = &[
    "SELECT", "FROM", "WHERE", "AND", "OR", "NOT", "IN", "IS", "NULL", "LIKE", "BETWEEN", "GROUP",
    "ORDER", "BY", "LIMIT", "ASC", "DESC", "AS", "JOIN", "INNER", "LEFT", "ON", "COUNT",
    "DISTINCT",
];

/// SQL aggregate functions `mongo_sql` maps to `$group` accumulators / countDocuments.
pub(super) const MONGO_SQL_FUNCTIONS: &[(&str, &str)] = &[
    ("COUNT", "COUNT(*) — count documents"),
    ("SUM", "SUM(field) — sum a numeric field"),
    ("AVG", "AVG(field) — average a numeric field"),
    ("MIN", "MIN(field) — minimum of a field"),
    ("MAX", "MAX(field) — maximum of a field"),
];

/// Collection operations supported by the runner's `db.<coll>.<op>(…)` shorthand.
pub(super) const MONGO_METHODS: &[(&str, &str)] = &[
    ("find", "read documents matching a filter"),
    ("aggregate", "run an aggregation pipeline"),
    ("countDocuments", "count documents matching a filter"),
    (
        "distinct",
        "unique field values, optionally matching a filter",
    ),
];

pub(super) const MONGO_OPERATORS: &[(&str, &str)] = &[
    // pipeline stages
    ("$match", "stage: filter documents"),
    ("$group", "stage: group by _id and accumulate"),
    ("$project", "stage: reshape documents"),
    ("$sort", "stage: order documents"),
    ("$limit", "stage: cap document count"),
    ("$skip", "stage: skip N documents"),
    ("$lookup", "stage: left outer join another collection"),
    ("$unwind", "stage: deconstruct an array field"),
    ("$count", "stage: count documents into a field"),
    ("$addFields", "stage: add computed fields"),
    ("$set", "stage: add/replace fields"),
    ("$unset", "stage: remove fields"),
    ("$replaceRoot", "stage: promote a document to root"),
    ("$facet", "stage: multiple sub-pipelines"),
    ("$bucket", "stage: group into buckets"),
    ("$sample", "stage: random sample of documents"),
    ("$out", "stage: write results to a collection"),
    ("$merge", "stage: merge results into a collection"),
    // query / comparison operators
    ("$eq", "match values equal to"),
    ("$ne", "match values not equal to"),
    ("$gt", "match values greater than"),
    ("$gte", "match values greater than or equal"),
    ("$lt", "match values less than"),
    ("$lte", "match values less than or equal"),
    ("$in", "match any value in an array"),
    ("$nin", "match no value in an array"),
    ("$exists", "match documents with the field"),
    ("$type", "match by BSON type"),
    ("$regex", "match by regular expression"),
    // logical
    ("$and", "logical AND"),
    ("$or", "logical OR"),
    ("$not", "logical NOT"),
    ("$nor", "logical NOR"),
    // accumulators / expressions
    ("$sum", "accumulate a sum"),
    ("$avg", "accumulate an average"),
    ("$min", "accumulate the minimum"),
    ("$max", "accumulate the maximum"),
    ("$first", "first value in a group"),
    ("$last", "last value in a group"),
    ("$push", "append values to an array"),
    ("$addToSet", "append unique values to an array"),
    ("$concat", "concatenate strings"),
    ("$cond", "conditional expression"),
    ("$ifNull", "fallback when null"),
    ("$size", "array length"),
];
