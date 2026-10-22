//! The in-app browser's HTML proxy (`/browser/proxy`): the one-shot ticket
//! store, the picker-script injection, and the capped body read. The ticket
//! mint route stays in `modules.rs` (the root-route guard looks for it there).

use super::*;

/// Picker script injected before </body>.
pub(crate) const PICKER_SCRIPT: &str = r#"<script>(function(){var on=false;function sel(el){if(!el||el===document.body||el.nodeType!==1)return 'body';var parts=[],e=el,d=0;while(e&&e.nodeType===1&&e!==document.body&&d<5){var p=e.tagName.toLowerCase();if(e.id){parts.unshift('#'+e.id);break;}var cls=[].slice.call(e.classList||[]).filter(function(c){return !/[0-9]/.test(c)&&c.length<24;}).slice(0,2);if(cls.length)p+='.'+cls.join('.');parts.unshift(p);e=e.parentElement;d++;}return parts.join(' > ');}function desc(el){var s=sel(el);var a=['aria-label','placeholder','name','href'].map(function(k){var v=el.getAttribute&&el.getAttribute(k);return v?'['+k+'="'+String(v).slice(0,60)+'"]':'';}).join('');var t=((el.textContent||'').trim()).slice(0,50);return s+a+(t?' "'+t+'"':'');}window.addEventListener('message',function(ev){if(ev.data&&ev.data.type==='otto-takeover'){on=!!ev.data.enabled;document.documentElement.style.cursor=on?'crosshair':'';}});document.addEventListener('click',function(e){if(!on)return;e.preventDefault();e.stopPropagation();try{parent.postMessage({type:'otto-element',desc:desc(e.target),x:Math.round(e.clientX),y:Math.round(e.clientY),url:location.href},'*');}catch(_){}},true);})();</script>"#;

/// CSP sent on EVERY proxy response (HTML, pass-through bytes, errors): the
/// document is sandboxed into an opaque origin — scripts run (the picker needs
/// them) but the page is never same-origin with the daemon.
pub(crate) const PROXY_CSP: &str = "sandbox allow-scripts";

/// How long a minted proxy ticket stays redeemable.
pub(crate) const PROXY_TICKET_TTL: std::time::Duration = std::time::Duration::from_secs(60);
/// Cap on outstanding tickets (each mint prunes expired ones first; this only
/// bounds a caller hammering the mint route).
pub(crate) const PROXY_TICKET_MAX: usize = 1024;

pub(crate) struct ProxyTicket {
    pub(crate) url: String,
    pub(crate) expires: std::time::Instant,
}

/// In-memory single-use ticket store. Process-local on purpose: a ticket only
/// needs to survive the ~instant between the UI minting it and the iframe
/// loading it, and a daemon restart invalidating every ticket is harmless.
pub(crate) static PROXY_TICKETS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, ProxyTicket>>,
> = std::sync::LazyLock::new(Default::default);

/// Mint a ticket for `url`, valid for `ttl`.
pub(crate) fn mint_proxy_ticket(url: &str, ttl: std::time::Duration) -> String {
    use rand::rand_core::UnwrapErr;
    use rand::rngs::SysRng;
    use rand::Rng;
    let mut bytes = [0u8; 32];
    UnwrapErr(SysRng).fill_bytes(&mut bytes);
    let ticket = hex::encode(bytes);
    let now = std::time::Instant::now();
    let mut map = PROXY_TICKETS.lock().unwrap_or_else(|p| p.into_inner());
    map.retain(|_, t| t.expires > now);
    if map.len() >= PROXY_TICKET_MAX {
        // Drop the soonest-to-expire ticket rather than refusing the mint.
        if let Some(k) = map
            .iter()
            .min_by_key(|(_, t)| t.expires)
            .map(|(k, _)| k.clone())
        {
            map.remove(&k);
        }
    }
    map.insert(
        ticket.clone(),
        ProxyTicket {
            url: url.to_owned(),
            expires: now + ttl,
        },
    );
    ticket
}

