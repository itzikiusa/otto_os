//! Query placeholders for multi-target / parameterised runs.
//!
//! The SAME placeholder syntax the query editor's Variables bar already uses
//! (`ui/src/modules/database/sql-util.ts` — `matchVars` / `codeMask` /
//! `renderVar`), so a statement written for a single run sweeps over values
//! unchanged:
//!
//! - `:name` — skipped when the preceding char is a word char, `:` or `}` (so a
//!   Postgres `::cast`, a Redis key `user:123` or a `{tag}:field` never reads
//!   as a placeholder);
//! - `{{name}}` and `{name}` — NOT in Redis, where `{…}` is a Cluster hash tag.
//!
//! Only CODE positions count: a placeholder inside a string literal, a quoted
//! identifier or a comment is left alone. Substitution is server-side and
//! typed ([`VarType`]): a `string` becomes an engine-correct quoted, escaped
//! literal, a `number` must actually parse as one (anything else is refused —
//! never spliced in), and `raw` is the explicit, verbatim opt-out.

use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};

use crate::types::Engine;

/// How a placeholder value is rendered into the statement. Mirrors the
/// editor's `VarType`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VarType {
    /// A quoted string literal (escaped when `escape` is on). The default.
    #[default]
    String,
    /// A numeric literal, validated — a non-number is refused.
    Number,
    /// Spliced verbatim (identifiers, lists, expressions). Caller-controlled.
    Raw,
}

/// Placeholder grammar: `;`-delimited engines vs Redis' one-command-per-line,
/// with PostgreSQL's own lexing (the editor's `pg` split mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceholderMode {
    Sql,
    /// PostgreSQL: `#` is an operator (`#>>`, XOR), not a comment; `$tag$…$tag$`
    /// bodies are literals; `\` escapes only inside `E'…'`; no backtick quote.
    Postgres,
    Line,
}

impl PlaceholderMode {
    pub fn for_engine(engine: Engine) -> Self {
        match engine {
            Engine::Redis => PlaceholderMode::Line,
            Engine::Postgres => PlaceholderMode::Postgres,
            _ => PlaceholderMode::Sql,
        }
    }
}

/// One placeholder occurrence: `name` plus the byte span of the whole token
/// (`:name`, `{name}` or `{{name}}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placeholder {
    pub name: String,
    pub start: usize,
    pub end: usize,
}

