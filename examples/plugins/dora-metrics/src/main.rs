//! dora-metrics — Otto runtime plugin (Rust sidecar).
//!
//! Otto spawns this with: OTTO_PLUGIN_PORT (bind here), OTTO_PLUGIN_TOKEN +
//! OTTO_HOST_API (call back for repos/agents), OTTO_PLUGIN_DATA_DIR (config).
//! Otto reverse-proxies /api/v1/plugins/dora-metrics/* to these routes.
//! All logic lives in the library (`metrics`, `suggest`, `config`, `routes`)
//! so the integration tests can drive it; this binary is just the socket loop.

use tiny_http::{Header, Method, Response, Server};

fn main() {
    let port: u16 = std::env::var("OTTO_PLUGIN_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    let server = Server::http(format!("127.0.0.1:{port}")).expect("bind plugin port");
    eprintln!("dora-metrics sidecar on :{port}");
    for mut req in server.incoming_requests() {
        let method = req.method().as_str().to_uppercase();
        let raw = req.url().to_string();
        let (path, query) = match raw.split_once('?') {
            Some((p, q)) => (p.to_string(), q.to_string()),
            None => (raw.clone(), String::new()),
        };
        let mut body = String::new();
        if matches!(req.method(), Method::Post | Method::Put) {
            let _ = req.as_reader().read_to_string(&mut body);
        }
        let (code, val) = dora_metrics::routes::handle(&method, &path, &query, &body);
        let data = serde_json::to_vec(&val).unwrap_or_default();
        let resp = Response::from_data(data)
            .with_status_code(code)
            .with_header(
                Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
            );
        let _ = req.respond(resp);
    }
}
