//! Tolerant parser for mongosh-style value literals → `serde_json::Value`.
//!
//! Otto's Mongo runner accepts pasted mongosh / Compass snippets, which are
//! JavaScript object literals — NOT strict JSON. Strict `serde_json` rejects
//! the superset they use (its first complaint on such a paste is the unhelpful
//! "key must be a string"). This parser accepts that superset:
//!
//! - unquoted identifier keys (`dashboardId: …`)
//! - single-quoted, double-quoted, and backtick strings (backtick = raw; `${…}`
//!   interpolation is rejected, since we don't evaluate expressions)
//! - trailing commas in objects/arrays
//! - `//` line and `/* */` block comments
//! - Mongo "constructor" helpers — `new Date()`, `Date()`, `ISODate()`,
//!   `ObjectId()`, `NumberLong/Int/Decimal()`, `UUID()` — emitted as MongoDB
//!   Extended JSON sentinels (`{"$oid": …}`, `{"$date": …}`, …) which
//!   `mongodb::json_to_bson` decodes.
//! - `/pattern/flags` regex literals → `{"$regularExpression": {…}}`
//!
//! It is deliberately NOT a JS engine: no variables, arithmetic, function
//! bodies, or template interpolation — only literals and the known
//! constructors. Anything else is a clear parse error.

use mongodb::bson::oid::ObjectId;
use otto_core::{Error, Result};
use serde_json::{Map, Number, Value};

/// Parse one mongosh value literal (object, array, or scalar). The whole input
/// must be a single value — trailing tokens are an error.
pub fn parse_value(input: &str) -> Result<Value> {
    let mut p = Parser::new(input);
    let v = p.value()?;
    p.skip_trivia();
    if p.pos < p.src.len() {
        return Err(err(format!(
            "unexpected trailing characters near offset {}",
            p.pos
        )));
    }
    Ok(v)
}

fn err(msg: impl Into<String>) -> Error {
    Error::Invalid(format!("mongo parse: {}", msg.into()))
}

