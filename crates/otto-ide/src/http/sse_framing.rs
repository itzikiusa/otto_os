//! Server-Sent Events (`text/event-stream`) framing helpers for IDE debug,
//! process-run, and live collaboration streams.

use std::convert::Infallible;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, HeaderName, HeaderValue};
use axum::response::Response;
use bytes::Bytes;
use futures_util::stream;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::broadcast::{self, error::RecvError};

use crate::types::{
    IdeCollabEvent, IdeCollabRoom, IdeDebugSessionSnapshot, IdeDebugState, IdeRunState,
    IdeRunStatus,
};

const SSE_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

fn sse_headers_response(body: Body) -> Response {
    let mut resp = Response::new(body);
    let headers = resp.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(
        HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    resp
}

fn format_sse_data(value: &Value) -> Bytes {
    let payload = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    Bytes::from(format!("data: {payload}\n\n"))
}

fn build_json_sse_response<T, F>(
    initial: T,
    rx: broadcast::Receiver<T>,
    latest_snapshot: F,
    stop_after: fn(&T) -> bool,
) -> Response
where
    T: Serialize + Clone + Send + 'static,
    F: Fn() -> Option<T> + Send + 'static,
{
    struct StreamState<T, F> {
        initial: Option<T>,
        rx: broadcast::Receiver<T>,
        latest_snapshot: F,
        stop_after: fn(&T) -> bool,
        done: bool,
    }

    let stream = stream::unfold(
        StreamState {
            initial: Some(initial),
            rx,
            latest_snapshot,
            stop_after,
            done: false,
        },
        |mut st| async move {
            if st.done {
                return None;
            }
            if let Some(first) = st.initial.take() {
                if (st.stop_after)(&first) {
                    st.done = true;
                }
                let val = serde_json::to_value(&first).unwrap_or_else(|_| json!({}));
                return Some((Ok::<Bytes, Infallible>(format_sse_data(&val)), st));
            }
            match tokio::time::timeout(SSE_KEEPALIVE_INTERVAL, st.rx.recv()).await {
                Ok(Ok(item)) => {
                    if (st.stop_after)(&item) {
                        st.done = true;
                    }
                    let val = serde_json::to_value(&item).unwrap_or_else(|_| json!({}));
                    Some((Ok(format_sse_data(&val)), st))
                }
                Ok(Err(RecvError::Lagged(skipped))) => {
                    let lag_frame = format!(
                        "event: lagged\ndata: {{\"lagged\":true,\"skipped\":{skipped}}}\n\n"
                    );
                    if let Some(latest) = (st.latest_snapshot)() {
                        if (st.stop_after)(&latest) {
                            st.done = true;
                        }
                        let val = serde_json::to_value(&latest).unwrap_or_else(|_| json!({}));
                        let snap_frame = format_sse_data(&val);
                        let mut combined = Vec::with_capacity(lag_frame.len() + snap_frame.len());
                        combined.extend_from_slice(lag_frame.as_bytes());
                        combined.extend_from_slice(&snap_frame);
                        Some((Ok(Bytes::from(combined)), st))
                    } else {
                        st.done = true;
                        Some((Ok(Bytes::from(lag_frame)), st))
                    }
                }
                Ok(Err(RecvError::Closed)) => None,
                Err(_) => Some((Ok(Bytes::from_static(b": keepalive\n\n")), st)),
            }
        },
    );

    sse_headers_response(Body::from_stream(stream))
}

pub(super) fn build_debug_sse_response<F>(
    initial: IdeDebugSessionSnapshot,
    rx: broadcast::Receiver<IdeDebugSessionSnapshot>,
    latest_snapshot: F,
) -> Response
where
    F: Fn() -> Option<IdeDebugSessionSnapshot> + Send + 'static,
{
    build_json_sse_response(initial, rx, latest_snapshot, |s| {
        matches!(s.state, IdeDebugState::Terminated | IdeDebugState::Error)
    })
}

pub(super) fn build_run_sse_response<F>(
    initial: IdeRunState,
    rx: broadcast::Receiver<IdeRunState>,
    latest_snapshot: F,
) -> Response
where
    F: Fn() -> Option<IdeRunState> + Send + 'static,
{
    build_json_sse_response(initial, rx, latest_snapshot, |s| {
        s.status != IdeRunStatus::Running
    })
}

pub(super) fn format_collab_event_frame(ev: &IdeCollabEvent) -> Bytes {
    let mut val = serde_json::to_value(ev).unwrap_or_else(|_| json!({}));
    if let Value::Object(ref mut map) = val {
        match ev {
            IdeCollabEvent::CursorMoved { .. } => {
                map.insert("kind".into(), Value::String("cursor".into()));
            }
            IdeCollabEvent::BufferUpdated { .. } => {
                map.insert("kind".into(), Value::String("edit".into()));
            }
            _ => {}
        }
    }
    format_sse_data(&val)
}

pub(super) fn build_collab_sse_response(
    room: IdeCollabRoom,
    rx: broadcast::Receiver<IdeCollabEvent>,
) -> Response {
    struct State {
        initial_room: Option<IdeCollabRoom>,
        rx: broadcast::Receiver<IdeCollabEvent>,
        done: bool,
    }

    let stream = stream::unfold(
        State {
            initial_room: Some(room),
            rx,
            done: false,
        },
        |mut st| async move {
            if st.done {
                return None;
            }
            if let Some(r) = st.initial_room.take() {
                let val = json!({ "type": "room_state", "room": r });
                return Some((Ok::<Bytes, Infallible>(format_sse_data(&val)), st));
            }
            match tokio::time::timeout(SSE_KEEPALIVE_INTERVAL, st.rx.recv()).await {
                Ok(Ok(ev)) => {
                    if matches!(ev, IdeCollabEvent::RoomClosed { .. }) {
                        st.done = true;
                    }
                    let frame = format_collab_event_frame(&ev);
                    Some((Ok(frame), st))
                }
                Ok(Err(RecvError::Lagged(skipped))) => Some((
                    Ok(Bytes::from(format!(
                        "event: lagged\ndata: {{\"lagged\":true,\"skipped\":{skipped}}}\n\n"
                    ))),
                    st,
                )),
                Ok(Err(RecvError::Closed)) => None,
                Err(_) => Some((Ok(Bytes::from_static(b": keepalive\n\n")), st)),
            }
        },
    );

    sse_headers_response(Body::from_stream(stream))
}
