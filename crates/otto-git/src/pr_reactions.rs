//! Reactions on PR comments — routes merged into `crate::http::router`.
//!
//! `PUT` adds the caller's reaction, `DELETE` removes it; both answer the
//! comment's reactions afterwards. GitHub reactions / GitLab award emoji;
//! Bitbucket Cloud has none (409).

use axum::extract::{Path, State};
use axum::routing::put;
use axum::{Extension, Json, Router};
use otto_core::api::{PrReaction, ReactionKey};
use otto_core::auth::AuthUser;
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id};

use crate::http::{provider_ctx, repo_ctx, ApiResult, GitCtx};

/// Routes owned by this module.
pub fn router<S: GitCtx>() -> Router<S> {
    Router::new().route(
        "/repos/{id}/prs/{number}/comments/{cid}/reactions/{key}",
        put(react::<S>).delete(unreact::<S>),
    )
}

/// The reaction a path segment names; anything else is a 400.
pub(crate) fn reaction_key(key: &str) -> Result<ReactionKey, Error> {
    ReactionKey::parse(key).ok_or_else(|| {
        Error::Invalid(format!(
            "unknown reaction {key:?} (one of: {})",
            ReactionKey::ALL.map(ReactionKey::as_str).join(", ")
        ))
    })
}

async fn set<S: GitCtx>(
    s: S,
    user: AuthUser,
    (id, number, cid, key): (Id, u64, String, String),
    on: bool,
) -> ApiResult<Json<Vec<PrReaction>>> {
    let key = reaction_key(&key)?;
    let (repo, _) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let (provider, remote) = provider_ctx(&s, &user, &repo).await?;
    Ok(Json(
        provider
            .set_reaction(&remote, number, &cid, key, on)
            .await?,
    ))
}

async fn react<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(p): Path<(Id, u64, String, String)>,
) -> ApiResult<Json<Vec<PrReaction>>> {
    set(s, user, p, true).await
}

async fn unreact<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(p): Path<(Id, u64, String, String)>,
) -> ApiResult<Json<Vec<PrReaction>>> {
    set(s, user, p, false).await
}

#[cfg(test)]
mod tests {
    use super::reaction_key;
    use otto_core::api::ReactionKey;

    #[test]
    fn path_keys_parse_and_unknown_ones_are_400() {
        assert_eq!(reaction_key("thumbs_up").unwrap(), ReactionKey::ThumbsUp);
        assert_eq!(reaction_key("eyes").unwrap(), ReactionKey::Eyes);
        let err = reaction_key("+1").expect_err("a GitHub content name is not a key");
        assert!(matches!(err, otto_core::Error::Invalid(_)), "{err}");
        assert!(reaction_key("THUMBS_UP").is_err());
        for k in ReactionKey::ALL {
            assert_eq!(
                serde_json::to_value(k).unwrap(),
                serde_json::json!(k.as_str()),
                "the path name is the serde name"
            );
        }
    }
}
