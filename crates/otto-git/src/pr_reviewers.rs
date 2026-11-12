//! Reviewers on an existing PR — routes merged into `crate::http::router`.
//!
//! `PUT /repos/{id}/prs/{number}/reviewers` `{add, remove}` requests and
//! un-requests reviewers by name (GitHub login; Bitbucket nickname, display
//! name or uuid; GitLab username, or a current reviewer's name). Outward:
//! the forge notifies the people added, so the UI confirms who first.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::put;
use axum::{Extension, Json, Router};
use otto_core::auth::AuthUser;
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id};
use serde::Deserialize;

use crate::http::{provider_ctx, repo_ctx, ApiResult, GitCtx};

/// At most this many names per call — a reviewer edit, not a bulk import.
const MAX_NAMES: usize = 30;

/// Body of `PUT …/prs/{number}/reviewers`.
#[derive(Debug, Default, Deserialize)]
pub struct EditPrReviewersReq {
    #[serde(default)]
    pub add: Vec<String>,
    #[serde(default)]
    pub remove: Vec<String>,
}

/// Routes owned by this module.
pub fn router<S: GitCtx>() -> Router<S> {
    Router::new().route("/repos/{id}/prs/{number}/reviewers", put(edit::<S>))
}

/// Trimmed, de-duplicated (case-insensitively) names; 400 for an empty
/// edit, a name on both sides, or more than [`MAX_NAMES`].
pub(crate) fn normalize(req: EditPrReviewersReq) -> Result<(Vec<String>, Vec<String>), Error> {
    let clean = |names: Vec<String>| {
        let mut out: Vec<String> = Vec::new();
        for n in names {
            let n = n.trim().trim_start_matches('@').to_string();
            if !n.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(&n)) {
                out.push(n);
            }
        }
        out
    };
    let (add, remove) = (clean(req.add), clean(req.remove));
    if add.is_empty() && remove.is_empty() {
        return Err(Error::Invalid(
            "name at least one reviewer to add or remove".into(),
        ));
    }
    if add.len() + remove.len() > MAX_NAMES {
        return Err(Error::Invalid(format!(
            "at most {MAX_NAMES} reviewers per change"
        )));
    }
    if let Some(both) = add
        .iter()
        .find(|a| remove.iter().any(|r| r.eq_ignore_ascii_case(a)))
    {
        return Err(Error::Invalid(format!("{both} is both added and removed")));
    }
    Ok((add, remove))
}

async fn edit<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((id, number)): Path<(Id, u64)>,
    Json(req): Json<EditPrReviewersReq>,
) -> ApiResult<StatusCode> {
    let (add, remove) = normalize(req)?;
    let (repo, _) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let (provider, remote) = provider_ctx(&s, &user, &repo).await?;
    provider
        .set_reviewers(&remote, number, &add, &remove)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(add: &[&str], remove: &[&str]) -> EditPrReviewersReq {
        EditPrReviewersReq {
            add: add.iter().map(|s| s.to_string()).collect(),
            remove: remove.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn names_are_trimmed_and_deduplicated() {
        let (add, remove) = normalize(req(&[" ann ", "@Ann", "bob"], &["carl"])).unwrap();
        assert_eq!(add, vec!["ann".to_string(), "bob".to_string()]);
        assert_eq!(remove, vec!["carl".to_string()]);
    }

    #[test]
    fn empty_conflicting_or_oversized_edits_are_400() {
        assert!(matches!(
            normalize(req(&[" "], &[])),
            Err(Error::Invalid(_))
        ));
        let both = normalize(req(&["ann"], &["ANN"])).expect_err("on both sides");
        assert!(
            both.to_string().contains("both added and removed"),
            "{both}"
        );
        let many: Vec<String> = (0..31).map(|i| format!("u{i}")).collect();
        let too_many = EditPrReviewersReq {
            add: many,
            remove: Vec::new(),
        };
        assert!(matches!(normalize(too_many), Err(Error::Invalid(_))));
    }
}
