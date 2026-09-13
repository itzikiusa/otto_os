use super::{invalid, ArchiveRow};
use crate::error::ApiResult;
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::TryStreamExt;
use serde_json::{json, Value};
use sqlx::{Column, Row, SqliteConnection, TypeInfo, ValueRef};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct ColumnSpec {
    pub name: String,
    pub nullable: bool,
    pub pk: i64,
}
#[derive(Clone, Debug)]
pub struct TableSpec {
    pub columns: Vec<ColumnSpec>,
    pub user_columns: Vec<String>,
}

pub fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
pub async fn schema(conn: &mut SqliteConnection) -> ApiResult<BTreeMap<String, TableSpec>> {
    let names:Vec<String>=sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_sqlx_%' ORDER BY name").fetch_all(&mut *conn).await.map_err(super::db_error)?;
    let mut tables = BTreeMap::new();
    for name in names {
        let columns = sqlx::query(&format!("PRAGMA table_info({})", quoted(&name)))
            .fetch_all(&mut *conn)
            .await
            .map_err(super::db_error)?;
        let foreign = sqlx::query(&format!("PRAGMA foreign_key_list({})", quoted(&name)))
            .fetch_all(&mut *conn)
            .await
            .map_err(super::db_error)?;
        let user_columns = foreign
            .iter()
            .filter(|r| r.get::<String, _>("table") == "users")
            .map(|r| r.get::<String, _>("from"))
            .collect();
        tables.insert(
            name,
            TableSpec {
                user_columns,
                columns: columns
                    .iter()
                    .map(|r| ColumnSpec {
                        name: r.get("name"),
                        nullable: r.get::<i64, _>("notnull") == 0,
                        pk: r.get("pk"),
                    })
                    .collect(),
            },
        );
    }
    Ok(tables)
}
pub fn excluded_table(name: &str, portable: bool) -> bool {
    if name.starts_with("sqlite_") || name.starts_with("_sqlx_") || name.contains("_fts") {
        return true;
    }
    if matches!(
        name,
        "resource_access_policies"
            | "resource_access_policy_versions"
            | "workspace_members"
            | "access_group_members"
            | "user_feature_grants"
            | "plugin_feature_grants"
            | "mcp_allowlist"
            | "mcp_policies"
            | "auth_sessions"
            | "transcript_index"
            | "provider_models"
            | "workflow_node_cache"
            | "vault_notes"
            | "vault_links"
            | "vault_tags"
            | "vault_files"
            | "plugin_migrations"
    ) {
        return true;
    }
    portable
        && !(matches!(
            name,
            "settings"
                | "workspaces"
                | "repos"
                | "connections"
                | "connection_sections"
                | "api_collections"
                | "api_requests"
                | "api_environments"
                | "api_automations"
                | "workflows"
                | "workflow_versions"
                | "workflow_triggers"
                | "scheduled_tasks"
                | "personal_agents"
                | "personal_agent_schedules"
                | "db_saved_queries"
                | "db_dashboards"
                | "db_widgets"
                | "vaults"
                | "canvas_scenes"
                | "canvas_scene_refs"
                | "product_attachments"
                | "product_stories"
                | "product_story_versions"
                | "product_notes"
                | "product_learnings"
                | "product_transcripts"
                | "repo_rules"
                | "memories"
                | "memory_links"
                | "name_themes"
                | "saved_views"
                | "swarms"
                | "swarm_agents"
                | "mcp_servers"
                | "mcp_policies"
                | "broker_clusters"
                | "aws_accounts"
                | "k8s_clusters"
                | "git_accounts"
                | "issue_accounts"
                | "users"
        ))
}
pub fn secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase().replace('-', "_");
    !matches!(
        key.as_str(),
        "secret_keys"
            | "secret_keys_json"
            | "secret_env_keys"
            | "secret_header_keys"
            | "token_type"
            | "tokens_input"
            | "tokens_output"
            | "tokens_total"
            | "total_tokens"
            | "max_tokens"
            | "max_total_tokens"
            | "max_output_tokens"
            | "max_input_tokens"
            | "token_budget"
            | "token_limit"
            | "token_count"
            | "input_tokens"
            | "output_tokens"
            | "cached_tokens"
            | "cache_read_input_tokens"
            | "cache_creation_input_tokens"
    ) && (super::super::routes::backup::is_secret_key(&key)
        || matches!(
            key.as_str(),
            "authorization"
                | "cookie"
                | "set-cookie"
                | "password_hash"
                | "otp_hash"
                | "token_hash"
                | "bot_token_ref"
                | "app_token_ref"
        ))
}
/// Sanitize structured transport credentials without rewriting free-form documents.
fn scrub_string(text: &str) -> String {
    if text.starts_with("keychain:")
        || text.starts_with("secret:")
        || text.starts_with("otto-keychain:")
    {
        return String::new();
    }
    let normalized = match otto_core::connection_credentials::extract_password(text) {
        Ok(Some((template, _))) => template.replace("{secret}", ""),
        Err(_) => return "[invalid credential URI removed]".into(),
        _ => text.to_owned(),
    };
    if let Ok(mut url) = reqwest::Url::parse(&normalized) {
        if matches!(
            url.scheme(),
            "http"
                | "https"
                | "mongodb"
                | "mongodb+srv"
                | "redis"
                | "rediss"
                | "mysql"
                | "postgres"
                | "postgresql"
                | "clickhouse"
                | "ssh"
        ) {
            let _ = url.set_password(None);
            let pairs: Vec<(String, String)> = url
                .query_pairs()
                .filter(|(key, _)| !secret_key(key))
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            url.set_query(None);
            if !pairs.is_empty() {
                url.query_pairs_mut().extend_pairs(pairs);
            }
            return url.to_string();
        }
    }
    // Mongo replica-set authorities are not accepted by URL parsers.
    if let Some((base, query)) = normalized.split_once('?') {
        if base.contains("://") {
            let mut parsed = reqwest::Url::parse("https://archive.invalid/").expect("constant URL");
            parsed.set_query(Some(query));
            let pairs: Vec<_> = parsed
                .query_pairs()
                .filter(|(k, _)| !secret_key(k))
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            parsed.set_query(None);
            if !pairs.is_empty() {
                parsed.query_pairs_mut().extend_pairs(pairs);
            }
            let query = parsed.query().unwrap_or_default();
            return if query.is_empty() {
                base.into()
            } else {
                format!("{base}?{query}")
            };
        }
    }
    if normalized.contains("-----BEGIN")
        || normalized.to_ascii_lowercase().starts_with("bearer ")
        || normalized.to_ascii_lowercase().starts_with("basic ")
    {
        return String::new();
    }
    normalized
}
fn has_credentials(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            map.get("type").and_then(Value::as_str).is_some_and(|kind| {
                crate::api_secrets::secret_members(kind)
                    .iter()
                    .any(|key| map.contains_key(*key))
            }) || map.keys().any(|k| secret_key(k) || k == "$secret")
                || ["key", "name"]
                    .iter()
                    .any(|k| map.get(*k).and_then(Value::as_str).is_some_and(secret_key))
                || map.values().any(has_credentials)
        }
        Value::Array(values) => values.iter().any(has_credentials),
        Value::String(text) => {
            if text.starts_with("keychain:")
                || text.starts_with("secret:")
                || text.starts_with("otto-keychain:")
                || text.contains("-----BEGIN")
                || text.to_ascii_lowercase().starts_with("bearer ")
                || text.to_ascii_lowercase().starts_with("basic ")
            {
                return true;
            }
            if matches!(
                otto_core::connection_credentials::extract_password(text),
                Ok(Some(_))
            ) {
                return true;
            }
            if let Ok(url) = reqwest::Url::parse(text) {
                if url.query_pairs().any(|(k, _)| secret_key(&k)) {
                    return true;
                }
            }
            serde_json::from_str::<Value>(text)
                .ok()
                .filter(|v| !v.is_string())
                .is_some_and(|v| has_credentials(&v))
        }
        _ => false,
    }
}
pub fn scrub_json(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let auth_type = map
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            for key in crate::api_secrets::secret_members(&auth_type) {
                map.remove(*key);
            }
            if map.contains_key("$secret") {
                map.clear();
                return;
            }
            let sensitive_pair = ["key", "name"].iter().any(|key| {
                map.get(*key)
                    .and_then(Value::as_str)
                    .is_some_and(secret_key)
            });
            if sensitive_pair {
                map.insert("value".into(), json!(""));
            }
            let keys: Vec<String> = map.keys().filter(|key| secret_key(key)).cloned().collect();
            for key in keys {
                map.remove(&key);
            }
            for value in map.values_mut() {
                scrub_json(value);
            }
        }
        Value::Array(values) => values.iter_mut().for_each(scrub_json),
        Value::String(text) => *text = scrub_string(text),
        _ => {}
    }
}
pub fn inert(table: &str, row: &mut ArchiveRow) {
    if table == "users" {
        row.insert("password_hash".into(), json!(""));
        row.insert("disabled".into(), json!(1));
        row.insert("is_root".into(), json!(0));
    }
    if matches!(
        table,
        "scheduled_tasks"
            | "personal_agents"
            | "personal_agent_schedules"
            | "workflow_triggers"
            | "swarm_channel_triggers"
            | "mcp_servers"
            | "plugins"
            | "workspace_integrations"
            | "broker_lag_alerts"
            | "k8s_monitor_configs"
    ) && row.contains_key("enabled")
    {
        row.insert("enabled".into(), json!(0));
    }
    if table == "sessions" {
        row.insert("status".into(), json!("exited"));
        row.insert("provider_session_id".into(), Value::Null);
    }
    if (table == "goal_loops" && row.get("status") == Some(&json!("running")))
        || (matches!(table, "swarms" | "swarm_agents")
            && row.get("status") == Some(&json!("active")))
    {
        row.insert("status".into(), json!("paused"));
    }
    if matches!(
        table,
        "workflow_runs"
            | "otto_runs"
            | "swarm_runs"
            | "scheduled_task_runs"
            | "personal_agent_runs"
            | "skill_evals"
            | "skill_reviews"
            | "product_analyses"
            | "product_analysis_agents"
            | "product_discovery_runs"
            | "goal_loop_iterations"
            | "improvement_runs"
    ) && row.get("status").and_then(Value::as_str).is_some_and(|s| {
        matches!(
            s,
            "pending"
                | "queued"
                | "running"
                | "executing"
                | "waiting"
                | "waiting_approval"
                | "planning"
                | "evaluating"
                | "digesting"
        )
    }) {
        row.insert(
            "status".into(),
            json!(if matches!(table, "otto_runs" | "improvement_runs") {
                "failed"
            } else {
                "error"
            }),
        );
    }
    if matches!(table, "mcp_approvals" | "work_approvals") {
        row.insert("status".into(), json!("expired"));
    }
    if table == "api_automation_runs" && row.get("status") == Some(&json!("running")) {
        row.insert("status".into(), json!("interrupted"));
        if let Some(Value::String(raw)) = row.get_mut("record_json") {
            if let Ok(mut record) = serde_json::from_str::<Value>(raw) {
                record["status"] = json!("interrupted");
                record["error"] = json!("Imported history; execution was not resumed.");
                *raw = record.to_string();
            }
        }
    }
    if table == "vaults" {
        row.insert("scan_state".into(), json!("idle"));
        row.insert("last_scan_at".into(), Value::Null);
    }
}
pub fn sanitize(
    table: &str,
    spec: &TableSpec,
    row: &mut ArchiveRow,
    reconnect: &mut BTreeSet<String>,
) {
    let id = row.get("id").map(Value::to_string).unwrap_or_default();
    let mcp_env_keys: Vec<String> = row
        .get("secret_env_keys")
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let mcp_header_keys: Vec<String> = row
        .get("secret_header_keys")
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let env_keys: Vec<String> = row
        .get("secret_keys_json")
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    if row.values().any(has_credentials)
        || !env_keys.is_empty()
        || !mcp_env_keys.is_empty()
        || !mcp_header_keys.is_empty()
    {
        reconnect.insert(format!(
            "{table}/{id}: review and reconnect saved credentials"
        ));
    }
    for column in &spec.columns {
        if let Some(value) = row.get_mut(&column.name) {
            if secret_key(&column.name) {
                if !value.is_null() && value != &json!("") {
                    reconnect.insert(format!("{table}/{id}: reconnect credentials"));
                }
                *value = if column.nullable {
                    Value::Null
                } else {
                    json!("")
                };
            } else if let Value::String(text) = value {
                if matches!(
                    column.name.as_str(),
                    "url" | "uri" | "remote_url" | "api_base_url" | "url_match"
                ) {
                    *text = scrub_string(text);
                }
                if let Ok(mut document) = serde_json::from_str::<Value>(text) {
                    if column.name == "variables_json" {
                        if let Some(map) = document.as_object_mut() {
                            for key in &env_keys {
                                map.remove(key);
                            }
                        }
                    }
                    if let Some(map) = document.as_object_mut() {
                        let secrets = if column.name == "env_json" {
                            &mcp_env_keys
                        } else if column.name == "headers_json" {
                            &mcp_header_keys
                        } else {
                            &env_keys[..0]
                        };
                        map.retain(|key, _| !secrets.iter().any(|s| s.eq_ignore_ascii_case(key)));
                    }
                    // User documents/history remain private, lossless content; only
                    // configuration/request JSON passes through credential filtering.
                    if matches!(
                        table,
                        "settings"
                            | "workspaces"
                            | "connections"
                            | "api_requests"
                            | "api_environments"
                            | "mcp_servers"
                            | "broker_clusters"
                            | "aws_accounts"
                            | "k8s_clusters"
                            | "workspace_integrations"
                            | "git_accounts"
                            | "issue_accounts"
                            | "repos"
                            | "plugins"
                            | "workflows"
                            | "workflow_versions"
                            | "scheduled_tasks"
                            | "personal_agents"
                            | "personal_agent_schedules"
                            | "swarms"
                            | "swarm_agents"
                            | "swarm_channel_triggers"
                            | "goal_loops"
                            | "api_automations"
                    ) {
                        scrub_json(&mut document);
                        *text = document.to_string();
                    }
                }
            }
        }
    }
    if matches!(
        table,
        "connections"
            | "api_requests"
            | "api_environments"
            | "mcp_servers"
            | "settings"
            | "broker_clusters"
            | "aws_accounts"
            | "k8s_clusters"
            | "workspace_integrations"
            | "git_accounts"
            | "issue_accounts"
    ) {
        for (key, value) in row.iter_mut() {
            if matches!(
                key.as_str(),
                "config"
                    | "config_json"
                    | "uri"
                    | "url"
                    | "headers_json"
                    | "auth_json"
                    | "value"
                    | "env_json"
                    | "kubeconfig_yaml"
                    | "params_json"
                    | "value_json"
                    | "api_base_url"
                    | "remote_url"
            ) {
                if let Value::String(text) = value {
                    if let Ok(mut doc) = serde_json::from_str::<Value>(text) {
                        scrub_json(&mut doc);
                        *text = doc.to_string();
                    } else if let Ok(mut doc) = serde_yaml::from_str::<Value>(text) {
                        if doc.is_object() || doc.is_array() {
                            scrub_json(&mut doc);
                            *text = serde_yaml::to_string(&doc).unwrap_or_default();
                        } else {
                            *text = otto_core::redact::redact_text(&scrub_string(text)).value;
                        }
                    } else {
                        *text = otto_core::redact::redact_text(&scrub_string(text)).value;
                    }
                }
            }
        }
    }
    inert(table, row);
}
pub async fn read_rows(conn: &mut SqliteConnection, table: &str) -> ApiResult<Vec<ArchiveRow>> {
    let sql = format!("SELECT * FROM {}", quoted(table));
    let mut rows = sqlx::query(&sql).fetch(conn);
    let mut output = Vec::new();
    let mut size = 0;
    while let Some(row) = rows.try_next().await.map_err(super::db_error)? {
        let mut values = ArchiveRow::new();
        for column in row.columns() {
            let raw = row.try_get_raw(column.ordinal()).map_err(super::db_error)?;
            let value = if raw.is_null() {
                Value::Null
            } else {
                match raw.type_info().name() {
                    "INTEGER" => json!(row
                        .try_get::<i64, _>(column.ordinal())
                        .map_err(super::db_error)?),
                    "REAL" => json!(row
                        .try_get::<f64, _>(column.ordinal())
                        .map_err(super::db_error)?),
                    "BLOB" => {
                        json!({"$base64":STANDARD.encode(row.try_get::<Vec<u8>,_>(column.ordinal()).map_err(super::db_error)?)})
                    }
                    _ => json!(row
                        .try_get::<String, _>(column.ordinal())
                        .map_err(super::db_error)?),
                }
            };
            values.insert(column.name().into(), value);
        }
        size += serde_json::to_vec(&values)
            .map_err(|e| invalid(&e.to_string()))?
            .len();
        if size > super::MAX_ARCHIVE_BYTES {
            return Err(invalid("Saved records exceed archive size limit"));
        }
        output.push(values);
    }
    output.sort_by_key(|row| serde_json::to_string(row).unwrap_or_default());
    Ok(output)
}
pub async fn insert_row(
    conn: &mut SqliteConnection,
    table: &str,
    spec: &TableSpec,
    row: &ArchiveRow,
) -> ApiResult<bool> {
    if row.is_empty()
        || row
            .keys()
            .any(|k| !spec.columns.iter().any(|c| &c.name == k))
    {
        return Err(invalid(&format!("Unknown or empty columns in {table}")));
    }
    let mut query =
        sqlx::QueryBuilder::<sqlx::Sqlite>::new(format!("INSERT INTO {} (", quoted(table)));
    query
        .push(row.keys().map(|k| quoted(k)).collect::<Vec<_>>().join(","))
        .push(") VALUES (");
    let mut values = query.separated(",");
    for value in row.values() {
        match value {
            Value::Null => {
                values.push_bind(Option::<String>::None);
            }
            Value::String(s) => {
                values.push_bind(s);
            }
            Value::Bool(b) => {
                values.push_bind(i64::from(*b));
            }
            Value::Number(n) => {
                if let Some(n) = n.as_i64() {
                    values.push_bind(n);
                } else if let Some(n) = n.as_f64() {
                    values.push_bind(n);
                } else {
                    return Err(invalid("Unsupported archive number"));
                }
            }
            Value::Object(object) if object.len() == 1 && object.contains_key("$base64") => {
                let bytes = STANDARD
                    .decode(
                        object["$base64"]
                            .as_str()
                            .ok_or_else(|| invalid("Invalid database blob"))?,
                    )
                    .map_err(|_| invalid("Invalid database blob"))?;
                values.push_bind(bytes);
            }
            _ => return Err(invalid("Archive rows must contain scalar database values")),
        }
    }
    query.push(") ON CONFLICT DO NOTHING");
    Ok(query
        .build()
        .execute(conn)
        .await
        .map_err(super::db_error)?
        .rows_affected()
        == 1)
}
