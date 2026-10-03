//! Engine error normalisation — the driver-side half of the Database Explorer's
//! error UX (the UI half is `ui/src/modules/database/error-normalize.ts`).
//!
//! Every driver used to flatten its failure with `Display` into
//! `Error::Upstream(String)`. That threw away exactly what a person needs
//! (Postgres's HINT/DETAIL/SQLSTATE/POSITION — sqlx's `Display` prints only the
//! message) and kept what nobody can use (ClickHouse's 20–40-frame C++ stack
//! trace and `(version …)` banner, Mongo's `labels: {}, source: None, server
//! response: Some(Document({…}))` dump).
//!
//! The wire contract is unchanged: still one `Upstream` string. Structure rides
//! as **tagged trailer lines** after the message — `DETAIL:`, `HINT:`,
//! `SQLSTATE:`, `POSITION:`, `ERRNO:`, `CODE:`, `STREAMED_ROWS:`, `SUGGEST:` —
//! which the UI parses (and which read fine as plain text anywhere else, e.g. a
//! history row or an MCP tool result).

use otto_core::Error;

use crate::types::Engine;

/// Turn any driver-level error into an [`Error::Upstream`], keeping the
/// structured parts of the errors this crate knows (sqlx's Postgres/MySQL
/// database errors, Mongo's command/write errors, klickhouse's server
/// exception) instead of their lossy `Display`. Backs [`crate::types::upstream`],
/// so every `map_err(types::upstream)` in the drivers gets it for free.
pub fn upstream_any<E: std::fmt::Display + 'static>(e: E) -> Error {
    let any = &e as &dyn std::any::Any;
    if let Some(e) = any.downcast_ref::<sqlx::Error>() {
        return Error::Upstream(sqlx_message(e));
    }
    if let Some(e) = any.downcast_ref::<mongodb::error::Error>() {
        return Error::Upstream(mongo_message(e));
    }
    if let Some(e) = any.downcast_ref::<klickhouse::KlickhouseError>() {
        return Error::Upstream(ch_native_message(e));
    }
    Error::Upstream(e.to_string())
}

// --- sqlx (Postgres / MySQL) --------------------------------------------------

/// A sqlx error as `{message}` + tagged trailer lines. Postgres keeps
/// DETAIL/HINT/SQLSTATE/POSITION; MySQL keeps ERRNO/SQLSTATE; both drop sqlx's
/// `error returned from database: ` prefix. Non-database variants keep their
/// own (already short) text.
pub fn sqlx_message(e: &sqlx::Error) -> String {
    let Some(db) = e.as_database_error() else {
        return e.to_string();
    };
    if let Some(pg) = db.try_downcast_ref::<sqlx::postgres::PgDatabaseError>() {
        let position = match pg.position() {
            Some(sqlx::postgres::PgErrorPosition::Original(n)) => Some(n),
            _ => None,
        };
        return pg_message(pg.message(), pg.detail(), pg.hint(), pg.code(), position);
    }
    if let Some(my) = db.try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>() {
        return mysql_message(my.message(), my.number(), my.code());
    }
    db.message().to_string()
}

/// Postgres: `{message}\nDETAIL: …\nHINT: …\nSQLSTATE: …\nPOSITION: n`, empty
/// parts omitted. `position` is the server's 1-based character offset into the
/// statement as sent.
pub fn pg_message(
    message: &str,
    detail: Option<&str>,
    hint: Option<&str>,
    code: &str,
    position: Option<usize>,
) -> String {
    let mut out = message.trim().to_string();
    push_tag(&mut out, "DETAIL", detail);
    push_tag(&mut out, "HINT", hint);
    push_tag(&mut out, "SQLSTATE", Some(code));
    if let Some(p) = position {
        push_tag(&mut out, "POSITION", Some(&p.to_string()));
    }
    out
}

/// MySQL: `{message}\nERRNO: n\nSQLSTATE: s`.
pub fn mysql_message(message: &str, number: u16, state: Option<&str>) -> String {
    let mut out = message.trim().to_string();
    push_tag(&mut out, "ERRNO", Some(&number.to_string()));
    push_tag(&mut out, "SQLSTATE", state);
    out
}

