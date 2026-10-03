//! Helpers for the API client routes that don't touch the DB or the network:
//! curl-command parsing ([`parse_curl`]), OpenAPI 3 export
//! ([`collection_to_openapi`]) and the automation runner's pure evaluation
//! pieces — a small JSON-path reader ([`json_path`]) and an assertion evaluator
//! ([`eval_assertion`]).

use otto_core::api::ParsedCurl;
use otto_core::domain::{ApiCollection, ApiRequest};
use otto_core::{Error, Result};
use serde_json::{json, Map, Value};

// ===========================================================================
// curl parsing
// ===========================================================================

/// Tokenize a curl command line, honouring single/double quotes, `$'...'`
/// ANSI-C-ish strings, backslash line-continuations and escapes. This is a
/// best-effort shell-word splitter, not a full POSIX parser.
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut has_token = false;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            // Whitespace separates tokens (outside quotes, handled above).
            c if c.is_whitespace() => {
                if has_token {
                    tokens.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            // Backslash: line continuation (\<newline>) is dropped; otherwise the
            // next char is taken literally.
            '\\' => {
                has_token = true;
                match chars.next() {
                    Some('\n') => {} // line continuation
                    Some('\r') => {
                        if chars.peek() == Some(&'\n') {
                            chars.next();
                        }
                    }
                    Some(next) => cur.push(next),
                    None => {}
                }
            }
            '\'' => {
                has_token = true;
                // single-quoted: literal until next '
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                    cur.push(q);
                }
            }
            '"' => {
                has_token = true;
                // double-quoted: allow \" \\ \$ \` escapes, otherwise literal.
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some(e @ ('"' | '\\' | '$' | '`')) => cur.push(e),
                            Some('\n') => {}
                            Some(other) => {
                                cur.push('\\');
                                cur.push(other);
                            }
                            None => {}
                        },
                        other => cur.push(other),
                    }
                }
            }
            '$' if chars.peek() == Some(&'\'') => {
                // $'...' ANSI-C quoting: handle common escapes.
                has_token = true;
                chars.next(); // consume the opening '
                while let Some(q) = chars.next() {
                    match q {
                        '\'' => break,
                        '\\' => match chars.next() {
                            Some('n') => cur.push('\n'),
                            Some('t') => cur.push('\t'),
                            Some('r') => cur.push('\r'),
                            Some('\'') => cur.push('\''),
                            Some('\\') => cur.push('\\'),
                            Some(other) => cur.push(other),
                            None => {}
                        },
                        other => cur.push(other),
                    }
                }
            }
            other => {
                has_token = true;
                cur.push(other);
            }
        }
    }
    if has_token {
        tokens.push(cur);
    }
    tokens
}

/// Split a `Header-Name: value` string into `(name, value)`, trimming.
fn split_header(raw: &str) -> Option<(String, String)> {
    let (name, value) = raw.split_once(':')?;
    Some((name.trim().to_string(), value.trim().to_string()))
}

fn header_entry(key: &str, value: &str) -> Value {
    json!({ "key": key, "value": value, "enabled": true })
}

/// Short flags that take a value — the value may be attached (`-XPOST`,
/// `-HAccept:x`, `-ualice:pw`), as curl itself allows.
const SHORT_VALUE_FLAGS: &[&str] = &[
    "-X", "-H", "-d", "-u", "-F", "-A", "-e", "-b", "-c", "-o", "-m", "-x", "-w", "-T", "-E", "-r",
    "-K", "-U", "-y", "-Y", "-D", "-Q", "-z", "-P",
];

/// Long flags that take a value we don't model — consumed so the value isn't
/// mistaken for the URL (`curl --cacert ca.pem https://x` must import
/// `https://x`, not `ca.pem`).
const IGNORED_VALUE_FLAGS: &[&str] = &[
    "--output",
    "--max-time",
    "--connect-timeout",
    "--proxy",
    "--write-out",
    "--upload-file",
    "--cert",
    "--key",
    "--cookie-jar",
    "--cacert",
    "--capath",
    "--resolve",
    "--connect-to",
    "--retry",
    "--retry-delay",
    "--retry-max-time",
    "--max-redirs",
    "--limit-rate",
    "--interface",
    "--local-port",
    "--range",
    "--proto",
    "--proto-redir",
    "--cert-type",
    "--key-type",
    "--pass",
    "--dns-servers",
    "--ciphers",
    "--tls-max",
    "--config",
    "--unix-socket",
    "--abstract-unix-socket",
    "--noproxy",
    "--proxy-user",
    "--proxy-header",
    "--expect100-timeout",
    "--speed-time",
    "--speed-limit",
    "--keepalive-time",
    "--trace",
    "--trace-ascii",
    "--stderr",
    "--dump-header",
    "--variable",
    "--hsts",
    "--alt-svc",
    "--etag-save",
    "--etag-compare",
    "--aws-sigv4",
    "--request-target",
    "--quote",
    "--time-cond",
    "--ftp-port",
    "--max-filesize",
    "--happy-eyeballs-timeout-ms",
    "--create-file-mode",
    "--output-dir",
    "--sasl-authzid",
    "--login-options",
    "--mail-from",
    "--mail-rcpt",
    "--krb",
    "--delegation",
    "--proxy-cacert",
    "--proxy-cert",
    "--proxy-key",
];

/// Split an attached short flag (`-XPOST` → `("-X", "POST")`). `None` for
/// anything else — long flags, bundled booleans (`-sSL`), bare `-X`.
fn split_attached_short(token: &str) -> Option<(&str, &str)> {
    if token.starts_with("--") || token.len() <= 2 || !token.is_char_boundary(2) {
        return None;
    }
    let (flag, value) = token.split_at(2);
    SHORT_VALUE_FLAGS.contains(&flag).then_some((flag, value))
}

/// Percent-encode a `--data-urlencode` value the way curl does
/// (RFC 3986 unreserved characters pass through; everything else is `%XX`).
fn urlencode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// One `--data-urlencode` argument, encoded per curl's rules: `content`
/// and `=content` encode the whole content; `name=content` keeps `name` and
/// encodes the content. `@file` / `name@file` read a file curl-side — we
/// can't, so those stay verbatim for the user to fill in.
fn encode_data_urlencode(arg: &str) -> String {
    if let Some(rest) = arg.strip_prefix('=') {
        return urlencode_component(rest);
    }
    match arg.split_once('=') {
        Some((name, content)) if !name.contains('@') => {
            format!("{name}={}", urlencode_component(content))
        }
        Some(_) => arg.to_string(),
        None if arg.starts_with('@') || (arg.contains('@') && !arg.contains(' ')) => {
            arg.to_string()
        }
        None => urlencode_component(arg),
    }
}

