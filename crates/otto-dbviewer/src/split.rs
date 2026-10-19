//! String/comment/dollar-quote-aware SQL statement splitter.
//!
//! One shared splitter so the drivers (multi-statement batches) and the
//! read-only write-guard (`types::sql_is_write`) agree on exactly where a
//! statement boundary is. A naive `text.split(';')` treats a `;` inside a
//! string literal or a comment as a boundary; that over-splits a single
//! statement into fragments (breaking real multi-statement execution) and, in
//! the guard, fakes phantom statements out of literal text. This scanner tracks
//! quoting/comment state so only a **top-level** `;` (or end of input) ends a
//! statement.
//!
//! The scanner is deliberately single-pass and regex-free, mirroring the small
//! hand-rolled parsers already used in this crate (`types::sql_first_keyword`).

/// SQL flavour, selecting the lexical quirks the scanner honours.
///
/// - `Mysql`: backtick identifiers, `#` line comments, backslash string escapes.
/// - `Clickhouse`: quoted identifiers, backslash escapes and `--` / `/* */` comments.
/// - `Postgres`: `$tag$…$tag$` dollar quoting (no backtick, no `#`, no backslash
///   escape — standard-conforming strings).
/// - `Generic`: the conservative common denominator used by the write-guard —
///   `'`/`"` quoting (with backslash escapes) and `--` / `/* */` comments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlDialect {
    Mysql,
    Clickhouse,
    Postgres,
    Generic,
}

/// One statement carved out of a batch. `text` is the trimmed statement with
/// the terminating `;` removed and any leading/trailing whitespace or comment
/// noise stripped; `start`/`end` are its byte offsets into the original input
/// such that `&sql[start..end] == text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementSpan {
    pub text: String,
    pub start: usize,
    pub end: usize,
}

/// Lexer state while scanning the batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Normal,
    Single,       // '…'
    Double,       // "…"
    Backtick,     // `…`   (Mysql)
    LineComment,  // -- … or # … (Mysql), until newline
    BlockComment, // /* … */
    Dollar,       // $tag$ … $tag$ (Postgres)
}

/// Splits on top-level `;`, aware of: '…' strings (backslash escapes for Mysql/Generic,
/// doubled '' always), "…" (identifier or string), `…` backticks (Mysql), -- and /* */
/// comments, # line comments (Mysql only), $tag$…$tag$ dollar quoting (Postgres only).
/// Trailing statement without `;` is included; blank/comment-only segments are dropped.
pub fn split_statements(sql: &str, dialect: SqlDialect) -> Vec<StatementSpan> {
    let mut scanner = StatementScanner::new(sql, dialect);
    std::iter::from_fn(|| scanner.next(false)).collect()
}

/// Incremental scanner: keeps delimiter/offset and one character index while
/// the caller obtains the current MySQL escape mode from each server response.
pub struct StatementScanner<'a> {
    sql: &'a str,
    chars: Vec<(usize, char)>,
    dialect: SqlDialect,
    delimiter: String,
    offset: usize,
}

impl<'a> StatementScanner<'a> {
    pub fn new(sql: &'a str, dialect: SqlDialect) -> Self {
        Self {
            sql,
            chars: sql.char_indices().collect(),
            dialect,
            delimiter: ";".into(),
            offset: 0,
        }
    }

    /// Probe once for single-statement paging without consuming scanner state.
    pub fn has_more(&mut self, no_backslash_escapes: bool) -> bool {
        let offset = self.offset;
        let delimiter = self.delimiter.clone();
        let more = self.next(no_backslash_escapes).is_some();
        self.offset = offset;
        self.delimiter = delimiter;
        more
    }