/// Redeem (and consume) a ticket for `url`. Single-use: the ticket is removed
/// whether or not it matches, so a leaked/guessed ticket can't be retried
/// against a different URL.
pub(crate) fn redeem_proxy_ticket(ticket: &str, url: &str) -> bool {
    let mut map = PROXY_TICKETS.lock().unwrap_or_else(|p| p.into_inner());
    match map.remove(ticket) {
        Some(t) => t.expires > std::time::Instant::now() && t.url == url,
        None => false,
    }
}

#[derive(serde::Deserialize)]
pub(crate) struct BrowserProxyQuery {
    pub(crate) url: Option<String>,
    pub(crate) ticket: Option<String>,
}

/// State carried by the root-level browser proxy router.
#[derive(Clone)]
pub(crate) struct BrowserProxyState {
    pub(crate) http: reqwest::Client,
}

/// Stamp the isolation headers onto every proxy response, whatever branch
/// produced it (an error page or a pass-through image is still attacker-
/// controlled content served from the daemon origin).
pub(crate) async fn browser_proxy_headers(
    mut resp: axum::response::Response,
) -> axum::response::Response {
    use axum::http::{header, HeaderValue};
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(PROXY_CSP),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

/// Root-level browser proxy router (self-authenticates via a single-use
/// `?ticket=` minted by `POST /api/v1/browser/proxy-ticket`).
pub fn browser_proxy_router() -> Router {
    // SSRF guard: the guarded resolver vets the address actually dialled (no
    // DNS rebinding after the pre-flight check) and each redirect hop is capped
    // + re-validated so an upstream 30x can't bounce the proxy inward.
    let http = otto_netguard::guarded_client_builder()
        .user_agent("Mozilla/5.0 (compatible; OttoProxy/1.0)")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("failed to build reqwest client for browser proxy");

    Router::new()
        .route("/browser/proxy", axum::routing::get(browser_proxy))
        .layer(axum::middleware::map_response(browser_proxy_headers))
        .with_state(BrowserProxyState { http })
}

/// Escape text for an HTML attribute value / text node.
pub(crate) fn proxy_html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub(crate) fn proxy_error_page(
    status: axum::http::StatusCode,
    msg: &str,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let body = format!(
        r#"<!doctype html><html><body><h2>Proxy error</h2><pre>{}</pre></body></html>"#,
        proxy_html_escape(msg)
    );
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body,
    )
        .into_response()
}

