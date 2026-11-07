//! Human-only composer persistence; never returns another person's draft.
use super::*;
use base64::Engine;
use otto_core::runtime_drafts::{RuntimeDraftContent, RuntimeDraftRecord, SaveRuntimeDraftReq};

pub(super) async fn get<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<Id>,
) -> ApiResult<Json<RuntimeDraftRecord>> {
    require_human(&ctx, &auth)?;
    let session = authorized(&ctx, &user, &id).await?;
    Ok(Json(
        ctx.agent_runtime()
            .repo()
            .composer_draft(&user.id, &session.workspace_id, &id)
            .await?,
    ))
}
pub(super) async fn put<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<Id>,
    Json(req): Json<SaveRuntimeDraftReq>,
) -> ApiResult<Json<RuntimeDraftRecord>> {
    require_human(&ctx, &auth)?;
    let session = authorized(&ctx, &user, &id).await?;
    ctx.roles()
        .check(&user, &session.workspace_id, WorkspaceRole::Editor)
        .await?;
    validate(&req.draft)?;
    Ok(Json(
        ctx.agent_runtime()
            .repo()
            .save_composer_draft(
                &user.id,
                &session.workspace_id,
                &id,
                req.revision,
                &req.draft,
            )
            .await?,
    ))
}
fn validate(draft: &RuntimeDraftContent) -> otto_core::Result<()> {
    if draft.text.len() > 1024 * 1024 || draft.images.len() > 12 || draft.context.len() > 32 {
        return Err(Error::PayloadTooLarge(
            "Draft allows 1 MiB text, 12 attachments and 32 context chips.".into(),
        ));
    }
    if draft.model.as_ref().is_some_and(|v| v.len() > 256)
        || draft.effort.as_ref().is_some_and(|v| {
            !["low", "medium", "high", "xhigh", "max", "ultra"].contains(&v.as_str())
        })
    {
        return Err(Error::Invalid("Invalid model or effort selection.".into()));
    }
    for item in &draft.context {
        if item.id.len() > 512
            || item.label.len() > 1024
            || item.text.len() > 128 * 1024
            || item.url.as_ref().is_some_and(|s| s.len() > 4096)
            || item.path.as_ref().is_some_and(|s| s.len() > 4096)
            || item.author.as_ref().is_some_and(|s| s.len() > 1024)
        {
            return Err(Error::PayloadTooLarge(
                "Review context is too large. Select a smaller excerpt.".into(),
            ));
        }
    }
    for image in &draft.images {
        if image.name.len() > 1024
            || image.data_base64.len() > 4 * (10_usize * 1024 * 1024).div_ceil(3)
        {
            return Err(Error::PayloadTooLarge(
                "Draft attachment exceeds its size limit.".into(),
            ));
        }
        if ![
            "image/png",
            "image/jpeg",
            "image/gif",
            "image/webp",
            "application/pdf",
        ]
        .contains(&image.mime.as_str())
        {
            return Err(Error::UnsupportedMedia(
                "Draft attachments support PNG, JPEG, GIF, WebP and PDF.".into(),
            ));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&image.data_base64)
            .map_err(|_| Error::Invalid("Draft attachment has invalid base64.".into()))?;
        if bytes.len() > 10 * 1024 * 1024 {
            return Err(Error::PayloadTooLarge(
                "Draft attachment exceeds 10 MiB.".into(),
            ));
        }
        let valid = match image.mime.as_str() {
            "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
            "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
            "image/webp" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
            "application/pdf" => bytes.starts_with(b"%PDF-"),
            _ => false,
        };
        if !valid {
            return Err(Error::Invalid(
                "Draft attachment content does not match its type.".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::agent_runtime::PromptFile;
    #[test]
    fn drafts_reject_malformed_and_mislabeled_attachments() {
        let mut draft = RuntimeDraftContent::default();
        draft.images.push(PromptFile {
            name: "a.png".into(),
            mime: "image/png".into(),
            data_base64: "not base64!".into(),
        });
        assert!(matches!(validate(&draft), Err(Error::Invalid(_))));
        draft.images[0].data_base64 = base64::engine::general_purpose::STANDARD.encode(b"%PDF-1.7");
        assert!(matches!(validate(&draft), Err(Error::Invalid(_))));
        draft.images[0].mime = "application/pdf".into();
        assert!(validate(&draft).is_ok());
        draft.images[0].mime = "image/svg+xml".into();
        assert!(matches!(validate(&draft), Err(Error::UnsupportedMedia(_))));
    }
}