/// One `-F` / `--form` field as a multipart row (`[{key,type,value,filename}]`,
/// the shape the request editor stores). `name=@path[;type=…]` is a file
/// upload: the row keeps the file NAME — the bytes live on the machine curl
/// ran on, so the user re-picks the file. `name=<path` (file contents as a
/// text value) is kept as written.
fn form_field(arg: &str, literal: bool) -> Option<Value> {
    let (key, value) = arg.split_once('=')?;
    if key.trim().is_empty() {
        return None;
    }
    if !literal {
        if let Some(path) = value.strip_prefix('@') {
            let path = path.split(';').next().unwrap_or(path).trim_matches('"');
            let filename = path.rsplit(['/', '\\']).next().unwrap_or(path);
            return Some(json!({ "key": key, "type": "file", "value": "", "filename": filename }));
        }
    }
    Some(json!({ "key": key, "type": "text", "value": value }))
}

/// Parse a curl command string into request fields. Tolerant of quotes, line
/// continuations and `$'...'`; ignores unknown flags. Returns an `Invalid`
/// error only when no URL can be found.
pub fn parse_curl(input: &str) -> Result<ParsedCurl> {
    let tokens = tokenize(input.trim());
    let mut iter = tokens.iter().peekable();

    let mut method: Option<String> = None;
    let mut url: Option<String> = None;
    let mut headers: Vec<Value> = Vec::new();
    let mut data_parts: Vec<String> = Vec::new();
    let mut form_fields: Vec<Value> = Vec::new();
    let mut is_urlencoded_data = false; // --data-urlencode used
    let mut is_json_flag = false; // --json used
    let mut get_with_data = false; // -G / --get: send data as query
    let mut head = false; // -I / --head
    let mut basic_user: Option<String> = None;
    let mut explicit_content_type: Option<String> = None;
    let mut auth: Value = json!({ "type": "none" });

    // Skip a leading "curl" token if present.
    if iter.peek().map(|t| t.as_str()) == Some("curl") {
        iter.next();
    }

    while let Some(tok) = iter.next() {
        let t = tok.as_str();

        // `--flag=value` splits on the first '='; `-XPOST` splits after the
        // two-character flag.
        let (flag, inline_val): (&str, Option<&str>) = if t.starts_with("--") {
            match t.split_once('=') {
                Some((f, v)) => (f, Some(v)),
                None => (t, None),
            }
        } else if let Some((f, v)) = split_attached_short(t) {
            (f, Some(v))
        } else {
            (t, None)
        };

        // Helper to fetch the value for a flag (inline `=val` or the next token).
        let mut take_val = |inline: Option<&str>| -> Option<String> {
            if let Some(v) = inline {
                Some(v.to_string())
            } else {
                iter.next().map(|s| s.to_string())
            }
        };
        let mut push_header = |name: &str, value: &str| {
            if !headers.iter().any(|h| {
                h["key"]
                    .as_str()
                    .is_some_and(|k| k.eq_ignore_ascii_case(name))
            }) {
                headers.push(header_entry(name, value));
            }
        };

        match flag {
            "-X" | "--request" => {
                if let Some(v) = take_val(inline_val) {
                    method = Some(v.to_uppercase());
                }
            }
            "-H" | "--header" => {
                if let Some(v) = take_val(inline_val) {
                    if let Some((name, value)) = split_header(&v) {
                        let lname = name.to_lowercase();
                        if lname == "authorization" {
                            auth = parse_auth_header(&value);
                            // Keep Content-Type-ish headers but drop Authorization
                            // (represented via `auth`).
                        } else {
                            if lname == "content-type" {
                                explicit_content_type = Some(value.clone());
                            }
                            headers.push(header_entry(&name, &value));
                        }
                    }
                }
            }
            "-d" | "--data" | "--data-raw" | "--data-ascii" | "--data-binary" => {
                if let Some(v) = take_val(inline_val) {
                    data_parts.push(v);
                }
            }
            "--data-urlencode" => {
                if let Some(v) = take_val(inline_val) {
                    is_urlencoded_data = true;
                    data_parts.push(encode_data_urlencode(&v));
                }
            }
            // `--json <data>`: the data, plus JSON Content-Type and Accept.
            "--json" => {
                if let Some(v) = take_val(inline_val) {
                    is_json_flag = true;
                    data_parts.push(v);
                }
            }
            "-F" | "--form" | "--form-string" => {
                if let Some(v) = take_val(inline_val) {
                    if let Some(field) = form_field(&v, flag == "--form-string") {
                        form_fields.push(field);
                    }
                }
            }
            "-u" | "--user" => {
                if let Some(v) = take_val(inline_val) {
                    basic_user = Some(v);
                }
            }
            "--oauth2-bearer" => {
                if let Some(v) = take_val(inline_val) {
                    auth = json!({ "type": "bearer", "token": v });
                }
            }
            "-A" | "--user-agent" => {
                if let Some(v) = take_val(inline_val) {
                    push_header("User-Agent", &v);
                }
            }
            "-e" | "--referer" => {
                if let Some(v) = take_val(inline_val) {
                    // `;auto` is curl's own redirect-referer switch, not a value.
                    let v = v.trim_end_matches(";auto");
                    if !v.is_empty() {
                        push_header("Referer", v);
                    }
                }
            }
            // `-b name=value; other=x` is a cookie string; `-b file` reads a
            // cookie jar from disk — only the former can be carried over.
            "-b" | "--cookie" => {
                if let Some(v) = take_val(inline_val) {
                    if v.contains('=') {
                        push_header("Cookie", &v);
                    }
                }
            }
            "-I" | "--head" => head = true,
            "-G" | "--get" => {
                get_with_data = true;
            }
            "--url" => {
                if let Some(v) = take_val(inline_val) {
                    url = Some(v);
                }
            }
            // Flags that take a value we don't model — consume the value so it
            // isn't mistaken for the URL.
            f if SHORT_VALUE_FLAGS.contains(&f) || IGNORED_VALUE_FLAGS.contains(&f) => {
                let _ = take_val(inline_val);
            }
            // Boolean flags we ignore.
            _ if flag.starts_with('-') => {}
            // Bare token → the URL (first one wins).
            _ => {
                if url.is_none() {
                    url = Some(tok.clone());
                }
            }
        }
    }

    let raw_url = url.ok_or_else(|| Error::Invalid("no URL found in curl command".into()))?;

    // Combine data parts. For --data-urlencode each part may be `name=value`.
    let combined_data = data_parts.join("&");
    let has_data = !data_parts.is_empty();
    let has_form = !form_fields.is_empty();

    if is_json_flag {
        if explicit_content_type.is_none() {
            headers.push(header_entry("Content-Type", "application/json"));
            explicit_content_type = Some("application/json".into());
        }
        if !headers.iter().any(|h| {
            h["key"]
                .as_str()
                .is_some_and(|k| k.eq_ignore_ascii_case("accept"))
        }) {
            headers.push(header_entry("Accept", "application/json"));
        }
    }

    // Determine the basic auth from -u if present (overrides header-derived auth
    // only when no Authorization header set one).
    if let Some(user) = basic_user {
        let (u, p) = match user.split_once(':') {
            Some((u, p)) => (u.to_string(), p.to_string()),
            None => (user, String::new()),
        };
        auth = json!({ "type": "basic", "username": u, "password": p });
    }

    // Split URL into base + query params.
    let (base_url, mut query) = split_url_query(&raw_url);

    // Resolve method.
    let method = method.unwrap_or_else(|| {
        if head {
            "HEAD".to_string()
        } else if (has_data && !get_with_data) || has_form {
            "POST".to_string()
        } else {
            "GET".to_string()
        }
    });

    // Decide body vs query for the data.
    let mut body = String::new();
    let mut body_mode = "none".to_string();
    if has_form {
        // multipart/form-data: the editor's row array. reqwest writes the
        // boundary Content-Type itself, so an explicit one is dropped.
        headers.retain(|h| {
            !h["key"]
                .as_str()
                .is_some_and(|k| k.eq_ignore_ascii_case("content-type"))
        });
        body_mode = "multipart".to_string();
        body = Value::Array(form_fields).to_string();
    } else if has_data {
        if get_with_data {
            // -G: data becomes query parameters.
            for part in parse_form_pairs(&combined_data) {
                query.push(part);
            }
        } else {
            let ct = explicit_content_type
                .as_deref()
                .unwrap_or("")
                .to_lowercase();
            let looks_form = is_urlencoded_data || ct.contains("application/x-www-form-urlencoded");
            let looks_json = ct.contains("application/json") || is_json(&combined_data);
            if looks_form && !looks_json {
                body_mode = "form".to_string();
                body = combined_data.clone();
            } else if looks_json {
                body_mode = "json".to_string();
                body = combined_data.clone();
            } else {
                body_mode = "raw".to_string();
                body = combined_data.clone();
            }
        }
    }

    Ok(ParsedCurl {
        method,
        url: base_url,
        headers: Value::Array(headers),
        query: Value::Array(query),
        body_mode,
        body,
        auth,
    })
}