fn push_tag(out: &mut String, tag: &str, value: Option<&str>) {
    let Some(v) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return;
    };
    // A multi-line DETAIL stays one tagged line — the trailer is line-based.
    let v = v.replace('\n', " ");
    out.push('\n');
    out.push_str(tag);
    out.push_str(": ");
    out.push_str(&v);
}

// --- ClickHouse -----------------------------------------------------------------

/// A klickhouse (native protocol) error. A server exception becomes
/// `Code: {code}. DB::Exception: {message}` — the HTTP interface's shape, so
/// one UI parser covers both transports — and its stack trace goes to the
/// debug log, never into the message.
pub fn ch_native_message(e: &klickhouse::KlickhouseError) -> String {
    match e {
        klickhouse::KlickhouseError::ServerException {
            code,
            name,
            message,
            stack_trace,
        } => {
            tracing::debug!(code = *code, name = %name, stack_trace = %stack_trace, "clickhouse server exception");
            ch_server_exception(*code, name, message)
        }
        other => other.to_string(),
    }
}

/// Build the cleaned `Code: …` text for a native server exception.
pub fn ch_server_exception(code: i32, name: &str, message: &str) -> String {
    let mut body = message.trim();
    // klickhouse's message already starts with `DB::Exception: ` (and the server
    // sometimes doubles it); strip every copy, then add exactly one back.
    while let Some(rest) = body.strip_prefix("DB::Exception:") {
        body = rest.trim_start();
    }
    let prefix = if name.is_empty() {
        "DB::Exception"
    } else {
        name
    };
    clean_ch_message(&format!("Code: {code}. {prefix}: {body}"))
}

/// Clean a ClickHouse error text (HTTP body or a native exception): cut the
/// stack trace, drop the trailing `(version X (official build))`, collapse a
/// duplicated `DB::Exception: ` prefix, shorten an `Expected one of: …` list
/// past 6 items, and drop the `FORMAT JSONCompact` this driver appends (it
/// shows up in syntax-error excerpts but is not the user's SQL).
pub fn clean_ch_message(raw: &str) -> String {
    let mut text = raw.replace("\r\n", "\n");
    if let Some(i) = text.find("\nStack trace:") {
        text.truncate(i);
    }
    // Native stack frames: `0. DB::Exception::Exception(...) @ 0x… in /usr/bin/clickhouse`.
    let mut kept: Vec<&str> = Vec::new();
    for line in text.lines() {
        if is_stack_frame(line) {
            break;
        }
        kept.push(line);
    }
    let mut text = kept.join("\n");
    while text.contains("DB::Exception: DB::Exception:") {
        text = text.replace("DB::Exception: DB::Exception:", "DB::Exception:");
    }
    text = text.replace("\nFORMAT JSONCompact", "");
    text = strip_version(text.trim());
    collapse_expected(&text)
}

/// `0. DB::…`, ` 1. …` — a numbered frame line (always starts at frame 0).
fn is_stack_frame(line: &str) -> bool {
    let t = line.trim_start();
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0 && t[digits..].starts_with(". ") && (t.contains(" @ 0x") || t.starts_with("0. "))
}

/// Drop a trailing ` (version 24.8.4.13 (official build))`.
fn strip_version(text: &str) -> String {
    if let Some(i) = text.rfind(" (version ") {
        let tail = &text[i..];
        if tail.ends_with(')') && !tail[1..].contains('\n') {
            return text[..i].trim_end().to_string();
        }
    }
    text.to_string()
}

/// Shorten `Expected one of: a, b, c, …` to its first 6 items + `…`.
fn collapse_expected(text: &str) -> String {
    const KEEP: usize = 6;
    let marker = "Expected one of: ";
    let Some(start) = text.find(marker) else {
        return text.to_string();
    };
    let list_start = start + marker.len();
    let rest = &text[list_start..];
    // The list runs to the `(CODE_NAME)` trailer, a newline, or the end.
    let end = [rest.find(" ("), rest.find('\n')]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(rest.len());
    let list = rest[..end].trim_end().trim_end_matches('.');
    let items: Vec<&str> = list.split(", ").collect();
    if items.len() <= KEEP {
        return text.to_string();
    }
    format!(
        "{}{}, ….{}",
        &text[..list_start],
        items[..KEEP].join(", "),
        &rest[end..]
    )
}

