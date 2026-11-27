//! Kotest spec trees for the gutter: every container and leaf whose name is a fixed
//! string, with its path of bare names (Kotest's test id; the `Given: ` / `It: ` prefixes
//! are display-only). Runs address them with Kotest's `pkg.Spec.a -- b -- c` pattern.

use std::collections::HashMap;

use super::kotlin_lex::{lex, Tok, Token};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KotestNode {
    /// The spec's simple class name.
    pub spec: String,
    pub path: Vec<String>,
    pub line: u32,
    pub container: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    Fun,
    Str,
    Should,
    Describe,
    Behavior,
    Word,
    Free,
    Feature,
    Expect,
    Annotation,
}

fn style_of(name: &str) -> Option<Style> {
    Some(match name {
        "FunSpec" => Style::Fun,
        "StringSpec" => Style::Str,
        "ShouldSpec" => Style::Should,
        "DescribeSpec" => Style::Describe,
        "BehaviorSpec" => Style::Behavior,
        "WordSpec" => Style::Word,
        "FreeSpec" => Style::Free,
        "FeatureSpec" => Style::Feature,
        "ExpectSpec" => Style::Expect,
        "AnnotationSpec" => Style::Annotation,
        _ => return None,
    })
}

/// `Some(true)` for a container call of `style`, `Some(false)` for a leaf call.
fn call_kind(style: Style, name: &str) -> Option<bool> {
    use Style::*;
    let name = name.strip_prefix('x').unwrap_or(name);
    match (style, name) {
        (Fun | Should | Expect, "context")
        | (Describe, "describe" | "context")
        | (
            Behavior,
            "given" | "Given" | "when" | "When" | "and" | "And" | "context" | "Context",
        )
        | (Feature, "feature") => Some(true),
        (Fun, "test")
        | (Should, "should")
        | (Describe, "it")
        | (Behavior, "then" | "Then")
        | (Feature, "scenario")
        | (Expect, "expect") => Some(false),
        _ => None,
    }
}

enum Frame {
    /// Statements that may declare tests: a spec body, `init`, or a container's body.
    Scope {
        style: Style,
        spec: String,
        path: Vec<String>,
    },
    /// A spec's class body (`init { … }`, AnnotationSpec's `@Test fun` methods).
    ClassBody { style: Style, spec: String },
    Other,
}

/// A fixed name Kotest addresses as written: no template, newline, ` -- ` (the path
/// separator) or `.` (Gradle reads a dot as the class/test boundary). Whitespace runs
/// collapse to one space, as Kotest does.
fn fixed_name(tok: &Tok) -> Option<String> {
    let Tok::Str { value, template } = tok else {
        return None;
    };
    if *template || value.contains('\n') || value.contains('.') {
        return None;
    }
    let name = value.split_whitespace().collect::<Vec<_>>().join(" ");
    (!name.is_empty() && !name.contains(" -- ")).then_some(name)
}

