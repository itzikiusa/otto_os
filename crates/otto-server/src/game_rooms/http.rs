use crate::{auth::CurrentAuthContext, ApiResult, ServerCtx};
use axum::{
    extract::State,
    http::{header, HeaderMap},
    Json,
};
use otto_rooms::games::{self, Create, Credential, Join};

pub(super) async fn create(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(request): Json<Create>,
) -> ApiResult<(HeaderMap, Json<Credential>)> {
    crate::rooms::owner_auth(&auth)?;
    let credential = games::open(&ctx, &auth.effective_user.id, request).await?;
    Ok((private_headers(), Json(credential)))
}
pub(super) async fn join(
    State(ctx): State<ServerCtx>,
    Json(request): Json<Join>,
) -> ApiResult<(HeaderMap, Json<Credential>)> {
    let credential = ctx
        .game_rooms
        .join(&request.room_id, &request.invite, &request.name)
        .await?;
    Ok((private_headers(), Json(credential)))
}
fn private_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CACHE_CONTROL,
        "no-store".parse().expect("static header"),
    );
    headers
}