/// A ClickHouse failure that arrived AFTER the 200 status (a mid-stream
/// failure): either the `"exception"` field a `JSONCompact` reply carries when
/// `http_write_exception_in_output_format` is on, or — with it off — a broken
/// JSON body ending in a raw `Code: N. DB::Exception: …`. Searches only the
/// last 4 KB of `body`. `None` when there's no such trailer.
pub fn ch_trailing_exception(body: &str) -> Option<String> {
    const TAIL: usize = 4096;
    let mut from = body.len().saturating_sub(TAIL);
    while !body.is_char_boundary(from) {
        from += 1;
    }
    let tail = &body[from..];
    let mut search = tail;
    let mut found = None;
    // The LAST `Code: <digits>. ` in the tail — earlier ones may be row data.
    while let Some(i) = search.find("Code: ") {
        let after = &search[i + 6..];
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 && after[digits..].starts_with(". ") {
            found = Some(tail.len() - search.len() + i);
        }
        search = &search[i + 6..];
    }
    let i = found?;
    let mut text = tail[i..].to_string();
    // The JSON shape escapes it inside a string; this path sees raw text, but a
    // stray closing quote/brace from the broken envelope may trail it.
    text = text.trim_end_matches(['"', '}', '\n', ' ']).to_string();
    Some(clean_ch_message(&text))
}

/// The message for a ClickHouse query that failed mid-stream after `rows`
/// rows had already arrived (`None` = an unknown number).
pub fn ch_midstream_message(exception: &str, rows: Option<usize>) -> String {
    let rows = rows.map_or_else(|| "unknown".to_string(), |n| n.to_string());
    format!("{}\nSTREAMED_ROWS: {rows}", clean_ch_message(exception))
}

// --- MongoDB --------------------------------------------------------------------

/// A Mongo driver error without its `labels: {}, source: None, server
/// response: Some(Document({…}))` dump: the server's own message plus a
/// `CODE:` trailer, the first write error (+N more), or — for an unreachable
/// deployment — the first server's real error pulled out of the topology dump.
pub fn mongo_message(e: &mongodb::error::Error) -> String {
    use mongodb::error::{ErrorKind, WriteFailure};
    match e.kind.as_ref() {
        ErrorKind::Command(c) => {
            let mut out = c.message.trim().to_string();
            let code = format!("{} {}", c.code, c.code_name);
            push_tag(&mut out, "CODE", Some(code.trim()));
            out
        }
        ErrorKind::Write(WriteFailure::WriteError(w)) => {
            let mut out = w.message.trim().to_string();
            push_tag(&mut out, "CODE", Some(&w.code.to_string()));
            out
        }
        ErrorKind::Write(WriteFailure::WriteConcernError(w)) => {
            let mut out = format!("Write concern failed: {}", w.message.trim());
            push_tag(&mut out, "CODE", Some(&w.code.to_string()));
            out
        }
        ErrorKind::InsertMany(m) => {
            let errs = m.write_errors.as_deref().unwrap_or_default();
            match errs.first() {
                Some(first) => {
                    let mut out = first.message.trim().to_string();
                    if errs.len() > 1 {
                        out.push_str(&format!(" (+{} more)", errs.len() - 1));
                    }
                    push_tag(&mut out, "CODE", Some(&first.code.to_string()));
                    out
                }
                None => match &m.write_concern_error {
                    Some(w) => format!("Write concern failed: {}", w.message.trim()),
                    None => "insert_many failed".to_string(),
                },
            }
        }
        ErrorKind::ServerSelection { message, .. } => mongo_selection_message(message),
        ErrorKind::Authentication { message, .. } => message.trim().to_string(),
        other => strip_mongo_noise(&other.to_string()),
    }
}

/// `Server selection timeout: No available servers. Topology: { … Servers: [ {
/// Address: db:27017, …, Error: Kind: I/O error: Connection refused (os error
/// 61), labels: {}, … } ] }` → `Can't reach MongoDB at db:27017: I/O error:
/// Connection refused (os error 61)`.
pub fn mongo_selection_message(message: &str) -> String {
    let address = message.split("Address: ").nth(1).map(|s| {
        s.split([',', ' ', '}'])
            .next()
            .unwrap_or_default()
            .to_string()
    });
    let error = message.split("Error: Kind: ").nth(1).map(|s| {
        let end = s.find(", labels:").unwrap_or(s.len());
        s[..end].trim().to_string()
    });
    match (address, error) {
        (Some(a), Some(e)) if !a.is_empty() => format!("Can't reach MongoDB at {a}: {e}"),
        (None, Some(e)) => format!("Can't reach MongoDB: {e}"),
        (Some(a), None) if !a.is_empty() => {
            format!("Can't reach MongoDB at {a}: no server answered in time")
        }
        _ => strip_mongo_noise(message),
    }
}

