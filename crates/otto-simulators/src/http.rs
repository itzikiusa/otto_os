//! Axum REST router for `/simulators/*` (nested under `/api/v1` by `otto-server`).

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use base64::Engine;
use otto_core::api::Problem;
use serde::Deserialize;

use crate::types::{
    AppearanceReq, BootSimDeviceReq, CreatePortForwardReq, CreateSimDeviceReq, DeviceState,
    InputAction, InstallAppReq, LaunchAppReq, OpenSimUrlReq, OrientationReq, Platform, PortsResp,
    RestartSimDeviceReq, ScreenshotEncoding, ScreenshotFormat, ScreenshotJsonResp, SetClipboardReq,
    SimInstallReq, StatusBarReq, TerminateAppReq,
};
use crate::SimulatorsCtx;

/// All `/simulators/*` REST routes (handler-relative; mounted under `/api/v1`).
pub fn api_router<S: SimulatorsCtx>() -> Router<S> {
    Router::new()
        .route("/simulators/status", get(get_status::<S>))
        .route("/simulators/catalog", get(get_catalog::<S>))
        .route("/simulators/install/status", get(get_install_status::<S>))
        .route("/simulators/install", post(start_install::<S>))
        .route("/simulators/install/cancel", post(cancel_install::<S>))
        .route(
            "/simulators/devices",
            get(list_devices::<S>).post(create_device::<S>),
        )
        .route(
            "/simulators/devices/{id}",
            get(get_device::<S>).delete(delete_device::<S>),
        )
        .route("/simulators/devices/{id}/boot", post(boot_device::<S>))
        .route(
            "/simulators/devices/{id}/shutdown",
            post(shutdown_device::<S>),
        )
        .route(
            "/simulators/devices/{id}/restart",
            post(restart_device::<S>),
        )
        .route("/simulators/devices/{id}/open-url", post(open_url::<S>))
        .route("/simulators/devices/{id}/screenshot", get(screenshot::<S>))
        .route("/simulators/devices/{id}/logs", get(recent_logs::<S>))
        .route("/simulators/devices/{id}/input", post(send_input::<S>))
        .route(
            "/simulators/devices/{id}/orientation",
            post(set_orientation::<S>),
        )
        .route(
            "/simulators/devices/{id}/appearance",
            post(set_appearance::<S>),
        )
        .route(
            "/simulators/devices/{id}/status-bar",
            post(set_status_bar::<S>),
        )
        .route(
            "/simulators/devices/{id}/clipboard",
            get(get_clipboard::<S>).post(set_clipboard::<S>),
        )
        .route(
            "/simulators/devices/{id}/apps/install",
            post(install_app::<S>),
        )
        .route(
            "/simulators/devices/{id}/apps/launch",
            post(launch_app::<S>),
        )
        .route(
            "/simulators/devices/{id}/apps/terminate",
            post(terminate_app::<S>),
        )
        .route(
            "/simulators/devices/{id}/ports",
            get(list_ports::<S>).post(add_port_forward::<S>),
        )
        .route(
            "/simulators/devices/{id}/ports/{remote_port}",
            delete(remove_port_forward::<S>),
        )
}