    /// Return the next statement using the session's current escape mode.
    pub fn next(&mut self, no_backslash_escapes: bool) -> Option<StatementSpan> {
        let sql = self.sql;
        let chars = &self.chars;
        let dialect = self.dialect;
        let delimiter = &mut self.delimiter;
        let n = chars.len();
        let total = sql.len();
        let backslash_escapes = matches!(dialect, SqlDialect::Clickhouse | SqlDialect::Generic)
            || (dialect == SqlDialect::Mysql && !no_backslash_escapes);
        // Exclusive byte offset just past the char at position `idx`.
        let byte_end = |idx: usize| -> usize {
            if idx + 1 < n {
                chars[idx + 1].0
            } else {
                total
            }
        };

        let mut state = State::Normal;
        let mut dollar_tag = String::new();
        let mut start: Option<usize> = None;
        let mut end: usize = 0;
        let span = |start: usize, end: usize| StatementSpan {
            text: sql[start..end].to_string(),
            start,
            end,
        };

        let mut i = self.offset;
        while i < n {
            let (bi, ch) = chars[i];
            let ce = byte_end(i);
            match state {
                State::Normal => {
                    // DELIMITER is a MySQL client directive, never SQL sent to the
                    // server. Recognize it only on its own line between statements.
                    if dialect == SqlDialect::Mysql
                        && start.is_none()
                        && ch.eq_ignore_ascii_case(&'d')
                    {
                        let line_start = sql[..bi].rfind('\n').map_or(0, |p| p + 1);
                        if sql[line_start..bi].trim().is_empty() {
                            let line_end = sql[bi..].find('\n').map_or(total, |p| bi + p);
                            let line = &sql[bi..line_end];
                            if let Some((verb, value)) = line.split_once(char::is_whitespace) {
                                let value = value.trim();
                                // mysql accepts a quoted delimiter token too.
                                let quoted = value.len() >= 2
                                    && matches!(value.as_bytes()[0], b'\'' | b'"' | b'`')
                                    && value.as_bytes().first() == value.as_bytes().last();
                                let value = if quoted {
                                    &value[1..value.len() - 1]
                                } else {
                                    value
                                };
                                if verb.eq_ignore_ascii_case("delimiter")
                                    && !value.is_empty()
                                    && (quoted || !value.chars().any(char::is_whitespace))
                                {
                                    *delimiter = value.to_string();
                                    while i < n && chars[i].0 < line_end {
                                        i += 1;
                                    }
                                    continue;
                                }
                            }
                        }
                    }
                    if sql[bi..].starts_with(delimiter.as_str()) {
                        let after = bi + delimiter.len();
                        while i < n && chars[i].0 < after {
                            i += 1;
                        }
                        if let Some(start) = start.take() {
                            self.offset = i;
                            return Some(span(start, end));
                        }
                        continue;
                    }
                    if ch == '-' && next_is(chars, i, '-') {
                        state = State::LineComment;
                        i += 2;
                        continue;
                    }
                    if ch == '#' && dialect == SqlDialect::Mysql {
                        state = State::LineComment;
                        i += 1;
                        continue;
                    }
                    if ch == '/' && next_is(chars, i, '*') {
                        state = State::BlockComment;
                        i += 2;
                        continue;
                    }
                    if ch == '\'' {
                        mark(&mut start, &mut end, bi, ce);
                        state = State::Single;
                        i += 1;
                        continue;
                    }
                    if ch == '"' {
                        mark(&mut start, &mut end, bi, ce);
                        state = State::Double;
                        i += 1;
                        continue;
                    }
                    if ch == '`' && matches!(dialect, SqlDialect::Mysql | SqlDialect::Clickhouse) {
                        mark(&mut start, &mut end, bi, ce);
                        state = State::Backtick;
                        i += 1;
                        continue;
                    }
                    if ch == '$' && dialect == SqlDialect::Postgres {
                        if let Some((tag, after)) = try_open_dollar(chars, i) {
                            mark(&mut start, &mut end, bi, byte_end(after - 1));
                            dollar_tag = tag;
                            state = State::Dollar;
                            i = after;
                            continue;
                        }
                        // Not a dollar-quote opener (e.g. `$1`): fall through.
                    }
                    if !ch.is_whitespace() {
                        mark(&mut start, &mut end, bi, ce);
                    }
                    i += 1;
                }
                State::Single => {
                    if backslash_escapes && ch == '\\' {
                        // Consume the backslash and the escaped char as literal.
                        mark(&mut start, &mut end, bi, ce);
                        i += 1;
                        if i < n {
                            mark(&mut start, &mut end, chars[i].0, byte_end(i));
                            i += 1;
                        }
                        continue;
                    }
                    mark(&mut start, &mut end, bi, ce);
                    if ch == '\'' {
                        // A doubled '' re-opens on the next char in Normal, so the
                        // toggle correctly keeps an embedded `;` inside the literal.
                        state = State::Normal;
                    }
                    i += 1;
                }
                State::Double => {
                    if backslash_escapes && ch == '\\' {
                        mark(&mut start, &mut end, bi, ce);
                        i += 1;
                        if i < n {
                            mark(&mut start, &mut end, chars[i].0, byte_end(i));
                            i += 1;
                        }
                        continue;
                    }
                    mark(&mut start, &mut end, bi, ce);
                    if ch == '"' {
                        state = State::Normal;
                    }
                    i += 1;
                }
                State::Backtick => {
                    if dialect == SqlDialect::Clickhouse && ch == '\\' {
                        mark(&mut start, &mut end, bi, ce);
                        i += 1;
                        if i < n {
                            mark(&mut start, &mut end, chars[i].0, byte_end(i));
                            i += 1;
                        }
                        continue;
                    }
                    mark(&mut start, &mut end, bi, ce);
                    if ch == '`' {
                        state = State::Normal;
                    }
                    i += 1;
                }
                State::LineComment => {
                    if ch == '\n' {
                        state = State::Normal;
                    }
                    i += 1;
                }
                State::BlockComment => {
                    if ch == '*' && next_is(chars, i, '/') {
                        state = State::Normal;
                        i += 2;
                        continue;
                    }
                    i += 1;
                }
                State::Dollar => {
                    if ch == '$' {
                        if let Some(after) = try_close_dollar(chars, i, &dollar_tag) {
                            mark(&mut start, &mut end, bi, byte_end(after - 1));
                            state = State::Normal;
                            i = after;
                            continue;
                        }
                    }
                    mark(&mut start, &mut end, bi, ce);
                    i += 1;
                }
            }
        }
        // Trailing statement without a terminating `;`.
        self.offset = n;
        start.map(|start| span(start, end))
    }
}

