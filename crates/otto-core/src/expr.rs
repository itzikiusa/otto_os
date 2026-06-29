//! A tiny, safe expression language for the workflow engine.
//!
//! Used by **edge conditions** (branching), the `condition` / `loop` nodes, and
//! `{{ … }}` **templating**. It is pure (no I/O, no async, no `eval`-style code
//! execution) and evaluated against a `serde_json::Value` context — typically
//! `{ "input": …, "output": …, "node": {…}, "run": {…}, "nodes": {id: out} }`.
//!
//! Grammar (loosest → tightest binding):
//! ```text
//! expr   := or
//! or     := and ('||' and)*
//! and    := cmp ('&&' cmp)*
//! cmp    := rel (('==' | '!=') rel)*
//! rel    := member (('<' | '<=' | '>' | '>=') member)*
//! member := add (('contains' | 'in') add)*
//! add    := mul (('+' | '-') mul)*
//! mul    := unary (('*' | '/' | '%') unary)*
//! unary  := ('!' | '-') unary | primary
//! primary:= number | string | 'true' | 'false' | 'null'
//!         | ident '(' args ')'        ; function call
//!         | path                       ; ident ('.' ident | '[' expr ']')*
//!         | '(' expr ')'
//! ```
//!
//! `contains` and `in` are reserved infix operators (so they cannot be used as
//! bare path keys). Functions: `len`, `lower`, `upper`, `default`, `has`, `int`,
//! `float`, `str`, `bool`, `not`.
//!
//! Missing path segments resolve to `null` (never an error) so `default()` and
//! conditions over absent fields behave predictably. Division / modulo by zero
//! is an evaluation error.

use serde_json::Value;
use std::fmt;

/// An expression parse or evaluation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExprError {
    Parse(String),
    Eval(String),
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExprError::Parse(m) => write!(f, "expr parse error: {m}"),
            ExprError::Eval(m) => write!(f, "expr eval error: {m}"),
        }
    }
}
impl std::error::Error for ExprError {}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse and evaluate `src` against `ctx`, returning the resulting JSON value.
pub fn eval(src: &str, ctx: &Value) -> Result<Value, ExprError> {
    let toks = lex(src)?;
    let mut p = Parser { toks, pos: 0 };
    let ast = p.parse_expr()?;
    p.expect_eof()?;
    eval_ast(&ast, ctx)
}

/// Evaluate `src` to a boolean using truthiness. Any parse or eval error yields
/// `false` — callers (edge conditions) treat a broken/unmet condition as "do not
/// take this edge" rather than crashing the run.
pub fn eval_bool(src: &str, ctx: &Value) -> bool {
    match eval(src, ctx) {
        Ok(v) => truthy(&v),
        Err(_) => false,
    }
}

/// Like [`eval_bool`] but surfaces the error (for nodes that want to log why a
/// condition failed to parse). `Ok(bool)` on success.
pub fn try_eval_bool(src: &str, ctx: &Value) -> Result<bool, ExprError> {
    eval(src, ctx).map(|v| truthy(&v))
}

