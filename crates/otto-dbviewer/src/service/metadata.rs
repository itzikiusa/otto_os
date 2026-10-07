//! Schema-only Mongo metadata shared by object browsing and scoped completion.
use serde_json::{Map, Value};

use crate::complete::{FieldSnap, Rank};
use crate::types::{CompletionContext, NodePath, ObjectDetail};

/// Collection field names/types are metadata, but `extra` also contains sample
/// documents and validators. Never forward that unstructured bag through an
/// enforced browse response. Native credentials and the database browse guard
/// still determine which collection may be introspected.
pub(super) fn mongo_schema_extra(extra: &Value) -> Value {
    let mut out = Map::new();
    for key in ["sampled_fields", "sampled_path_types"] {
        if let Some(fields) = extra.get(key).and_then(Value::as_object) {
            out.insert(
                key.into(),
                Value::Object(
                    fields
                        .iter()
                        .filter(|(_, ty)| ty.is_string())
                        .map(|(name, ty)| (name.clone(), ty.clone()))
                        .collect(),
                ),
            );
        }
    }
    if let Some(paths) = extra.get("sampled_paths").and_then(Value::as_array) {
        out.insert(
            "sampled_paths".into(),
            Value::Array(paths.iter().filter(|p| p.is_string()).cloned().collect()),
        );
    }
    if out.is_empty() {
        Value::Null
    } else {
        Value::Object(out)
    }
}

/// Sample only when the cursor needs fields, and use the same collection/alias
/// resolution as the Mongo assembler. Collection and method slots stay cheap.
pub(super) fn mongo_completion_collection(ctx: &CompletionContext) -> Option<String> {
    use crate::complete::{mongo, sql};
    let node_coll = ctx
        .node
        .as_deref()
        .and_then(|node| NodePath::parse(node).get("coll").map(str::to_owned));
    if crate::drivers::mongo_sql::looks_like_sql(sql::current_statement(&ctx.prefix)) {
        let parsed = sql::analyze(&ctx.prefix, &ctx.suffix);
        return matches!(parsed.expect, sql::SqlExpect::Column { .. })
            .then(|| crate::drivers::mongodb::resolve_sql_collection(&parsed, node_coll.as_deref()))
            .flatten();
    }
    let parsed = mongo::analyze(&ctx.prefix);
    if matches!(parsed.expect, mongo::MongoExpect::Field { .. }) {
        parsed.collection.or(node_coll)
    } else {
        None
    }
}

/// Preserve index ranking, then append the sampled schema paths. This reads
/// only names/types, never the document values carried by the raw driver extra.
pub(super) fn mongo_completion_fields(detail: &ObjectDetail) -> Vec<FieldSnap> {
    let extra = mongo_schema_extra(&detail.extra);
    let mut types = std::collections::BTreeMap::new();
    for key in ["sampled_fields", "sampled_path_types"] {
        if let Some(map) = extra.get(key).and_then(Value::as_object) {
            for (name, ty) in map {
                types.insert(name.clone(), ty.as_str().map(str::to_owned));
            }
        }
    }
    if let Some(paths) = extra.get("sampled_paths").and_then(Value::as_array) {
        for name in paths.iter().filter_map(Value::as_str) {
            types.entry(name.to_owned()).or_insert(None);
        }
    }
    let mut fields = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for index in &detail.indexes {
        for name in &index.columns {
            if seen.insert(name.clone()) {
                let rank = if name == "_id" {
                    Rank::Pk
                } else if index.unique {
                    Rank::Unique
                } else {
                    Rank::Index
                };
                fields.push(FieldSnap::new(
                    name.clone(),
                    types.remove(name).flatten(),
                    rank,
                ));
            }
        }
    }
    fields.extend(
        types
            .into_iter()
            .map(|(name, ty)| FieldSnap::new(name, ty, Rank::Plain)),
    );
    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mongo_metadata_drops_values_and_malformed_schema_entries() {
        assert_eq!(mongo_schema_extra(&Value::Null), Value::Null);
        assert_eq!(
            mongo_schema_extra(&json!({"sample": {"private": 1}})),
            Value::Null
        );
        assert_eq!(
            mongo_schema_extra(&json!({
                "sampled_fields": {"name": "string", "not_a_type": {"private": 1}},
                "sampled_path_types": ["not a type map"],
                "sampled_paths": ["name", {"private": 1}, 7],
                "sample": {"name": "PRIVATE_VALUE"}, "stats": {"private": 1}
            })),
            json!({"sampled_fields": {"name": "string"}, "sampled_paths": ["name"]})
        );
    }
}