/// Map an `Authorization` header value to an `auth` object.
fn parse_auth_header(value: &str) -> Value {
    let trimmed = value.trim();
    if let Some(token) = trimmed
        .strip_prefix("Bearer ")
        .or_else(|| trimmed.strip_prefix("bearer "))
    {
        return json!({ "type": "bearer", "token": token.trim() });
    }
    if let Some(b64) = trimmed
        .strip_prefix("Basic ")
        .or_else(|| trimmed.strip_prefix("basic "))
    {
        use base64::engine::general_purpose::STANDARD as B64;
        use base64::Engine;
        if let Ok(decoded) = B64.decode(b64.trim()) {
            if let Ok(s) = String::from_utf8(decoded) {
                let (u, p) = match s.split_once(':') {
                    Some((u, p)) => (u.to_string(), p.to_string()),
                    None => (s, String::new()),
                };
                return json!({ "type": "basic", "username": u, "password": p });
            }
        }
    }
    // Unknown scheme → keep as a raw header-style api_key (header location).
    json!({ "type": "api_key", "in": "header", "key": "Authorization", "value": trimmed })
}

/// Split `url?a=1&b=2` into (`url`, `[{key,value,enabled}]`).
fn split_url_query(raw: &str) -> (String, Vec<Value>) {
    match raw.split_once('?') {
        Some((base, qs)) if !qs.is_empty() => (base.to_string(), parse_form_pairs(qs)),
        Some((base, _)) => (base.to_string(), Vec::new()),
        None => (raw.to_string(), Vec::new()),
    }
}

/// Parse `a=1&b=2` (or `a` alone) into query/form entries. Values are
/// percent-decoded best-effort.
fn parse_form_pairs(s: &str) -> Vec<Value> {
    s.split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| {
            let (k, v) = match pair.split_once('=') {
                Some((k, v)) => (percent_decode(k), percent_decode(v)),
                None => (percent_decode(pair), String::new()),
            };
            header_entry(&k, &v)
        })
        .collect()
}