/// The index after the `)` that balances the `(` at `open`.
fn skip_parens(t: &[Token], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (j, tok) in t.iter().enumerate().skip(open) {
        match tok.tok {
            Tok::LParen => depth += 1,
            Tok::RParen => {
                depth -= 1;
                if depth == 0 {
                    return Some(j + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// From `j`, skip `.config(…)` chains and return the index of the `{` that opens the body.
fn body_brace(t: &[Token], mut j: usize) -> Option<usize> {
    loop {
        match t.get(j).map(|x| &x.tok) {
            Some(Tok::LBrace) => return Some(j),
            Some(Tok::Dot) if t.get(j + 1).map(|x| &x.tok) == Some(&Tok::Ident("config".into())) => {
                (t.get(j + 2)?.tok == Tok::LParen).then_some(())?;
                j = skip_parens(t, j + 2)?;
            }
            _ => return None,
        }
    }
}

fn ident(t: &[Token], j: usize) -> Option<&str> {
    match t.get(j).map(|x| &x.tok) {
        Some(Tok::Ident(s)) => Some(s),
        _ => None,
    }
}

/// A test declared at `i` in a `style` scope (`nested`: inside a container): its name
/// token, whether it is a container, and the index of its body's `{`.
fn match_test(t: &[Token], i: usize, style: Style, nested: bool) -> Option<(usize, bool, usize)> {
    if i > 0 && t[i - 1].tok == Tok::Dot {
        return None;
    }
    match &t[i].tok {
        Tok::Ident(name) => {
            let container = call_kind(style, name)?;
            (t.get(i + 1)?.tok == Tok::LParen).then_some(())?;
            matches!(t.get(i + 2)?.tok, Tok::Str { .. }).then_some(())?;
            (t.get(i + 3)?.tok == Tok::RParen).then_some(())?;
            Some((i + 2, container, body_brace(t, i + 4)?))
        }
        Tok::Str { .. } => {
            let next = &t.get(i + 1)?.tok;
            match style {
                Style::Free if *next == Tok::Minus => {
                    (t.get(i + 2)?.tok == Tok::LBrace).then_some((i, true, i + 2))
                }
                Style::Word if matches!(next, Tok::Ident(k) if k == "should" || k == "When" || k == "when") => {
                    (t.get(i + 2)?.tok == Tok::LBrace).then_some((i, true, i + 2))
                }
                Style::Str | Style::Free => Some((i, false, body_brace(t, i + 1)?)),
                Style::Word if nested => Some((i, false, body_brace(t, i + 1)?)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// A `class Name … : [io.kotest.core.spec.style.]Style(` at `i`: the spec, its style,
/// and the `{` that opens its tests (the constructor lambda) or its class body.
fn match_spec(t: &[Token], i: usize) -> Option<(String, Style, usize, bool)> {
    (ident(t, i)? == "class").then_some(())?;
    if (i.saturating_sub(3)..i).any(|k| ident(t, k) == Some("abstract")) {
        return None;
    }
    let spec = ident(t, i + 1)?.to_string();
    let mut j = i + 2;
    if t.get(j)?.tok == Tok::LParen {
        j = skip_parens(t, j)?;
    }
    while t.get(j)?.tok == Tok::Newline {
        j += 1;
    }
    (t.get(j)?.tok == Tok::Colon).then_some(())?;
    j += 1;
    while t.get(j)?.tok == Tok::Newline {
        j += 1;
    }
    let mut last = ident(t, j)?;
    while t.get(j + 1)?.tok == Tok::Dot {
        j += 2;
        last = ident(t, j)?;
    }
    let style = style_of(last)?;
    (t.get(j + 1)?.tok == Tok::LParen).then_some(())?;
    if t.get(j + 2)?.tok == Tok::LBrace {
        return Some((spec, style, j + 2, true));
    }
    let mut k = skip_parens(t, j + 1)?;
    while t.get(k)?.tok != Tok::LBrace {
        k += 1;
    }
    Some((spec, style, k, false))
}

/// The Kotest tests of every spec class in `src`, in source order.
pub(crate) fn kotest_tree(src: &str) -> Vec<KotestNode> {
    let t = lex(src);
    let mut out = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();
    let mut opens: HashMap<usize, Frame> = HashMap::new();
    let mut i = 0usize;
    while i < t.len() {
        match t[i].tok {
            Tok::LBrace => {
                stack.push(opens.remove(&i).unwrap_or(Frame::Other));
                i += 1;
                continue;
            }
            Tok::RBrace => {
                stack.pop();
                i += 1;
                continue;
            }
            _ => {}
        }
        match stack.last() {
            Some(Frame::Scope { style, spec, path }) => {
                if let Some((name_idx, container, brace)) =
                    match_test(&t, i, *style, !path.is_empty())
                {
                    let frame = match fixed_name(&t[name_idx].tok) {
                        Some(name) => {
                            let mut p = path.clone();
                            p.push(name);
                            out.push(KotestNode {
                                spec: spec.clone(),
                                path: p.clone(),
                                line: t[i].line,
                                container,
                            });
                            if container {
                                Frame::Scope {
                                    style: *style,
                                    spec: spec.clone(),
                                    path: p,
                                }
                            } else {
                                Frame::Other
                            }
                        }
                        None => Frame::Other,
                    };
                    opens.insert(brace, frame);
                    i = brace;
                    continue;
                }
            }
            Some(Frame::ClassBody { style, spec }) => {
                if ident(&t, i) == Some("init") && t.get(i + 1).map(|x| &x.tok) == Some(&Tok::LBrace) {
                    let frame = Frame::Scope {
                        style: *style,
                        spec: spec.clone(),
                        path: Vec::new(),
                    };
                    opens.insert(i + 1, frame);
                } else if *style == Style::Annotation
                    && t[i].tok == Tok::At
                    && ident(&t, i + 1) == Some("Test")
                {
                    let mut j = i + 2;
                    while j < t.len() && ident(&t, j) != Some("fun") && t[j].tok != Tok::LBrace {
                        j += 1;
                    }
                    if let Some(name) = ident(&t, j).and(ident(&t, j + 1)) {
                        out.push(KotestNode {
                            spec: spec.clone(),
                            path: vec![name.to_string()],
                            line: t[i].line,
                            container: false,
                        });
                    }
                }
            }
            Some(Frame::Other) | None => {
                if let Some((spec, style, brace, lambda)) = match_spec(&t, i) {
                    let frame = if lambda {
                        Frame::Scope {
                            style,
                            spec,
                            path: Vec::new(),
                        }
                    } else {
                        Frame::ClassBody { style, spec }
                    };
                    opens.insert(brace, frame);
                    i = brace;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(src: &str) -> Vec<(String, u32, bool)> {
        kotest_tree(src)
            .into_iter()
            .map(|n| (n.path.join(" -- "), n.line, n.container))
            .collect()
    }

    #[test]
    fn fun_spec_nested_contexts_config_and_disabled() {
        let src = r#"package com.acme
class LoginSpec : FunSpec({
    context("a user") {
        context("logs   in") {
            test("sees dashboard") { }
            test("sees failing sibling").config(enabled = true, timeout = { 1 }) { }
        }
    }
    xtest("disabled one") { }
})
"#;
        assert_eq!(
            paths(src),
            vec![
                ("a user".into(), 3, true),
                ("a user -- logs in".into(), 4, true),
                ("a user -- logs in -- sees dashboard".into(), 5, false),
                ("a user -- logs in -- sees failing sibling".into(), 6, false),
                ("disabled one".into(), 9, false),
            ]
        );
    }

    #[test]
    fn dynamic_dotted_and_data_driven_names_get_no_item_but_their_container_does() {
        let src = r#"class P : FunSpec({
    context("parsing") {
        withData("a", "b") { test("inner") { } }
        listOf(1, 2).forEach { n -> test("case $n") { } }
        test("v1.2 works") { }
        test("a -- b") { }
    }
    context("x ${y}") { test("hidden child") { } }
    helper { test("in a helper") { } }
})
"#;
        assert_eq!(paths(src), vec![("parsing".into(), 2, true)]);
    }

    #[test]
    fn every_style_yields_its_containers_and_leaves() {
        let src = r#"
class S : StringSpec({
    "adds numbers" { }
    "configured".config(enabled = false) { }
})
class D : io.kotest.core.spec.style.DescribeSpec({
    describe("a cart") { context("with items") { it("totals") { } } }
})
class B : BehaviorSpec({
    given("a stack") { `when`("pushed") { then("top") { } }
        When("popped") { Then("throws") { } } }
})
class W : WordSpec({ "a stack" should { "push" { } }
    "the queue" When { "empty" should { "fail" { } } } })
class F : FreeSpec({ "outer" - { "inner" - { "leaf" { } } } })
class Fe : FeatureSpec({ feature("login") { scenario("works") { } } })
class E : ExpectSpec({ context("math") { expect("adds") { } } })
class Sh : ShouldSpec({ context("ctx") { should("go") { } } })
class I : FunSpec() {
    init { test("in init") { } }
}
class A : AnnotationSpec() {
    @Test
    fun plainMethod() { }
}
abstract class Base : FunSpec({ test("not runnable") { } })
"#;
        let got: Vec<(String, String, bool)> = kotest_tree(src)
            .into_iter()
            .map(|n| (n.spec, n.path.join(" -- "), n.container))
            .collect();
        let want: Vec<(&str, &str, bool)> = vec![
            ("S", "adds numbers", false),
            ("S", "configured", false),
            ("D", "a cart", true),
            ("D", "a cart -- with items", true),
            ("D", "a cart -- with items -- totals", false),
            ("B", "a stack", true),
            ("B", "a stack -- pushed", true),
            ("B", "a stack -- pushed -- top", false),
            ("B", "a stack -- popped", true),
            ("B", "a stack -- popped -- throws", false),
            ("W", "a stack", true),
            ("W", "a stack -- push", false),
            ("W", "the queue", true),
            ("W", "the queue -- empty", true),
            ("W", "the queue -- empty -- fail", false),
            ("F", "outer", true),
            ("F", "outer -- inner", true),
            ("F", "outer -- inner -- leaf", false),
            ("Fe", "login", true),
            ("Fe", "login -- works", false),
            ("E", "math", true),
            ("E", "math -- adds", false),
            ("Sh", "ctx", true),
            ("Sh", "ctx -- go", false),
            ("I", "in init", false),
            ("A", "plainMethod", false),
        ];
        let want: Vec<(String, String, bool)> = want
            .into_iter()
            .map(|(a, b, c)| (a.to_string(), b.to_string(), c))
            .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn a_string_spec_leaf_is_not_a_container_body_test() {
        // A plain string statement inside a StringSpec leaf is not another test.
        let src = "class S : StringSpec({\n  \"a\" { val s = \"b\" { } }\n})\n";
        assert_eq!(paths(src), vec![("a".into(), 2, false)]);
    }
}