/// Per-byte "is code" mask — the port of the editor's `codeMask`: `--` / `#`
/// line comments, `/* */` block comments, and `'` / `"` / `` ` `` quoted spans
/// (backtick is not a quote in Redis line mode; a backslash escapes the next
/// char and a doubled quote is a literal quote). Every delimiter is ASCII, so
/// scanning bytes is safe on UTF-8 input.
fn code_mask(s: &[u8], mode: PlaceholderMode) -> Vec<bool> {
    if mode == PlaceholderMode::Postgres {
        return pg_code_mask(s);
    }
    let n = s.len();
    let mut mask = vec![true; n];
    let mut off = |a: usize, b: usize| {
        for m in mask.iter_mut().take(b.min(n)).skip(a) {
            *m = false;
        }
    };
    let mut i = 0;
    while i < n {
        let c = s[i];
        let c2 = s.get(i + 1).copied();
        if (c == b'-' && c2 == Some(b'-')) || c == b'#' {
            let mut j = i;
            while j < n && s[j] != b'\n' {
                j += 1;
            }
            off(i, j);
            i = j;
            continue;
        }
        if c == b'/' && c2 == Some(b'*') {
            let mut j = i + 2;
            while j < n && !(s[j] == b'*' && s.get(j + 1) == Some(&b'/')) {
                j += 1;
            }
            let j = (j + 2).min(n);
            off(i, j);
            i = j;
            continue;
        }
        if c == b'\'' || c == b'"' || (mode != PlaceholderMode::Line && c == b'`') {
            let q = c;
            let mut j = i + 1;
            while j < n {
                if s[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if s[j] == q {
                    if s.get(j + 1) == Some(&q) {
                        j += 2;
                        continue;
                    }
                    j += 1;
                    break;
                }
                j += 1;
            }
            let j = j.min(n);
            off(i, j);
            i = j;
            continue;
        }
        i += 1;
    }
    mask
}

/// [`code_mask`] for PostgreSQL — the port of the editor's `pgSpanEnd`: `--`
/// line comments, NESTED `/* */` comments, `'…'` with only `''` doubling
/// (a backslash escapes only in an `E'…'` string), `"…"` identifiers, and
/// `$tag$…$tag$` dollar bodies (the same opener/closer rules as
/// [`crate::split`]). `#` and backtick are plain code.
fn pg_code_mask(s: &[u8]) -> Vec<bool> {
    let n = s.len();
    let mut mask = vec![true; n];
    let mut off = |a: usize, b: usize| {
        for m in mask.iter_mut().take(b.min(n)).skip(a) {
            *m = false;
        }
    };
    // Dollar tags are scanned by char like the splitter; built lazily since
    // most statements have no `$`.
    let mut chars: Option<Vec<(usize, char)>> = None;
    let mut i = 0;
    while i < n {
        let c = s[i];
        let c2 = s.get(i + 1).copied();
        if c == b'-' && c2 == Some(b'-') {
            let mut j = i;
            while j < n && s[j] != b'\n' {
                j += 1;
            }
            off(i, j);
            i = j;
            continue;
        }
        if c == b'/' && c2 == Some(b'*') {
            let mut depth = 1;
            let mut j = i + 2;
            while j < n && depth > 0 {
                if s[j] == b'/' && s.get(j + 1) == Some(&b'*') {
                    depth += 1;
                    j += 2;
                } else if s[j] == b'*' && s.get(j + 1) == Some(&b'/') {
                    depth -= 1;
                    j += 2;
                } else {
                    j += 1;
                }
            }
            let j = j.min(n);
            off(i, j);
            i = j;
            continue;
        }
        if c == b'\'' || c == b'"' {
            let prev = if i > 0 { s[i - 1] } else { 0 };
            let before = if i > 1 { s[i - 2] } else { 0 };
            let escape = c == b'\'' && (prev == b'E' || prev == b'e') && !is_word(before);
            let mut j = i + 1;
            while j < n {
                if escape && s[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if s[j] == c {
                    if s.get(j + 1) == Some(&c) {
                        j += 2;
                        continue;
                    }
                    j += 1;
                    break;
                }
                j += 1;
            }
            let j = j.min(n);
            off(i, j);
            i = j;
            continue;
        }
        // `$tag$` only opens where it can't be the tail of an identifier
        // (`a$b$`) — PostgreSQL allows `$` inside identifiers.
        if c == b'$' && !(i > 0 && (is_word(s[i - 1]) || s[i - 1] == b'$')) {
            let chars = chars.get_or_insert_with(|| {
                String::from_utf8_lossy(s)
                    .char_indices()
                    .collect::<Vec<_>>()
            });
            if let Ok(k) = chars.binary_search_by_key(&i, |&(b, _)| b) {
                if let Some((tag, body)) = crate::split::try_open_dollar(chars, k) {
                    let mut end = n;
                    let mut m = body;
                    while m < chars.len() {
                        if chars[m].1 == '$' {
                            if let Some(after) = crate::split::try_close_dollar(chars, m, &tag) {
                                end = chars.get(after).map_or(n, |&(b, _)| b);
                                break;
                            }
                        }
                        m += 1;
                    }
                    off(i, end);
                    i = end;
                    continue;
                }
            }
        }
        i += 1;
    }
    mask
}

fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

/// End (exclusive) of the identifier starting at `from` (`from` must be an
/// ident-start byte).
fn ident_end(s: &[u8], from: usize) -> usize {
    let mut e = from + 1;
    while e < s.len() && is_word(s[e]) {
        e += 1;
    }
    e
}

/// Every placeholder occurrence at a code position, ordered by `start`.
pub fn find_placeholders(sql: &str, mode: PlaceholderMode) -> Vec<Placeholder> {
    let s = sql.as_bytes();
    let n = s.len();
    let mask = code_mask(s, mode);
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        if !mask[i] {
            i += 1;
            continue;
        }
        let c = s[i];
        if c == b':' && i + 1 < n && is_ident_start(s[i + 1]) {
            let prev = if i > 0 { s[i - 1] } else { 0 };
            let e = ident_end(s, i + 1);
            if !(is_word(prev) || prev == b':' || prev == b'}') {
                out.push(Placeholder {
                    name: sql[i + 1..e].to_string(),
                    start: i,
                    end: e,
                });
                i = e;
                continue;
            }
        } else if c == b'{' && mode != PlaceholderMode::Line {
            // `{{name}}` first, so the whole token is replaced (never `{` +
            // `{name}` + `}`); the inner `{name}` is then skipped by its `{`
            // predecessor check.
            if s.get(i + 1) == Some(&b'{') && i + 2 < n && is_ident_start(s[i + 2]) {
                let e = ident_end(s, i + 2);
                if s.get(e) == Some(&b'}') && s.get(e + 1) == Some(&b'}') {
                    out.push(Placeholder {
                        name: sql[i + 2..e].to_string(),
                        start: i,
                        end: e + 2,
                    });
                    i = e + 2;
                    continue;
                }
            } else if i + 1 < n && is_ident_start(s[i + 1]) && !(i > 0 && s[i - 1] == b'{') {
                let e = ident_end(s, i + 1);
                if s.get(e) == Some(&b'}') {
                    out.push(Placeholder {
                        name: sql[i + 1..e].to_string(),
                        start: i,
                        end: e + 1,
                    });
                    i = e + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

/// Unique placeholder names in first-seen order.
pub fn placeholder_names(sql: &str, mode: PlaceholderMode) -> Vec<String> {
    let mut seen = Vec::<String>::new();
    for p in find_placeholders(sql, mode) {
        if !seen.contains(&p.name) {
            seen.push(p.name);
        }
    }
    seen
}

/// Replace every placeholder whose name has a rendered value; others are left
/// untouched. Right-to-left so earlier spans stay valid.
pub fn substitute(
    sql: &str,
    mode: PlaceholderMode,
    rendered: &std::collections::HashMap<String, String>,
) -> String {
    let mut out = sql.to_string();
    let mut hits = find_placeholders(sql, mode);
    hits.reverse();
    for p in hits {
        if let Some(v) = rendered.get(&p.name) {
            out.replace_range(p.start..p.end, v);
        }
    }
    out
}

/// A strict numeric literal: optional sign, digits with an optional fraction,
/// optional exponent. Hex, `NaN`, `1;DROP` and friends are refused.
fn is_number(v: &str) -> bool {
    let b = v.as_bytes();
    let mut i = 0;
    if matches!(b.first(), Some(b'+' | b'-')) {
        i += 1;
    }
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut digits = i - int_start;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let f = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        digits += i - f;
    }
    if digits == 0 {
        return false;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let e = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == e {
            return false;
        }
    }
    i == b.len()
}

/// Render one value as a literal for `engine`.
///
/// `string` literals per engine: MySQL / ClickHouse `'…'` with `\` and `'`
/// escaped (both treat backslash as an escape); PostgreSQL `'…'` with only `'`
/// doubled (standard-conforming strings — doubling a backslash would change
/// the value); MongoDB a JSON string (valid JavaScript and Extended JSON alike,
/// where SQL's `''` doubling would be a syntax error); Redis `"…"` with `"` and
/// `\` backslash-escaped. `escape: false` only drops the escaping, never the
/// quotes.
pub fn render_value(
    name: &str,
    value: &str,
    ty: VarType,
    escape: bool,
    engine: Engine,
) -> Result<String> {
    match ty {
        VarType::Raw => Ok(value.to_string()),
        VarType::Number => {
            let v = value.trim();
            if is_number(v) {
                Ok(v.to_string())
            } else {
                Err(Error::Invalid(format!(
                    "parameter `{name}`: `{}` is not a number (use type `string` or `raw`)",
                    clip(value, 40)
                )))
            }
        }
        VarType::String => Ok(match engine {
            Engine::Mysql | Engine::Clickhouse => {
                let inner = if escape {
                    value.replace('\\', "\\\\").replace('\'', "''")
                } else {
                    value.to_string()
                };
                format!("'{inner}'")
            }
            Engine::Postgres => {
                let inner = if escape {
                    value.replace('\'', "''")
                } else {
                    value.to_string()
                };
                format!("'{inner}'")
            }
            Engine::Mongodb => {
                if escape {
                    serde_json::to_string(value).unwrap_or_else(|_| format!("\"{value}\""))
                } else {
                    format!("\"{value}\"")
                }
            }
            Engine::Redis => {
                let inner = if escape {
                    value.replace('\\', "\\\\").replace('"', "\\\"")
                } else {
                    value.to_string()
                };
                format!("\"{inner}\"")
            }
        }),
    }
}

/// First `max` chars of `s` (for error messages), with an ellipsis when cut.
pub(crate) fn clip(s: &str, max: usize) -> String {
    let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    otto_core::text::clip_chars(&flat, max)
}

/// A valid placeholder name (`[A-Za-z_][A-Za-z0-9_]*`).
pub fn valid_name(name: &str) -> bool {
    let b = name.as_bytes();
    !b.is_empty() && is_ident_start(b[0]) && b.iter().all(|c| is_word(*c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn names(sql: &str) -> Vec<String> {
        placeholder_names(sql, PlaceholderMode::Sql)
    }

    #[test]
    fn finds_all_three_forms_in_first_seen_order() {
        assert_eq!(
            names(
                "SELECT * FROM t WHERE a = :brand AND b = {region} OR c = {{tier}} AND d = :brand"
            ),
            vec!["brand", "region", "tier"]
        );
    }

    #[test]
    fn ignores_casts_keys_strings_and_comments() {
        assert!(names("SELECT x::int, 'a :b {c}', \"{{d}}\" -- :e\n/* {f} */ # :g").is_empty());
        // `{tag}:field` — the `:field` follows `}` and is not a placeholder;
        // the `{tag}` itself is one in SQL mode …
        assert_eq!(names("GET {tag}:field"), vec!["tag"]);
        // … but never in Redis line mode (a Cluster hash tag), and `user:123`
        // is a key, not `:123`.
        assert!(placeholder_names("GET {tag}:field", PlaceholderMode::Line).is_empty());
        assert_eq!(
            placeholder_names("HGET user:1 :field", PlaceholderMode::Line),
            vec!["field"]
        );
    }

    #[test]
    fn mustache_is_one_token() {
        let p = find_placeholders("x = {{a}}", PlaceholderMode::Sql);
        assert_eq!(p.len(), 1);
        assert_eq!((p[0].start, p[0].end), (4, 9));
    }

    #[test]
    fn mongo_object_keys_are_not_placeholders() {
        assert_eq!(
            names("db.orders.find({brand: :brand, \"x\": 1, nested: {a: {{v}}}})"),
            vec!["brand", "v"]
        );
    }

    #[test]
    fn substitute_replaces_right_to_left_and_keeps_unknown() {
        let mut m = HashMap::new();
        m.insert("a".to_string(), "'xx'".to_string());
        m.insert("b".to_string(), "7".to_string());
        assert_eq!(
            substitute("f(:a, {b}, {{a}}, :c)", PlaceholderMode::Sql, &m),
            "f('xx', 7, 'xx', :c)"
        );
    }

    #[test]
    fn substitute_handles_multibyte_text() {
        let mut m = HashMap::new();
        m.insert("n".to_string(), "1".to_string());
        assert_eq!(
            substitute("SELECT 'héllo', :n /* ünï */", PlaceholderMode::Sql, &m),
            "SELECT 'héllo', 1 /* ünï */"
        );
    }

    #[test]
    fn numbers_are_validated() {
        for ok in ["1", "-2", "+3.5", ".5", "6.", "1e9", "2.5E-3", " 42 "] {
            assert!(
                render_value("n", ok, VarType::Number, true, Engine::Mysql).is_ok(),
                "{ok}"
            );
        }
        for bad in [
            "",
            "abc",
            "1;DROP TABLE t",
            "0x10",
            "1e",
            "--1",
            "NaN",
            "1 2",
        ] {
            assert!(
                render_value("n", bad, VarType::Number, true, Engine::Mysql).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn strings_are_quoted_per_engine() {
        let v = r"it's \ok";
        assert_eq!(
            render_value("s", v, VarType::String, true, Engine::Mysql).unwrap(),
            r"'it''s \\ok'"
        );
        assert_eq!(
            render_value("s", v, VarType::String, true, Engine::Clickhouse).unwrap(),
            r"'it''s \\ok'"
        );
        assert_eq!(
            render_value("s", v, VarType::String, true, Engine::Postgres).unwrap(),
            r"'it''s \ok'"
        );
        assert_eq!(
            render_value("s", v, VarType::String, true, Engine::Mongodb).unwrap(),
            r#""it's \\ok""#
        );
        assert_eq!(
            render_value("s", "a\"b", VarType::String, true, Engine::Redis).unwrap(),
            r#""a\"b""#
        );
        assert_eq!(
            render_value("s", "x", VarType::Raw, true, Engine::Mysql).unwrap(),
            "x"
        );
    }

    #[test]
    fn names_are_validated() {
        assert!(valid_name("brand_id"));
        assert!(valid_name("_x1"));
        assert!(!valid_name("1x"));
        assert!(!valid_name("a-b"));
        assert!(!valid_name(""));
    }

    #[test]
    fn postgres_hash_operators_are_code_and_dollar_bodies_are_literals() {
        let pg = |sql: &str| placeholder_names(sql, PlaceholderMode::Postgres);
        assert_eq!(
            PlaceholderMode::for_engine(Engine::Postgres),
            PlaceholderMode::Postgres
        );
        // `#>>` / `#>` / `#` are operators, not a MySQL line comment.
        assert_eq!(
            pg("SELECT data #>> '{a,b}', data #> '{c}', 5 # 3 FROM t WHERE id = :id AND k = {k}"),
            vec!["id", "k"]
        );
        // The Sql mode would have hidden them — the bug this mode fixes.
        assert!(placeholder_names(
            "SELECT data #>> '{a}' FROM t WHERE id = :id",
            PlaceholderMode::Sql
        )
        .is_empty());
        // `$$` and `$tag$` bodies are literals; `$1` is a positional param.
        assert_eq!(
            pg("DO $$ BEGIN PERFORM :inner; END $$; SELECT $fn$ :x $fn$, $1, :outer"),
            vec!["outer"]
        );
        // Standard strings: a backslash does NOT escape the closing quote …
        assert_eq!(pg(r"SELECT 'a\' || :v"), vec!["v"]);
        // … but does inside E'…'.
        assert!(pg(r"SELECT E'a\' :v'").is_empty());
        // Nested block comments and `--` still hide placeholders.
        assert_eq!(pg("/* a /* :b */ :c */ SELECT :d -- :e"), vec!["d"]);
        let mut rendered = HashMap::new();
        rendered.insert("id".to_string(), "7".to_string());
        rendered.insert("inner".to_string(), "X".to_string());
        assert_eq!(
            substitute(
                "SELECT data #>> '{a}' FROM t WHERE id = :id; DO $$ :inner $$",
                PlaceholderMode::Postgres,
                &rendered
            ),
            "SELECT data #>> '{a}' FROM t WHERE id = 7; DO $$ :inner $$"
        );
    }
}