/// Inject `<base href>` (HTML-escaped — the URL is caller-supplied) after
/// `<head>` and the picker script before `</body>`.
pub(crate) fn proxy_transform_html(html: &str, url: &str) -> String {
    let base_tag = format!(r#"<base href="{}">"#, proxy_html_escape(url));
    let html = {
        // Try to find <head> (case-insensitive). `to_ascii_lowercase` keeps
        // byte offsets aligned with `html` (full Unicode lowercasing doesn't).
        let lower = html.to_ascii_lowercase();
        if let Some(pos) = lower.find("<head>") {
            let insert_at = pos + "<head>".len();
            format!("{}{}{}", &html[..insert_at], base_tag, &html[insert_at..])
        } else if let Some(pos) = lower.find("<head ") {
            // <head …> with attributes: advance to the closing >.
            if let Some(close) = lower[pos..].find('>') {
                let insert_at = pos + close + 1;
                format!("{}{}{}", &html[..insert_at], base_tag, &html[insert_at..])
            } else {
                format!("{}{}", base_tag, html)
            }
        } else {
            format!("{}{}", base_tag, html)
        }
    };
    let lower = html.to_ascii_lowercase();
    if let Some(pos) = lower.rfind("</body>") {
        format!("{}{}{}", &html[..pos], PICKER_SCRIPT, &html[pos..])
    } else {
        format!("{}{}", html, PICKER_SCRIPT)
    }
}

pub(crate) async fn browser_proxy(
    Query(q): Query<BrowserProxyQuery>,
    State(st): State<BrowserProxyState>,
) -> axum::response::Response {
    use axum::http::{HeaderValue, StatusCode};
    use axum::response::IntoResponse;

    let unauthorized = |message: &str| {
        (
            StatusCode::UNAUTHORIZED,
            axum::Json(otto_core::api::Problem {
                code: "unauthorized".into(),
                message: message.into(),
            }),
        )
            .into_response()
    };

    // --- Validate target URL ---
    let url = match q.url {
        Some(u) if !u.is_empty() => u,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(otto_core::api::Problem {
                    code: "bad_request".into(),
                    message: "missing ?url= parameter".into(),
                }),
            )
                .into_response();
        }
    };

    // --- Auth: redeem the single-use ?ticket= (bound to this exact url) ---
    let ticket = match q.ticket {
        Some(t) if !t.is_empty() => t,
        _ => return unauthorized("missing ?ticket= parameter"),
    };
    if !redeem_proxy_ticket(&ticket, &url) {
        return unauthorized("invalid, expired or already-used ticket");
    }

    // --- SSRF guard: resolve + classify before fetching ---
    if let Err(msg) = otto_netguard::check_url(&url).await {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(otto_core::api::Problem {
                code: "bad_request".into(),
                message: msg,
            }),
        )
            .into_response();
    }

    // --- Fetch upstream ---
    let upstream = match st.http.get(&url).send().await {
        Ok(r) => r,
        Err(e) => return proxy_error_page(StatusCode::BAD_GATEWAY, &e.to_string()),
    };

    // Determine content-type before consuming the body.
    let content_type = upstream
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();

    let is_html = content_type.contains("text/html");

    if !is_html {
        // Stream bytes through with the upstream content-type (the response
        // layer still stamps the sandbox CSP + nosniff on it).
        let ct_val = HeaderValue::from_str(&content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
        let bytes = match read_capped(upstream, PROXY_BODY_CAP).await {
            Ok(b) => b,
            Err(e) => return proxy_error_page(StatusCode::BAD_GATEWAY, &e),
        };
        return ([(axum::http::header::CONTENT_TYPE, ct_val)], bytes).into_response();
    }

    // --- HTML: read, transform, return ---
    let html = match read_capped(upstream, PROXY_BODY_CAP).await {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => return proxy_error_page(StatusCode::BAD_GATEWAY, &e),
    };

    (
        [(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        )],
        proxy_transform_html(&html, &url),
    )
        .into_response()
}

/// Most bytes the take-over proxy buffers from one upstream response (S11-10).
pub(crate) const PROXY_BODY_CAP: usize = 20 * 1024 * 1024;

/// Read an upstream body chunk by chunk, refusing it once it passes `cap` —
/// a ticketed take-over of a multi-GB URL must never be buffered whole in
/// daemon memory (the 15 s timeout alone does not bound the size).
pub(crate) async fn read_capped(
    mut resp: reqwest::Response,
    cap: usize,
) -> std::result::Result<axum::body::Bytes, String> {
    if resp.content_length().is_some_and(|n| n > cap as u64) {
        return Err(format!("page too large (over {} MB)", cap / (1024 * 1024)));
    }
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        if buf.len() + chunk.len() > cap {
            return Err(format!("page too large (over {} MB)", cap / (1024 * 1024)));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf.into())
}