/// Drop a trailing `, labels: {…}, source: …, server response: …` dump.
fn strip_mongo_noise(text: &str) -> String {
    match text.find(", labels: ") {
        Some(i) => text[..i].trim().to_string(),
        None => text.trim().to_string(),
    }
}

// --- "Did you mean" (E8) ----------------------------------------------------------

/// What an engine error says is missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnknownName {
    Column(String),
    Table(String),
}

/// Recognise an "unknown column / table" error (MySQL 1054/1146, Postgres
/// 42703/42P01, ClickHouse 47/60 when the server gave no `Maybe you meant`).
/// Reads the normalised message (with its tagged trailer lines).
pub fn unknown_name(engine: Engine, message: &str) -> Option<UnknownName> {
    let msg = message.strip_prefix("upstream: ").unwrap_or(message);
    match engine {
        Engine::Mysql => {
            let errno = tag_value(msg, "ERRNO");
            if errno == Some("1054") {
                return quoted_after(msg, "Unknown column '", '\'').map(UnknownName::Column);
            }
            if errno == Some("1146") {
                return quoted_after(msg, "Table '", '\'')
                    .map(|t| UnknownName::Table(last_part(&t)));
            }
            None
        }
        Engine::Postgres => {
            if msg.contains("HINT: Perhaps you meant") {
                return None;
            }
            match tag_value(msg, "SQLSTATE") {
                Some("42703") => quoted_after(msg, "column \"", '"')
                    .or_else(|| {
                        // `column u.nme does not exist` (qualified, unquoted).
                        let rest = msg.strip_prefix("column ")?;
                        rest.split_whitespace().next().map(str::to_string)
                    })
                    .map(|c| UnknownName::Column(last_part(&c))),
                Some("42P01") => {
                    quoted_after(msg, "relation \"", '"').map(|t| UnknownName::Table(last_part(&t)))
                }
                _ => None,
            }
        }
        Engine::Clickhouse => {
            if msg.contains("Maybe you meant") {
                return None;
            }
            let code = msg
                .strip_prefix("Code: ")
                .map(|s| s.split('.').next().unwrap_or_default());
            match code {
                Some("47") => quoted_after(msg, "identifier `", '`')
                    .or_else(|| quoted_after(msg, "Missing columns: '", '\''))
                    .map(UnknownName::Column),
                Some("60") => quoted_after(msg, "Table ", ' ')
                    .or_else(|| quoted_after(msg, "identifier '", '\''))
                    .map(|t| UnknownName::Table(last_part(&t))),
                _ => None,
            }
        }
        Engine::Mongodb | Engine::Redis => None,
    }
}

/// The value of a `TAG: value` trailer line.
pub fn tag_value<'a>(msg: &'a str, tag: &str) -> Option<&'a str> {
    msg.lines().find_map(|l| {
        l.strip_prefix(tag)
            .and_then(|r| r.strip_prefix(": "))
            .map(str::trim)
    })
}

fn quoted_after(msg: &str, open: &str, close: char) -> Option<String> {
    let start = msg.find(open)? + open.len();
    let rest = &msg[start..];
    let end = rest.find(close).unwrap_or(rest.len());
    let v = rest[..end].trim_end_matches('.').trim();
    (!v.is_empty()).then(|| v.to_string())
}

fn last_part(name: &str) -> String {
    name.rsplit('.').next().unwrap_or(name).to_string()
}

