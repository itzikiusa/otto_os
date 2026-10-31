//! MCP profile CRUD — merged into [`crate::api_router`]. Reads are MCP View,
//! every change MCP Admin (`otto-server` policy, like the other control-plane
//! settings) AND a person's own credential: an agent session's token or an MCP
//! credential cannot re-shape which tools sessions see. Every change is
//! written to the admin audit log (`mcp.profile.*`).
//!
//!   GET    /mcp/profiles               profiles + the tool groups
//!   POST   /mcp/profiles               create (201)
//!   PATCH  /mcp/profiles/{id}          edit (the locked Full profile: 400)
//!   DELETE /mcp/profiles/{id}          delete (Full: 400; the default → Full)
//!   POST   /mcp/profiles/{id}/default  make it the default

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use axum::{Extension, Json, Router};
use otto_core::auth::{AuthContext, AuthUser};
use otto_core::Error;
use otto_state::{McpProfile, McpProfileFields, McpProfileRepo, NewAuditEntry};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{canonical, canonical_group_id, group, group_of, normalize_fields, GROUPS};
use crate::http::{ApiErr, McpCtx};

type ApiResult<T> = std::result::Result<T, ApiErr>;

pub fn router<S: McpCtx>() -> Router<S> {
    Router::new()
        .route(
            "/mcp/profiles",
            get(list_profiles::<S>).post(create_profile::<S>),
        )
        .route(
            "/mcp/profiles/{id}",
            patch(update_profile::<S>).delete(delete_profile::<S>),
        )
        .route("/mcp/profiles/{id}/default", post(set_default::<S>))
}

fn repo<S: McpCtx>(ctx: &S) -> McpProfileRepo {
    McpProfileRepo::new(ctx.mcp_pool().clone())
}

/// A person signed in to Otto — not an agent session's managed token, an
/// internal/outward MCP credential or a share token (same rule as deciding
/// an approval). A router mounted without the auth context (unit harness) is
/// treated as a person.
fn require_person(auth: Option<&AuthContext>) -> ApiResult<()> {
    if auth.is_some_and(|a| {
        a.managed_session_id.is_some()
            || a.mcp_session_id.is_some()
            || a.mcp_only
            || a.mcp_scope.is_some()
            || a.is_scoped()
    }) {
        return Err(ApiErr(Error::Forbidden(
            "MCP profiles can only be changed by a person signed in to Otto — an agent \
             session's or MCP credential cannot change which tools sessions see"
                .into(),
        )));
    }
    Ok(())
}

async fn audit<S: McpCtx>(ctx: &S, user: &otto_core::domain::User, verb: &str, p: &McpProfile) {
    let entry = NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: format!("mcp.profile.{verb}"),
        target: Some(p.id.clone()),
        detail: serde_json::to_value(p).ok(),
        ip: None,
    };
    if let Err(e) = otto_state::AuditRepo::new(ctx.mcp_pool().clone())
        .insert(entry)
        .await
    {
        tracing::warn!("audit mcp.profile.{verb} failed (best-effort): {e}");
    }
}

/// The groups with their member tools: every outward-catalog tool of the
/// group plus its exact names (session-only native tools included), and the
/// prefix families it also covers.
fn groups_view() -> Vec<Value> {
    let catalog: Vec<&str> = crate::outward::otto_tool_specs_cached()
        .iter()
        .filter_map(|s| s["name"].as_str())
        .map(canonical)
        .collect();
    GROUPS
        .iter()
        .map(|g| {
            let mut tools: Vec<&str> = g.exact.to_vec();
            for t in &catalog {
                if !tools.contains(t) && group_of(t).is_some_and(|x| x.id == g.id) {
                    tools.push(t);
                }
            }
            json!({
                "id": g.id,
                "label": g.label,
                "description": g.description,
                "prefixes": g.prefixes,
                "tools": tools,
            })
        })
        .collect()
}

/// `GET /mcp/profiles`
async fn list_profiles<S: McpCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
) -> ApiResult<Json<Value>> {
    let profiles = repo(&ctx).list().await?;
    let default_id = profiles
        .iter()
        .find(|p| p.is_default)
        .map(|p| p.id.clone())
        .unwrap_or_else(|| otto_state::FULL_PROFILE_ID.to_string());
    Ok(Json(json!({
        "profiles": profiles,
        "groups": groups_view(),
        "default_id": default_id,
    })))
}

/// Body of `POST /mcp/profiles` and `PATCH /mcp/profiles/{id}` (PATCH: every
/// field optional, missing ones keep their value).
#[derive(Debug, Default, Deserialize)]
pub struct ProfileReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub all_tools: Option<bool>,
    #[serde(default)]
    pub groups: Option<Vec<String>>,
    #[serde(default)]
    pub include_tools: Option<Vec<String>>,
    #[serde(default)]
    pub exclude_tools: Option<Vec<String>>,
}

