//! Per-(workspace, user) cookie jars and the pooled / per-settings outbound clients.

use super::*;

/// The cookie-jar scope of one user in one workspace ([`jar_scope`]): the
/// key every client / jar below is cached under.
pub fn jar_scope(wid: &Id, user_id: &Id) -> Id {
    format!("{wid}\u{1f}{user_id}")
}

/// Per-(WORKSPACE, USER) cookie jars (captures Set-Cookie and resends on
/// matching requests, so login/session flows just work). Keyed by
/// [`jar_scope`]: cookies captured in workspace A are never replayed for
/// workspace B, and on a shared workspace user A's login session is never
/// replayed on — or listed to — user B (S6-19). Automation runs use their
/// actor's jar. In-memory per daemon run, like the old global; a jar no
/// client holds and nobody used for [`JAR_IDLE_TTL`] is dropped (S6-310 —
/// per-user keys made the map grow with every user × workspace).
pub fn cookie_jar(scope: &Id) -> Arc<reqwest_cookie_store::CookieStoreMutex> {
    static JARS: OnceLock<StdMutex<JarMap>> = OnceLock::new();
    let jars = JARS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut map = jars.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    sweep_idle_jars(&mut map, now);
    let (jar, used) = map.entry(scope.clone()).or_insert_with(|| {
        (
            Arc::new(reqwest_cookie_store::CookieStoreMutex::new(
                reqwest_cookie_store::CookieStore::default(),
            )),
            now,
        )
    });
    *used = now;
    jar.clone()
}

pub(crate) type CookieJar = reqwest_cookie_store::CookieStoreMutex;
/// Scope → (jar, last use); swept by [`sweep_idle_jars`].
pub(crate) type JarMap = HashMap<Id, (Arc<CookieJar>, Instant)>;

/// How long a cookie jar nobody touches is kept once no cached client holds
/// it (clients themselves go after [`TUNNEL_IDLE_TTL`]).
pub(crate) const JAR_IDLE_TTL: Duration = Duration::from_secs(12 * 3600);

/// Drop jars idle past [`JAR_IDLE_TTL`] that only the map still references —
/// a jar a live client holds is kept, so a send and a listing never diverge
/// onto two jars for one scope.
pub(crate) fn sweep_idle_jars(map: &mut JarMap, now: Instant) {
    map.retain(|_, (jar, used)| {
        Arc::strong_count(jar) > 1 || now.saturating_duration_since(*used) < JAR_IDLE_TTL
    });
}

/// Shared outbound HTTP client per (workspace, allow_local). Follows
/// redirects, generous body timeout enforced per-request via `.timeout()`.
/// `allow_local` (the workspace's explicit opt-in) swaps the SSRF-guarded
/// resolver + redirect policy for a plain bounded client — the pre-flight
/// check is skipped by the caller under the same flag.
/// Idle entries are dropped after [`TUNNEL_IDLE_TTL`] (S6-310), which also
/// releases their hold on the scope's cookie jar.
pub(crate) fn http_client(wid: &Id, allow_local: bool) -> reqwest::Client {
    static CLIENTS: OnceLock<StdMutex<HashMap<String, (reqwest::Client, Instant)>>> =
        OnceLock::new();
    let clients = CLIENTS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut map = clients.lock().unwrap_or_else(|e| e.into_inner());
    map.retain(|_, (_, used)| used.elapsed() < TUNNEL_IDLE_TTL);
    let (client, used) = map
        .entry(format!("{wid}|{allow_local}"))
        .or_insert_with(|| {
            let builder = if allow_local {
                reqwest::Client::builder().redirect(reqwest::redirect::Policy::limited(10))
            } else {
                // SSRF guard: the guarded resolver re-vets every address at
                // CONNECT time (a DNS-rebinding answer can't swap in 127.0.0.1
                // after the pre-flight check), and each redirect hop is capped
                // + re-validated.
                otto_netguard::guarded_client_builder()
            };
            let client = builder
                .user_agent("Otto-ApiClient/1.0")
                .cookie_provider(cookie_jar(wid))
                .build()
                .unwrap_or_default();
            (client, Instant::now())
        });
    *used = Instant::now();
    client.clone()
}

/// Workspace opt-in for local/private targets: reads
/// `settings.api_client.allow_local` off the workspace row. Off by default;
/// flipping it is an ADMIN action (workspace settings PATCH). Scope: only the
/// API client's user-initiated requests — the rest of the daemon's outbound
/// paths keep the full guard.
pub async fn workspace_allows_local<C: ApiClientCtx>(ctx: &C, wid: &Id) -> bool {
    ctx.workspace_settings(wid)
        .await
        .and_then(|settings| settings.get("api_client")?.get("allow_local")?.as_bool())
        .unwrap_or(false)
}

