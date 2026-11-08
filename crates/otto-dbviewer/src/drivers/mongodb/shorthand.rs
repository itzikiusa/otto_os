//! The `db.<coll>.<op>(...)` shell shorthand the Mongo runner parses natively.

use super::*;

/// Tolerant `db.<coll>.<op>(<arg>)` parser with optional `.limit(n)`/`.sort({})`.
pub(super) fn parse_shorthand(raw: &str) -> Result<ParsedCommand> {
    let s = raw.strip_prefix("db.").unwrap_or(raw);
    let (collection, rest) = match get_collection_call(s)? {
        Some(found) => found,
        None => {
            // The last dot before the first call separates a dotted collection
            // name from its method (e.g. db.system.profile.find()).
            let head = s.split_once('(').map(|(head, _)| head).unwrap_or(s);
            let dot = head
                .rfind('.')
                .ok_or_else(|| types::invalid("expected db.<collection>.<op>(...)"))?;
            (s[..dot].trim().to_string(), &s[dot + 1..])
        }
    };

    let mut cmd = ParsedCommand {
        collection,
        ..Default::default()
    };

    // Walk method calls: name(args) ['.' name(args)]*
    let mut cursor = rest;
    while !cursor.trim().is_empty() {
        let cursor_t = cursor.trim_start();
        let paren = cursor_t
            .find('(')
            .ok_or_else(|| types::invalid("expected method call like find(...)"))?;
        let method = cursor_t[..paren].trim().to_string();
        let (arg, after) = extract_balanced(&cursor_t[paren..])?;

        // Only cursor operations can be chained. Never reinterpret an invalid
        // chain as a different collection operation or silently drop options.
        if let Some(op) = cmd.op_kind {
            let allowed = match op {
                MongoOp::Find => matches!(
                    method.as_str(),
                    "sort"
                        | "limit"
                        | "skip"
                        | "projection"
                        | "explain"
                        | "count"
                        | "toArray"
                        | "pretty"
                        | "maxTimeMS"
                ),
                MongoOp::Aggregate => {
                    matches!(
                        method.as_str(),
                        "explain" | "toArray" | "pretty" | "maxTimeMS"
                    )
                }
                _ => false,
            };
            if !allowed || cmd.explain || (method == "count" && !arg.trim().is_empty()) {
                return Err(types::invalid("unsupported Mongo method chain"));
            }
        } else if matches!(
            method.as_str(),
            "sort"
                | "limit"
                | "skip"
                | "projection"
                | "explain"
                | "toArray"
                | "pretty"
                | "maxTimeMS"
        ) {
            return Err(types::invalid("cursor method requires find or aggregate"));
        }
        let max_args = match method.as_str() {
            "find" | "findOne" | "distinct" | "updateOne" | "updateMany" | "replaceOne"
            | "createIndex" => 2,
            "explain"
            | "getIndexes"
            | "getIndices"
            | "toArray"
            | "pretty"
            | "estimatedDocumentCount" => 0,
            _ => 1,
        };
        let argc = if arg.trim().is_empty() {
            0
        } else {
            split_top_level_args(&arg).len()
        };
        if argc > max_args {
            return Err(types::invalid("Mongo options require mongosh execution"));
        }
        // Neither returns a cursor in mongosh, so nothing chains after them.
        if matches!(method.as_str(), "findOne" | "toArray") && !skip_mongo_trivia(after)?.is_empty()
        {
            return Err(types::invalid(format!(
                "{method}() must end the method chain"
            )));
        }
        match method.as_str() {
            "find" | "findOne" => {
                cmd.op_kind = Some(MongoOp::Find);
                if method == "findOne" {
                    cmd.limit = Some(1);
                }
                // `find(filter)` or `find(filter, projection)` (mongosh 2-arg form).
                let parts = split_top_level_args(&arg);
                if let Some(f) = parts.first() {
                    if !f.trim().is_empty() {
                        cmd.filter = Some(parse_doc_arg(f)?);
                    }
                }
                if let Some(p) = parts.get(1) {
                    if !p.trim().is_empty() {
                        cmd.projection = Some(parse_doc_arg(p)?);
                    }
                }
            }
            "aggregate" => {
                cmd.op_kind = Some(MongoOp::Aggregate);
                cmd.pipeline = Some(parse_pipeline_arg(&arg)?);
            }
            "distinct" => {
                if cmd.op_kind.is_some()
                    || cmd.limit.is_some()
                    || cmd.sort.is_some()
                    || cmd.projection.is_some()
                    || cmd.explain
                {
                    return Err(types::invalid(
                        "distinct must be called directly on a collection",
                    ));
                }
                let parts = split_top_level_args(&arg);
                if !(1..=2).contains(&parts.len()) {
                    return Err(types::invalid("distinct expects a field name and an optional filter; options are unsupported"));
                }
                let field = mongo_parse::parse_value(parts[0].trim())?;
                cmd.distinct_field = Some(
                    field
                        .as_str()
                        .filter(|f| !f.is_empty())
                        .ok_or_else(|| {
                            types::invalid("distinct requires a non-empty string field name")
                        })?
                        .to_string(),
                );
                if let Some(filter) = parts.get(1) {
                    cmd.filter = Some(parse_doc_arg(filter)?);
                }
                cmd.op_kind = Some(MongoOp::Distinct);
            }
            // With no filter the native `count` command answers from collection
            // metadata, which is what estimatedDocumentCount is.
            "countDocuments" | "count" | "estimatedDocumentCount" => {
                cmd.op_kind = Some(if method == "countDocuments" {
                    MongoOp::CountDocuments
                } else {
                    MongoOp::Count
                });
                if !arg.trim().is_empty() {
                    cmd.filter = Some(parse_doc_arg(&arg)?);
                }
            }
            "updateOne" | "updateMany" => {
                cmd.op_kind = Some(if method == "updateOne" {
                    MongoOp::UpdateOne
                } else {
                    MongoOp::UpdateMany
                });
                let parts = split_top_level_args(&arg);
                let filter = parts
                    .first()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| types::invalid("update requires a filter"))?;
                let update = parts
                    .get(1)
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| types::invalid("update requires an update document"))?;
                cmd.filter = Some(parse_doc_arg(filter)?);
                cmd.update = Some(parse_doc_arg(update)?);
            }
            "replaceOne" => {
                // Whole-document replacement (the grid's JSON-view editor emits
                // this); the replacement rides the `update` slot like updateOne.
                cmd.op_kind = Some(MongoOp::ReplaceOne);
                let parts = split_top_level_args(&arg);
                let filter = parts
                    .first()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| types::invalid("replaceOne requires a filter"))?;
                let replacement = parts
                    .get(1)
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| types::invalid("replaceOne requires a replacement document"))?;
                cmd.filter = Some(parse_doc_arg(filter)?);
                cmd.update = Some(parse_doc_arg(replacement)?);
            }
            "insertOne" => {
                cmd.op_kind = Some(MongoOp::InsertOne);
                cmd.documents = Some(vec![parse_doc_arg(&arg)?]);
            }
            "insertMany" => {
                cmd.op_kind = Some(MongoOp::InsertMany);
                cmd.documents = Some(parse_pipeline_arg(&arg)?);
            }
            "deleteOne" | "deleteMany" => {
                if arg.trim().is_empty() {
                    return Err(types::invalid("delete requires an explicit filter"));
                }
                cmd.op_kind = Some(if method == "deleteOne" {
                    MongoOp::DeleteOne
                } else {
                    MongoOp::DeleteMany
                });
                if !arg.trim().is_empty() {
                    cmd.filter = Some(parse_doc_arg(&arg)?);
                }
            }
            "createIndex" => {
                cmd.op_kind = Some(MongoOp::CreateIndex);
                let parts = split_top_level_args(&arg);
                let keys = parts
                    .first()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| types::invalid("createIndex requires a key spec"))?;
                cmd.index_keys = Some(parse_doc_arg(keys)?);
                if let Some(opts) = parts.get(1).filter(|s| !s.trim().is_empty()) {
                    cmd.index_options = Some(parse_doc_arg(opts)?);
                }
            }
            "dropIndex" => {
                cmd.op_kind = Some(MongoOp::DropIndex);
                let name = mongo_parse::parse_value(arg.trim())?;
                cmd.index_name = Some(
                    name.as_str()
                        .ok_or_else(|| types::invalid("dropIndex key documents require mongosh"))?
                        .to_string(),
                );
            }
            // Takes no arguments; anything passed is ignored, as in mongosh.
            "getIndexes" | "getIndices" => {
                cmd.op_kind = Some(MongoOp::GetIndexes);
            }
            "explain" => {
                // A trailing `.explain()` modifies the preceding find/aggregate.
                cmd.explain = true;
            }
            "limit" => {
                let limit = arg
                    .trim()
                    .parse::<i64>()
                    .map_err(|_| types::invalid("limit must be an integer"))?;
                if limit < 0 {
                    return Err(types::invalid(
                        "negative single-batch limits require mongosh",
                    ));
                }
                cmd.limit = (limit > 0).then_some(limit);
            }
            "skip" => {
                let skip = arg
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| types::invalid("skip must be a non-negative integer"))?;
                cmd.skip = (skip > 0).then_some(skip);
            }
            "maxTimeMS" => {
                let ms = arg
                    .trim()
                    .parse::<i64>()
                    .ok()
                    .filter(|ms| *ms >= 0)
                    .ok_or_else(|| types::invalid("maxTimeMS must be a non-negative integer"))?;
                cmd.max_time_ms = (ms > 0).then_some(ms);
            }
            // Result formatting only: Otto always returns the documents.
            "toArray" | "pretty" => {}
            "sort" => {
                cmd.sort = Some(parse_doc_arg(&arg)?);
            }
            "projection" => {
                cmd.projection = Some(parse_doc_arg(&arg)?);
            }
            other => {
                return Err(types::invalid(format!("unsupported method '{other}'")));
            }
        }

        // Comments after a method are trivia, not another method or option.
        // Reject unclosed block comments instead of swallowing malformed input.
        cursor = skip_mongo_trivia(after)?;
        if !cursor.is_empty() {
            cursor = cursor
                .strip_prefix('.')
                .filter(|rest| !rest.trim().is_empty())
                .ok_or_else(|| types::invalid("expected '.' between Mongo method calls"))?;
        }
    }

    Ok(cmd)
}

/// `getCollection("name").<rest>`: the shell's spelling for a collection whose
/// name isn't an identifier. `None` for anything else, including a collection
/// really called `getCollection` (`db.getCollection.find()`).
fn get_collection_call(s: &str) -> Result<Option<(String, &str)>> {
    let Some(call) = s
        .strip_prefix("getCollection")
        .map(str::trim_start)
        .filter(|call| call.starts_with('('))
    else {
        return Ok(None);
    };
    let (arg, after) = extract_balanced(call)?;
    let name = mongo_parse::parse_value(arg.trim())?;
    let name = name
        .as_str()
        .ok_or_else(|| types::invalid("getCollection expects a collection name string"))?
        .to_string();
    let rest = skip_mongo_trivia(after)?
        .strip_prefix('.')
        .ok_or_else(|| types::invalid("expected a method after getCollection(...)"))?;
    Ok(Some((name, rest)))
}