struct Parser<'a> {
    src: &'a [u8],
    text: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            src: text.as_bytes(),
            text,
            pos: 0,
        }
    }

    fn at(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek(&self, n: usize) -> Option<u8> {
        self.src.get(self.pos + n).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.at();
        if b.is_some() {
            self.pos += 1;
        }
        b
    }

    /// Skip whitespace plus `//` and `/* */` comments.
    fn skip_trivia(&mut self) {
        loop {
            match self.at() {
                Some(b) if b.is_ascii_whitespace() => self.pos += 1,
                Some(b'/') if self.peek(1) == Some(b'/') => {
                    self.pos += 2;
                    while let Some(b) = self.bump() {
                        if b == b'\n' {
                            break;
                        }
                    }
                }
                Some(b'/') if self.peek(1) == Some(b'*') => {
                    self.pos += 2;
                    while self.pos < self.src.len() {
                        if self.at() == Some(b'*') && self.peek(1) == Some(b'/') {
                            self.pos += 2;
                            break;
                        }
                        self.pos += 1;
                    }
                }
                _ => break,
            }
        }
    }

    fn value(&mut self) -> Result<Value> {
        self.skip_trivia();
        match self.at() {
            None => Err(err("unexpected end of input")),
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') | Some(b'\'') | Some(b'`') => Ok(Value::String(self.string()?)),
            Some(b'/') => self.regex(),
            Some(b) if b == b'-' || b == b'+' || b == b'.' || b.is_ascii_digit() => self.number(),
            Some(b) if is_ident_start(b) => self.ident_value(),
            Some(b) => Err(err(format!("unexpected character {:?}", b as char))),
        }
    }

    fn object(&mut self) -> Result<Value> {
        self.expect(b'{')?;
        let mut map = Map::new();
        loop {
            self.skip_trivia();
            match self.at() {
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                None => return Err(err("unterminated object")),
                _ => {}
            }
            let key = self.object_key()?;
            self.skip_trivia();
            self.expect(b':')?;
            let val = self.value()?;
            map.insert(key, val);
            self.skip_trivia();
            match self.at() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(err("expected ',' or '}' in object")),
            }
        }
        Ok(Value::Object(map))
    }

    fn object_key(&mut self) -> Result<String> {
        self.skip_trivia();
        match self.at() {
            Some(b'"') | Some(b'\'') | Some(b'`') => self.string(),
            Some(b) if is_ident_start(b) => Ok(self.ident_raw()),
            Some(b) if b.is_ascii_digit() || b == b'-' => Ok(self.number_token()),
            _ => Err(err("expected object key")),
        }
    }

    fn array(&mut self) -> Result<Value> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        loop {
            self.skip_trivia();
            match self.at() {
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                None => return Err(err("unterminated array")),
                _ => {}
            }
            items.push(self.value()?);
            self.skip_trivia();
            match self.at() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(err("expected ',' or ']' in array")),
            }
        }
        Ok(Value::Array(items))
    }

    /// Read a quoted string. The opening quote (`"`, `'`, or `` ` ``) sets the
    /// terminator. Backtick strings are raw text; `${…}` is rejected.
    fn string(&mut self) -> Result<String> {
        let quote = self.bump().ok_or_else(|| err("expected string"))?;
        let mut out = String::new();
        while let Some(b) = self.at() {
            if b == quote {
                self.pos += 1;
                return Ok(out);
            }
            if b == b'\\' {
                self.pos += 1;
                let e = self.bump().ok_or_else(|| err("dangling escape in string"))?;
                match e {
                    b'n' => out.push('\n'),
                    b't' => out.push('\t'),
                    b'r' => out.push('\r'),
                    b'b' => out.push('\u{0008}'),
                    b'f' => out.push('\u{000C}'),
                    b'0' => out.push('\0'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'\'' => out.push('\''),
                    b'"' => out.push('"'),
                    b'`' => out.push('`'),
                    b'\n' => {} // escaped newline = line continuation
                    b'u' => {
                        let cp = self.read_hex4()?;
                        out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                    }
                    other => out.push(other as char),
                }
            } else if quote == b'`' && b == b'$' && self.peek(1) == Some(b'{') {
                return Err(err("template interpolation `${…}` is not supported"));
            } else {
                // Copy one whole UTF-8 scalar (delimiters are all ASCII, so the
                // byte cursor is always at a char boundary here).
                let start = self.pos;
                let end = (start + utf8_len(b)).min(self.src.len());
                out.push_str(&self.text[start..end]);
                self.pos = end;
            }
        }
        Err(err("unterminated string"))
    }

    fn read_hex4(&mut self) -> Result<u32> {
        let mut v: u32 = 0;
        for _ in 0..4 {
            let b = self.bump().ok_or_else(|| err("truncated \\u escape"))?;
            let d = (b as char)
                .to_digit(16)
                .ok_or_else(|| err("invalid \\u escape"))?;
            v = v * 16 + d;
        }
        Ok(v)
    }

    /// Parse a number into a `serde_json::Number` (i64 / u64 / f64, plus `0x` hex).
    fn number(&mut self) -> Result<Value> {
        let tok = self.number_token();
        if let Some(hex) = tok.strip_prefix("0x").or_else(|| tok.strip_prefix("0X")) {
            let n = i64::from_str_radix(hex, 16).map_err(|_| err(format!("bad hex number '{tok}'")))?;
            return Ok(Value::Number(n.into()));
        }
        if let Ok(i) = tok.parse::<i64>() {
            return Ok(Value::Number(i.into()));
        }
        if let Ok(u) = tok.parse::<u64>() {
            return Ok(Value::Number(u.into()));
        }
        match tok.parse::<f64>() {
            Ok(f) => Number::from_f64(f)
                .map(Value::Number)
                .ok_or_else(|| err(format!("number '{tok}' is not finite"))),
            Err(_) => Err(err(format!("invalid number '{tok}'"))),
        }
    }

    /// Consume the raw run of characters that make up a numeric token.
    fn number_token(&mut self) -> String {
        let start = self.pos;
        if matches!(self.at(), Some(b'+') | Some(b'-')) {
            self.pos += 1;
        }
        while let Some(b) = self.at() {
            if b.is_ascii_hexdigit()
                || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-' | b'x' | b'X')
            {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.text[start..self.pos].to_string()
    }

    fn ident_raw(&mut self) -> String {
        let start = self.pos;
        while let Some(b) = self.at() {
            if is_ident_continue(b) {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.text[start..self.pos].to_string()
    }

    /// An identifier in value position: a literal keyword, or a constructor call.
    fn ident_value(&mut self) -> Result<Value> {
        let name = self.ident_raw();
        match name.as_str() {
            "true" => return Ok(Value::Bool(true)),
            "false" => return Ok(Value::Bool(false)),
            "null" | "undefined" => return Ok(Value::Null),
            "NaN" | "Infinity" => return Ok(Value::Null),
            "new" => {
                self.skip_trivia();
                if !matches!(self.at(), Some(b) if is_ident_start(b)) {
                    return Err(err("expected a constructor after 'new'"));
                }
                let ctor = self.ident_raw();
                return self.constructor(&ctor);
            }
            _ => {}
        }
        self.skip_trivia();
        if self.at() == Some(b'(') {
            return self.constructor(&name);
        }
        Err(err(format!(
            "unexpected identifier '{name}' (variables/expressions aren't supported)"
        )))
    }

    /// Parse `Name( args )` and map the known Mongo constructors to EJSON.
    fn constructor(&mut self, name: &str) -> Result<Value> {
        self.skip_trivia();
        self.expect(b'(')?;
        let mut args = Vec::new();
        loop {
            self.skip_trivia();
            match self.at() {
                Some(b')') => {
                    self.pos += 1;
                    break;
                }
                None => return Err(err("unterminated constructor call")),
                _ => {}
            }
            args.push(self.value()?);
            self.skip_trivia();
            match self.at() {
                Some(b',') => self.pos += 1,
                Some(b')') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(err("expected ',' or ')' in constructor call")),
            }
        }
        build_constructor(name, args)
    }

    /// `/pattern/flags` → `{"$regularExpression": {pattern, options}}`.
    fn regex(&mut self) -> Result<Value> {
        self.expect(b'/')?;
        let mut pattern = String::new();
        loop {
            match self.bump() {
                None | Some(b'\n') => return Err(err("unterminated regex literal")),
                Some(b'\\') => {
                    pattern.push('\\');
                    match self.bump() {
                        Some(e) => pattern.push(e as char),
                        None => return Err(err("dangling escape in regex")),
                    }
                }
                Some(b'/') => break,
                Some(b) => {
                    let start = self.pos - 1;
                    let end = (start + utf8_len(b)).min(self.src.len());
                    pattern.push_str(&self.text[start..end]);
                    self.pos = end;
                }
            }
        }
        let mut options = String::new();
        while let Some(b) = self.at() {
            if b.is_ascii_alphabetic() {
                options.push(b as char);
                self.pos += 1;
            } else {
                break;
            }
        }
        let mut inner = Map::new();
        inner.insert("pattern".into(), Value::String(pattern));
        inner.insert("options".into(), Value::String(options));
        Ok(sentinel("$regularExpression", Value::Object(inner)))
    }

    fn expect(&mut self, b: u8) -> Result<()> {
        if self.at() == Some(b) {
            self.pos += 1;
            Ok(())
        } else {
            Err(err(format!(
                "expected '{}' but found {}",
                b as char,
                self.at().map(|c| format!("'{}'", c as char)).unwrap_or_else(|| "end of input".into())
            )))
        }
    }
}

/// Wrap a value as a single-key EJSON sentinel object.
fn sentinel(key: &str, value: Value) -> Value {
    let mut m = Map::new();
    m.insert(key.into(), value);
    Value::Object(m)
}

/// Map a constructor name + args to an EJSON sentinel.
fn build_constructor(name: &str, args: Vec<Value>) -> Result<Value> {
    match name {
        "Date" | "ISODate" => {
            let s = match args.first() {
                None => chrono::Utc::now().to_rfc3339(),
                Some(Value::String(s)) => s.clone(),
                Some(Value::Number(n)) => {
                    let ms = n.as_i64().ok_or_else(|| err("Date(millis) must be an integer"))?;
                    chrono::DateTime::from_timestamp_millis(ms)
                        .ok_or_else(|| err("Date(millis) out of range"))?
                        .to_rfc3339()
                }
                Some(_) => return Err(err("Date() expects a string or millis number")),
            };
            Ok(sentinel("$date", Value::String(s)))
        }
        "ObjectId" => {
            let hex = match args.first() {
                None => ObjectId::new().to_hex(),
                Some(Value::String(s)) => s.clone(),
                Some(_) => return Err(err("ObjectId() expects a hex string")),
            };
            Ok(sentinel("$oid", Value::String(hex)))
        }
        "NumberLong" => Ok(sentinel("$numberLong", Value::String(num_arg(&args, name)?))),
        "NumberInt" => Ok(sentinel("$numberInt", Value::String(num_arg(&args, name)?))),
        "NumberDecimal" => Ok(sentinel("$numberDecimal", Value::String(num_arg(&args, name)?))),
        "UUID" => Ok(sentinel("$uuid", Value::String(str_arg(&args, name)?))),
        other => Err(err(format!("unsupported constructor '{other}(…)'"))),
    }
}

/// First arg of a numeric constructor, as a digit string (accepts string or number).
fn num_arg(args: &[Value], name: &str) -> Result<String> {
    match args.first() {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(Value::Number(n)) => Ok(n.to_string()),
        _ => Err(err(format!("{name}() expects a number or numeric string"))),
    }
}

fn str_arg(args: &[Value], name: &str) -> Result<String> {
    match args.first() {
        Some(Value::String(s)) => Ok(s.clone()),
        _ => Err(err(format!("{name}() expects a string argument"))),
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// Byte length of the UTF-8 scalar whose lead byte is `b`.
fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unquoted_keys_and_trailing_commas() {
        let v = parse_value(r#"{ a: 1, b: "x", c: [1, 2, 3,], }"#).unwrap();
        assert_eq!(v, json!({ "a": 1, "b": "x", "c": [1, 2, 3] }));
    }

    #[test]
    fn single_double_and_backtick_strings() {
        let v = parse_value(r#"{ a: 'one', b: "two", c: `three (3) 'x'` }"#).unwrap();
        assert_eq!(v, json!({ "a": "one", "b": "two", "c": "three (3) 'x'" }));
    }

    #[test]
    fn backtick_preserves_sql_with_parens_and_quotes() {
        let v = parse_value("{ query: `SELECT a FROM t WHERE x IN ('A','B') AND f(y) > 1` }").unwrap();
        assert_eq!(
            v["query"],
            json!("SELECT a FROM t WHERE x IN ('A','B') AND f(y) > 1")
        );
    }

    #[test]
    fn line_and_block_comments() {
        let v = parse_value("{\n // a comment\n a: 1, /* inline */ b: 2\n}").unwrap();
        assert_eq!(v, json!({ "a": 1, "b": 2 }));
    }

    #[test]
    fn literals_null_bool_undefined() {
        let v = parse_value("{ a: null, b: true, c: false, d: undefined }").unwrap();
        assert_eq!(v, json!({ "a": null, "b": true, "c": false, "d": null }));
    }

    #[test]
    fn new_date_no_args_is_iso_sentinel() {
        let v = parse_value("{ createdAt: new Date() }").unwrap();
        let s = v["createdAt"]["$date"].as_str().unwrap();
        assert!(s.contains('T'), "expected an ISO date, got {s}");
    }

    #[test]
    fn date_string_and_objectid_and_numberlong() {
        let v = parse_value(
            r#"{ d: ISODate("2024-01-02T03:04:05Z"), id: ObjectId("507f1f77bcf86cd799439011"), n: NumberLong(42) }"#,
        )
        .unwrap();
        assert_eq!(v["d"], json!({ "$date": "2024-01-02T03:04:05Z" }));
        assert_eq!(v["id"], json!({ "$oid": "507f1f77bcf86cd799439011" }));
        assert_eq!(v["n"], json!({ "$numberLong": "42" }));
    }

    #[test]
    fn objectid_no_args_generates_hex() {
        let v = parse_value("{ id: ObjectId() }").unwrap();
        let hex = v["id"]["$oid"].as_str().unwrap();
        assert_eq!(hex.len(), 24);
    }

    #[test]
    fn numeric_and_nested() {
        let v = parse_value(r#"{ n: -3, f: 1.5, big: 1e3, arr: [{ k: 1 }] }"#).unwrap();
        assert_eq!(v["n"], json!(-3));
        assert_eq!(v["f"], json!(1.5));
        assert_eq!(v["big"], json!(1000.0));
        assert_eq!(v["arr"], json!([{ "k": 1 }]));
    }

    #[test]
    fn template_interpolation_is_rejected() {
        let e = parse_value("{ a: `x ${y} z` }").unwrap_err();
        assert!(e.to_string().contains("interpolation"), "{e}");
    }

    #[test]
    fn bare_identifier_is_rejected() {
        let e = parse_value("{ a: someVar }").unwrap_err();
        assert!(e.to_string().contains("aren't supported"), "{e}");
    }

    #[test]
    fn parses_a_dashboard_shaped_payload() {
        // The exact troublesome shape: unquoted keys, new Date(), a backtick SQL
        // template full of parens/quotes, nested arrays, trailing commas, comments.
        let src = r#"{
            dashboardId: "player-activities", // the id
            createdAt: new Date(),
            isActive: true,
            scopeId: null,
            filters: [
                { id: false, name: "No" },
                { id: 1, name: "Silver" },
            ],
            widgets: [
                { id: "w1", query: `WITH x AS (SELECT 1) SELECT multiIf(a=1,'Y','N') FROM t WHERE c IN ('A','B')` },
            ],
        }"#;
        let v = parse_value(src).unwrap();
        assert_eq!(v["dashboardId"], json!("player-activities"));
        assert!(v["createdAt"]["$date"].is_string());
        assert_eq!(v["filters"][0]["id"], json!(false));
        assert_eq!(v["filters"][1]["id"], json!(1));
        assert!(v["widgets"][0]["query"].as_str().unwrap().contains("multiIf"));
    }
}