/// Minimal percent-decoder (handles `%XX` and `+` → space). Leaves malformed
/// sequences untouched.
pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(h), Some(l)) = (hi, lo) {
                    out.push((h * 16 + l) as u8);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// True when `s` parses as a JSON object or array.
fn is_json(s: &str) -> bool {
    let t = s.trim();
    (t.starts_with('{') || t.starts_with('[')) && serde_json::from_str::<Value>(t).is_ok()
}

// ===========================================================================
// OpenAPI 3 export
// ===========================================================================

/// Build a minimal but valid OpenAPI 3.0 document from a collection and its
/// requests. Paths are keyed by the request URL's path; methods grouped under
/// each path; query/header params and a request-body example are derived from
/// the saved request.
pub fn collection_to_openapi(collection: &ApiCollection, requests: &[ApiRequest]) -> Value {
    let mut paths = Map::new();

    for req in requests {
        let (path, _host) = url_to_path(&req.url);
        let method = req.method.to_lowercase();
        // Only the standard OpenAPI operations.
        if !matches!(
            method.as_str(),
            "get" | "put" | "post" | "delete" | "options" | "head" | "patch" | "trace"
        ) {
            continue;
        }

        let mut operation = Map::new();
        operation.insert("summary".into(), json!(req.name));
        operation.insert("operationId".into(), json!(operation_id(req)));

        // Durable extras: request docs become the operation description;
        // settings + GraphQL variables ride along as x-otto extensions.
        // Scripts and auth are NEVER serialized here (auth may hold Keychain
        // markers; even those stay out of the export).
        if let Some(extras) = &req.extras {
            if let Some(docs) = extras.get("docs_md").and_then(Value::as_str) {
                if !docs.trim().is_empty() {
                    operation.insert("description".into(), json!(docs));
                }
            }
            if let Some(settings) = extras.get("settings").filter(|s| s.is_object()) {
                operation.insert("x-otto-settings".into(), settings.clone());
            }
            if let Some(gql) = extras.get("graphql_variables").and_then(Value::as_str) {
                if !gql.trim().is_empty() {
                    operation.insert("x-otto-graphql-variables".into(), json!(gql));
                }
            }
        }

        // Parameters: query + headers.
        let mut params: Vec<Value> = Vec::new();
        for entry in enabled_entries(&req.query) {
            params.push(json!({
                "name": entry.0,
                "in": "query",
                "required": false,
                "schema": { "type": "string" },
                "example": entry.1,
            }));
        }
        for entry in enabled_entries(&req.headers) {
            // Skip headers OpenAPI manages itself.
            let lname = entry.0.to_lowercase();
            if lname == "content-type" || lname == "accept" || lname == "authorization" {
                continue;
            }
            params.push(json!({
                "name": entry.0,
                "in": "header",
                "required": false,
                "schema": { "type": "string" },
                "example": entry.1,
            }));
        }
        if !params.is_empty() {
            operation.insert("parameters".into(), Value::Array(params));
        }

        // Request body example from the saved body.
        if !req.body.trim().is_empty() && req.body_mode != "none" {
            let (media_type, example) = body_example(&req.body_mode, &req.body);
            operation.insert(
                "requestBody".into(),
                json!({
                    "content": {
                        media_type: { "example": example }
                    }
                }),
            );
        }

        operation.insert(
            "responses".into(),
            json!({
                "default": { "description": "Default response" }
            }),
        );

        // Merge into paths[path][method].
        let path_item = paths
            .entry(path)
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(map) = path_item {
            map.insert(method, Value::Object(operation));
        }
    }

    json!({
        "openapi": "3.0.3",
        "info": {
            "title": collection.name,
            "version": "1.0.0",
        },
        "paths": Value::Object(paths),
    })
}

/// `[{key,value,enabled}]` → enabled `(key, value)` pairs.
fn enabled_entries(v: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(arr) = v.as_array() {
        for item in arr {
            let enabled = item.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            if !enabled {
                continue;
            }
            let key = item.get("key").and_then(Value::as_str).unwrap_or("");
            if key.is_empty() {
                continue;
            }
            let value = item.get("value").and_then(Value::as_str).unwrap_or("");
            out.push((key.to_string(), value.to_string()));
        }
    }
    out
}

/// Body example for OpenAPI: returns (media_type, example value). JSON bodies
/// are parsed into a structured example when possible.
fn body_example(body_mode: &str, body: &str) -> (String, Value) {
    match body_mode {
        "json" | "graphql" => {
            // GraphQL-over-HTTP requests are JSON-encoded, same as `json`.
            let media = "application/json";
            let example = serde_json::from_str::<Value>(body).unwrap_or_else(|_| json!(body));
            (media.to_string(), example)
        }
        "form" => ("application/x-www-form-urlencoded".to_string(), json!(body)),
        _ => ("text/plain".to_string(), json!(body)),
    }
}

/// Extract the path component of a URL, returning (path, host). Falls back to
/// the raw string when it isn't a recognisable absolute URL. `{{var}}`
/// placeholders are preserved.
fn url_to_path(url: &str) -> (String, String) {
    let trimmed = url.trim();
    // Strip scheme.
    let after_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    // Strip query/fragment.
    let no_query = after_scheme
        .split(['?', '#'])
        .next()
        .unwrap_or(after_scheme);
    match no_query.split_once('/') {
        Some((host, rest)) => (format!("/{rest}"), host.to_string()),
        None => {
            // No path component.
            if trimmed.contains("://") {
                ("/".to_string(), no_query.to_string())
            } else {
                // Relative URL — use it verbatim as the path.
                let p = if no_query.starts_with('/') {
                    no_query.to_string()
                } else {
                    format!("/{no_query}")
                };
                (p, String::new())
            }
        }
    }
}

/// A stable-ish operationId from method + path.
fn operation_id(req: &ApiRequest) -> String {
    let (path, _) = url_to_path(&req.url);
    let slug: String = path
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    format!("{}{}", req.method.to_lowercase(), slug)
}

// ===========================================================================
// JSON-path evaluation (automation runner)
// ===========================================================================

/// Resolve a dot/bracket path against a JSON value, returning the addressed
/// node when present.
///
/// Supported syntax (a deliberately small subset, enough for response
/// extraction and assertions):
/// - dotted object keys: `data.token`
/// - bracketed array indices: `items[0]`, `data.items[2].id`
/// - bracketed quoted keys: `data["odd key"]`, `obj['k']`
/// - a leading `$` is accepted and ignored (`$.data.id` == `data.id`)
///
/// An empty path returns the root. Any miss (absent key, out-of-range or
/// negative index, indexing a non-array, keying a non-object) yields `None`.
/// Never panics.
pub fn json_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = root;
    for seg in parse_path_segments(path) {
        match seg {
            PathSeg::Key(k) => {
                cur = cur.as_object()?.get(&k)?;
            }
            PathSeg::Index(i) => {
                cur = cur.as_array()?.get(i)?;
            }
        }
    }
    Some(cur)
}

#[derive(Debug, PartialEq)]
enum PathSeg {
    Key(String),
    Index(usize),
}

/// Tokenize a path string into key/index segments. Tolerant of a leading `$`,
/// leading/trailing dots and empty fragments.
fn parse_path_segments(path: &str) -> Vec<PathSeg> {
    let mut segs = Vec::new();
    let mut chars = path.chars().peekable();
    let mut cur = String::new();

    // Flush an accumulated bare (dot) key, if any.
    fn flush(cur: &mut String, segs: &mut Vec<PathSeg>) {
        if !cur.is_empty() {
            // A bare numeric fragment in dot position is still a key (objects can
            // have numeric string keys); bracket indices are handled separately.
            segs.push(PathSeg::Key(std::mem::take(cur)));
        }
    }

    // Skip an optional leading "$".
    if chars.peek() == Some(&'$') {
        chars.next();
    }

    while let Some(c) = chars.next() {
        match c {
            '.' => flush(&mut cur, &mut segs),
            '[' => {
                flush(&mut cur, &mut segs);
                let mut inner = String::new();
                for b in chars.by_ref() {
                    if b == ']' {
                        break;
                    }
                    inner.push(b);
                }
                let trimmed = inner.trim();
                // Quoted key: ["k"] or ['k'].
                if (trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2)
                    || (trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2)
                {
                    segs.push(PathSeg::Key(trimmed[1..trimmed.len() - 1].to_string()));
                } else if let Ok(idx) = trimmed.parse::<usize>() {
                    segs.push(PathSeg::Index(idx));
                } else {
                    // Unquoted, non-numeric bracket content → treat as a key.
                    segs.push(PathSeg::Key(trimmed.to_string()));
                }
            }
            other => cur.push(other),
        }
    }
    flush(&mut cur, &mut segs);
    segs
}

// ===========================================================================
// Assertion evaluation (automation runner)
// ===========================================================================

/// Outcome of evaluating a single assertion: a worded description that carries
/// the actual value ("Status code is less than 400: got 500"), whether it held,
/// and the actual value itself (`Null` when the target was missing).
pub struct AssertionResult {
    pub desc: String,
    pub passed: bool,
    pub actual: Value,
}