/// Nearest candidate names to `token` (case-insensitive optimal-string-
/// alignment distance ≤ 2, and less than half the token), best first, at most
/// 3. `preferred` names (e.g. columns of tables the statement mentions) win
/// ties.
pub fn nearest_names<'a>(
    token: &str,
    candidates: impl IntoIterator<Item = (&'a str, bool)>,
) -> Vec<String> {
    let t = token.to_lowercase();
    let max = 2.min(t.chars().count().saturating_sub(1) / 2).max(1);
    let mut scored: Vec<(usize, bool, &str)> = Vec::new();
    for (name, preferred) in candidates {
        let n = name.to_lowercase();
        if n == t {
            continue;
        }
        let d = osa_distance(&t, &n);
        if d <= max && !scored.iter().any(|(_, _, s)| *s == name) {
            scored.push((d, !preferred, name));
        }
    }
    scored.sort();
    scored
        .into_iter()
        .take(3)
        .map(|(_, _, n)| n.to_string())
        .collect()
}

/// Optimal string alignment distance (Levenshtein + adjacent transposition).
fn osa_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=m {
        d[0][j] = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = v;
        }
    }
    d[n][m]
}

/// Suggestions for an unknown column/table from a cached completion snapshot:
/// columns of tables named in `statement` are preferred.
pub fn suggest_from_snapshot(
    unknown: &UnknownName,
    snap: &crate::complete::SchemaSnapshot,
    statement: &str,
) -> Vec<String> {
    let stmt = statement.to_lowercase();
    let mentioned = |table: &str| stmt.contains(&table.to_lowercase());
    match unknown {
        UnknownName::Table(t) => {
            nearest_names(t, snap.objects.iter().map(|o| (o.name.as_str(), false)))
        }
        UnknownName::Column(c) => nearest_names(
            c,
            snap.objects.iter().flat_map(|o| {
                let pref = mentioned(&o.name);
                o.fields.iter().map(move |f| (f.name.as_str(), pref))
            }),
        ),
    }
}