/// Render a template, replacing every `{{ expr }}` with the stringified result of
/// evaluating `expr` against `ctx`. Text outside `{{ … }}` is copied verbatim, so
/// the legacy `{key}` substitution can run first/independently. An expression that
/// fails to evaluate is replaced with an empty string.
pub fn render_template(tmpl: &str, ctx: &Value) -> String {
    let mut out = String::with_capacity(tmpl.len());
    let bytes = tmpl.as_bytes();
    let mut i = 0;
    while i < tmpl.len() {
        if i + 1 < tmpl.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            if let Some(close) = tmpl[i + 2..].find("}}") {
                let inner = &tmpl[i + 2..i + 2 + close];
                if let Ok(v) = eval(inner.trim(), ctx) {
                    out.push_str(&stringify(&v));
                }
                i = i + 2 + close + 2;
                continue;
            }
        }
        // copy one UTF-8 char
        let ch = tmpl[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// JSON truthiness: bool→self, null→false, number→`!=0`, string→non-empty,
/// array/object→non-empty.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Null => false,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Stringify a value for templating: strings as-is (no quotes), everything else
/// as compact JSON.
fn stringify(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64, bool), // value, is_integer
    Str(String),
    Ident(String),
    True,
    False,
    Null,
    Contains,
    In,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Dot,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
    Bang,
}

fn lex(src: &str) -> Result<Vec<Tok>, ExprError> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            '[' => {
                out.push(Tok::LBracket);
                i += 1;
            }
            ']' => {
                out.push(Tok::RBracket);
                i += 1;
            }
            ',' => {
                out.push(Tok::Comma);
                i += 1;
            }
            '.' => {
                out.push(Tok::Dot);
                i += 1;
            }
            '+' => {
                out.push(Tok::Plus);
                i += 1;
            }
            '-' => {
                out.push(Tok::Minus);
                i += 1;
            }
            '*' => {
                out.push(Tok::Star);
                i += 1;
            }
            '/' => {
                out.push(Tok::Slash);
                i += 1;
            }
            '%' => {
                out.push(Tok::Percent);
                i += 1;
            }
            '=' => {
                if chars.get(i + 1) == Some(&'=') {
                    out.push(Tok::EqEq);
                    i += 2;
                } else {
                    return Err(ExprError::Parse("'=' must be '=='".into()));
                }
            }
            '!' => {
                if chars.get(i + 1) == Some(&'=') {
                    out.push(Tok::NotEq);
                    i += 2;
                } else {
                    out.push(Tok::Bang);
                    i += 1;
                }
            }
            '<' => {
                if chars.get(i + 1) == Some(&'=') {
                    out.push(Tok::Le);
                    i += 2;
                } else {
                    out.push(Tok::Lt);
                    i += 1;
                }
            }
            '>' => {
                if chars.get(i + 1) == Some(&'=') {
                    out.push(Tok::Ge);
                    i += 2;
                } else {
                    out.push(Tok::Gt);
                    i += 1;
                }
            }
            '&' => {
                if chars.get(i + 1) == Some(&'&') {
                    out.push(Tok::AndAnd);
                    i += 2;
                } else {
                    return Err(ExprError::Parse("'&' must be '&&'".into()));
                }
            }
            '|' => {
                if chars.get(i + 1) == Some(&'|') {
                    out.push(Tok::OrOr);
                    i += 2;
                } else {
                    return Err(ExprError::Parse("'|' must be '||'".into()));
                }
            }
            '\'' | '"' => {
                let quote = c;
                i += 1;
                let mut s = String::new();
                let mut closed = false;
                while i < chars.len() {
                    let d = chars[i];
                    if d == '\\' {
                        // escape next char
                        if let Some(&n) = chars.get(i + 1) {
                            let mapped = match n {
                                'n' => '\n',
                                't' => '\t',
                                'r' => '\r',
                                other => other,
                            };
                            s.push(mapped);
                            i += 2;
                            continue;
                        }
                    }
                    if d == quote {
                        closed = true;
                        i += 1;
                        break;
                    }
                    s.push(d);
                    i += 1;
                }
                if !closed {
                    return Err(ExprError::Parse("unterminated string".into()));
                }
                out.push(Tok::Str(s));
            }
            '0'..='9' => {
                let start = i;
                let mut is_int = true;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    if chars[i] == '.' {
                        is_int = false;
                    }
                    i += 1;
                }
                let lit: String = chars[start..i].iter().collect();
                let val: f64 = lit
                    .parse()
                    .map_err(|_| ExprError::Parse(format!("bad number '{lit}'")))?;
                out.push(Tok::Num(val, is_int));
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                match word.as_str() {
                    "true" => out.push(Tok::True),
                    "false" => out.push(Tok::False),
                    "null" => out.push(Tok::Null),
                    "contains" => out.push(Tok::Contains),
                    "in" => out.push(Tok::In),
                    _ => out.push(Tok::Ident(word)),
                }
            }
            other => return Err(ExprError::Parse(format!("unexpected character '{other}'"))),
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// AST + parser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Seg {
    Key(String),
    Index(Box<Ast>),
}