/// Records `[cs, ce)` as covered content, opening the span on first mark.
#[inline]
fn mark(start: &mut Option<usize>, end: &mut usize, cs: usize, ce: usize) {
    if start.is_none() {
        *start = Some(cs);
    }
    *end = ce;
}

/// True when the char right after position `i` equals `want`.
#[inline]
fn next_is(chars: &[(usize, char)], i: usize, want: char) -> bool {
    chars.get(i + 1).is_some_and(|&(_, c)| c == want)
}

/// If a dollar-quote opener starts at `i` (`chars[i] == '$'`), return its tag
/// and the position just past the opening `$tag$`. Tags follow identifier rules
/// (letters/underscore first) so a positional parameter like `$1` is rejected.
fn try_open_dollar(chars: &[(usize, char)], i: usize) -> Option<(String, usize)> {
    let mut j = i + 1;
    let mut tag = String::new();
    while let Some(&(_, c)) = chars.get(j) {
        if c == '$' {
            return Some((tag, j + 1));
        }
        let ok = c == '_' || c.is_ascii_alphabetic() || (c.is_ascii_digit() && !tag.is_empty());
        if !ok {
            return None;
        }
        tag.push(c);
        j += 1;
    }
    None
}

/// If the closing `$tag$` for `tag` starts at `i`, return the position just past
/// it; otherwise `None` (the `$` is ordinary content inside the dollar body).
fn try_close_dollar(chars: &[(usize, char)], i: usize, tag: &str) -> Option<usize> {
    let mut j = i + 1;
    for tc in tag.chars() {
        match chars.get(j) {
            Some(&(_, c)) if c == tc => j += 1,
            _ => return None,
        }
    }
    match chars.get(j) {
        Some(&(_, '$')) => Some(j + 1),
        _ => None,
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn incremental_mysql_mode_and_delimiter_changes() {
        let sql = r"SET sql_mode='NO_BACKSLASH_ESCAPES'; SELECT 'ends\'; SELECT 2;";
        let mut scanner = super::StatementScanner::new(sql, super::SqlDialect::Mysql);
        assert_eq!(
            scanner.next(false).unwrap().text,
            "SET sql_mode='NO_BACKSLASH_ESCAPES'"
        );
        assert!(scanner.has_more(true));
        let span = scanner.next(true).unwrap();
        assert_eq!(span.text, r"SELECT 'ends\'");
        assert_eq!(&sql[span.start..span.end], span.text);
        assert_eq!(scanner.next(true).unwrap().text, "SELECT 2");
        assert!(scanner.next(true).is_none());
        let mut scanner = super::StatementScanner::new(
            "DELIMITER $$\nSELECT 1$$\nDELIMITER ;\nSELECT 2;",
            super::SqlDialect::Mysql,
        );
        assert_eq!(scanner.next(false).unwrap().text, "SELECT 1");
        assert!(scanner.has_more(true));
        assert_eq!(scanner.next(true).unwrap().text, "SELECT 2");
    }
    use super::*;

    #[test]
    fn mysql_delimiter_with_quoted_whitespace() {
        let spans = split_statements(
            "DELIMITER 'x y'\nSELECT 1x y\nDELIMITER ;\nSELECT 2;",
            SqlDialect::Mysql,
        );
        assert_eq!(
            spans.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
            ["SELECT 1", "SELECT 2"]
        );
    }

    #[test]
    fn mysql_quoted_delimiters() {
        for delimiter in ["'$$'", "\"$$\"", "`$$`"] {
            let sql = format!("DELIMITER {delimiter}\nSELECT 1$$\nDELIMITER ;\nSELECT 2;");
            let spans = split_statements(&sql, SqlDialect::Mysql);
            assert_eq!(
                spans.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
                ["SELECT 1", "SELECT 2"]
            );
        }
    }

    #[test]
    fn clickhouse_escapes_and_quoted_identifiers_keep_semicolons() {
        let sql = r"SELECT 'it\'s;valid', `a; b`, `a\`;b`; SELECT 2";
        assert_eq!(texts(sql, SqlDialect::Clickhouse).len(), 2);
        assert_eq!(texts(sql, SqlDialect::Clickhouse)[1], "SELECT 2");
    }

    #[test]
    fn mysql_delimiter_preserves_stored_programs() {
        let sql = "DELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT 'x;$$'; SELECT 2; END$$\nDELIMITER ;\nCALL p(); SELECT 3;";
        assert_eq!(
            texts(sql, SqlDialect::Mysql),
            vec![
                "CREATE PROCEDURE p() BEGIN SELECT 'x;$$'; SELECT 2; END",
                "CALL p()",
                "SELECT 3",
            ]
        );
        for span in split_statements(sql, SqlDialect::Mysql) {
            assert_eq!(&sql[span.start..span.end], span.text);
        }
    }

    #[test]
    fn delimiter_inside_comments_or_strings_is_not_a_directive() {
        let sql = "SELECT 'DELIMITER $$'; /*\nDELIMITER //\n*/ SELECT 2;";
        assert_eq!(
            texts(sql, SqlDialect::Mysql),
            vec!["SELECT 'DELIMITER $$'", "SELECT 2"]
        );
    }

    /// Just the trimmed statement texts — the common assertion shape.
    fn texts(sql: &str, dialect: SqlDialect) -> Vec<String> {
        split_statements(sql, dialect)
            .into_iter()
            .map(|s| s.text)
            .collect()
    }

    #[test]
    fn plain_single_statement() {
        assert_eq!(texts("SELECT 1", SqlDialect::Generic), vec!["SELECT 1"]);
    }

    #[test]
    fn multi_statement_happy_path() {
        assert_eq!(
            texts("SELECT 1; SELECT 2", SqlDialect::Generic),
            vec!["SELECT 1", "SELECT 2"]
        );
    }

    #[test]
    fn trailing_semicolon_dropped() {
        assert_eq!(texts("SELECT 1;", SqlDialect::Generic), vec!["SELECT 1"]);
    }

    #[test]
    fn trailing_statement_without_semicolon_kept() {
        assert_eq!(
            texts("SELECT 1; SELECT 2", SqlDialect::Generic).len(),
            2,
            "trailing SELECT 2 (no terminator) must be included"
        );
    }

    #[test]
    fn empty_segments_dropped() {
        assert!(texts(";;", SqlDialect::Generic).is_empty());
        assert_eq!(
            texts("SELECT 1;;SELECT 2", SqlDialect::Generic),
            vec!["SELECT 1", "SELECT 2"]
        );
        assert_eq!(texts(";SELECT 1;", SqlDialect::Generic), vec!["SELECT 1"]);
    }

    #[test]
    fn semicolon_inside_single_quote_is_not_a_boundary() {
        assert_eq!(
            texts("SELECT 'a;b'", SqlDialect::Generic),
            vec!["SELECT 'a;b'"]
        );
        assert_eq!(
            texts("SELECT 'a;b'; SELECT 2", SqlDialect::Generic),
            vec!["SELECT 'a;b'", "SELECT 2"]
        );
    }

    #[test]
    fn semicolon_inside_double_quote_is_not_a_boundary() {
        assert_eq!(
            texts("SELECT \"a;b\"", SqlDialect::Generic),
            vec!["SELECT \"a;b\""]
        );
    }

    #[test]
    fn backslash_escaped_quote_keeps_string_open() {
        // MySQL/Generic: `\'` does NOT close the string, so the `;` stays inside.
        assert_eq!(
            texts(r"SELECT 'it\'s ; ok'", SqlDialect::Mysql),
            vec![r"SELECT 'it\'s ; ok'"]
        );
        assert_eq!(
            texts(r"SELECT 'it\'s ; ok'", SqlDialect::Generic),
            vec![r"SELECT 'it\'s ; ok'"]
        );
    }

    #[test]
    fn backslash_escape_then_real_semicolon_splits() {
        // The string closes at the un-escaped quote; the following `;` is top-level.
        assert_eq!(
            texts(r"SELECT 'it\'s' ; SELECT 2", SqlDialect::Mysql),
            vec![r"SELECT 'it\'s'", "SELECT 2"]
        );
    }

    #[test]
    fn doubled_quote_escape_keeps_string_open() {
        // Doubled '' is handled by the toggle scan; the `;` between them is inside.
        assert_eq!(
            texts("SELECT 'it''s ; ok'", SqlDialect::Generic),
            vec!["SELECT 'it''s ; ok'"]
        );
        // Empty string '' followed by a real terminator.
        assert_eq!(
            texts("SELECT '';SELECT 2", SqlDialect::Generic),
            vec!["SELECT ''", "SELECT 2"]
        );
    }

    #[test]
    fn backtick_identifier_with_semicolon_mysql_only() {
        // MySQL: backticks quote the identifier, so the `;` is inside it.
        assert_eq!(
            texts("SELECT `a;b`", SqlDialect::Mysql),
            vec!["SELECT `a;b`"]
        );
        // Generic: backticks are not special, so the `;` splits (conservative).
        assert_eq!(texts("SELECT `a;b`", SqlDialect::Generic).len(), 2);
    }

    #[test]
    fn line_comment_dash_hides_semicolon() {
        assert_eq!(
            texts("SELECT 1 -- x;y\nFROM t", SqlDialect::Generic),
            vec!["SELECT 1 -- x;y\nFROM t"]
        );
        // A real `;` after the comment line is still a boundary.
        assert_eq!(
            texts("SELECT 1 -- c\n; SELECT 2", SqlDialect::Generic),
            vec!["SELECT 1", "SELECT 2"]
        );
    }

    #[test]
    fn hash_comment_mysql_only() {
        assert_eq!(texts("SELECT 1 # x;y", SqlDialect::Mysql), vec!["SELECT 1"]);
        // Generic: `#` is an ordinary char, so the `;` splits.
        assert_eq!(texts("SELECT 1 # x;y", SqlDialect::Generic).len(), 2);
    }

    #[test]
    fn block_comment_hides_semicolon() {
        assert_eq!(
            texts("SELECT 1 /* ; ; */ FROM t", SqlDialect::Generic),
            vec!["SELECT 1 /* ; ; */ FROM t"]
        );
        // `;` after the block comment is a real boundary.
        assert_eq!(
            texts("SELECT 1 /* c */; SELECT 2", SqlDialect::Generic),
            vec!["SELECT 1", "SELECT 2"]
        );
    }

    #[test]
    fn leading_and_trailing_comment_noise_trimmed() {
        assert_eq!(
            texts("/* lead */ SELECT 1 -- trail", SqlDialect::Generic),
            vec!["SELECT 1"]
        );
    }

    #[test]
    fn internal_comment_preserved_in_text() {
        // Only leading/trailing noise is trimmed; a mid-statement comment stays.
        assert_eq!(
            texts("SELECT 1 /* c */ FROM t", SqlDialect::Generic),
            vec!["SELECT 1 /* c */ FROM t"]
        );
    }

    #[test]
    fn comment_or_blank_only_input_yields_no_spans() {
        assert!(texts("-- just a comment", SqlDialect::Generic).is_empty());
        assert!(texts("/* block only */", SqlDialect::Generic).is_empty());
        assert!(texts("   \n  ", SqlDialect::Generic).is_empty());
        assert!(texts("", SqlDialect::Generic).is_empty());
    }

    #[test]
    fn dollar_quoting_postgres_only() {
        // Unnamed dollar quote hides the `;`.
        assert_eq!(
            texts("SELECT $$a; b$$", SqlDialect::Postgres),
            vec!["SELECT $$a; b$$"]
        );
        // Tagged dollar quote hides the `;`.
        assert_eq!(
            texts("SELECT $tag$ x ; y $tag$ ; SELECT 2", SqlDialect::Postgres),
            vec!["SELECT $tag$ x ; y $tag$", "SELECT 2"]
        );
        // A function body full of `;` stays one statement until the closing `$$`.
        assert_eq!(
            texts(
                "DO $$ BEGIN PERFORM 1; PERFORM 2; END $$; SELECT 2",
                SqlDialect::Postgres
            ),
            vec!["DO $$ BEGIN PERFORM 1; PERFORM 2; END $$", "SELECT 2"]
        );
        // Generic does NOT dollar-quote: the inner `;` splits.
        assert_eq!(texts("SELECT $$a; b$$", SqlDialect::Generic).len(), 2);
    }

    #[test]
    fn dollar_sign_positional_param_is_not_a_dollar_quote() {
        // `$1` is a Postgres positional parameter, not a dollar-quote opener.
        assert_eq!(
            texts("SELECT $1; SELECT $2", SqlDialect::Postgres),
            vec!["SELECT $1", "SELECT $2"]
        );
    }

    #[test]
    fn span_offsets_slice_back_to_text() {
        let sql = "  SELECT 1 ;  UPDATE t SET a=1 ; ";
        for span in split_statements(sql, SqlDialect::Generic) {
            assert_eq!(
                &sql[span.start..span.end],
                span.text,
                "start/end must slice back to text"
            );
        }
    }

    #[test]
    fn multibyte_content_keeps_char_boundaries() {
        // Non-ASCII bytes must never land mid-char in start/end.
        let sql = "SELECT 'café ; ☕'; SELECT 'π'";
        let spans = split_statements(sql, SqlDialect::Generic);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].text, "SELECT 'café ; ☕'");
        assert_eq!(spans[1].text, "SELECT 'π'");
        for span in &spans {
            assert_eq!(&sql[span.start..span.end], span.text);
        }
    }
}
