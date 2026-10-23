//! Recap owner routes: authenticate the owner, then call `otto_rooms::recap`
//! with the effective user (every archive lookup is scoped to it).
use crate::{
    auth::{require_root, CurrentAuthContext},
    ApiResult, ServerCtx,
};
use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use otto_core::api::*;
use otto_rooms::{
    auth::owner_auth,
    recap::{media, service, service::Page},
};

pub async fn get_settings(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
) -> ApiResult<Json<RecapEngineSettings>> {
    owner_auth(&auth)?;
    require_root(&auth.effective_user)?;
    Ok(Json(service::settings(&ctx).await?))
}
pub async fn put_settings(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<RecapEngineSettings>,
) -> ApiResult<Json<RecapEngineSettings>> {
    owner_auth(&auth)?;
    require_root(&auth.effective_user)?;
    Ok(Json(service::put_settings(&ctx, req).await?))
}
pub async fn capabilities(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
) -> ApiResult<Json<serde_json::Value>> {
    owner_auth(&auth)?;
    Ok(Json(service::capabilities(&ctx).await?))
}
pub async fn create(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
    Json(req): Json<CreateRecapReq>,
) -> ApiResult<Json<RecapMetadata>> {
    owner_auth(&auth)?;
    let owner = auth.effective_user.id.as_str();
    Ok(Json(service::create(&ctx, owner, &id, req).await?))
}
pub async fn list(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
) -> ApiResult<Json<Vec<RecapMetadata>>> {
    owner_auth(&auth)?;
    Ok(Json(
        service::list(&ctx, auth.effective_user.id.as_str()).await?,
    ))
}
pub async fn revision(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
) -> ApiResult<Json<RecapRevision>> {
    owner_auth(&auth)?;
    let owner = auth.effective_user.id.as_str();
    Ok(Json(service::revision(&ctx, owner, id).await?))
}
pub async fn detail(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> ApiResult<Json<RecapDetail>> {
    owner_auth(&auth)?;
    let owner = auth.effective_user.id.as_str();
    Ok(Json(service::detail(&ctx, owner, &id, page).await?))
}
pub async fn image(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path((id, image)): Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    owner_auth(&auth)?;
    let owner = auth.effective_user.id.as_str();
    let bytes = service::image(&ctx, owner, &id, image).await?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        bytes,
    ))
}
pub async fn export(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    owner_auth(&auth)?;
    let export = service::export(&ctx, auth.effective_user.id.as_str(), &id).await?;
    use futures_util::StreamExt;
    use tokio::io::AsyncReadExt;
    let tail = export.tail;
    let stream = tokio_util::io::ReaderStream::new(export.file.take(export.extent)).chain(
        futures_util::stream::once(async move {
            Ok::<_, std::io::Error>(axum::body::Bytes::from(tail))
        }),
    );
    Ok((
        [
            (header::CONTENT_TYPE, "application/x-ndjson"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=room-recap.jsonl",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        axum::body::Body::from_stream(stream),
    ))
}
pub async fn summary(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
) -> ApiResult<Json<RecapQueued>> {
    owner_auth(&auth)?;
    let owner = auth.effective_user.id.as_str();
    Ok(Json(service::summary(&ctx, owner, id).await?))
}
pub async fn cancel_summary(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    owner_auth(&auth)?;
    service::cancel_summary(&ctx, auth.effective_user.id.as_str(), &id).await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn audio(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
    Json(req): Json<RecapAudioReq>,
) -> ApiResult<Json<RecapQueued>> {
    owner_auth(&auth)?;
    let owner = auth.effective_user.id.as_str();
    Ok(Json(media::audio(&ctx, owner, id, req).await?))
}
pub async fn screen(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
    Json(req): Json<RecapScreenReq>,
) -> ApiResult<Json<RecapQueued>> {
    owner_auth(&auth)?;
    let owner = auth.effective_user.id.as_str();
    Ok(Json(media::screen(&ctx, owner, id, req).await?))
}
pub async fn gap(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
    Json(req): Json<RecapGapReq>,
) -> ApiResult<Json<RecapQueued>> {
    owner_auth(&auth)?;
    let owner = auth.effective_user.id.as_str();
    Ok(Json(media::gap(&ctx, owner, id, req).await?))
}