#[derive(Debug, Clone)]
enum Ast {
    Lit(Value),
    Path(Vec<Seg>),
    Unary(char, Box<Ast>),
    Bin(BinOp, Box<Ast>, Box<Ast>),
    Call(String, Vec<Ast>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum BinOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Contains,
    In,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn bump(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }
    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == Some(t) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect_eof(&self) -> Result<(), ExprError> {
        if self.pos == self.toks.len() {
            Ok(())
        } else {
            Err(ExprError::Parse(format!(
                "trailing tokens after expression ({} left)",
                self.toks.len() - self.pos
            )))
        }
    }

    fn parse_expr(&mut self) -> Result<Ast, ExprError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Ast, ExprError> {
        let mut left = self.parse_and()?;
        while self.eat(&Tok::OrOr) {
            let right = self.parse_and()?;
            left = Ast::Bin(BinOp::Or, Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn parse_and(&mut self) -> Result<Ast, ExprError> {
        let mut left = self.parse_cmp()?;
        while self.eat(&Tok::AndAnd) {
            let right = self.parse_cmp()?;
            left = Ast::Bin(BinOp::And, Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn parse_cmp(&mut self) -> Result<Ast, ExprError> {
        let mut left = self.parse_rel()?;
        loop {
            let op = match self.peek() {
                Some(Tok::EqEq) => BinOp::Eq,
                Some(Tok::NotEq) => BinOp::Ne,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_rel()?;
            left = Ast::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn parse_rel(&mut self) -> Result<Ast, ExprError> {
        let mut left = self.parse_member()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Lt) => BinOp::Lt,
                Some(Tok::Le) => BinOp::Le,
                Some(Tok::Gt) => BinOp::Gt,
                Some(Tok::Ge) => BinOp::Ge,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_member()?;
            left = Ast::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn parse_member(&mut self) -> Result<Ast, ExprError> {
        let mut left = self.parse_add()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Contains) => BinOp::Contains,
                Some(Tok::In) => BinOp::In,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_add()?;
            left = Ast::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn parse_add(&mut self) -> Result<Ast, ExprError> {
        let mut left = self.parse_mul()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Plus) => BinOp::Add,
                Some(Tok::Minus) => BinOp::Sub,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_mul()?;
            left = Ast::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn parse_mul(&mut self) -> Result<Ast, ExprError> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Star) => BinOp::Mul,
                Some(Tok::Slash) => BinOp::Div,
                Some(Tok::Percent) => BinOp::Rem,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_unary()?;
            left = Ast::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn parse_unary(&mut self) -> Result<Ast, ExprError> {
        match self.peek() {
            Some(Tok::Bang) => {
                self.pos += 1;
                Ok(Ast::Unary('!', Box::new(self.parse_unary()?)))
            }
            Some(Tok::Minus) => {
                self.pos += 1;
                Ok(Ast::Unary('-', Box::new(self.parse_unary()?)))
            }
            _ => self.parse_primary(),
        }
    }
    fn parse_primary(&mut self) -> Result<Ast, ExprError> {
        match self.bump() {
            Some(Tok::Num(v, is_int)) => Ok(Ast::Lit(num_value(v, is_int))),
            Some(Tok::Str(s)) => Ok(Ast::Lit(Value::String(s))),
            Some(Tok::True) => Ok(Ast::Lit(Value::Bool(true))),
            Some(Tok::False) => Ok(Ast::Lit(Value::Bool(false))),
            Some(Tok::Null) => Ok(Ast::Lit(Value::Null)),
            Some(Tok::LParen) => {
                let inner = self.parse_expr()?;
                if !self.eat(&Tok::RParen) {
                    return Err(ExprError::Parse("expected ')'".into()));
                }
                Ok(inner)
            }
            Some(Tok::Ident(name)) => {
                // function call?
                if self.eat(&Tok::LParen) {
                    let mut args = Vec::new();
                    if self.peek() != Some(&Tok::RParen) {
                        loop {
                            args.push(self.parse_expr()?);
                            if self.eat(&Tok::Comma) {
                                continue;
                            }
                            break;
                        }
                    }
                    if !self.eat(&Tok::RParen) {
                        return Err(ExprError::Parse("expected ')' after args".into()));
                    }
                    Ok(Ast::Call(name, args))
                } else {
                    // path: name ('.' key | '[' expr ']')*
                    let mut segs = vec![Seg::Key(name)];
                    self.parse_path_tail(&mut segs)?;
                    Ok(Ast::Path(segs))
                }
            }
            other => Err(ExprError::Parse(format!("unexpected token {other:?}"))),
        }
    }
    fn parse_path_tail(&mut self, segs: &mut Vec<Seg>) -> Result<(), ExprError> {
        loop {
            if self.eat(&Tok::Dot) {
                match self.bump() {
                    Some(Tok::Ident(k)) => segs.push(Seg::Key(k)),
                    // allow reserved words as keys after a dot (e.g. .in)
                    Some(Tok::In) => segs.push(Seg::Key("in".into())),
                    Some(Tok::Contains) => segs.push(Seg::Key("contains".into())),
                    _ => return Err(ExprError::Parse("expected key after '.'".into())),
                }
            } else if self.eat(&Tok::LBracket) {
                let idx = self.parse_expr()?;
                if !self.eat(&Tok::RBracket) {
                    return Err(ExprError::Parse("expected ']'".into()));
                }
                segs.push(Seg::Index(Box::new(idx)));
            } else {
                break;
            }
        }
        Ok(())
    }
}

fn num_value(v: f64, is_int: bool) -> Value {
    if is_int && v.fract() == 0.0 && v.abs() < 9.007e15 {
        Value::Number((v as i64).into())
    } else {
        serde_json::Number::from_f64(v)
            .map(Value::Number)
            .unwrap_or(Value::Null)
    }
}

// ---------------------------------------------------------------------------
// Evaluator
// ---------------------------------------------------------------------------

fn eval_ast(ast: &Ast, ctx: &Value) -> Result<Value, ExprError> {
    match ast {
        Ast::Lit(v) => Ok(v.clone()),
        Ast::Path(segs) => Ok(resolve_path(segs, ctx)?),
        Ast::Unary(op, e) => {
            let v = eval_ast(e, ctx)?;
            match op {
                '!' => Ok(Value::Bool(!truthy(&v))),
                '-' => {
                    let n = as_f64(&v).ok_or_else(|| ExprError::Eval("unary '-' on non-number".into()))?;
                    Ok(num_value(-n, v.is_i64() || v.is_u64()))
                }
                _ => unreachable!(),
            }
        }
        Ast::Bin(op, l, r) => eval_bin(*op, l, r, ctx),
        Ast::Call(name, args) => eval_call(name, args, ctx),
    }
}

fn resolve_path(segs: &[Seg], ctx: &Value) -> Result<Value, ExprError> {
    let mut cur = ctx;
    for seg in segs {
        match seg {
            Seg::Key(k) => match cur {
                Value::Object(m) => {
                    cur = m.get(k).unwrap_or(&Value::Null);
                }
                _ => return Ok(Value::Null),
            },
            Seg::Index(e) => {
                let idx = eval_ast(e, ctx)?;
                match (cur, &idx) {
                    (Value::Array(a), Value::Number(n)) => {
                        let i = n.as_i64().unwrap_or(-1);
                        if i < 0 {
                            return Ok(Value::Null);
                        }
                        cur = a.get(i as usize).unwrap_or(&Value::Null);
                    }
                    (Value::Object(m), Value::String(s)) => {
                        cur = m.get(s).unwrap_or(&Value::Null);
                    }
                    _ => return Ok(Value::Null),
                }
            }
        }
    }
    Ok(cur.clone())
}

fn eval_bin(op: BinOp, l: &Ast, r: &Ast, ctx: &Value) -> Result<Value, ExprError> {
    // Short-circuit boolean ops.
    match op {
        BinOp::And => {
            let lv = eval_ast(l, ctx)?;
            if !truthy(&lv) {
                return Ok(Value::Bool(false));
            }
            return Ok(Value::Bool(truthy(&eval_ast(r, ctx)?)));
        }
        BinOp::Or => {
            let lv = eval_ast(l, ctx)?;
            if truthy(&lv) {
                return Ok(Value::Bool(true));
            }
            return Ok(Value::Bool(truthy(&eval_ast(r, ctx)?)));
        }
        _ => {}
    }
    let a = eval_ast(l, ctx)?;
    let b = eval_ast(r, ctx)?;
    match op {
        BinOp::Eq => Ok(Value::Bool(json_eq(&a, &b))),
        BinOp::Ne => Ok(Value::Bool(!json_eq(&a, &b))),
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let ord = compare(&a, &b)?;
            let res = match op {
                BinOp::Lt => ord == std::cmp::Ordering::Less,
                BinOp::Le => ord != std::cmp::Ordering::Greater,
                BinOp::Gt => ord == std::cmp::Ordering::Greater,
                BinOp::Ge => ord != std::cmp::Ordering::Less,
                _ => unreachable!(),
            };
            Ok(Value::Bool(res))
        }
        BinOp::Contains => Ok(Value::Bool(contains(&a, &b))),
        BinOp::In => Ok(Value::Bool(contains(&b, &a))),
        BinOp::Add => add(&a, &b),
        BinOp::Sub => arith(&a, &b, |x, y| x - y),
        BinOp::Mul => arith(&a, &b, |x, y| x * y),
        BinOp::Div => {
            let y = as_f64(&b).ok_or_else(|| ExprError::Eval("'/' on non-number".into()))?;
            if y == 0.0 {
                return Err(ExprError::Eval("division by zero".into()));
            }
            arith(&a, &b, |x, y| x / y)
        }
        BinOp::Rem => {
            let y = as_f64(&b).ok_or_else(|| ExprError::Eval("'%' on non-number".into()))?;
            if y == 0.0 {
                return Err(ExprError::Eval("modulo by zero".into()));
            }
            arith(&a, &b, |x, y| x % y)
        }
        BinOp::And | BinOp::Or => unreachable!(),
    }
}

fn eval_call(name: &str, args: &[Ast], ctx: &Value) -> Result<Value, ExprError> {
    let argv: Result<Vec<Value>, ExprError> = args.iter().map(|a| eval_ast(a, ctx)).collect();
    let argv = argv?;
    let arity = |n: usize| {
        if argv.len() == n {
            Ok(())
        } else {
            Err(ExprError::Eval(format!(
                "{name}() expects {n} arg(s), got {}",
                argv.len()
            )))
        }
    };
    match name {
        "len" => {
            arity(1)?;
            let n = match &argv[0] {
                Value::String(s) => s.chars().count(),
                Value::Array(a) => a.len(),
                Value::Object(o) => o.len(),
                _ => return Err(ExprError::Eval("len() expects string/array/object".into())),
            };
            Ok(Value::Number(n.into()))
        }
        "lower" => {
            arity(1)?;
            Ok(Value::String(stringify(&argv[0]).to_lowercase()))
        }
        "upper" => {
            arity(1)?;
            Ok(Value::String(stringify(&argv[0]).to_uppercase()))
        }
        "default" => {
            arity(2)?;
            Ok(if argv[0].is_null() {
                argv[1].clone()
            } else {
                argv[0].clone()
            })
        }
        "has" => {
            arity(1)?;
            Ok(Value::Bool(!argv[0].is_null()))
        }
        "not" => {
            arity(1)?;
            Ok(Value::Bool(!truthy(&argv[0])))
        }
        "bool" => {
            arity(1)?;
            Ok(Value::Bool(truthy(&argv[0])))
        }
        "int" => {
            arity(1)?;
            let n = coerce_f64(&argv[0]).ok_or_else(|| ExprError::Eval("int() bad arg".into()))?;
            Ok(Value::Number((n.trunc() as i64).into()))
        }
        "float" => {
            arity(1)?;
            let n = coerce_f64(&argv[0]).ok_or_else(|| ExprError::Eval("float() bad arg".into()))?;
            Ok(serde_json::Number::from_f64(n)
                .map(Value::Number)
                .unwrap_or(Value::Null))
        }
        "str" => {
            arity(1)?;
            Ok(Value::String(stringify(&argv[0])))
        }
        _ => Err(ExprError::Eval(format!("unknown function '{name}'"))),
    }
}

fn json_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(_), Value::Number(_)) => {
            a.as_f64().zip(b.as_f64()).map(|(x, y)| x == y).unwrap_or(false)
        }
        _ => a == b,
    }
}

fn as_f64(v: &Value) -> Option<f64> {
    v.as_f64()
}

/// Coerce numbers and numeric strings to f64 (for `int()`/`float()`).
fn coerce_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

fn compare(a: &Value, b: &Value) -> Result<std::cmp::Ordering, ExprError> {
    if let (Some(x), Some(y)) = (a.as_f64(), b.as_f64()) {
        return x
            .partial_cmp(&y)
            .ok_or_else(|| ExprError::Eval("NaN comparison".into()));
    }
    if let (Value::String(x), Value::String(y)) = (a, b) {
        return Ok(x.cmp(y));
    }
    Err(ExprError::Eval("cannot order these values".into()))
}

fn contains(hay: &Value, needle: &Value) -> bool {
    match hay {
        Value::String(s) => s.contains(&stringify(needle)),
        Value::Array(a) => a.iter().any(|e| json_eq(e, needle)),
        Value::Object(m) => match needle {
            Value::String(k) => m.contains_key(k),
            _ => false,
        },
        _ => false,
    }
}

fn add(a: &Value, b: &Value) -> Result<Value, ExprError> {
    if a.is_string() || b.is_string() {
        return Ok(Value::String(format!("{}{}", stringify(a), stringify(b))));
    }
    arith(a, b, |x, y| x + y)
}

fn arith(a: &Value, b: &Value, f: impl Fn(f64, f64) -> f64) -> Result<Value, ExprError> {
    let x = as_f64(a).ok_or_else(|| ExprError::Eval("arithmetic on non-number".into()))?;
    let y = as_f64(b).ok_or_else(|| ExprError::Eval("arithmetic on non-number".into()))?;
    let both_int = (a.is_i64() || a.is_u64()) && (b.is_i64() || b.is_u64());
    Ok(num_value(f(x, y), both_int))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(src: &str, ctx: &Value) -> Value {
        eval(src, ctx).unwrap()
    }

    #[test]
    fn literals() {
        let c = json!({});
        assert_eq!(ev("1", &c), json!(1));
        assert_eq!(ev("1.5", &c), json!(1.5));
        assert_eq!(ev("'hi'", &c), json!("hi"));
        assert_eq!(ev("\"hi\"", &c), json!("hi"));
        assert_eq!(ev("true", &c), json!(true));
        assert_eq!(ev("null", &c), json!(null));
    }

    #[test]
    fn paths() {
        let c = json!({"a": {"b": [10, 20]}, "k": "v"});
        assert_eq!(ev("a.b[1]", &c), json!(20));
        assert_eq!(ev("k", &c), json!("v"));
        assert_eq!(ev("a.missing", &c), json!(null));
        assert_eq!(ev("nope.deep.path", &c), json!(null));
    }

    #[test]
    fn comparisons_and_bools() {
        let c = json!({"n": 3, "ok": true, "s": "abc"});
        assert_eq!(ev("n == 3", &c), json!(true));
        assert_eq!(ev("n != 4", &c), json!(true));
        assert_eq!(ev("n < 5 && n > 1", &c), json!(true));
        assert_eq!(ev("n > 5 || ok", &c), json!(true));
        assert_eq!(ev("!ok", &c), json!(false));
        assert_eq!(ev("1 == 1.0", &c), json!(true));
        assert_eq!(ev("s == 'abc'", &c), json!(true));
    }

    #[test]
    fn arithmetic() {
        let c = json!({});
        assert_eq!(ev("2 + 3 * 4", &c), json!(14));
        assert_eq!(ev("(2 + 3) * 4", &c), json!(20));
        assert_eq!(ev("7 / 2", &c), json!(3.5));
        assert_eq!(ev("7 % 3", &c), json!(1));
        assert_eq!(ev("-5 + 2", &c), json!(-3));
        assert_eq!(ev("'a' + 'b'", &c), json!("ab"));
        assert_eq!(ev("'n=' + 5", &c), json!("n=5"));
    }

    #[test]
    fn contains_and_in() {
        let c = json!({"arr": [1, 2, 3], "s": "hello world", "obj": {"x": 1}});
        assert_eq!(ev("arr contains 2", &c), json!(true));
        assert_eq!(ev("arr contains 9", &c), json!(false));
        assert_eq!(ev("s contains 'world'", &c), json!(true));
        assert_eq!(ev("2 in arr", &c), json!(true));
        assert_eq!(ev("obj contains 'x'", &c), json!(true));
    }

    #[test]
    fn functions() {
        let c = json!({"s": "AbC", "arr": [1, 2], "maybe": null});
        assert_eq!(ev("len(s)", &c), json!(3));
        assert_eq!(ev("len(arr)", &c), json!(2));
        assert_eq!(ev("lower(s)", &c), json!("abc"));
        assert_eq!(ev("upper(s)", &c), json!("ABC"));
        assert_eq!(ev("default(maybe, 'fallback')", &c), json!("fallback"));
        assert_eq!(ev("default(s, 'fallback')", &c), json!("AbC"));
        assert_eq!(ev("has(s)", &c), json!(true));
        assert_eq!(ev("has(maybe)", &c), json!(false));
        assert_eq!(ev("int('42')", &c), json!(42));
        assert_eq!(ev("not(false)", &c), json!(true));
    }

    #[test]
    fn truthiness_rules() {
        let c = json!({"empty": "", "zero": 0, "arr": [], "obj": {}, "s": "x"});
        assert!(!eval_bool("empty", &c));
        assert!(!eval_bool("zero", &c));
        assert!(!eval_bool("arr", &c));
        assert!(!eval_bool("obj", &c));
        assert!(eval_bool("s", &c));
        assert!(!eval_bool("missing", &c));
    }

    #[test]
    fn eval_bool_swallows_errors() {
        let c = json!({});
        // parse error → false, not panic
        assert!(!eval_bool("1 +", &c));
        assert!(!eval_bool("@@@", &c));
    }

    #[test]
    fn div_by_zero_errors() {
        let c = json!({});
        assert!(eval("1 / 0", &c).is_err());
        assert!(eval("1 % 0", &c).is_err());
    }

    #[test]
    fn templating() {
        let c = json!({"a": {"b": "X"}, "n": 2});
        assert_eq!(render_template("v={{a.b}} n={{ n + 1 }}", &c), "v=X n=3");
        assert_eq!(render_template("no exprs here", &c), "no exprs here");
        // bad expr → empty
        assert_eq!(render_template("[{{ bad + }}]", &c), "[]");
        // legacy single-brace untouched
        assert_eq!(render_template("{key} kept", &c), "{key} kept");
    }

    #[test]
    fn realistic_edge_conditions() {
        let review_pass = json!({"output": {"passed": true, "blocking": 0}});
        assert!(eval_bool("output.passed == true", &review_pass));
        assert!(eval_bool("output.blocking == 0", &review_pass));
        let review_fail = json!({"output": {"passed": false, "blocking": 3}});
        assert!(eval_bool("output.passed == false", &review_fail));
        assert!(!eval_bool("output.blocking == 0", &review_fail));
        assert!(eval_bool("output.blocking > 0", &review_fail));
    }
}
