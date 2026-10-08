//! Exercise the mounted auth boundary and actual WebSocket relay, without owner data.
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::{
    tungstenite::{client::IntoClientRequest, Message},
    MaybeTlsStream, WebSocketStream,
};
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
async fn connect(
    origin: &str,
    id: &str,
    token: &str,
) -> Result<Socket, tokio_tungstenite::tungstenite::Error> {
    let mut request = format!("{}/ws/game-rooms/{id}", origin.replacen("http:", "ws:", 1))
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        format!("otto-game, {token}").parse().unwrap(),
    );
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(socket, _)| socket)
}
async fn event(socket: &mut Socket, kind: &str) -> Value {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Message::Text(text) = socket.next().await.unwrap().unwrap() {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value["type"] == kind {
                    return value;
                }
            }
        }
    })
    .await
    .expect("event deadline")
}
async fn send(socket: &mut Socket, value: Value) {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}
#[tokio::test]
async fn public_join_and_socket_never_grant_owner_authority() {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .unwrap();
    let pool: otto_state::DbPool = pool.into();
    let ctx = crate::ServerCtx::for_tests(&pool, tmp.path()).await;
    let owner = otto_state::UsersRepo::new(pool.clone())
        .create("owner", "unused", "Host", true)
        .await
        .unwrap();
    let auth = otto_rbac::AuthRepo::new(pool.clone());
    let owner_token = auth.issue(&owner.id).await.unwrap();
    let mcp_token = auth.issue_mcp_token(&owner.id, None).await.unwrap();
    let app = crate::build_router(ctx, vec![], vec![]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    let create = format!("{origin}/api/v1/game-rooms");
    let body = json!({"name":"Host","config":{"game":"shooter","map":"station"}});
    assert_eq!(
        client
            .post(&create)
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .post(&create)
            .bearer_auth(&mcp_token)
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let response = client
        .post(&create)
        .bearer_auth(&owner_token)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let host: Value = response.json().await.unwrap();
    let id = host["room_id"].as_str().unwrap();
    let htoken = host["token"].as_str().unwrap();
    let mut hs = connect(&origin, id, htoken).await.unwrap();
    event(&mut hs, "state").await;
    let join = format!("{origin}/api/v1/game-room-join");
    let join_body = json!({"room_id":id,"invite":host["invite"],"name":"Guest"});
    let response = client.post(&join).json(&join_body).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let guest: Value = response.json().await.unwrap();
    let gtoken = guest["token"].as_str().unwrap();
    assert_eq!(
        client
            .post(&join)
            .json(&join_body)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .post(&create)
            .bearer_auth(gtoken)
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert!(connect(&origin, id, &owner_token).await.is_err());
    assert!(connect(&origin, id, host["invite"].as_str().unwrap())
        .await
        .is_err());
    let mut gs = connect(&origin, id, gtoken).await.unwrap();
    let state = event(&mut gs, "state").await;
    let generation = state["room"]["generation"].as_u64().unwrap();
    send(&mut hs, json!({"type":"ready","ready":true})).await;
    send(&mut gs, json!({"type":"ready","ready":true})).await;
    loop {
        let state = event(&mut hs, "state").await;
        if state["room"]["members"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["ready"] == true)
        {
            break;
        }
    }
    send(
        &mut gs,
        json!({"type":"start","round":1,"generation":generation}),
    )
    .await;
    assert!(event(&mut gs, "error").await["message"]
        .as_str()
        .unwrap()
        .contains("host"));
    send(
        &mut hs,
        json!({"type":"start","round":1,"generation":generation}),
    )
    .await;
    loop {
        if event(&mut gs, "state").await["room"]["phase"] == "playing" {
            break;
        }
    }
    send(
        &mut gs,
        json!({"type":"input","round":1,"generation":generation,"seq":1,"data":{"forward":1}}),
    )
    .await;
    assert_eq!(event(&mut hs, "input").await["data"]["forward"], 1);
    send(
        &mut hs,
        json!({"type":"snapshot","round":1,"generation":generation,"seq":1,"data":{"tick":99}}),
    )
    .await;
    assert_eq!(event(&mut gs, "snapshot").await["data"]["tick"], 99);
    send(
        &mut hs,
        json!({"type":"finish","round":1,"generation":generation,"result":{"winner":0}}),
    )
    .await;
    assert_eq!(event(&mut gs, "finished").await["result"]["winner"], 0);
    send(&mut hs, json!({"type":"rematch"})).await;
    send(&mut gs, json!({"type":"rematch"})).await;
    loop {
        let state = event(&mut hs, "state").await;
        if state["room"]["round"] == 2 {
            assert_eq!(state["room"]["phase"], "lobby");
            break;
        }
    }
    send(&mut gs, json!({"type":"leave"})).await;
    event(&mut hs, "closed").await;
    task.abort();
}