#[cfg(test)]
mod proxy_cap_tests {
    /// S11-10: a body over the cap is refused (with or without a declared
    /// length); one under it reads in full.
    #[tokio::test]
    async fn upstream_body_over_the_cap_is_refused() {
        use axum::routing::get;
        let app = axum::Router::new()
            .route("/big", get(|| async { vec![b'x'; 4096] }))
            .route(
                "/chunked",
                get(|| async {
                    let parts = (0..8).map(|_| Ok::<_, std::io::Error>(vec![b'y'; 1024]));
                    axum::body::Body::from_stream(futures_util::stream::iter(parts))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let get = |p: &str| reqwest::get(format!("http://{addr}{p}"));
        assert!(super::read_capped(get("/big").await.unwrap(), 1024)
            .await
            .is_err());
        assert!(super::read_capped(get("/chunked").await.unwrap(), 1024)
            .await
            .is_err());
        assert_eq!(
            super::read_capped(get("/big").await.unwrap(), 8192)
                .await
                .unwrap()
                .len(),
            4096
        );
    }
}

#[cfg(test)]
mod browser_proxy_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    fn assert_isolation_headers(resp: &axum::response::Response) {
        let h = resp.headers();
        assert_eq!(h[header::CONTENT_SECURITY_POLICY], "sandbox allow-scripts");
        assert!(!h[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .contains("allow-same-origin"));
        assert_eq!(h[header::REFERRER_POLICY], "no-referrer");
        assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    }

    #[tokio::test]
    async fn every_proxy_response_carries_sandbox_csp() {
        let app = browser_proxy_router();
        // Missing url, missing ticket, bogus ticket, and an SSRF-refused
        // (loopback) target all still get the isolation headers.
        let t = mint_proxy_ticket("http://127.0.0.1:7700/", PROXY_TICKET_TTL);
        for uri in [
            "/browser/proxy".to_string(),
            "/browser/proxy?url=https%3A%2F%2Fexample.com%2F".to_string(),
            "/browser/proxy?url=https%3A%2F%2Fexample.com%2F&ticket=nope".to_string(),
            format!("/browser/proxy?url=http%3A%2F%2F127.0.0.1%3A7700%2F&ticket={t}"),
        ] {
            let resp = app
                .clone()
                .oneshot(Request::get(&uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert!(!resp.status().is_success(), "{uri} -> {}", resp.status());
            assert_isolation_headers(&resp);
        }
    }

    #[tokio::test]
    async fn bearer_token_param_is_no_longer_accepted() {
        let resp = browser_proxy_router()
            .oneshot(
                Request::get("/browser/proxy?url=https%3A%2F%2Fexample.com%2F&token=anything")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn ticket_is_single_use() {
        let t = mint_proxy_ticket("https://example.com/a", PROXY_TICKET_TTL);
        assert!(redeem_proxy_ticket(&t, "https://example.com/a"));
        assert!(!redeem_proxy_ticket(&t, "https://example.com/a"));
    }

    #[test]
    fn ticket_is_bound_to_its_url_and_burned_on_mismatch() {
        let t = mint_proxy_ticket("https://example.com/a", PROXY_TICKET_TTL);
        assert!(!redeem_proxy_ticket(&t, "https://evil.example/"));
        // Burned by the failed attempt — can't be retried with the right url.
        assert!(!redeem_proxy_ticket(&t, "https://example.com/a"));
    }

    #[test]
    fn expired_ticket_is_rejected() {
        let t = mint_proxy_ticket("https://example.com/a", std::time::Duration::ZERO);
        assert!(!redeem_proxy_ticket(&t, "https://example.com/a"));
    }

    #[test]
    fn base_href_is_html_escaped() {
        let url = r#"https://x.example/"><script>alert(1)</script>"#;
        let out = proxy_transform_html("<html><head></head><body></body></html>", url);
        assert!(!out.contains("<script>alert(1)"), "{out}");
        assert!(out.contains(
            r#"<base href="https://x.example/&quot;&gt;&lt;script&gt;alert(1)&lt;/script&gt;">"#
        ));
        assert!(out.contains("otto-element"), "picker still injected");
    }
}

// ---------------------------------------------------------------------------
// Assembly
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// DB Explorer: "examine schema / result with an agent"
// ---------------------------------------------------------------------------
