use super::{
    registry::{SharedRoom, RATE_LIMITED},
    types::*,
};
use crate::rooms::capability::socket_token;
use crate::{ApiResult, ServerCtx};
use axum::{
    extract::{
        ws::{Message, WebSocket},
        Path, State, WebSocketUpgrade,
    },
    http::{HeaderMap, Uri},
    response::Response,
};
use otto_core::{Error, Result};
use std::time::{Duration, Instant};

pub(super) async fn upgrade(
    State(ctx): State<ServerCtx>,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    ws: WebSocketUpgrade,
) -> ApiResult<Response> {
    if uri.query().is_some() {
        return Err(Error::Unauthorized.into());
    }
    let token = socket_token(&headers, "otto-game")?;
    let member = ctx.game_rooms.authenticate(&id, &token).await?;
    let shared = ctx.game_rooms.get(&id).await?;
    Ok(ws
        .protocols(["otto-game"])
        .max_message_size(20480)
        .max_frame_size(20480)
        .on_upgrade(move |socket| run(shared, member, socket)))
}
async fn send(socket: &mut WebSocket, event: &Event) -> Result<()> {
    let text = serde_json::to_string(event).map_err(|e| Error::Internal(e.to_string()))?;
    tokio::time::timeout(
        Duration::from_secs(2),
        socket.send(Message::Text(text.into())),
    )
    .await
    .map_err(|_| Error::Conflict("Game connection too slow".into()))?
    .map_err(|_| Error::Conflict("Game connection closed".into()))
}
async fn run(shared: SharedRoom, index: usize, mut socket: WebSocket) {
    let (mut rx, connection) = {
        let mut room = shared.lock().await;
        let rx = room.tx.subscribe();
        let connection = match room.connect(index) {
            Ok(connection) => connection,
            Err(error) => {
                drop(room);
                let _ = send(
                    &mut socket,
                    &Event::Error {
                        message: error.to_string(),
                    },
                )
                .await;
                return;
            }
        };
        (rx, connection)
    };
    let mut heartbeat = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            // FIFO broadcast establishes generation state before stream events.
            notice=rx.recv()=>match notice {
                Ok(notice)=>if notice.to.includes(index) {
                    let closed=matches!(notice.event,Event::Closed{..});
                    if send(&mut socket,&notice.event).await.is_err() || closed {break;}
                },
                Err(_)=>{let _=send(&mut socket,&Event::Error{message:"Connection fell behind; reconnect to the game".into()}).await;break;}
            },
            frame=socket.recv()=>match frame {
                Some(Ok(Message::Text(text)))=>{
                    heartbeat=Instant::now();
                    let result=match serde_json::from_str::<Command>(&text) {
                        Ok(command)=>shared.lock().await.apply(index,connection,command),
                        Err(_)=>Err(Error::Invalid("Invalid game command".into())),
                    };
                    if let Err(error)=result {
                        let fatal=matches!(&error,Error::Unauthorized) || error.to_string().contains(RATE_LIMITED);
                        let _=send(&mut socket,&Event::Error{message:error.to_string()}).await;
                        // Invalid protocol/rate traffic cannot occupy a live socket indefinitely.
                        if fatal || serde_json::from_str::<Command>(&text).is_err() {break;}
                    }
                }
                Some(Ok(Message::Pong(_)))=>{heartbeat=Instant::now();}
                Some(Ok(Message::Ping(_)))=>{}
                _=>break,
            },
            _=tick.tick()=>if heartbeat.elapsed()>Duration::from_secs(15) {break;},
        }
    }
    shared.lock().await.disconnect(index, connection);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn socket_requires_exact_capability_protocol_not_owner_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer owner".parse().unwrap());
        assert!(socket_token(&headers, "otto-game").is_err());
        headers.insert(
            "sec-websocket-protocol",
            format!("otto-game, {}", "a".repeat(43)).parse().unwrap(),
        );
        assert_eq!(socket_token(&headers, "otto-game").unwrap(), "a".repeat(43));
        headers.append("sec-websocket-protocol", "other".parse().unwrap());
        assert!(socket_token(&headers, "otto-game").is_err());
    }
}
