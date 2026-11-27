//! A small Kotlin lexer for the gutter and the debugger: identifiers (backtick names
//! unquoted), string literals (raw or not, with a "has a template" flag), the brackets and
//! punctuation the callers read, and newlines. Comments (nested block comments too), char
//! literals and `${ … }` template expressions, including strings and braces inside them,
//! never produce brace tokens.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Tok {
    Ident(String),
    /// A string literal's value (common escapes decoded) and whether it has a `$` template.
    Str {
        value: String,
        template: bool,
    },
    LBrace,
    RBrace,
    LParen,
    RParen,
    Minus,
    Dot,
    Comma,
    Colon,
    At,
    Newline,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Token {
    pub tok: Tok,
    /// 1-based source line of the token's first character.
    pub line: u32,
}

/// Tokens of `src`, in order.
pub(crate) fn lex(src: &str) -> Vec<Token> {
    let mut lx = Lexer {
        chars: src.chars().collect(),
        i: 0,
        line: 1,
    };
    let mut out = Vec::new();
    lx.code(&mut out, false);
    out
}

struct Lexer {
    chars: Vec<char>,
    i: usize,
    line: u32,
}

impl Lexer {
    fn peek(&self, off: usize) -> Option<char> {
        self.chars.get(self.i + off).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.chars.get(self.i).copied()?;
        self.i += 1;
        if c == '\n' {
            self.line += 1;
        }
        Some(c)
    }

    /// Code until the end, or (`in_template`) until the `}` that closes a `${`.
    fn code(&mut self, out: &mut Vec<Token>, in_template: bool) {
        let mut depth = 0usize;
        while let Some(c) = self.peek(0) {
            let line = self.line;
            let simple = match c {
                '{' => {
                    depth += 1;
                    Some(Tok::LBrace)
                }
                '}' => {
                    if in_template && depth == 0 {
                        self.bump();
                        return;
                    }
                    depth = depth.saturating_sub(1);
                    Some(Tok::RBrace)
                }
                '(' => Some(Tok::LParen),
                ')' => Some(Tok::RParen),
                '-' => Some(Tok::Minus),
                '.' => Some(Tok::Dot),
                ',' => Some(Tok::Comma),
                ':' => Some(Tok::Colon),
                '@' => Some(Tok::At),
                '\n' => Some(Tok::Newline),
                _ => None,
            };
            if let Some(tok) = simple {
                self.bump();
                out.push(Token { tok, line });
                continue;
            }
            match c {
                '/' if self.peek(1) == Some('/') => {
                    while self.peek(0).is_some_and(|c| c != '\n') {
                        self.bump();
                    }
                }
                '/' if self.peek(1) == Some('*') => self.block_comment(),
                '"' => {
                    let tok = self.string();
                    out.push(Token { tok, line });
                }
                '\'' => self.char_literal(),
                '`' => {
                    self.bump();
                    let mut name = String::new();
                    while let Some(c) = self.peek(0).filter(|&c| c != '`' && c != '\n') {
                        name.push(c);
                        self.bump();
                    }
                    if self.peek(0) == Some('`') {
                        self.bump();
                    }
                    out.push(Token {
                        tok: Tok::Ident(name),
                        line,
                    });
                }
                c if c.is_alphabetic() || c == '_' => {
                    let mut name = String::new();
                    while let Some(c) = self.peek(0).filter(|&c| c.is_alphanumeric() || c == '_') {
                        name.push(c);
                        self.bump();
                    }
                    out.push(Token {
                        tok: Tok::Ident(name),
                        line,
                    });
                }
                c if c.is_whitespace() => {
                    self.bump();
                }
                _ => {
                    self.bump();
                    out.push(Token {
                        tok: Tok::Other,
                        line,
                    });
                }
            }
        }
    }

    fn block_comment(&mut self) {
        self.i += 2;
        let mut depth = 1usize;
        while depth > 0 && self.peek(0).is_some() {
            if self.peek(0) == Some('/') && self.peek(1) == Some('*') {
                self.i += 2;
                depth += 1;
            } else if self.peek(0) == Some('*') && self.peek(1) == Some('/') {
                self.i += 2;
                depth -= 1;
            } else {
                self.bump();
            }
        }
    }

    fn char_literal(&mut self) {
        self.bump();
        while let Some(c) = self.peek(0) {
            if c == '\n' {
                return;
            }
            self.bump();
            match c {
                '\\' => {
                    self.bump();
                }
                '\'' => return,
                _ => {}
            }
        }
    }

    /// A `"…"` or `"""…"""` literal, the cursor on its first quote.
    fn string(&mut self) -> Tok {
        let raw = self.peek(1) == Some('"') && self.peek(2) == Some('"');
        self.i += if raw { 3 } else { 1 };
        let mut value = String::new();
        let mut template = false;
        while let Some(c) = self.peek(0) {
            if raw && c == '"' && self.peek(1) == Some('"') && self.peek(2) == Some('"') {
                // The closing quotes are the last three of a run (`""""` ends with a `"`).
                while self.peek(3) == Some('"') {
                    value.push('"');
                    self.i += 1;
                }
                self.i += 3;
                break;
            }
            if !raw && c == '"' {
                self.bump();
                break;
            }
            if !raw && c == '\n' {
                break;
            }
            if c == '$' && self.peek(1) == Some('{') {
                template = true;
                self.i += 2;
                self.code(&mut Vec::new(), true);
                continue;
            }
            if c == '$' && self.peek(1).is_some_and(|n| n.is_alphabetic() || n == '_' || n == '`') {
                template = true;
            }
            self.bump();
            if !raw && c == '\\' {
                match self.bump() {
                    Some('n') => value.push('\n'),
                    Some('t') => value.push('\t'),
                    Some('r') => value.push('\r'),
                    Some(e) => value.push(e),
                    None => {}
                }
                continue;
            }
            value.push(c);
        }
        Tok::Str { value, template }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<Tok> {
        lex(src).into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn braces_inside_strings_templates_and_comments_are_not_tokens() {
        let src = r#"a { "}" /* { /* } */ } */ "${ "}{" }" '{' """ { """ // {
}"#;
        let braces: Vec<Tok> = toks(src)
            .into_iter()
            .filter(|t| matches!(t, Tok::LBrace | Tok::RBrace))
            .collect();
        assert_eq!(braces, vec![Tok::LBrace, Tok::RBrace]);
    }

    #[test]
    fn strings_carry_value_template_flag_and_line() {
        let t = lex(r#"x("a \"b\"")
"v $x" "${1}" `when`"#);
        assert_eq!(
            t[2].tok,
            Tok::Str {
                value: "a \"b\"".into(),
                template: false
            }
        );
        assert!(matches!(&t[5].tok, Tok::Str { template: true, .. }));
        assert_eq!(t[5].line, 2);
        assert!(matches!(&t[6].tok, Tok::Str { template: true, .. }));
        assert_eq!(t[7].tok, Tok::Ident("when".into()));
    }

    #[test]
    fn an_escaped_dollar_is_not_a_template() {
        assert_eq!(
            toks(r#""cost \$5""#)[0],
            Tok::Str {
                value: "cost $5".into(),
                template: false
            }
        );
    }
}