/// Everything an assertion can be evaluated against: one executed step's
/// response. `headers` is the response's `[{key,value}]` array (`Null` when
/// unknown); `body_text` the raw body; `body` its JSON parse (`Null` when the
/// body wasn't JSON).
pub struct StepOutcome<'a> {
    pub status: Option<u16>,
    pub duration_ms: i64,
    pub body: &'a Value,
    pub body_text: &'a str,
    pub headers: &'a Value,
}

/// Longest regex pattern a `matches` assertion accepts, and the compiled-size
/// cap handed to the regex engine — a pasted pathological pattern fails the
/// assertion instead of burning CPU/memory in the run.
const MAX_REGEX_PATTERN: usize = 1_000;
const MAX_REGEX_COMPILED: usize = 1 << 20;
/// Longest haystack a `matches`/`contains` on the body considers.
const MAX_MATCH_HAYSTACK: usize = 1 << 20;

/// Evaluate one assertion with only status/duration/JSON-body known (no
/// headers or raw text) — see [`eval_assertion_with`].
pub fn eval_assertion(
    assertion: &Value,
    status: Option<u16>,
    duration_ms: i64,
    body: &Value,
) -> AssertionResult {
    eval_assertion_with(
        assertion,
        &StepOutcome {
            status,
            duration_ms,
            body,
            body_text: "",
            headers: &Value::Null,
        },
    )
}

/// Evaluate one assertion against an executed step's outcome.
///
/// `assertion` is a `{ "kind", "op", "value", "path"? }` object:
/// - `kind = "status"`      — the numeric HTTP status.
/// - `kind = "duration_ms"` — elapsed milliseconds.
/// - `kind = "json_path"`   — `path` evaluated against the parsed JSON body.
/// - `kind = "header"`      — the response header named by `path`
///   (case-insensitive; repeated headers are joined with `, `).
/// - `kind = "body_text"`   — the raw response body text.
///
/// `op` is one of `eq | ne | contains | lt | gt | lte | gte | exists |
/// not_exists | matches`. `eq`/`ne`/`contains` compare loosely (string- and
/// number-aware); `lt`/`gt`/`lte`/`gte` are numeric; `matches` is a regex
/// (size-capped). A MISSING target (absent JSON field, absent header, no
/// response) fails every op except `not_exists` — `ne` on a missing field used
/// to pass silently. A malformed assertion never panics — it returns a failed
/// result describing the problem.
pub fn eval_assertion_with(assertion: &Value, o: &StepOutcome<'_>) -> AssertionResult {
    let kind = assertion.get("kind").and_then(Value::as_str).unwrap_or("");
    let op = assertion.get("op").and_then(Value::as_str).unwrap_or("eq");
    let expected = assertion.get("value").cloned().unwrap_or(Value::Null);
    let path = assertion.get("path").and_then(Value::as_str).unwrap_or("");

    // (subject label, actual value or None when missing, what "missing" means)
    let (subject, actual, missing): (String, Option<Value>, &str) = match kind {
        "status" => (
            "Status code".into(),
            o.status.map(Value::from),
            "no response",
        ),
        "duration_ms" => (
            "Response time (ms)".into(),
            Some(Value::from(o.duration_ms)),
            "no response",
        ),
        "json_path" => (
            if path.is_empty() {
                "$".into()
            } else {
                path.to_string()
            },
            json_path(o.body, path).cloned(),
            "field missing",
        ),
        "header" => (
            format!("Header {path}"),
            header_value(o.headers, path).map(Value::String),
            "header missing",
        ),
        "body_text" => (
            "Body".into(),
            Some(Value::String(o.body_text.to_string())),
            "no body",
        ),
        other => {
            return AssertionResult {
                desc: format!("unknown assertion kind '{other}'"),
                passed: false,
                actual: Value::Null,
            }
        }
    };

    let phrase = op_phrase(op);
    let takes_value = !matches!(op, "exists" | "not_exists");
    let head = if takes_value {
        format!("{subject} {phrase} {}", value_label(&expected))
    } else {
        format!("{subject} {phrase}")
    };
    let Some(actual) = actual else {
        return AssertionResult {
            desc: format!("{head}: {missing}"),
            passed: op == "not_exists",
            actual: Value::Null,
        };
    };
    let (passed, note) = match op {
        "exists" => (true, None),
        "not_exists" => (false, None),
        "matches" => match regex_matches(&actual, &expected) {
            Ok(m) => (m, None),
            Err(e) => (false, Some(e)),
        },
        _ => (compare(&actual, op, &expected), None),
    };
    let desc = match note {
        Some(e) => format!("{head}: {e}"),
        None => format!("{head}: got {}", actual_label(&actual)),
    };
    AssertionResult {
        desc,
        passed,
        actual,
    }
}

/// Human wording for an assertion operator.
fn op_phrase(op: &str) -> String {
    match op {
        "eq" => "equals".into(),
        "ne" => "does not equal".into(),
        "contains" => "contains".into(),
        "lt" => "is less than".into(),
        "gt" => "is greater than".into(),
        "lte" => "is at most".into(),
        "gte" => "is at least".into(),
        "exists" => "exists".into(),
        "not_exists" => "does not exist".into(),
        "matches" => "matches".into(),
        other => format!("{other} (unknown operator)"),
    }
}

/// The response header `name` (case-insensitive) from a `[{key,value}]` array;
/// repeated headers join with `, `. `None` when absent (or `name` is empty).
pub fn header_value(headers: &Value, name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let vals: Vec<&str> = headers
        .as_array()?
        .iter()
        .filter(|h| {
            h.get("key")
                .and_then(Value::as_str)
                .is_some_and(|k| k.eq_ignore_ascii_case(name))
        })
        .filter_map(|h| h.get("value").and_then(Value::as_str))
        .collect();
    if vals.is_empty() {
        None
    } else {
        Some(vals.join(", "))
    }
}

/// `matches`: `expected` is a regex tested against the stringified actual.
/// Pattern length and compiled size are capped; the haystack is cut at
/// [`MAX_MATCH_HAYSTACK`] (on a char boundary).
fn regex_matches(actual: &Value, expected: &Value) -> std::result::Result<bool, String> {
    let pattern = coerce_str(expected);
    if pattern.len() > MAX_REGEX_PATTERN {
        return Err(format!(
            "pattern is longer than {MAX_REGEX_PATTERN} characters"
        ));
    }
    let re = regex::RegexBuilder::new(&pattern)
        .size_limit(MAX_REGEX_COMPILED)
        .build()
        .map_err(|_| "invalid regular expression".to_string())?;
    let hay = coerce_str(actual);
    let mut cut = hay.len().min(MAX_MATCH_HAYSTACK);
    while !hay.is_char_boundary(cut) {
        cut -= 1;
    }
    Ok(re.is_match(&hay[..cut]))
}

/// A compact label for an actual value in a report line — long values (a
/// whole body) are cut so the report stays readable.
fn actual_label(v: &Value) -> String {
    const MAX: usize = 120;
    let s = value_label(v);
    if s.chars().count() <= MAX {
        s
    } else {
        let cut: String = s.chars().take(MAX).collect();
        format!("{cut}…")
    }
}