/// Build a one-off client when per-request settings deviate from the defaults
/// (disable redirects, skip TLS verification) or the request is tunnelled
/// through a SOCKS5 proxy. `Ok(None)` → use the shared client. TLS verification
/// is only ever skipped when the request explicitly sets `verify_ssl=false`
/// (never the default).
///
/// Errors instead of degrading: a proxy URL reqwest rejects, or a builder that
/// fails, must fail the send. Falling back would silently route a tunnelled
/// request DIRECT through an unguarded builder (`guarded` is false whenever a
/// proxy is set), or drop `follow_redirects=false` / `verify_ssl=false`.
pub(crate) fn build_settings_client(
    wid: &Id,
    req: &ExecuteApiReq,
    proxy: Option<&str>,
    allow_local: bool,
) -> Result<Option<reqwest::Client>, String> {
    let no_redirect = req.follow_redirects == Some(false);
    let no_verify = req.verify_ssl == Some(false);
    if !no_redirect && !no_verify && proxy.is_none() {
        return Ok(None);
    }
    // Memoized per settings combination (perf F8): every SSH-tunnelled,
    // no-verify or no-redirect send — and every step of such an automation —
    // built a fresh client and paid a new SOCKS + TCP + TLS dial. Errors are
    // never cached; a key is only ever a working client.
    static CLIENTS: OnceLock<StdMutex<HashMap<String, (reqwest::Client, Instant)>>> =
        OnceLock::new();
    let key = format!(
        "{wid}|{allow_local}|{no_redirect}|{no_verify}|{}",
        proxy.unwrap_or("")
    );
    let clients = CLIENTS.get_or_init(|| StdMutex::new(HashMap::new()));
    {
        let mut map = clients.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, (_, used)| used.elapsed() < TUNNEL_IDLE_TTL);
        if let Some((client, used)) = map.get_mut(&key) {
            *used = Instant::now();
            return Ok(Some(client.clone()));
        }
    }
    let client = build_settings_client_uncached(wid, no_redirect, no_verify, proxy, allow_local)?;
    let mut map = clients.lock().unwrap_or_else(|e| e.into_inner());
    if map.len() >= SETTINGS_CLIENTS_MAX {
        if let Some(oldest) = map
            .iter()
            .min_by_key(|(_, (_, used))| *used)
            .map(|(k, _)| k.clone())
        {
            map.remove(&oldest);
        }
    }
    map.insert(key, (client.clone(), Instant::now()));
    Ok(Some(client))
}

/// Most per-settings clients kept by [`build_settings_client`].
pub(crate) const SETTINGS_CLIENTS_MAX: usize = 32;

pub(crate) fn build_settings_client_uncached(
    wid: &Id,
    no_redirect: bool,
    no_verify: bool,
    proxy: Option<&str>,
    allow_local: bool,
) -> Result<reqwest::Client, String> {
    // Tunnelled: the target resolves and is dialled at the FAR end — the local
    // guard would only false-block bastion-only hosts (and would refuse the
    // local SOCKS endpoint itself). allow_local: the workspace explicitly opted
    // in to private targets. Everything else keeps the connect-time guard.
    let guarded = proxy.is_none() && !allow_local;
    let base = if guarded {
        otto_netguard::guarded_client_builder()
    } else {
        reqwest::Client::builder()
    };
    let mut builder = base
        .user_agent("Otto-ApiClient/1.0")
        .cookie_provider(cookie_jar(wid));
    builder = if no_redirect {
        builder.redirect(reqwest::redirect::Policy::none())
    } else if !guarded {
        builder.redirect(reqwest::redirect::Policy::limited(10))
    } else {
        // Redirects still follow the SSRF-guarded policy.
        builder.redirect(otto_netguard::guarded_redirect_policy())
    };
    if no_verify {
        builder = builder.danger_accept_invalid_certs(true);
    }
    if let Some(url) = proxy {
        // `socks5h://` so the bastion does the DNS — the target host is resolved
        // and dialled from the SSH server's network, not ours.
        let px = reqwest::Proxy::all(url)
            .map_err(|e| format!("Couldn't route through the SSH tunnel ({e})"))?;
        builder = builder.proxy(px);
    }
    builder.build().map_err(|e| {
        if proxy.is_some() {
            format!("Couldn't route through the SSH tunnel ({e})")
        } else {
            format!("couldn't build the HTTP client for this request's settings ({e})")
        }
    })
}
