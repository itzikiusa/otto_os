# Agent 1 — Outbound SSRF guard + host-file sandbox (audit S1, S2)

Scope (owned files only):
- `crates/otto-server/src/routes/api_client.rs`
- `crates/otto-server/src/routes/api_stream.rs`
- `crates/otto-server/src/routes/grpc.rs`
- `crates/otto-server/src/routes/fs.rs`
- `crates/otto-server/src/modules.rs` (browser_proxy / outbound-fetch only)

## [x] T1 — SSRF guard (audit S1)

New private module `net_guard` (declared `pub(crate)`) lives in
`routes/api_client.rs` and is reused by the other outbound paths via
`crate::routes::api_client::net_guard::*`. std + tokio only; URL parsing uses
the already-vendored `reqwest::Url` (no new crate).

What it provides:
- `is_blocked_ip(IpAddr) -> bool` — rejects loopback, RFC1918 private, link-local
  (incl. explicit `169.254.169.254` cloud metadata + `fe80::/10`), CGNAT
  `100.64.0.0/10`, unspecified (`0.0.0.0`/`::`), broadcast, multicast, IPv4
  documentation, and ULA `fc00::/7`. Unwraps IPv4-mapped/compat IPv6
  (`::ffff:127.0.0.1`) so v4 rules apply to v6-wrapped literals.
- `check_url(&str)` — async pre-flight: rejects non-http(s)/ws(s)/grpc(s) schemes
  and host-less URLs (`file:`, `data:`); resolves the host via
  `tokio::net::lookup_host` and rejects if **any** resolved address is blocked;
  IP literals are checked directly (no DNS).
- `redirect_policy()` — a `reqwest::redirect::Policy::custom` that caps hops at
  `MAX_REDIRECTS` (10, matching the prior `Policy::limited(10)`) and
  **re-validates each redirect hop's host** (fails closed on resolution error),
  so an upstream 30x cannot bounce a fetch into a private/loopback address.

Wiring (each outbound user-URL fetch now guarded):
- `api_client.rs`
  - `http_client()` and `build_settings_client()` now use
    `net_guard::redirect_policy()` instead of `Policy::limited(10)`.
  - `build_and_send()` pre-flights `check_url(&url)` right after `{{var}}`
    substitution (covers `/execute` and the automation runner, which shares this
    path); rejection flows into history + a 502 via the existing `String` error.
  - `oauth2_token()` pre-flights `check_url(&req.token_url)` → `Error::Invalid`.
  - TLS-skip gating: `danger_accept_invalid_certs(true)` is still only set when a
    request explicitly sends `verify_ssl=false`; comment clarified that it is
    never the default. (No behavioral change — was already per-request.)
- `api_stream.rs`
  - `serve_sse()` pre-flights `check_url(&spec.url)` and builds its reqwest
    client with `net_guard::redirect_policy()`.
  - `serve_websocket()` pre-flights `check_url(&spec.url)` before
    `tokio_tungstenite::connect_async`.
- `grpc.rs`
  - `invoke()` pre-flights `check_url(&req.url)` before dialing the tonic channel.
  - `connect_channel()` (used by `reflect_pool` → `reflect`/reflection-based
    `invoke`) pre-flights `check_url(url)`.
- `modules.rs` (`browser_proxy`)
  - Proxy client built with `net_guard::redirect_policy()`.
  - Handler pre-flights `check_url(&url)` and returns a 400 `Problem` on rejection
    (matches the handler's existing Problem pattern).

Known limitation (documented in code intent): tonic's `Channel` and
tokio-tungstenite do their own DNS at connect time, so there is a theoretical
TOCTOU window between the pre-flight check and the actual connect. The pre-flight
catches the overwhelming majority of SSRF attempts (literal IPs, metadata
hostnames, internal hostnames). Closing TOCTOU fully needs a custom connector,
which is out of scope for a std+tokio-only change.

Tests added (in `api_client.rs` `mod tests`):
- `net_guard_blocks_internal_addresses` — table of blocked vs. allowed IPs.
- `net_guard_check_url_rejects_loopback_and_schemes` — async; loopback/metadata/
  `file:`/garbage all rejected.

## [x] T2 — fs.rs host-file sandbox (audit S2)

`GET /fs/browse` and `/fs/read` back a general-purpose folder/file **picker**
(FolderPicker starts at `~` and must roam the home tree to pick a session cwd /
repo path; FileTree browses arbitrary roots), so a strict workspace-root
allowlist would break legitimate browsing. Per the audit's fallback guidance, the
fix is a **deny-list sandbox** + traversal/symlink-escape protection, and it now
**consults the `CurrentUser`** the handlers previously discarded.

New private `mod sandbox` in `fs.rs`:
- `is_denied_dir(&Path)` — denies `$HOME/{.ssh,.aws,.gnupg,.kube,.docker,
  .config/gcloud,.config/gh,.azure,.password-store}` and absolute prefixes
  `/etc`, `/private/etc`, `/root`, `/var/root`, `/proc`, `/sys` (and any path
  under them). `$HOME` is itself canonicalized before comparison.
- `is_denied_file(&Path)` — denies known secret filenames (`id_rsa`, `id_dsa`,
  `id_ecdsa`, `id_ed25519`, `credentials`, `.env`, `.netrc`, `.pgpass`, `.npmrc`,
  `.pypirc`, `.dockercfg`) and secret extensions (`.pem`, `.key`, `.pfx`, `.p12`,
  `.keystore`), case-insensitively.

Handler wiring:
- `guard_dir(canonical, &user)` / `guard_file(canonical, &user)` take the
  authenticated `User` (route requires `CurrentUser`; reserved for future
  per-user root scoping — root vs. non-root does **not** relax the deny-list).
- `browse()` binds `CurrentUser(user)` and calls `guard_dir` **after**
  `canonicalize()` — so `..` and symlink escapes resolve first and a symlink into
  `~/.ssh` is caught by its target, not its name. Returns `Error::Forbidden`.
- `read_file()` binds `CurrentUser(user)` and calls `guard_file` after
  `canonicalize()` (dir-deny + secret-file-deny). Returns `Error::Forbidden`.
- Legitimate browsing preserved: only secret stores/files are denied; the rest of
  the tree still lists/reads exactly as before. Directory **listing** still
  returns entry names (descending or reading them re-canonicalizes and re-guards).

Tests added (in `fs.rs` `mod tests`):
- `denies_system_secret_dirs`, `denies_home_secret_dirs` (hermetic temp `$HOME`),
  `denies_secret_files_by_name_and_ext`.

## Build status — `cargo check -p otto-server`

Does **not** currently compile, but **no error originates in any file I own**.
All errors are `missing field user_id` in `NewNotice` / `Event::Notification`
inside files owned by the concurrent *per-user notifications* agent:
`src/monitor.rs`, `src/routes/activity.rs`, `src/skill_eval.rs`, `src/state.rs`
(the `NewNotice`/`Event` structs in `otto-state`/`otto-core` gained a `user_id`
field; those callers haven't been updated yet). My five files produce zero
errors and zero warnings (verified by filtering the compiler output for each
file path). All five files were formatted with `rustfmt --edition 2021`.

Once the notifications agent finishes wiring `user_id` into its callers, the
crate (and my added unit tests) should build cleanly.