/// Apply a comparison operator between an actual and expected JSON value.
fn compare(actual: &Value, op: &str, expected: &Value) -> bool {
    let num = |f: fn(f64, f64) -> bool| match (as_number(actual), as_number(expected)) {
        (Some(a), Some(b)) => f(a, b),
        _ => false,
    };
    match op {
        "eq" => loose_eq(actual, expected),
        "ne" => !loose_eq(actual, expected),
        "contains" => contains(actual, expected),
        "lt" => num(|a, b| a < b),
        "gt" => num(|a, b| a > b),
        "lte" => num(|a, b| a <= b),
        "gte" => num(|a, b| a >= b),
        _ => false,
    }
}

/// Equality that bridges JSON/string/number representation gaps: `200 == "200"`,
/// numeric tolerance via f64, and exact match for everything else.
fn loose_eq(a: &Value, b: &Value) -> bool {
    if a == b {
        return true;
    }
    if let (Some(x), Some(y)) = (as_number(a), as_number(b)) {
        return (x - y).abs() < f64::EPSILON;
    }
    coerce_str(a) == coerce_str(b)
}

/// `contains` semantics: substring for strings, membership for arrays, key
/// presence for objects (matching the expected as a key or a contained value).
fn contains(actual: &Value, expected: &Value) -> bool {
    match actual {
        Value::String(s) => s.contains(&coerce_str(expected)),
        Value::Array(arr) => arr.iter().any(|item| loose_eq(item, expected)),
        Value::Object(map) => {
            let needle = coerce_str(expected);
            map.contains_key(&needle) || map.values().any(|v| loose_eq(v, expected))
        }
        // Fall back to a stringified haystack (e.g. number contains digits).
        other => coerce_str(other).contains(&coerce_str(expected)),
    }
}