/// Merge a request over `base` and validate the result.
fn fields_from(req: ProfileReq, base: McpProfileFields) -> Result<McpProfileFields, Error> {
    let name = req.name.map(|n| n.trim().to_string()).unwrap_or(base.name);
    if name.is_empty() {
        return Err(Error::Invalid("a profile needs a name".into()));
    }
    if name.chars().count() > 60 {
        return Err(Error::Invalid(
            "a profile name is at most 60 characters".into(),
        ));
    }
    let (groups, include_tools, exclude_tools) = normalize_fields(
        &req.groups.unwrap_or(base.groups),
        &req.include_tools.unwrap_or(base.include_tools),
        &req.exclude_tools.unwrap_or(base.exclude_tools),
    )?;
    let all_tools = req.all_tools.unwrap_or(base.all_tools);
    if !all_tools && groups.is_empty() && include_tools.is_empty() {
        return Err(Error::Invalid(
            "a profile needs at least one tool group or tool".into(),
        ));
    }
    Ok(McpProfileFields {
        name,
        description: req
            .description
            .map(|d| d.trim().to_string())
            .unwrap_or(base.description),
        all_tools,
        groups,
        include_tools,
        exclude_tools,
    })
}

/// `POST /mcp/profiles` (MCP Admin, person).
async fn create_profile<S: McpCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<ProfileReq>,
) -> ApiResult<(StatusCode, Json<McpProfile>)> {
    require_person(auth.as_ref().map(|a| &a.0))?;
    let fields = fields_from(req, McpProfileFields::default())?;
    let p = repo(&ctx).create(&fields, Some(&user.id)).await?;
    audit(&ctx, &user, "create", &p).await;
    Ok((StatusCode::CREATED, Json(p)))
}

/// `PATCH /mcp/profiles/{id}` (MCP Admin, person).
async fn update_profile<S: McpCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    auth: Option<Extension<AuthContext>>,
    Path(id): Path<String>,
    Json(req): Json<ProfileReq>,
) -> ApiResult<Json<McpProfile>> {
    require_person(auth.as_ref().map(|a| &a.0))?;
    let cur = repo(&ctx).get(&id).await?;
    let clean_groups = cur
        .groups
        .iter()
        .map(|g| canonical_group_id(g.trim()).to_string())
        .filter(|g| group(g).is_some())
        .collect();
    let clean_tools = |ts: &[String]| -> Vec<String> {
        ts.iter()
            .filter(|t| {
                let k = canonical(t.trim());
                !k.is_empty() && group_of(k).is_some()
            })
            .cloned()
            .collect()
    };
    let base = McpProfileFields {
        name: cur.name.clone(),
        description: cur.description.clone(),
        all_tools: cur.all_tools,
        groups: clean_groups,
        include_tools: clean_tools(&cur.include_tools),
        exclude_tools: clean_tools(&cur.exclude_tools),
    };
    let fields = fields_from(req, base)?;
    let p = repo(&ctx).update(&id, &fields).await?;
    if p != cur {
        audit(&ctx, &user, "update", &p).await;
    }
    Ok(Json(p))
}

/// `DELETE /mcp/profiles/{id}` (MCP Admin, person). Sessions that still name
/// it fall back to the remaining profiles, or the default.
async fn delete_profile<S: McpCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    auth: Option<Extension<AuthContext>>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_person(auth.as_ref().map(|a| &a.0))?;
    let p = repo(&ctx).delete(&id).await?;
    audit(&ctx, &user, "delete", &p).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /mcp/profiles/{id}/default` (MCP Admin, person).
async fn set_default<S: McpCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    auth: Option<Extension<AuthContext>>,
    Path(id): Path<String>,
) -> ApiResult<Json<McpProfile>> {
    require_person(auth.as_ref().map(|a| &a.0))?;
    let p = repo(&ctx).set_default(&id).await?;
    audit(&ctx, &user, "default", &p).await;
    Ok(Json(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(groups: &[&str], include: &[&str]) -> ProfileReq {
        ProfileReq {
            name: Some("  Mine ".into()),
            groups: Some(groups.iter().map(|s| s.to_string()).collect()),
            include_tools: Some(include.iter().map(|s| s.to_string()).collect()),
            ..Default::default()
        }
    }

    #[test]
    fn fields_are_validated_and_normalized() {
        let f = fields_from(
            req(
                &["database", "database"],
                &["otto.create_pr", "otto_create_pr"],
            ),
            McpProfileFields::default(),
        )
        .unwrap();
        assert_eq!(f.name, "Mine");
        assert_eq!(f.groups, ["database"]);
        assert_eq!(f.include_tools, ["create_pr"]);

        assert!(fields_from(req(&["nope"], &[]), McpProfileFields::default()).is_err());
        assert!(fields_from(req(&[], &["not_a_tool"]), McpProfileFields::default()).is_err());
        assert!(fields_from(req(&[], &[]), McpProfileFields::default()).is_err());
        let blank = ProfileReq {
            name: Some(" ".into()),
            ..req(&["git"], &[])
        };
        assert!(fields_from(blank, McpProfileFields::default()).is_err());
    }

    #[test]
    fn agent_and_mcp_credentials_cannot_change_profiles() {
        let user = otto_core::domain::User {
            id: "u".into(),
            username: "u".into(),
            display_name: "U".into(),
            is_root: true,
            disabled: false,
            created_at: chrono::Utc::now(),
        };
        let person = AuthContext::automation(&user, "test");
        assert!(require_person(Some(&person)).is_ok());
        assert!(require_person(None).is_ok());
        let mut agent = person.clone();
        agent.managed_session_id = Some("s".into());
        assert!(require_person(Some(&agent)).is_err());
        let mut mcp = person;
        mcp.mcp_only = true;
        assert!(require_person(Some(&mcp)).is_err());
    }
}
