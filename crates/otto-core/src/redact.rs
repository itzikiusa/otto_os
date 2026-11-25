//! Redaction — a regex-free, dependency-light pass that strips likely secrets
//! and PII from text and JSON before it leaves a trust boundary. Two callers:
//!
//! * the first-party agent tools / "send-to-agent" packets, which must show the
//!   operator *what* was redacted before a packet is handed to an agent; and
//! * the database / broker "mask PII/prod" presets.
//!
//! It is deliberately conservative and heuristic (no entropy guessing): it
//! redacts values under sensitive JSON keys, and a small set of high-confidence
//! text shapes (JWTs, AWS access keys, PEM blocks, `Bearer` tokens, emails).
//! `Redacted::hits` reports the kinds + counts so a UI can summarize the diff.
//!
//! **Data** (DB rows, debugger locals, process output, HTTP responses bound for
//! an agent) gets a stronger pass, never used on code or prose: a value is
//! masked when the NAME it is stored under is sensitive ([`sensitive_name_kind`]
//! — column, variable / struct field, env var, header, Redis key), and inside
//! text a value written under a sensitive name is masked in the assignment forms
//! debuggers and loggers print (`PASSWORD=x`, `password: x`, `Password:"x y"`,
//! `"api_key": "x"`, `token='x'`, `Secret: main.S {…}` with its braces,
//! `Authorization: Bearer x`), as are DSN passwords (`user:pw@tcp(…)`,
//! `scheme://user:pw@host`). Entry points: [`redact_named`], [`redact_row`],
//! [`redact_vars`], [`redact_data_json`], [`redact_data_text`].
//!
//! Not caught (no parser): an unquoted value with spaces is masked up to its
//! first space (`Password:two words` from Go `%+v` leaks `words`); a composite
//! under a sensitive name that spans lines; secrets under innocent names that
//! have no known shape (`Note: "hunter2"`); a DSN password containing `/`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// One class of redaction and how many were applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactionHit {
    pub kind: String,
    pub count: usize,
}

/// Result of a redaction pass: the cleaned value plus a per-kind tally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Redacted<T> {
    pub value: T,
    pub hits: Vec<RedactionHit>,
}

const PLACEHOLDER: &str = "[redacted]";

/// Tally helper: accumulates kind→count and emits a stable, sorted hit list.
#[derive(Default)]
struct Tally(BTreeMap<&'static str, usize>);

impl Tally {
    fn bump(&mut self, kind: &'static str) {
        *self.0.entry(kind).or_insert(0) += 1;
    }
    fn into_hits(self) -> Vec<RedactionHit> {
        self.0
            .into_iter()
            .map(|(kind, count)| RedactionHit {
                kind: kind.to_string(),
                count,
            })
            .collect()
    }
}

/// The redaction kind a NAME suggests — a JSON key, table column, variable or
/// struct field (`cfg.DB.Password`: pass the whole path), env var, header or
/// Redis key — if it looks sensitive. Case- and separator-insensitive
/// (`api_key`, `X-Api-Key`, `apiKey` alike); short words only as a whole
/// segment (`dbPwd`, `user_pass`, `DSN`, `salt`, but not `bypass`).
pub fn sensitive_name_kind(name: &str) -> Option<&'static str> {
    let k: String = name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let has = |p: &str| k.contains(p);
    if has("password") || has("passwd") || has("passphrase") {
        Some("password")
    } else if has("apikey") || has("accesskey") {
        Some("api_key")
    } else if has("privatekey") || has("signingkey") {
        Some("private_key")
    } else if has("secret") {
        Some("secret")
    } else if has("authorization") {
        Some("authorization")
    } else if has("credential") {
        Some("credential")
    } else if has("cookie") {
        Some("cookie")
    } else if has("token") || has("authcode") {
        Some("token")
    } else if has("connectionstring") || has("connstring") || k.ends_with("connstr") {
        Some("connection_string")
    } else {
        match name_segment(name, &["pwd", "pass", "dsn", "salt"])? {
            "dsn" => Some("connection_string"),
            "salt" => Some("secret"),
            _ => Some("password"),
        }
    }
}

/// Whether [`sensitive_name_kind`] masks values stored under `name`.
pub fn is_sensitive_name(name: &str) -> bool {
    sensitive_name_kind(name).is_some()
}

