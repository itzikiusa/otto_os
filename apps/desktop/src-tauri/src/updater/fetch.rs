//! The updater's only network access: the manifest and the archive, each
//! fetched from an allowed host with a hard size cap that applies BEFORE the
//! bytes are parsed or verified (the plugin's own `check`/`download` follow
//! any redirect and buffer any size — D-017).

use std::time::Duration;

use reqwest::{redirect, StatusCode, Url};

/// Where a release's manifest and assets may come from (https only).
/// `github.com/<o>/<r>/releases/latest/download/…` redirects to one of the
/// two asset hosts.
const ALLOWED_HOSTS: [&str; 3] = [
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];
/// `latest.json` is about 1 KB; the notes make it a few.
pub const MANIFEST_MAX: u64 = 256 * 1024;
/// The archive is about 125 MB today (the app bundle is ~312 MB unpacked).
pub const ARCHIVE_MAX: u64 = 512 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

#[derive(Debug, PartialEq, Eq)]
pub enum FetchError {
    /// No connection, DNS failure or a timeout.
    Offline(String),
    /// Anything else: refused host, too big, bad status.
    Refused(String),
}

/// Whether Otto may fetch from `url`: https on an allowed host and port.
pub fn allowed(url: &Url) -> bool {
    (url.scheme() == "https"
        && url.port().is_none()
        && url.host_str().is_some_and(|h| ALLOWED_HOSTS.contains(&h)))
        || test_seam(url)
}

/// Tests serve the manifest and archive from a loopback port. `cfg(test)`
/// exists only in the unit-test binary, so no build of the app — debug or
/// release — can ever accept a non-GitHub host.
#[cfg(test)]
fn test_seam(url: &Url) -> bool {
    url.scheme() == "http" && url.host_str() == Some("127.0.0.1")
}
#[cfg(not(test))]
fn test_seam(_: &Url) -> bool {
    false
}

fn client() -> Result<reqwest::Client, FetchError> {
    reqwest::Client::builder()
        .user_agent(concat!("Otto/", env!("CARGO_PKG_VERSION"), " updater"))
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        // Every hop must stay on an allowed host.
        .redirect(redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                attempt.error("too many redirects")
            } else if !allowed(attempt.url()) {
                let host = attempt.url().host_str().unwrap_or("?").to_string();
                attempt.error(format!("redirect to {host}, which is not an allowed host"))
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| FetchError::Refused(format!("could not build the HTTP client: {e}")))
}

fn net_error(e: reqwest::Error) -> FetchError {
    if e.is_connect() || e.is_timeout() {
        FetchError::Offline(e.to_string())
    } else {
        // A refused redirect lands here too; its message is in the source chain.
        let mut msg = e.to_string();
        let mut src = std::error::Error::source(&e);
        while let Some(s) = src {
            msg = format!("{msg}: {s}");
            src = s.source();
        }
        FetchError::Refused(msg)
    }
}

/// GET `url` into memory, refusing more than `max` bytes (by Content-Length
/// up front, and by counting as it streams). `progress(received, total)` runs
/// after every chunk.
pub async fn get(
    url: &Url,
    max: u64,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<Vec<u8>, FetchError> {
    if !allowed(url) {
        return Err(FetchError::Refused(format!(
            "{} is not an allowed update host",
            url.host_str().unwrap_or("?")
        )));
    }
    let mut res = client()?.get(url.clone()).send().await.map_err(net_error)?;
    if res.status() != StatusCode::OK {
        return Err(FetchError::Refused(format!(
            "{url} answered {}",
            res.status()
        )));
    }
    let total = res.content_length();
    if total.is_some_and(|n| n > max) {
        return Err(too_big(max));
    }
    let mut buf = Vec::with_capacity(total.unwrap_or(0) as usize);
    while let Some(chunk) = res.chunk().await.map_err(net_error)? {
        if buf.len() as u64 + chunk.len() as u64 > max {
            return Err(too_big(max));
        }
        buf.extend_from_slice(&chunk);
        progress(buf.len() as u64, total);
    }
    Ok(buf)
}

fn too_big(max: u64) -> FetchError {
    FetchError::Refused(format!(
        "the download is larger than {} MB",
        max / 1024 / 1024
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updater::testkit::Server;

    fn u(s: &str) -> Url {
        s.parse().unwrap()
    }

    #[test]
    fn only_https_github_hosts_are_allowed() {
        assert!(allowed(&u(
            "https://github.com/o/r/releases/latest/download/latest.json"
        )));
        assert!(allowed(&u("https://objects.githubusercontent.com/x")));
        assert!(allowed(&u(
            "https://release-assets.githubusercontent.com/x"
        )));
        assert!(!allowed(&u("http://github.com/x")), "plain http");
        assert!(!allowed(&u("https://github.com:8443/x")), "odd port");
        assert!(!allowed(&u("https://github.com.evil.example/x")));
        assert!(!allowed(&u("https://evil.example/github.com")));
        assert!(!allowed(&u("https://127.0.0.1/x")));
        assert!(
            !allowed(&u("http://localhost/x")),
            "the test seam is 127.0.0.1 only"
        );
    }

    #[tokio::test]
    async fn caps_apply_before_and_while_reading() {
        let srv = Server::start(vec![
            ("/small", 200, b"hello".to_vec(), None),
            ("/big", 200, vec![b'x'; 2048], None),
        ]);
        let got = get(&srv.url("/small"), 5, |_, _| {}).await.unwrap();
        assert_eq!(got, b"hello");
        let err = get(&srv.url("/big"), 1024, |_, _| {}).await.unwrap_err();
        assert!(
            matches!(err, FetchError::Refused(ref m) if m.contains("larger")),
            "{err:?}"
        );
        let err = get(&srv.url("/missing"), 1024, |_, _| {})
            .await
            .unwrap_err();
        assert!(
            matches!(err, FetchError::Refused(ref m) if m.contains("404")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn progress_reports_every_chunk_against_the_total() {
        let srv = Server::start(vec![("/a", 200, vec![7u8; 300_000], None)]);
        let mut last = (0, None);
        let got = get(&srv.url("/a"), ARCHIVE_MAX, |r, t| last = (r, t))
            .await
            .unwrap();
        assert_eq!(got.len(), 300_000);
        assert_eq!(last, (300_000, Some(300_000)));
    }

    #[tokio::test]
    async fn a_redirect_off_the_allowed_hosts_is_refused() {
        let srv = Server::start(vec![
            ("/hop", 302, vec![], Some("http://localhost:9/evil".into())),
            ("/ok", 200, b"fine".to_vec(), None),
        ]);
        let ok_hop = format!("{}", srv.url("/ok"));
        let srv2 = Server::start(vec![("/hop", 302, vec![], Some(ok_hop))]);
        assert_eq!(
            get(&srv2.url("/hop"), 64, |_, _| {}).await.unwrap(),
            b"fine"
        );
        let err = get(&srv.url("/hop"), 64, |_, _| {}).await.unwrap_err();
        assert!(
            matches!(err, FetchError::Refused(ref m) if m.contains("not an allowed host")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn a_refused_host_is_never_contacted_and_offline_is_told_apart() {
        let err = get(&u("https://evil.example/x"), 64, |_, _| {})
            .await
            .unwrap_err();
        assert!(matches!(err, FetchError::Refused(_)));
        // A closed loopback port: connection refused → offline.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let err = get(&u(&format!("http://127.0.0.1:{port}/x")), 64, |_, _| {})
            .await
            .unwrap_err();
        assert!(matches!(err, FetchError::Offline(_)), "{err:?}");
    }
}