/// Interpret a value as f64 when it is a number or a numeric string.
fn as_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// Stringify a value for comparison (strings keep their content; others use
/// their JSON form, e.g. numbers → "200").
fn coerce_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// A compact label for an expected value used in assertion descriptions.
fn value_label(v: &Value) -> String {
    match v {
        Value::String(s) => format!("\"{s}\""),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn keys(v: &Value) -> Vec<(String, String)> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|e| {
                (
                    e["key"].as_str().unwrap().to_string(),
                    e["value"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn simple_get() {
        let p = parse_curl("curl https://api.example.com/users").unwrap();
        assert_eq!(p.method, "GET");
        assert_eq!(p.url, "https://api.example.com/users");
        assert_eq!(p.auth["type"], "none");
    }

    #[test]
    fn get_with_query_string() {
        let p = parse_curl("curl 'https://api.example.com/search?q=rust&page=2'").unwrap();
        assert_eq!(p.url, "https://api.example.com/search");
        let q = keys(&p.query);
        assert_eq!(
            q,
            vec![("q".into(), "rust".into()), ("page".into(), "2".into())]
        );
    }

    #[test]
    fn post_json_with_headers_and_bearer() {
        let cmd = r#"curl -X POST https://api.example.com/v1/items \
            -H 'Content-Type: application/json' \
            -H "Authorization: Bearer abc123" \
            -d '{"name":"widget","qty":3}'"#;
        let p = parse_curl(cmd).unwrap();
        assert_eq!(p.method, "POST");
        assert_eq!(p.url, "https://api.example.com/v1/items");
        assert_eq!(p.body_mode, "json");
        assert_eq!(p.auth["type"], "bearer");
        assert_eq!(p.auth["token"], "abc123");
        // Authorization header is folded into auth, Content-Type stays.
        let hdrs = keys(&p.headers);
        assert_eq!(
            hdrs,
            vec![("Content-Type".into(), "application/json".into())]
        );
        // Body parses back to JSON.
        let v: Value = serde_json::from_str(&p.body).unwrap();
        assert_eq!(v["name"], "widget");
    }

    #[test]
    fn data_implies_post() {
        let p = parse_curl("curl https://x.test/submit -d 'a=1&b=2'").unwrap();
        assert_eq!(p.method, "POST");
        // Not JSON, no form content-type → raw.
        assert_eq!(p.body_mode, "raw");
        assert_eq!(p.body, "a=1&b=2");
    }

    #[test]
    fn form_urlencode() {
        let p = parse_curl(
            "curl https://x.test/login --data-urlencode 'user=jane' --data-urlencode 'pw=p@ss'",
        )
        .unwrap();
        assert_eq!(p.method, "POST");
        assert_eq!(p.body_mode, "form");
        assert!(p.body.contains("user=jane"));
    }

    #[test]
    fn basic_auth_from_user_flag() {
        let p = parse_curl("curl -u alice:secret https://x.test/").unwrap();
        assert_eq!(p.auth["type"], "basic");
        assert_eq!(p.auth["username"], "alice");
        assert_eq!(p.auth["password"], "secret");
    }

    #[test]
    fn get_flag_sends_data_as_query() {
        let p = parse_curl("curl -G https://x.test/search -d 'q=rust' -d 'n=10'").unwrap();
        assert_eq!(p.method, "GET");
        assert_eq!(p.body_mode, "none");
        let q = keys(&p.query);
        assert_eq!(
            q,
            vec![("q".into(), "rust".into()), ("n".into(), "10".into())]
        );
    }

    #[test]
    fn ansi_c_quoting() {
        let p = parse_curl("curl https://x.test/ -H $'X-Token: a\\tb'").unwrap();
        let hdrs = keys(&p.headers);
        assert_eq!(hdrs, vec![("X-Token".into(), "a\tb".into())]);
    }

    #[test]
    fn attached_short_flags_are_split() {
        let p =
            parse_curl("curl -XPUT -HAccept:text/plain -ualice:pw -d'{\"a\":1}' https://x.test/a")
                .unwrap();
        assert_eq!(p.method, "PUT");
        assert_eq!(p.url, "https://x.test/a");
        assert_eq!(
            keys(&p.headers),
            vec![("Accept".into(), "text/plain".into())]
        );
        assert_eq!(p.auth["username"], "alice");
        assert_eq!(p.body_mode, "json");
    }

    #[test]
    fn value_flags_never_become_the_url() {
        let p = parse_curl(
            "curl --cacert ca.pem --resolve x.test:443:1.2.3.4 --retry 3 -c jar.txt \
             --max-redirs 5 -sSL --compressed https://x.test/ok",
        )
        .unwrap();
        assert_eq!(p.url, "https://x.test/ok");
        assert_eq!(p.method, "GET");
    }

    #[test]
    fn form_flag_builds_a_multipart_body() {
        let p = parse_curl(
            "curl https://x.test/upload -H 'Content-Type: multipart/form-data' \
             -F 'title=Hello world' -F 'file=@/tmp/pics/cat.png;type=image/png' \
             --form-string 'raw=@not-a-file'",
        )
        .unwrap();
        assert_eq!(p.method, "POST");
        assert_eq!(p.body_mode, "multipart");
        assert!(
            keys(&p.headers).is_empty(),
            "boundary content-type is reqwest's"
        );
        let rows: Value = serde_json::from_str(&p.body).unwrap();
        assert_eq!(
            rows[0],
            json!({"key":"title","type":"text","value":"Hello world"})
        );
        assert_eq!(
            rows[1],
            json!({"key":"file","type":"file","value":"","filename":"cat.png"})
        );
        assert_eq!(
            rows[2],
            json!({"key":"raw","type":"text","value":"@not-a-file"})
        );
    }

    #[test]
    fn json_flag_sets_body_and_headers() {
        let p = parse_curl(r#"curl --json '{"q":1}' https://x.test/j"#).unwrap();
        assert_eq!(p.method, "POST");
        assert_eq!(p.body_mode, "json");
        assert_eq!(p.body, r#"{"q":1}"#);
        assert_eq!(
            keys(&p.headers),
            vec![
                ("Content-Type".into(), "application/json".into()),
                ("Accept".into(), "application/json".into())
            ]
        );
    }

    #[test]
    fn data_urlencode_encodes_like_curl() {
        let p = parse_curl(
            "curl https://x.test/f --data-urlencode 'q=a b&c' --data-urlencode '=x/y' \
             --data-urlencode 'plain text'",
        )
        .unwrap();
        assert_eq!(p.body_mode, "form");
        assert_eq!(p.body, "q=a%20b%26c&x%2Fy&plain%20text");
    }

    #[test]
    fn convenience_flags_map_to_headers_auth_and_method() {
        let p = parse_curl(
            "curl -I -A 'otto/1' -e https://ref.test -b 'sid=1; theme=dark' \
             --oauth2-bearer tok123 https://x.test/",
        )
        .unwrap();
        assert_eq!(p.method, "HEAD");
        assert_eq!(p.auth, json!({"type":"bearer","token":"tok123"}));
        assert_eq!(
            keys(&p.headers),
            vec![
                ("User-Agent".into(), "otto/1".into()),
                ("Referer".into(), "https://ref.test".into()),
                ("Cookie".into(), "sid=1; theme=dark".into())
            ]
        );
        // `-b <file>` is a cookie jar on disk — dropped, not sent as a header.
        let p = parse_curl("curl -b cookies.txt https://x.test/").unwrap();
        assert!(keys(&p.headers).is_empty());
        assert_eq!(p.url, "https://x.test/");
    }

    #[test]
    fn basic_auth_header_decoded() {
        // base64 of "user:pass" = dXNlcjpwYXNz
        let p = parse_curl("curl https://x.test/ -H 'Authorization: Basic dXNlcjpwYXNz'").unwrap();
        assert_eq!(p.auth["type"], "basic");
        assert_eq!(p.auth["username"], "user");
        assert_eq!(p.auth["password"], "pass");
    }

    #[test]
    fn openapi_builds_paths_and_methods() {
        use chrono::Utc;
        let col = ApiCollection {
            id: "c1".into(),
            workspace_id: "w1".into(),
            name: "Test API".into(),
            parent_id: None,
            position: 0,
            created_at: Utc::now(),
        };
        let req = ApiRequest {
            id: "r1".into(),
            workspace_id: "w1".into(),
            collection_id: Some("c1".into()),
            name: "create item".into(),
            method: "POST".into(),
            url: "https://api.example.com/v1/items".into(),
            headers: json!([{"key":"X-Trace","value":"1","enabled":true}]),
            query: json!([{"key":"dry","value":"true","enabled":true}]),
            body_mode: "json".into(),
            body: r#"{"name":"x"}"#.into(),
            auth: json!({"type":"none"}),
            ssh_connection_id: None,
            extras: Some(json!({
                "v": 1,
                "docs_md": "Creates an item.",
                "settings": { "timeout_ms": 5000 },
                "graphql_variables": "",
                "scripts": { "pre": "pm.environment.set('x','1')" },
            })),
            position: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = collection_to_openapi(&col, &[req]);
        assert_eq!(doc["openapi"], "3.0.3");
        assert_eq!(doc["info"]["title"], "Test API");
        let op = &doc["paths"]["/v1/items"]["post"];
        assert_eq!(op["summary"], "create item");
        // Extras: docs → description; settings ride as x-otto-settings; empty
        // graphql_variables and scripts are NOT serialized.
        assert_eq!(op["description"], "Creates an item.");
        assert_eq!(op["x-otto-settings"]["timeout_ms"], 5000);
        assert!(op.get("x-otto-graphql-variables").is_none());
        assert!(op.to_string().find("pm.environment").is_none());
        assert!(op["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "dry"));
        assert!(op["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "X-Trace"));
        assert_eq!(
            op["requestBody"]["content"]["application/json"]["example"]["name"],
            "x"
        );
        assert!(op["responses"]["default"].is_object());
    }

    // --- json_path ----------------------------------------------------------

    #[test]
    fn json_path_dot_and_bracket() {
        let v = json!({
            "data": { "items": [ {"id": 10}, {"id": 20} ], "token": "abc" },
            "count": 2
        });
        assert_eq!(json_path(&v, "data.token").unwrap(), &json!("abc"));
        assert_eq!(json_path(&v, "data.items[0].id").unwrap(), &json!(10));
        assert_eq!(json_path(&v, "data.items[1].id").unwrap(), &json!(20));
        assert_eq!(json_path(&v, "count").unwrap(), &json!(2));
        // leading $ accepted
        assert_eq!(json_path(&v, "$.data.token").unwrap(), &json!("abc"));
        // empty path → root
        assert_eq!(json_path(&v, "").unwrap(), &v);
    }

    #[test]
    fn json_path_quoted_keys_and_misses() {
        let v = json!({ "odd key": 1, "nested": { "x": null } });
        assert_eq!(json_path(&v, "[\"odd key\"]").unwrap(), &json!(1));
        assert_eq!(json_path(&v, "nested['x']").unwrap(), &Value::Null);
        // missing key / out of range / wrong type → None
        assert!(json_path(&v, "nope").is_none());
        assert!(json_path(&v, "nested.x[5]").is_none());
        assert!(json_path(&json!([1, 2]), "[9]").is_none());
        assert!(json_path(&json!("scalar"), "a.b").is_none());
    }

    // --- eval_assertion -----------------------------------------------------

    #[test]
    fn assert_status_and_duration() {
        // status eq (number vs number, and number vs string both pass)
        assert!(
            eval_assertion(
                &json!({"kind":"status","op":"eq","value":200}),
                Some(200),
                5,
                &Value::Null
            )
            .passed
        );
        assert!(
            eval_assertion(
                &json!({"kind":"status","op":"eq","value":"200"}),
                Some(200),
                5,
                &Value::Null
            )
            .passed
        );
        assert!(
            eval_assertion(
                &json!({"kind":"status","op":"ne","value":404}),
                Some(200),
                5,
                &Value::Null
            )
            .passed
        );
        // missing status (network error) never matches a numeric eq
        assert!(
            !eval_assertion(
                &json!({"kind":"status","op":"eq","value":200}),
                None,
                5,
                &Value::Null
            )
            .passed
        );
        // duration lt / gt
        assert!(
            eval_assertion(
                &json!({"kind":"duration_ms","op":"lt","value":1000}),
                Some(200),
                42,
                &Value::Null
            )
            .passed
        );
        assert!(
            !eval_assertion(
                &json!({"kind":"duration_ms","op":"gt","value":1000}),
                Some(200),
                42,
                &Value::Null
            )
            .passed
        );
    }

    #[test]
    fn assert_json_path_ops() {
        let body = json!({ "token": "xyz", "n": 7, "tags": ["a", "b"], "msg": "hello world" });
        // eq on extracted string
        assert!(
            eval_assertion(
                &json!({"kind":"json_path","path":"token","op":"eq","value":"xyz"}),
                Some(200),
                1,
                &body
            )
            .passed
        );
        // numeric lt/gt
        assert!(
            eval_assertion(
                &json!({"kind":"json_path","path":"n","op":"gt","value":5}),
                Some(200),
                1,
                &body
            )
            .passed
        );
        assert!(
            !eval_assertion(
                &json!({"kind":"json_path","path":"n","op":"lt","value":5}),
                Some(200),
                1,
                &body
            )
            .passed
        );
        // contains on array membership and on string substring
        assert!(
            eval_assertion(
                &json!({"kind":"json_path","path":"tags","op":"contains","value":"a"}),
                Some(200),
                1,
                &body
            )
            .passed
        );
        assert!(
            eval_assertion(
                &json!({"kind":"json_path","path":"msg","op":"contains","value":"world"}),
                Some(200),
                1,
                &body
            )
            .passed
        );
        // missing path → Null, eq against a value fails (but ne passes)
        assert!(
            !eval_assertion(
                &json!({"kind":"json_path","path":"nope","op":"eq","value":"x"}),
                Some(200),
                1,
                &body
            )
            .passed
        );
        // …and a missing field fails `ne` too — it used to pass silently.
        let r = eval_assertion(
            &json!({"kind":"json_path","path":"nope","op":"ne","value":"x"}),
            Some(200),
            1,
            &body,
        );
        assert!(!r.passed);
        assert_eq!(r.desc, "nope does not equal \"x\": field missing");
        assert_eq!(r.actual, Value::Null);
    }

    #[test]
    fn assertion_desc_carries_the_actual_value() {
        let r = eval_assertion(
            &json!({"kind":"status","op":"lt","value":400}),
            Some(500),
            1,
            &Value::Null,
        );
        assert!(!r.passed);
        assert_eq!(r.desc, "Status code is less than 400: got 500");
        assert_eq!(r.actual, json!(500));
        let r = eval_assertion(
            &json!({"kind":"status","op":"eq","value":200}),
            None,
            0,
            &Value::Null,
        );
        assert_eq!(r.desc, "Status code equals 200: no response");
    }

    #[test]
    fn assert_exists_not_exists_lte_gte() {
        let body = json!({ "data": { "id": 7, "gone": null } });
        let a = |a: Value| eval_assertion(&a, Some(200), 10, &body).passed;
        assert!(a(
            json!({"kind":"json_path","path":"$.data.id","op":"exists"})
        ));
        // present-but-null still exists
        assert!(a(
            json!({"kind":"json_path","path":"data.gone","op":"exists"})
        ));
        assert!(!a(
            json!({"kind":"json_path","path":"data.nope","op":"exists"})
        ));
        assert!(a(
            json!({"kind":"json_path","path":"data.nope","op":"not_exists"})
        ));
        assert!(!a(
            json!({"kind":"json_path","path":"data.id","op":"not_exists"})
        ));
        assert!(a(
            json!({"kind":"json_path","path":"data.id","op":"lte","value":7})
        ));
        assert!(a(
            json!({"kind":"json_path","path":"data.id","op":"gte","value":"7"})
        ));
        assert!(!a(
            json!({"kind":"json_path","path":"data.id","op":"gte","value":8})
        ));
        assert!(a(json!({"kind":"duration_ms","op":"lte","value":10})));
    }

    #[test]
    fn assert_header_and_body_text_and_regex() {
        let headers = json!([
            {"key":"Content-Type","value":"application/json; charset=utf-8"},
            {"key":"set-cookie","value":"a=1"},
            {"key":"Set-Cookie","value":"b=2"},
        ]);
        let o = StepOutcome {
            status: Some(201),
            duration_ms: 3,
            body: &Value::Null,
            body_text: "order ORD-1234 created",
            headers: &headers,
        };
        let a = |a: Value| eval_assertion_with(&a, &o);
        assert!(
            a(json!({"kind":"header","path":"content-type","op":"contains","value":"json"})).passed
        );
        assert!(
            a(json!({"kind":"header","path":"Set-Cookie","op":"eq","value":"a=1, b=2"})).passed
        );
        let miss = a(json!({"kind":"header","path":"X-Trace","op":"ne","value":"x"}));
        assert!(!miss.passed);
        assert_eq!(
            miss.desc,
            "Header X-Trace does not equal \"x\": header missing"
        );
        assert!(a(json!({"kind":"header","path":"X-Trace","op":"not_exists"})).passed);
        assert!(a(json!({"kind":"body_text","op":"contains","value":"created"})).passed);
        assert!(a(json!({"kind":"body_text","op":"matches","value":"ORD-\\d{4}"})).passed);
        assert!(!a(json!({"kind":"body_text","op":"matches","value":"^created"})).passed);
        let bad = a(json!({"kind":"body_text","op":"matches","value":"("}));
        assert!(!bad.passed);
        assert!(
            bad.desc.ends_with("invalid regular expression"),
            "{}",
            bad.desc
        );
        let long =
            a(json!({"kind":"body_text","op":"matches","value":"a".repeat(MAX_REGEX_PATTERN + 1)}));
        assert!(!long.passed);
    }

    #[test]
    fn assert_unknown_kind_is_failed_not_panic() {
        let r = eval_assertion(
            &json!({"kind":"weird","op":"eq","value":1}),
            Some(200),
            1,
            &Value::Null,
        );
        assert!(!r.passed);
        assert!(r.desc.contains("unknown"));
    }
}