/// The first of `wanted` that is a whole segment of `name`, split at
/// non-alphanumerics and camel-case humps (`dbPWD`, `DBPass` → `PWD`, `Pass`).
fn name_segment(name: &str, wanted: &[&'static str]) -> Option<&'static str> {
    let b = name.as_bytes();
    let mut start: Option<usize> = None;
    for i in 0..=b.len() {
        let alnum = i < b.len() && b[i].is_ascii_alphanumeric();
        let hump = alnum
            && i > 0
            && b[i].is_ascii_uppercase()
            && (b[i - 1].is_ascii_lowercase()
                || b[i - 1].is_ascii_digit()
                || (b[i - 1].is_ascii_uppercase()
                    && b.get(i + 1).is_some_and(u8::is_ascii_lowercase)));
        if !alnum || hump {
            if let Some(s) = start.take() {
                if let Some(w) = wanted.iter().find(|w| name[s..i].eq_ignore_ascii_case(w)) {
                    return Some(w);
                }
            }
        }
        if alnum && start.is_none() {
            start = Some(i);
        }
    }
    None
}

/// Trim a small set of surrounding punctuation that commonly wraps tokens in
/// prose, without stripping characters that are part of tokens (`.-_/+=@`).
fn trim_punct(w: &str) -> &str {
    w.trim_matches(|c: char| {
        matches!(
            c,
            '"' | '\'' | '(' | ')' | '[' | ']' | ',' | ';' | ':' | '<' | '>' | '{' | '}'
        )
    })
}

fn is_b64url_dotted(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+' | '/' | '='))
}

fn looks_like_email(w: &str) -> bool {
    let at = match w.find('@') {
        Some(i) => i,
        None => return false,
    };
    let (local, rest) = w.split_at(at);
    let domain = &rest[1..];
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

fn is_token_body(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

fn is_basic_b64(w: &str) -> bool {
    if w.len() < 8
        || !w
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
    {
        return false;
    }
    let has_upper = w.bytes().any(|b| b.is_ascii_uppercase());
    let has_lower = w.bytes().any(|b| b.is_ascii_lowercase());
    let has_digit = w.bytes().any(|b| b.is_ascii_digit());
    w.ends_with('=') || (has_upper && has_lower) || has_digit || w.contains('+') || w.contains('/')
}

/// Whether `tail` (starting at `-----BEGIN`) begins with a real PEM header line
/// for a key/certificate/PGP block, even if the closing `-----END` was clipped
/// by a read/log size cap.
fn looks_like_pem_header(tail: &str) -> bool {
    let first_line = tail.lines().next().unwrap_or(tail).trim_end_matches('\r');
    let Some(rest) = first_line.strip_prefix("-----BEGIN ") else {
        return false;
    };
    let Some(label) = rest.strip_suffix("-----") else {
        return false;
    };
    !label.is_empty()
        && label
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || matches!(b, b' ' | b'-'))
        && (label.contains("KEY") || label.contains("CERTIFICATE") || label.contains("PGP"))
}

/// Classify a single (already punctuation-trimmed) word. `auth_scheme` is set
/// when the previous word was a literal `Bearer`/`Basic` auth scheme keyword.
fn classify_word(w: &str, auth_scheme: Option<&'static str>) -> Option<&'static str> {
    if w.starts_with("eyJ") && w.len() > 20 && is_b64url_dotted(w) {
        return Some("jwt");
    }
    if w.len() == 20
        && (w.starts_with("AKIA") || w.starts_with("ASIA"))
        && w[4..]
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return Some("aws_key");
    }
    if (w.starts_with("ghp_")
        || w.starts_with("gho_")
        || w.starts_with("ghu_")
        || w.starts_with("ghs_")
        || w.starts_with("ghr_"))
        && w.len() >= 20
        && is_token_body(&w[4..])
    {
        return Some("github_token");
    }
    if let Some(rest) = w.strip_prefix("github_pat_") {
        if rest.len() >= 20 && is_token_body(rest) {
            return Some("github_token");
        }
    }
    if let Some(rest) = w
        .strip_prefix("sk-ant-")
        .or_else(|| w.strip_prefix("sk-proj-"))
    {
        if rest.len() >= 16 && is_token_body(rest) {
            return Some("api_key");
        }
    }
    if let Some(rest) = w.strip_prefix("sk-") {
        if rest.len() >= 24 && is_token_body(rest) && rest.bytes().any(|b| b.is_ascii_digit()) {
            return Some("api_key");
        }
    }
    if (w.starts_with("xoxb-")
        || w.starts_with("xoxp-")
        || w.starts_with("xoxo-")
        || w.starts_with("xoxa-")
        || w.starts_with("xoxr-"))
        && w.len() >= 20
        && is_token_body(&w[5..])
    {
        return Some("slack_token");
    }
    if w.len() == 39 && w.starts_with("AIza") && is_token_body(&w[4..]) {
        return Some("api_key");
    }
    if looks_like_email(w) {
        return Some("email");
    }
    match auth_scheme {
        Some("bearer") if w.len() >= 8 && is_b64url_dotted(w) => return Some("bearer"),
        Some("basic") if is_basic_b64(w) => return Some("basic"),
        _ => {}
    }
    None
}

/// Redact a chunk of free text. Whitespace is preserved; matched words are
/// swapped for `[redacted]`. PEM key blocks (possibly multi-line) are collapsed.
pub fn redact_text(input: &str) -> Redacted<String> {
    let mut tally = Tally::default();
    let cleaned = redact_text_into(input, &mut tally);
    Redacted {
        value: cleaned,
        hits: tally.into_hits(),
    }
}

fn redact_text_into(input: &str, tally: &mut Tally) -> String {
    // 1. Collapse PEM blocks first (they span multiple whitespace-delimited words).
    let mut s = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("-----BEGIN") {
        s.push_str(&rest[..start]);
        let tail = &rest[start..];
        if let Some(end_rel) = tail.find("-----END") {
            // Include the closing "-----END ...-----" line up to the next newline.
            let after_end = &tail[end_rel..];
            let close = after_end
                .find('\n')
                .map(|n| end_rel + n)
                .unwrap_or(tail.len());
            s.push_str(PLACEHOLDER);
            tally.bump("private_key");
            rest = &tail[close..];
        } else if looks_like_pem_header(tail) {
            // Truncated PEM block (e.g. clipped at a read/log cap before
            // `-----END`): fail closed and redact the remainder of the block.
            s.push_str(PLACEHOLDER);
            tally.bump("private_key");
            rest = "";
            break;
        } else {
            s.push_str("-----BEGIN");
            rest = &tail["-----BEGIN".len()..];
        }
    }
    s.push_str(rest);

    // 2. Word scan for the remaining single-token shapes.
    let mut out = String::with_capacity(s.len());
    let mut word = String::new();
    let mut auth_scheme: Option<&'static str> = None;
    let flush = |word: &mut String,
                 out: &mut String,
                 auth_scheme: &mut Option<&'static str>,
                 tally: &mut Tally| {
        if word.is_empty() {
            return;
        }
        let trimmed = trim_punct(word);
        let next_scheme = match trimmed {
            "Bearer" | "bearer" => Some("bearer"),
            "Basic" | "basic" => Some("basic"),
            _ => None,
        };
        match classify_word(trimmed, *auth_scheme) {
            Some(kind) => {
                // Preserve any leading/trailing punctuation we trimmed.
                let lead = &word[..word.find(trimmed).unwrap_or(0)];
                let tail_start = word
                    .find(trimmed)
                    .map(|i| i + trimmed.len())
                    .unwrap_or(word.len());
                out.push_str(lead);
                out.push_str(PLACEHOLDER);
                out.push_str(&word[tail_start..]);
                tally.bump(kind);
            }
            None => {
                // Also inspect the value half of `KEY=VALUE` pairs (`.env`,
                // query strings, CLI flags like `--token=ghp_...`).
                if let Some((lhs, rhs)) = trimmed.split_once('=') {
                    let val = trim_punct(rhs);
                    if let Some(kind) = classify_word(val, None) {
                        let trimmed_start = word.find(trimmed).unwrap_or(0);
                        let rhs_offset = trimmed_start + lhs.len() + 1;
                        let val_offset = rhs_offset + rhs.find(val).unwrap_or(0);
                        out.push_str(&word[..val_offset]);
                        out.push_str(PLACEHOLDER);
                        out.push_str(&word[val_offset + val.len()..]);
                        tally.bump(kind);
                    } else {
                        out.push_str(word);
                    }
                } else {
                    out.push_str(word);
                }
            }
        }
        *auth_scheme = next_scheme;
        word.clear();
    };
    for ch in s.chars() {
        if ch.is_whitespace() {
            flush(&mut word, &mut out, &mut auth_scheme, tally);
            out.push(ch);
        } else {
            word.push(ch);
        }
    }
    flush(&mut word, &mut out, &mut auth_scheme, tally);
    out
}

/// Redact a JSON value: values under sensitive keys are replaced wholesale;
/// every other string is passed through [`redact_text`].
pub fn redact_json(input: &Value) -> Redacted<Value> {
    let mut tally = Tally::default();
    let value = redact_value(input, &mut tally, false);
    Redacted {
        value,
        hits: tally.into_hits(),
    }
}

/// [`redact_json`] for DATA (a debug stop, an HTTP response, query output):
/// strings get [`redact_data_text`] instead of [`redact_text`].
pub fn redact_data_json(input: &Value) -> Redacted<Value> {
    let mut tally = Tally::default();
    let value = redact_value(input, &mut tally, true);
    Redacted {
        value,
        hits: tally.into_hits(),
    }
}

/// Redact rendered data text (a debugger value, `os.Environ()`, process
/// output, a cell): [`redact_text`]'s shapes, plus values written under a
/// sensitive name and DSN passwords (see the module doc for what is and is not
/// caught). Not for code or prose: `password: String` there is not a secret.
pub fn redact_data_text(input: &str) -> Redacted<String> {
    let mut tally = Tally::default();
    let value = redact_data_text_into(input, &mut tally);
    Redacted {
        value,
        hits: tally.into_hits(),
    }
}

/// The value stored under `name` (a column, a variable or field path, a Redis
/// key): wholesale `[redacted]` when the name is sensitive (`null` stays
/// `null` — "no password" is not a secret), else [`redact_data_json`]. A
/// non-empty `hits` means something was masked ("changed · hidden").
pub fn redact_named(name: &str, value: &Value) -> Redacted<Value> {
    let mut tally = Tally::default();
    let value = match sensitive_name_kind(name) {
        Some(kind) if !value.is_null() => {
            tally.bump(kind);
            Value::String(PLACEHOLDER.to_string())
        }
        _ => redact_value(value, &mut tally, true),
    };
    Redacted {
        value,
        hits: tally.into_hits(),
    }
}

/// A result row with named columns, masked in place by [`redact_named`];
/// returns, per cell, whether it was masked. A cell past the last column name
/// is treated as unnamed. For Redis `key, type, value, ttl` rows pass the
/// key itself as the value's name.
pub fn redact_row<S: AsRef<str>>(columns: &[S], row: &mut [Value]) -> Vec<bool> {
    row.iter_mut()
        .enumerate()
        .map(|(i, cell)| {
            let r = redact_named(columns.get(i).map_or("", AsRef::as_ref), cell);
            *cell = r.value;
            !r.hits.is_empty()
        })
        .collect()
}

/// Debugger locals / struct fields as `name → rendered value` (a string, or
/// an object of children), masked in place by [`redact_named`]; returns the
/// names whose value was masked.
pub fn redact_vars(vars: &mut serde_json::Map<String, Value>) -> Vec<String> {
    let mut masked = Vec::new();
    for (name, value) in vars.iter_mut() {
        let r = redact_named(name, value);
        if !r.hits.is_empty() {
            masked.push(name.clone());
        }
        *value = r.value;
    }
    masked
}

fn redact_value(v: &Value, tally: &mut Tally, data: bool) -> Value {
    match v {
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, val) in map {
                if let Some(kind) = sensitive_name_kind(k) {
                    out.insert(k.clone(), Value::String(PLACEHOLDER.to_string()));
                    tally.bump(kind);
                } else {
                    out.insert(k.clone(), redact_value(val, tally, data));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => {
            Value::Array(items.iter().map(|i| redact_value(i, tally, data)).collect())
        }
        Value::String(s) if data => Value::String(redact_data_text_into(s, tally)),
        Value::String(s) => Value::String(redact_text_into(s, tally)),
        other => other.clone(),
    }
}

fn redact_data_text_into(input: &str, tally: &mut Tally) -> String {
    let shaped = redact_text_into(input, tally);
    mask_assignments(&shaped, tally).unwrap_or(shaped)
}

/// Mask values written under a sensitive name, and DSN passwords, in `s`
/// (one byte scan; every cut is at an ASCII byte, so UTF-8 stays whole).
/// `None` when nothing was masked.
fn mask_assignments(s: &str, tally: &mut Tally) -> Option<String> {
    let b = s.as_bytes();
    let mut out: Option<String> = None;
    let mut copied = 0;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c != b':' && c != b'=' {
            i += 1;
            continue;
        }
        let rest = &b[i + 1..];
        if c == b':' && rest.starts_with(b"//") {
            i += 3; // `scheme://`: the userinfo after it is checked at its own `:`
            continue;
        }
        if c == b'=' && rest.first() == Some(&b'=') {
            i += 2; // `==`
            continue;
        }
        let sep_end = i + 1 + usize::from(c == b':' && rest.first() == Some(&b'='));
        let named = key_before(b, i)
            .and_then(|(ks, ke)| sensitive_name_kind(&s[ks..ke]))
            .and_then(|kind| value_after(b, sep_end).map(|span| (span, kind)));
        let span = named.or_else(|| {
            (c == b':')
                .then(|| dsn_password(b, i))
                .flatten()
                .map(|span| (span, "password"))
        });
        match span {
            Some(((vs, ve), kind)) if !is_literal_non_secret(&s[vs..ve]) => {
                let o = out.get_or_insert_with(|| String::with_capacity(s.len()));
                o.push_str(&s[copied..vs]);
                o.push_str(PLACEHOLDER);
                tally.bump(kind);
                copied = ve;
                i = ve.max(i + 1);
            }
            _ => i += 1,
        }
    }
    let mut o = out?;
    o.push_str(&s[copied..]);
    Some(o)
}

/// The name right before the `:` / `=` at `sep` (spaces allowed between):
/// `"X-Api-Key"`, `'password'`, or a bare `cfg.DB_PASSWORD` / `--token`.
fn key_before(b: &[u8], sep: usize) -> Option<(usize, usize)> {
    const MAX_KEY: usize = 128;
    let mut end = sep;
    while end > 0 && matches!(b[end - 1], b' ' | b'\t') {
        end -= 1;
    }
    let last = *b.get(end.checked_sub(1)?)?;
    if matches!(last, b'"' | b'\'') {
        let close = end - 1;
        let floor = close.saturating_sub(MAX_KEY);
        let open = floor + b[floor..close].iter().rposition(|&x| x == last)?;
        return Some((open + 1, close));
    }
    let mut start = end;
    while start > 0
        && end - start < MAX_KEY
        && (b[start - 1].is_ascii_alphanumeric() || matches!(b[start - 1], b'_' | b'-' | b'.'))
    {
        start -= 1;
    }
    // `auth:token:42` is a Redis-style path, not a `token` field.
    let path_segment = b[sep] == b':' && start > 0 && b[start - 1] == b':';
    (start < end && !path_segment).then_some((start, end))
}

/// The value after a separator ending at `from`, on the same line: a quoted
/// string (inside the quotes), else a bare run up to `, ; &`, a space or an
/// unbalanced closer, with a following `{…}` / `[…]` body
/// (`Secret: main.S {…}`). After an auth scheme (`Bearer x`) only the
/// credential.
fn value_after(b: &[u8], from: usize) -> Option<(usize, usize)> {
    let skip_spaces = |mut j: usize| {
        while j < b.len() && matches!(b[j], b' ' | b'\t') {
            j += 1;
        }
        j
    };
    let j = skip_spaces(from);
    let first = *b.get(j)?;
    if matches!(first, b'"' | b'\'' | b'`') {
        let mut k = j + 1;
        while k < b.len() && b[k] != first && b[k] != b'\n' {
            k += if b[k] == b'\\' { 2 } else { 1 };
        }
        return Some((j + 1, k.min(b.len())));
    }
    let mut end = bare_end(b, j);
    let word = &b[j..end];
    if [&b"bearer"[..], b"basic", b"token", b"digest"]
        .iter()
        .any(|s| word.eq_ignore_ascii_case(s))
    {
        let cred = skip_spaces(end);
        return Some((cred, bare_end(b, cred)));
    }
    let body = skip_spaces(end);
    if end > j && matches!(b.get(body), Some(b'{' | b'[')) {
        end = bare_end(b, body);
    }
    Some((j, end))
}

/// End of a bare value starting at `j`: brackets nest; at depth 0 a space,
/// `, ; &`, a quote or an unbalanced closer ends it; a newline always does.
fn bare_end(b: &[u8], mut j: usize) -> usize {
    let mut depth = 0u32;
    while j < b.len() {
        match b[j] {
            b'\n' | b'\r' => break,
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => match depth.checked_sub(1) {
                Some(d) => depth = d,
                None => break,
            },
            b' ' | b'\t' | b',' | b';' | b'&' | b'"' | b'\'' | b'`' if depth == 0 => break,
            _ => {}
        }
        j += 1;
    }
    j
}

/// `user:PASSWORD@host` around the `:` at `colon` (Go `root:pw@tcp(…)/db`,
/// or the userinfo of `scheme://user:pw@host`): the password span, up to the
/// LAST `@` before a space, quote, bracket, `/`, `?` or `#` (a raw `@` in it stays
/// covered), when a host follows.
fn dsn_password(b: &[u8], colon: usize) -> Option<(usize, usize)> {
    let is_user =
        |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-' | b'~' | b'%' | b'+');
    let mut u = colon;
    while u > 0 && is_user(b[u - 1]) {
        u -= 1;
    }
    let boundary = u == 0
        || matches!(
            b[u - 1],
            b'/' | b' '
                | b'\t'
                | b'\n'
                | b'"'
                | b'\''
                | b'`'
                | b'='
                | b'('
                | b','
                | b'['
                | b'{'
                | b'<'
                | b':'
        );
    // An empty user only right after `scheme://` (`redis://:pw@cache`).
    let empty_user_ok = u == colon && b[..u].ends_with(b"//");
    if (u == colon && !empty_user_ok) || !boundary {
        return None;
    }
    let mut end = colon + 1;
    let mut at = None;
    while end < b.len()
        && !matches!(
            b[end],
            b' ' | b'\t'
                | b'\n'
                | b'\r'
                | b'"'
                | b'\''
                | b'`'
                | b','
                | b';'
                | b'/'
                | b'?'
                | b'#'
                | b'<'
                | b'>'
                | b'('
                | b')'
                | b'{'
                | b'}'
                | b'['
                | b']'
        )
    {
        if b[end] == b'@' {
            at = Some(end);
        }
        end += 1;
    }
    let at = at.filter(|&a| a > colon + 1)?;
    let host = *b.get(at + 1)?;
    (host.is_ascii_alphanumeric() || host == b'[').then_some((colon + 1, at))
}

/// A value that holds no secret: empty, already masked, or a null/bool literal.
fn is_literal_non_secret(v: &str) -> bool {
    v.is_empty()
        || v == PLACEHOLDER
        || ["null", "nil", "<nil>", "none", "undefined", "true", "false"]
            .iter()
            .any(|l| v.eq_ignore_ascii_case(l))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plain_text_passes_through() {
        let r = redact_text("the quick brown fox jumps over 3 lazy dogs");
        assert_eq!(r.value, "the quick brown fox jumps over 3 lazy dogs");
        assert!(r.hits.is_empty());
    }

    #[test]
    fn redacts_jwt_and_email_and_bearer() {
        let input = "auth Bearer abcdef123456 token eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.payload.sig mail a.b@example.com";
        let r = redact_text(input);
        assert!(r.value.contains(PLACEHOLDER));
        assert!(!r.value.contains("eyJhbGci"));
        assert!(!r.value.contains("a.b@example.com"));
        let kinds: Vec<&str> = r.hits.iter().map(|h| h.kind.as_str()).collect();
        assert!(kinds.contains(&"jwt"));
        assert!(kinds.contains(&"email"));
        assert!(kinds.contains(&"bearer"));
    }

    #[test]
    fn redacts_aws_key_and_pem() {
        let input = "key AKIAIOSFODNN7EXAMPLE then\n-----BEGIN RSA PRIVATE KEY-----\nMIIBjunk\n-----END RSA PRIVATE KEY-----\ntail";
        let r = redact_text(input);
        assert!(!r.value.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(!r.value.contains("MIIBjunk"));
        assert!(r.value.contains("tail"));
        let kinds: Vec<&str> = r.hits.iter().map(|h| h.kind.as_str()).collect();
        assert!(kinds.contains(&"aws_key"));
        assert!(kinds.contains(&"private_key"));
    }

    #[test]
    fn json_redacts_sensitive_keys_and_nested_strings() {
        let v = json!({
            "username": "alice",
            "password": "hunter2",
            "config": { "api_key": "xyz", "note": "contact a.b@example.com" },
            "items": [ { "Authorization": "Bearer t" } ]
        });
        let r = redact_json(&v);
        assert_eq!(r.value["username"], json!("alice"));
        assert_eq!(r.value["password"], json!(PLACEHOLDER));
        assert_eq!(r.value["config"]["api_key"], json!(PLACEHOLDER));
        assert!(r.value["config"]["note"]
            .as_str()
            .unwrap()
            .contains(PLACEHOLDER));
        assert_eq!(r.value["items"][0]["Authorization"], json!(PLACEHOLDER));
        let total: usize = r.hits.iter().map(|h| h.count).sum();
        assert!(total >= 4);
    }

    #[test]
    fn redacts_truncated_pem_private_key() {
        for header in [
            "-----BEGIN RSA PRIVATE KEY-----",
            "-----BEGIN OPENSSH PRIVATE KEY-----",
            "-----BEGIN EC PRIVATE KEY-----",
            "-----BEGIN PRIVATE KEY-----",
            "-----BEGIN PGP PRIVATE KEY BLOCK-----",
        ] {
            let input = format!("prefix\n{header}\nMIIEpAIBAAKCAQEAt0truncatedsecretbytes");
            let r = redact_text(&input);
            assert!(
                !r.value.contains("MIIEpAIBAAKCAQEAt0truncatedsecretbytes"),
                "key body leaked for {header}: {}",
                r.value
            );
            assert!(r.value.starts_with("prefix\n"));
            assert!(r.value.contains(PLACEHOLDER));
            assert!(r.hits.iter().any(|h| h.kind == "private_key"));
        }
        // Prose mentioning `-----BEGIN` without a real PEM key header passes through.
        let prose = redact_text("note: -----BEGIN is a marker in PEM files");
        assert_eq!(prose.value, "note: -----BEGIN is a marker in PEM files");
        assert!(prose.hits.is_empty());
    }

    #[test]
    fn redacts_modern_token_prefixes_and_basic_auth() {
        let gh = "ghp_1234567890abcdef1234567890abcdef1234";
        let gh_pat = "github_pat_11AAAAAA0123456789_abcdef0123456789ABCDEF";
        let anth = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz123456";
        let oai = "sk-proj-abcdefghijklmnopqrstuvwxyz1234567890";
        let slack = "xoxb-fake-token-for-redaction-test";
        let sts = format!("ASIA{}", "X".repeat(16));
        let goog = format!("AIza{}", "x".repeat(35));
        let basic = "dXNlcjpwYXNz";
        let input = format!(
            "gh={gh} pat={gh_pat} anth={anth} oai={oai} slack={slack} sts={sts} goog={goog} Authorization: Basic {basic}"
        );
        let r = redact_text(&input);
        for secret in [gh, gh_pat, anth, oai, slack, &sts, &goog, basic] {
            assert!(
                !r.value.contains(secret),
                "secret leaked ({secret}): {}",
                r.value
            );
        }
        // Normal English after "Basic" must not false-positive.
        let ok = redact_text("Basic configuration and sk-short are fine");
        assert_eq!(ok.value, "Basic configuration and sk-short are fine");
        assert!(ok.hits.is_empty());
    }

    // ---- data / named redaction ------------------------------------------

    fn data(s: &str) -> String {
        redact_data_text(s).value
    }

    #[test]
    fn sensitive_names_by_column_variable_field_header_and_key() {
        for n in [
            "password",
            "user_password",
            "DB_PASSWORD",
            "passwd",
            "Passphrase",
            "pwd",
            "dbPwd",
            "PWD",
            "user_pass",
            "DBPass",
            "pass",
            "api_key",
            "X-Api-Key",
            "apiKey",
            "access_key",
            "AWS_SECRET_ACCESS_KEY",
            "private_key",
            "signingKey",
            "signing_key",
            "client_secret",
            "Authorization",
            "Proxy-Authorization",
            "credentials",
            "Set-Cookie",
            "token",
            "refresh_token",
            "X-Auth-Token",
            "auth_code",
            "dsn",
            "DSN",
            "dbDSN",
            "DATABASE_DSN",
            "connStr",
            "conn_string",
            "ConnectionString",
            "salt",
            "password_salt",
            "cfg.DB.Password",
            "auth:token:42",
        ] {
            assert!(is_sensitive_name(n), "{n} should be sensitive");
        }
        for n in [
            "id",
            "name",
            "email",
            "host",
            "port",
            "user",
            "username",
            "bypass",
            "compass",
            "passenger",
            "passed",
            "OLDPWD",
            "reconnStrategy",
            "value",
            "key",
            "type",
            "ttl",
            "salty_snack",
            "created_at",
            "description",
            "author",
        ] {
            assert!(!is_sensitive_name(n), "{n} should not be sensitive");
        }
    }

    #[test]
    fn redact_json_masks_separator_variants_but_leaves_code_text_alone() {
        let r = redact_json(&json!({"X-Api-Key": "k1", "dbPwd": "p", "note": "password: String"}));
        assert_eq!(r.value["X-Api-Key"], json!(PLACEHOLDER));
        assert_eq!(r.value["dbPwd"], json!(PLACEHOLDER));
        // Code/prose strings keep the shapes-only pass (diffs, files, messages).
        assert_eq!(r.value["note"], json!("password: String"));
        assert_eq!(
            redact_text("let password = get(); token: Option<String>")
                .hits
                .len(),
            0
        );
    }

    #[test]
    fn data_text_masks_env_style_pairs() {
        let env = "PATH=/usr/bin\nDB_PASSWORD=hunter2\nGITHUB_TOKEN=abc123\nHOME=/Users/me\nexport API_KEY='k y'";
        assert_eq!(
            data(env),
            "PATH=/usr/bin\nDB_PASSWORD=[redacted]\nGITHUB_TOKEN=[redacted]\nHOME=/Users/me\nexport API_KEY='[redacted]'"
        );
        assert_eq!(
            data("--token=abc --verbose"),
            "--token=[redacted] --verbose"
        );
        assert_eq!(
            data("GET /cb?code=1&access_token=zzz&state=s"),
            "GET /cb?code=1&access_token=[redacted]&state=s"
        );
        assert_eq!(
            data("Server=db;User Id=sa;Password=hunter2;Database=app"),
            "Server=db;User Id=sa;Password=[redacted];Database=app"
        );
    }

    #[test]
    fn data_text_masks_debugger_struct_renderings() {
        // Delve (single line, as DAP renders a struct value), from a real spike.
        assert_eq!(
            data(
                r#"main.Config {Host: "localhost", Password: "hunter2 two", APIToken: "tok_abc123XYZ",...+3 more}"#
            ),
            r#"main.Config {Host: "localhost", Password: "[redacted]", APIToken: "[redacted]",...+3 more}"#
        );
        // Delve nested + map; DSN under a sensitive field name.
        assert_eq!(
            data(r#"DB: main.DB {DSN: "root:s3cr3t@tcp(127.0.0.1:3306)/app", MaxConns: 4}"#),
            r#"DB: main.DB {DSN: "[redacted]", MaxConns: 4}"#
        );
        assert_eq!(
            data(
                r#"map[string]string ["Authorization": "Bearer abcdef123456", "X-Api-Key": "k-999", "Accept": "*/*"]"#
            ),
            r#"map[string]string ["Authorization": "[redacted]", "X-Api-Key": "[redacted]", "Accept": "*/*"]"#
        );
        // Go %+v (unquoted).
        assert_eq!(
            data("{Host:localhost Password:hunter2 DB:{DSN:root:pw@tcp(h:3306)/app}}"),
            "{Host:localhost Password:[redacted] DB:{DSN:[redacted]}}"
        );
        // Python dict / dataclass repr, Node preview, Java toString.
        assert_eq!(
            data("{'user': 'bob', 'password': 'hunter2'}"),
            "{'user': 'bob', 'password': '[redacted]'}"
        );
        assert_eq!(
            data("Config(host='db', secret_key='abc', port=5432)"),
            "Config(host='db', secret_key='[redacted]', port=5432)"
        );
        assert_eq!(
            data("{host: 'db', password: 'x'}"),
            "{host: 'db', password: '[redacted]'}"
        );
        assert_eq!(
            data("Config{host='db', apiKey='abc'}"),
            "Config{host='db', apiKey='[redacted]'}"
        );
        // A composite under a sensitive name goes whole, braces included.
        assert_eq!(
            data(r#"cfg = {Secret: main.Secret {Value: "x", Id: 1}, Name: "n"}"#),
            r#"cfg = {Secret: [redacted], Name: "n"}"#
        );
        assert_eq!(
            data(r#"{"secrets": {"a": "b"}, "ok": 1}"#),
            r#"{"secrets": [redacted], "ok": 1}"#
        );
        // Escaped quotes stay inside the masked string.
        assert_eq!(
            data(r#"Password: "a\"b", X: 1"#),
            r#"Password: "[redacted]", X: 1"#
        );
    }

    #[test]
    fn data_text_masks_dsn_passwords_and_auth_headers() {
        assert_eq!(
            data("dial root:s3cr3t@tcp(127.0.0.1:3306)/app?parseTime=true"),
            "dial root:[redacted]@tcp(127.0.0.1:3306)/app?parseTime=true"
        );
        assert_eq!(
            data("url postgres://app:pg_pw@db:5432/x"),
            "url postgres://app:[redacted]@db:5432/x"
        );
        assert_eq!(
            data("mongodb://u:p@ss@h1,h2/db"),
            "mongodb://u:[redacted]@h1,h2/db"
        );
        assert_eq!(
            data("redis://:onlypw@cache:6379"),
            "redis://:[redacted]@cache:6379"
        );
        assert_eq!(
            data("Authorization: Bearer abc.def-ghi"),
            "Authorization: Bearer [redacted]"
        );
        assert_eq!(
            data("Authorization: Token abc"),
            "Authorization: Token [redacted]"
        );
        assert_eq!(
            data("Cookie: sid=abc; theme=dark"),
            "Cookie: [redacted]; theme=dark"
        );
        assert_eq!(data("password := \"x\""), "password := \"[redacted]\"");
        assert_eq!(
            data(r#"{"access_token":"abc","expires_in":3600}"#),
            r#"{"access_token":"[redacted]","expires_in":3600}"#
        );
    }

    #[test]
    fn data_text_leaves_ordinary_text_and_empty_values() {
        for s in [
            "at 12:30 the job ran; ratio=3:1 ok",
            "https://example.com:8080/path?q=1#frag",
            "git@github.com:org/repo.git",
            "password: null, token=, secret: nil, api_key: \"\"",
            "if a == b && c != d { x = 1 }",
            "Host: localhost, Port: 5432",
        ] {
            assert_eq!(data(s), s, "changed: {s}");
            assert!(redact_data_text(s).hits.is_empty(), "hits for: {s}");
        }
        // Idempotent: masking masked text changes nothing and counts nothing.
        let once = data("PASSWORD=x Password: \"y\" root:pw@tcp(h)/d");
        let twice = redact_data_text(&once);
        assert_eq!(twice.value, once);
        assert!(twice.hits.is_empty());
        // Multi-byte text around a mask stays valid.
        assert_eq!(data("ключ token=é€ ok"), "ключ token=[redacted] ok");
    }

    #[test]
    fn redact_row_masks_by_column_name_and_reports_cells() {
        let mut row = vec![
            json!(7),
            json!("bob"),
            json!("$2b$12$hash"),
            Value::Null,
            json!("uses root:pw@tcp(db)/x"),
            json!({"api_key": "k", "plan": "pro"}),
        ];
        let cols = [
            "id",
            "login",
            "password_hash",
            "reset_token",
            "notes",
            "meta",
        ];
        let masked = redact_row(&cols, &mut row);
        assert_eq!(masked, vec![false, false, true, false, true, true]);
        assert_eq!(row[0], json!(7));
        assert_eq!(row[2], json!(PLACEHOLDER));
        assert_eq!(row[3], Value::Null, "an empty secret column stays NULL");
        assert_eq!(row[4], json!("uses root:[redacted]@tcp(db)/x"));
        assert_eq!(row[5], json!({"api_key": PLACEHOLDER, "plan": "pro"}));
    }

    #[test]
    fn redact_row_masks_redis_values_by_key_name() {
        for (key, value, hidden) in [
            ("auth:token:42", "abc", true),
            ("user:1:password", "x", true),
            ("cache:page:home", "<html>", false),
        ] {
            let mut row = vec![json!(key), json!("string"), json!(value), json!(30)];
            let masked = redact_row(&["key", "type", key, "ttl"], &mut row);
            assert_eq!(masked[2], hidden, "{key}");
            assert_eq!(row[0], json!(key), "the key name itself is shown");
        }
    }

    #[test]
    fn redact_vars_masks_locals_by_name_and_inside_values() {
        let mut vars = json!({
            "pass": "hunter2",
            "cfg": "main.Config {Host: \"h\", Password: \"p\"}",
            "env": ["PATH=/bin", "DB_PASSWORD=x"],
            "n": "42",
            "cfg.DB.Password": "p2",
        })
        .as_object()
        .cloned()
        .unwrap();
        let mut masked = redact_vars(&mut vars);
        masked.sort(); // map order depends on serde_json's `preserve_order`
        assert_eq!(masked, vec!["cfg", "cfg.DB.Password", "env", "pass"]);
        assert_eq!(vars["pass"], json!(PLACEHOLDER));
        assert_eq!(
            vars["cfg"],
            json!("main.Config {Host: \"h\", Password: \"[redacted]\"}")
        );
        assert_eq!(vars["env"], json!(["PATH=/bin", "DB_PASSWORD=[redacted]"]));
        assert_eq!(vars["n"], json!("42"));
    }

    /// Runs per debug stop on up to 150 rows × N tables: 4 tables × 150 rows
    /// × 12 mixed cells plus 200 struct-rendered locals must stay well inside
    /// one frame of UI work even in an unoptimized test build.
    #[test]
    fn data_redaction_stays_inside_its_budget() {
        let cols: Vec<String> = [
            "id",
            "email",
            "name",
            "status",
            "payload",
            "dsn",
            "created_at",
            "notes",
            "token",
            "amount",
            "url",
            "meta",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let row = |i: usize| -> Vec<Value> {
            vec![
                json!(i),
                json!(format!("user{i}@example.com")),
                json!(format!("User Number {i}")),
                json!("active"),
                json!(format!(
                    r#"{{"order":{i},"items":[1,2,3],"note":"ship by 12:30, ratio=3:1"}}"#
                )),
                json!(format!("root:pw{i}@tcp(127.0.0.1:3306)/app?parseTime=true")),
                json!("2026-11-25T10:20:30Z"),
                json!("a fairly ordinary sentence about the order, nothing secret in it at all"),
                json!(format!("tok_{i:032}")),
                json!(i as f64 * 1.5),
                json!(format!(
                    "https://api.example.com/v1/orders/{i}?expand=items"
                )),
                json!({"plan": "pro", "flags": ["a", "b"], "nested": {"password": "x"}}),
            ]
        };
        let tables: Vec<Vec<Vec<Value>>> = (0..4).map(|_| (0..150).map(row).collect()).collect();
        let locals: serde_json::Map<String, Value> = (0..200)
            .map(|i| {
                (
                    format!("v{i}"),
                    json!(format!(
                        r#"main.Config {{Host: "h{i}", Port: {i}, Password: "p{i}", DB: main.DB {{DSN: "u:p@tcp(h)/d", Max: 4}}, Tags: []string len: 2, cap: 2, ["a","b"]}}"#
                    )),
                )
            })
            .collect();
        let started = std::time::Instant::now();
        let mut hidden = 0;
        for table in &tables {
            for r in table {
                let mut r = r.clone();
                hidden += redact_row(&cols, &mut r).iter().filter(|m| **m).count();
            }
        }
        let mut l = locals.clone();
        hidden += redact_vars(&mut l).len();
        let elapsed = started.elapsed();
        assert_eq!(
            hidden,
            4 * 150 * 4 + 200,
            "email, dsn, token and meta per row; every local"
        );
        eprintln!("one stop redacted in {elapsed:?}");
        assert!(
            elapsed < std::time::Duration::from_millis(250),
            "redacting one stop took {elapsed:?}"
        );
    }
}