fn err_resp(err: otto_core::Error) -> Response {
    let status = match &err {
        otto_core::Error::NotFound(_) => StatusCode::NOT_FOUND,
        otto_core::Error::Invalid(_) => StatusCode::BAD_REQUEST,
        otto_core::Error::Conflict(_) => StatusCode::CONFLICT,
        otto_core::Error::Forbidden(_) => StatusCode::FORBIDDEN,
        otto_core::Error::Unauthorized => StatusCode::UNAUTHORIZED,
        otto_core::Error::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
        otto_core::Error::UnsupportedMedia(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
        otto_core::Error::Upstream(_) => StatusCode::BAD_GATEWAY,
        otto_core::Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(Problem {
            code: err.code().to_string(),
            message: err.to_string(),
        }),
    )
        .into_response()
}

#[derive(Debug, Default, Deserialize)]
struct StatusQuery {
    #[serde(default)]
    refresh: bool,
}

async fn get_status<S: SimulatorsCtx>(
    State(ctx): State<S>,
    Query(q): Query<StatusQuery>,
) -> Response {
    Json(ctx.simulators().status(q.refresh).await).into_response()
}

async fn get_install_status<S: SimulatorsCtx>(State(ctx): State<S>) -> Response {
    Json(ctx.simulators().install_status().await).into_response()
}

async fn start_install<S: SimulatorsCtx>(
    State(ctx): State<S>,
    Json(req): Json<SimInstallReq>,
) -> Response {
    match ctx.simulators().start_install(req).await {
        Ok(st) => (StatusCode::ACCEPTED, Json(st)).into_response(),
        Err(e) => err_resp(e),
    }
}

async fn cancel_install<S: SimulatorsCtx>(State(ctx): State<S>) -> Response {
    Json(ctx.simulators().cancel_install().await).into_response()
}

#[derive(Debug, Default, Deserialize)]
struct CatalogQuery {
    #[serde(default)]
    platform: Option<Platform>,
}

async fn get_catalog<S: SimulatorsCtx>(
    State(ctx): State<S>,
    Query(q): Query<CatalogQuery>,
) -> Response {
    match ctx.simulators().catalog(q.platform).await {
        Ok(c) => Json(c).into_response(),
        Err(e) => err_resp(e),
    }
}

#[derive(Debug, Default, Deserialize)]
struct DevicesQuery {
    #[serde(default)]
    platform: Option<Platform>,
    #[serde(default)]
    state: Option<DeviceState>,
    #[serde(default)]
    refresh: bool,
}

async fn list_devices<S: SimulatorsCtx>(
    State(ctx): State<S>,
    Query(q): Query<DevicesQuery>,
) -> Response {
    match ctx
        .simulators()
        .list_devices(q.platform, q.state, q.refresh)
        .await
    {
        Ok(r) => Json(r).into_response(),
        Err(e) => err_resp(e),
    }
}

async fn create_device<S: SimulatorsCtx>(
    State(ctx): State<S>,
    Json(req): Json<CreateSimDeviceReq>,
) -> Response {
    match ctx.simulators().create_device(req).await {
        Ok(dev) => (StatusCode::CREATED, Json(dev)).into_response(),
        Err(e) => err_resp(e),
    }
}

async fn get_device<S: SimulatorsCtx>(State(ctx): State<S>, Path(id): Path<String>) -> Response {
    match ctx.simulators().get_device(&id).await {
        Ok(dev) => Json(dev).into_response(),
        Err(e) => err_resp(e),
    }
}

async fn delete_device<S: SimulatorsCtx>(State(ctx): State<S>, Path(id): Path<String>) -> Response {
    match ctx.simulators().delete_device(&id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err_resp(e),
    }
}

async fn boot_device<S: SimulatorsCtx>(
    State(ctx): State<S>,
    Path(id): Path<String>,
    body: Option<Json<BootSimDeviceReq>>,
) -> Response {
    let req = body.map(|b| b.0).unwrap_or_default();
    match ctx.simulators().boot_device(&id, &req).await {
        Ok(dev) => Json(dev).into_response(),
        Err(e) => err_resp(e),
    }
}

async fn shutdown_device<S: SimulatorsCtx>(
    State(ctx): State<S>,
    Path(id): Path<String>,
) -> Response {
    match ctx.simulators().shutdown_device(&id).await {
        Ok(dev) => Json(dev).into_response(),
        Err(e) => err_resp(e),
    }
}

async fn restart_device<S: SimulatorsCtx>(
    State(ctx): State<S>,
    Path(id): Path<String>,
    body: Option<Json<RestartSimDeviceReq>>,
) -> Response {
    let req = body.map(|b| b.0).unwrap_or_default();
    match ctx.simulators().restart_device(&id, &req).await {
        Ok(dev) => Json(dev).into_response(),
        Err(e) => err_resp(e),
    }
}

async fn open_url<S: SimulatorsCtx>(
    State(ctx): State<S>,
    Path(id): Path<String>,
    Json(req): Json<OpenSimUrlReq>,
) -> Response {
    match ctx
        .simulators()
        .open_url(&id, &req.url, req.auto_forward_localhost.unwrap_or(true))
        .await
    {
        Ok(r) => Json(r).into_response(),
        Err(e) => err_resp(e),
    }
}

#[derive(Debug, Default, Deserialize)]
struct ScreenshotQuery {
    #[serde(default)]
    format: ScreenshotFormat,
    #[serde(default)]
    encoding: ScreenshotEncoding,
}

async fn screenshot<S: SimulatorsCtx>(
    State(ctx): State<S>,
    Path(id): Path<String>,
    Query(q): Query<ScreenshotQuery>,
) -> Response {
    let shot = match ctx.simulators().screenshot(&id, q.format).await {
        Ok(s) => s,
        Err(e) => return err_resp(e),
    };
    if q.encoding == ScreenshotEncoding::Base64 {
        let encoded = base64::engine::general_purpose::STANDARD.encode(&shot.bytes);
        return Json(ScreenshotJsonResp {
            mime: shot.mime,
            width: shot.width,
            height: shot.height,
            base64: encoded,
            captured_at: shot.captured_at,
        })
        .into_response();
    }
    let mut resp = (StatusCode::OK, shot.bytes).into_response();
    if let Ok(v) = HeaderValue::from_str(&shot.mime) {
        resp.headers_mut().insert(header::CONTENT_TYPE, v);
    }
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

#[derive(Debug, Default, Deserialize)]
struct RecentLogsQuery {
    #[serde(default)]
    filter: Option<String>,
    #[serde(default)]
    level: Option<String>,
    #[serde(default)]
    lines: Option<usize>,
}

async fn recent_logs<S: SimulatorsCtx>(
    State(ctx): State<S>,
    Path(id): Path<String>,
    Query(q): Query<RecentLogsQuery>,
) -> Response {
    let logs_q = crate::ws::LogsQuery {
        filter: q.filter,
        level: q.level,
    };
    let (filter, min_level) = match logs_q.validate() {
        Ok(v) => v,
        Err(e) => return err_resp(e),
    };
    let max_lines = q.lines.unwrap_or(100).clamp(1, 500);
    match ctx
        .simulators()
        .recent_logs(&id, filter.as_deref(), min_level, max_lines)
        .await
    {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => err_resp(e),
    }
}

async fn send_input<S: SimulatorsCtx>(
    State(ctx): State<S>,
    Path(id): Path<String>,
    Json(req): Json<InputAction>,
) -> Response {
    match ctx.simulators().send_input(&id, &req).await {
        Ok(r) => Json(r).into_response(),
        Err(e) => err_resp(e),
    }
}

async fn set_orientation<S: SimulatorsCtx>(
    State(ctx): State<S>,
    Path(id): Path<String>,
    Json(req): Json<OrientationReq>,
) -> Response {
    match ctx.simulators().set_orientation(&id, req.orientation).await {
        Ok(r) => Json(r).into_response(),
        Err(e) => err_resp(e),
    }
}

async fn set_appearance<S: SimulatorsCtx>(
    State(ctx): State<S>,
    Path(id): Path<String>,
    Json(req): Json<AppearanceReq>,
) -> Response {
    match ctx.simulators().set_appearance(&id, req.appearance).await {
        Ok(r) => Json(r).into_response(),
        Err(e) => err_resp(e),
    }
}

async fn set_status_bar<S: SimulatorsCtx>(
    State(ctx): State<S>,
    Path(id): Path<String>,
    Json(req): Json<StatusBarReq>,
) -> Response {
    match ctx.simulators().set_status_bar(&id, &req).await {
        Ok(r) => Json(r).into_response(),
        Err(e) => err_resp(e),
    }
}

async fn get_clipboard<S: SimulatorsCtx>(State(ctx): State<S>, Path(id): Path<String>) -> Response {
    match ctx.simulators().get_clipboard(&id).await {
        Ok(r) => Json(r).into_response(),
        Err(e) => err_resp(e),
    }
}

async fn set_clipboard<S: SimulatorsCtx>(
    State(ctx): State<S>,
    Path(id): Path<String>,
    Json(req): Json<SetClipboardReq>,
) -> Response {
    match ctx.simulators().set_clipboard(&id, &req.text).await {
        Ok(r) => Json(r).into_response(),
        Err(e) => err_resp(e),
    }
}

async fn install_app<S: SimulatorsCtx>(
    State(ctx): State<S>,
    Path(id): Path<String>,
    Json(req): Json<InstallAppReq>,
) -> Response {
    match ctx.simulators().install_app(&id, &req.path).await {
        Ok(r) => Json(r).into_response(),
        Err(e) => err_resp(e),
    }
}

async fn launch_app<S: SimulatorsCtx>(
    State(ctx): State<S>,
    Path(id): Path<String>,
    Json(req): Json<LaunchAppReq>,
) -> Response {
    match ctx
        .simulators()
        .launch_app(&id, &req.app_id, &req.args)
        .await
    {
        Ok(r) => Json(r).into_response(),
        Err(e) => err_resp(e),
    }
}

async fn terminate_app<S: SimulatorsCtx>(
    State(ctx): State<S>,
    Path(id): Path<String>,
    Json(req): Json<TerminateAppReq>,
) -> Response {
    match ctx.simulators().terminate_app(&id, &req.app_id).await {
        Ok(r) => Json(r).into_response(),
        Err(e) => err_resp(e),
    }
}

async fn list_ports<S: SimulatorsCtx>(State(ctx): State<S>, Path(id): Path<String>) -> Response {
    match ctx.simulators().list_ports(&id).await {
        Ok(ports) => Json(PortsResp { ports }).into_response(),
        Err(e) => err_resp(e),
    }
}

async fn add_port_forward<S: SimulatorsCtx>(
    State(ctx): State<S>,
    Path(id): Path<String>,
    Json(req): Json<CreatePortForwardReq>,
) -> Response {
    match ctx
        .simulators()
        .add_port_forward(&id, req.remote_port, req.local_port)
        .await
    {
        Ok(pf) => (StatusCode::CREATED, Json(pf)).into_response(),
        Err(e) => err_resp(e),
    }
}

async fn remove_port_forward<S: SimulatorsCtx>(
    State(ctx): State<S>,
    Path((id, remote_port)): Path<(String, u16)>,
) -> Response {
    match ctx.simulators().remove_port_forward(&id, remote_port).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err_resp(e),
    }
}