/// Append a `SUGGEST: a, b` trailer to an Upstream message (no-op when empty).
pub fn with_suggestions(message: String, names: &[String]) -> String {
    if names.is_empty() {
        return message;
    }
    format!("{message}\nSUGGEST: {}", names.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixtures: the raw → clean table in review 04-db-editor-errors.md.

    #[test]
    fn ch_native_exception_drops_stack_and_double_prefix() {
        let msg = "DB::Exception: Table default.userz does not exist. Maybe you meant default.users?. (UNKNOWN_TABLE)";
        let out = ch_server_exception(60, "DB::Exception", msg);
        assert_eq!(
            out,
            "Code: 60. DB::Exception: Table default.userz does not exist. Maybe you meant default.users?. (UNKNOWN_TABLE)"
        );
        // And the whole klickhouse Display text, should it ever reach the cleaner.
        let raw = "Code: 60. DB::Exception: DB::Exception: Table default.userz does not exist. (UNKNOWN_TABLE)\n0. DB::Exception::Exception(DB::Exception::MessageMasked&&, int, bool) @ 0x000000000c6f4a3b in /usr/bin/clickhouse\n1. DB::Foo @ 0x1 in /usr/bin/clickhouse";
        assert_eq!(
            clean_ch_message(raw),
            "Code: 60. DB::Exception: Table default.userz does not exist. (UNKNOWN_TABLE)"
        );
    }

    #[test]
    fn ch_http_strips_version_and_stack_trace() {
        let raw = "Code: 47. DB::Exception: Unknown expression identifier `nme` in scope SELECT nme FROM users. Maybe you meant: ['name']. (UNKNOWN_IDENTIFIER) (version 24.8.4.13 (official build))";
        assert_eq!(
            clean_ch_message(raw),
            "Code: 47. DB::Exception: Unknown expression identifier `nme` in scope SELECT nme FROM users. Maybe you meant: ['name']. (UNKNOWN_IDENTIFIER)"
        );
        let with_trace = "Code: 241. DB::Exception: Memory limit (for query) exceeded: would use 9.31 GiB (attempt to allocate chunk of 4194304 bytes), maximum: 9.31 GiB.: While executing AggregatingTransform. (MEMORY_LIMIT_EXCEEDED)\nStack trace:\n0. x @ 0x1\n (version 24.8.4.13 (official build))";
        assert_eq!(
            clean_ch_message(with_trace),
            "Code: 241. DB::Exception: Memory limit (for query) exceeded: would use 9.31 GiB (attempt to allocate chunk of 4194304 bytes), maximum: 9.31 GiB.: While executing AggregatingTransform. (MEMORY_LIMIT_EXCEEDED)"
        );
    }

    #[test]
    fn ch_syntax_error_collapses_expected_list_and_format_echo() {
        let raw = "Code: 62. DB::Exception: Syntax error: failed at position 8 ('FORM') (line 1, col 8): FORM users\nFORMAT JSONCompact. Expected one of: token, Comma, AS, FROM, PREWHERE, WHERE, GROUP BY, ORDER BY, LIMIT. (SYNTAX_ERROR) (version 24.8.4.13 (official build))";
        assert_eq!(
            clean_ch_message(raw),
            "Code: 62. DB::Exception: Syntax error: failed at position 8 ('FORM') (line 1, col 8): FORM users. Expected one of: token, Comma, AS, FROM, PREWHERE, WHERE, …. (SYNTAX_ERROR)"
        );
        // A short list is left alone.
        let short = "Syntax error. Expected one of: FROM, AS. (SYNTAX_ERROR)";
        assert_eq!(clean_ch_message(short), short);
    }

    #[test]
    fn ch_auth_and_readonly_keep_message() {
        let raw = "Code: 516. DB::Exception: default: Authentication failed: password is incorrect, or there is no user with such name.. (AUTHENTICATION_FAILED) (version 24.3.1.1 (official build))";
        assert_eq!(
            clean_ch_message(raw),
            "Code: 516. DB::Exception: default: Authentication failed: password is incorrect, or there is no user with such name.. (AUTHENTICATION_FAILED)"
        );
    }

    #[test]
    fn ch_trailing_exception_finds_the_last_code() {
        let body = "{\"meta\":[{\"name\":\"x\",\"type\":\"UInt8\"}],\"data\":[[1],[2]\nCode: 241. DB::Exception: Memory limit (for query) exceeded. (MEMORY_LIMIT_EXCEEDED) (version 24.8.4.13 (official build))\n";
        assert_eq!(
            ch_trailing_exception(body).as_deref(),
            Some("Code: 241. DB::Exception: Memory limit (for query) exceeded. (MEMORY_LIMIT_EXCEEDED)")
        );
        assert_eq!(ch_trailing_exception("{\"data\":[[1]"), None);
        // `Code: ` without the `N. ` shape is row data, not an exception.
        assert_eq!(ch_trailing_exception("[\"Code: abc\"]"), None);
        assert_eq!(
            ch_midstream_message("Code: 241. DB::Exception: boom. (X)", Some(12400)),
            "Code: 241. DB::Exception: boom. (X)\nSTREAMED_ROWS: 12400"
        );
    }

    #[test]
    fn pg_message_keeps_hint_detail_code_position() {
        assert_eq!(
            pg_message(
                "column \"nme\" does not exist",
                None,
                Some("Perhaps you meant to reference the column \"users.name\"."),
                "42703",
                Some(8),
            ),
            "column \"nme\" does not exist\nHINT: Perhaps you meant to reference the column \"users.name\".\nSQLSTATE: 42703\nPOSITION: 8"
        );
        assert_eq!(
            pg_message(
                "duplicate key value violates unique constraint \"users_email_key\"",
                Some("Key (email)=(a@b.c) already exists."),
                None,
                "23505",
                None,
            ),
            "duplicate key value violates unique constraint \"users_email_key\"\nDETAIL: Key (email)=(a@b.c) already exists.\nSQLSTATE: 23505"
        );
    }

    #[test]
    fn mysql_message_keeps_errno_and_state() {
        assert_eq!(
            mysql_message("Unknown column 'nme' in 'field list'", 1054, Some("42S22")),
            "Unknown column 'nme' in 'field list'\nERRNO: 1054\nSQLSTATE: 42S22"
        );
        assert_eq!(mysql_message("x", 2013, None), "x\nERRNO: 2013");
    }

    #[test]
    fn sqlx_non_database_errors_keep_their_text() {
        assert_eq!(
            sqlx_message(&sqlx::Error::PoolTimedOut),
            "pool timed out while waiting for an open connection"
        );
        // Generic path: a non-database error type is plain Display.
        assert!(matches!(upstream_any("boom"), Error::Upstream(s) if s == "boom"));
    }

    #[test]
    fn mongo_selection_pulls_out_the_real_error() {
        let raw = "Server selection timeout: No available servers. Topology: { Type: Unknown, Servers: [ { Address: db:27017, Type: Unknown, Error: Kind: I/O error: Connection refused (os error 61), labels: {}, source: None, server response: None } ] }";
        assert_eq!(
            mongo_selection_message(raw),
            "Can't reach MongoDB at db:27017: I/O error: Connection refused (os error 61)"
        );
        assert_eq!(
            strip_mongo_noise("Kind: boom, labels: {}, source: None"),
            "Kind: boom"
        );
    }

    #[test]
    fn unknown_name_per_engine() {
        assert_eq!(
            unknown_name(
                Engine::Mysql,
                "Unknown column 'nme' in 'field list'\nERRNO: 1054\nSQLSTATE: 42S22"
            ),
            Some(UnknownName::Column("nme".into()))
        );
        assert_eq!(
            unknown_name(
                Engine::Mysql,
                "Table 'app.userz' doesn't exist\nERRNO: 1146"
            ),
            Some(UnknownName::Table("userz".into()))
        );
        assert_eq!(
            unknown_name(
                Engine::Postgres,
                "column \"nme\" does not exist\nSQLSTATE: 42703\nPOSITION: 8"
            ),
            Some(UnknownName::Column("nme".into()))
        );
        assert_eq!(
            unknown_name(
                Engine::Postgres,
                "relation \"public.userz\" does not exist\nSQLSTATE: 42P01"
            ),
            Some(UnknownName::Table("userz".into()))
        );
        // The server already suggested — don't second-guess it.
        assert_eq!(
            unknown_name(
                Engine::Postgres,
                "column \"nme\" does not exist\nHINT: Perhaps you meant to reference the column \"users.name\".\nSQLSTATE: 42703"
            ),
            None
        );
        assert_eq!(
            unknown_name(
                Engine::Clickhouse,
                "Code: 47. DB::Exception: Unknown expression identifier `nme` in scope SELECT nme FROM users. (UNKNOWN_IDENTIFIER)"
            ),
            Some(UnknownName::Column("nme".into()))
        );
        assert_eq!(
            unknown_name(
                Engine::Clickhouse,
                "Code: 60. DB::Exception: Table default.userz does not exist. Maybe you meant default.users?. (UNKNOWN_TABLE)"
            ),
            None
        );
        assert_eq!(
            unknown_name(
                Engine::Clickhouse,
                "Code: 60. DB::Exception: Table default.userz does not exist. (UNKNOWN_TABLE)"
            ),
            Some(UnknownName::Table("userz".into()))
        );
    }

    #[test]
    fn nearest_names_ranks_and_prefers() {
        let cands = [
            ("name", false),
            ("email", false),
            ("nmae", true),
            ("id", false),
        ];
        assert_eq!(nearest_names("nme", cands), vec!["nmae", "name"]);
        assert!(nearest_names("zzzzzz", [("name", false)]).is_empty());
        assert_eq!(
            nearest_names("userz", [("users", false), ("orders", false)]),
            vec!["users"]
        );
    }

    #[test]
    fn suggestions_from_snapshot() {
        use crate::complete::{FieldSnap, ObjKind, ObjectSnap, Rank, SchemaSnapshot};
        let snap = SchemaSnapshot {
            databases: vec![],
            objects: vec![
                ObjectSnap {
                    name: "users".into(),
                    kind: ObjKind::Table,
                    fields: vec![FieldSnap::new("name", None, Rank::Plain)],
                    fields_ready: true,
                },
                ObjectSnap {
                    name: "pets".into(),
                    kind: ObjKind::Table,
                    fields: vec![FieldSnap::new("nm", None, Rank::Plain)],
                    fields_ready: true,
                },
            ],
            routines: vec![],
        };
        let col = UnknownName::Column("nme".into());
        assert_eq!(
            suggest_from_snapshot(&col, &snap, "SELECT nme FROM users"),
            vec!["name", "nm"]
        );
        assert_eq!(
            suggest_from_snapshot(&UnknownName::Table("user".into()), &snap, ""),
            vec!["users"]
        );
        assert_eq!(
            with_suggestions("m".into(), &["a".into(), "b".into()]),
            "m\nSUGGEST: a, b"
        );
        assert_eq!(with_suggestions("m".into(), &[]), "m");
    }
}
