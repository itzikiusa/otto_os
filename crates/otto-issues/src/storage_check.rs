//! Checks on Confluence storage-format XHTML.
//!
//! - [`check_well_formed`] — a small XML parser: every tag closed and
//!   properly nested, quoted unique attributes, `&` only as an entity, CDATA
//!   and comments terminated. Used on caller-supplied `body_html` (which may
//!   hold any macro) before it is sent.
//! - [`validate_storage`] — well-formedness plus the element rules for what
//!   Otto's Markdown converter emits: known elements only, list items only in
//!   lists, rows only in tables, task parts only in tasks, no block element
//!   inside a paragraph / heading / inline span, `<br />` and `<hr />` empty.
//! - [`storage_outline`] / [`storage_plain_text`] — the block structure and
//!   text of a body, so a page read back after a write can be compared with
//!   what was sent (Confluence adds attrs such as `ac:macro-id`; ignored).

/// A parsed element (text and CDATA are kept as `#text` children).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Node {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
    /// Decoded text for `#text` nodes.
    pub text: String,
}

impl Node {
    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn elements(&self) -> impl Iterator<Item = &Node> {
        self.children.iter().filter(|c| c.name != "#text")
    }
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')
}

/// Decode one entity starting at `&` (`s` begins after the `&`); returns
/// (decoded, bytes consumed incl. `;`).
fn entity(s: &str) -> Result<(String, usize), String> {
    let end = s.find(';').filter(|&e| e > 0 && e <= 32).ok_or_else(|| {
        format!(
            "a bare `&` (escape it as &amp;) near \"&{}\"",
            s.chars().take(12).collect::<String>()
        )
    })?;
    let name = &s[..end];
    let decoded = if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16),
            None => num.parse(),
        }
        .map_err(|_| format!("bad character reference &{name};"))?;
        char::from_u32(code)
            .ok_or_else(|| format!("bad character reference &{name};"))?
            .to_string()
    } else if name.chars().all(|c| c.is_ascii_alphanumeric()) {
        match name {
            "amp" => "&".into(),
            "lt" => "<".into(),
            "gt" => ">".into(),
            "quot" => "\"".into(),
            "apos" => "'".into(),
            "nbsp" => " ".into(),
            // Confluence accepts the HTML named entities; their text is not
            // needed for a structural comparison.
            _ => " ".into(),
        }
    } else {
        return Err(format!("bad entity &{name};"));
    };
    Ok((decoded, end + 1))
}

fn decode(s: &str) -> Result<String, String> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        let (d, n) = entity(&rest[p + 1..])?;
        out.push_str(&d);
        rest = &rest[p + 1 + n..];
    }
    out.push_str(rest);
    Ok(out)
}

fn text_node(text: String) -> Node {
    Node {
        name: "#text".into(),
        attrs: Vec::new(),
        children: Vec::new(),
        text,
    }
}

/// Deepest element nesting `parse` accepts. The tree it builds is as deep as
/// the input, and its drop glue plus every later walk (`rules`, `outline`,
/// `collect_text`) recurse per level, so an unbounded body (100k nested
/// `<b>`, well under the body limit) would overflow the stack and abort the
/// daemon. Real Confluence storage nests a few dozen levels at most.
const MAX_DEPTH: usize = 256;

/// Parse a storage fragment into a synthetic `#root` element.
pub(crate) fn parse(xhtml: &str) -> Result<Node, String> {
    let mut stack: Vec<Node> = vec![Node {
        name: "#root".into(),
        attrs: Vec::new(),
        children: Vec::new(),
        text: String::new(),
    }];
    let mut i = 0;
    let b = xhtml;
    while i < b.len() {
        let rest = &b[i..];
        if !rest.starts_with('<') {
            let end = rest.find('<').unwrap_or(rest.len());
            let raw = &rest[..end];
            if raw.contains("]]>") {
                return Err("a stray `]]>` in text".into());
            }
            let text = decode(raw)?;
            stack.last_mut().unwrap().children.push(text_node(text));
            i += end;
            continue;
        }
        if let Some(body) = rest.strip_prefix("<![CDATA[") {
            let end = body.find("]]>").ok_or("an unterminated CDATA section")?;
            stack
                .last_mut()
                .unwrap()
                .children
                .push(text_node(body[..end].to_string()));
            i += 9 + end + 3;
            continue;
        }
        if let Some(body) = rest.strip_prefix("<!--") {
            let end = body.find("-->").ok_or("an unterminated comment")?;
            i += 4 + end + 3;
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            return Err("a DOCTYPE / processing instruction is not allowed in a body".into());
        }
        let close = tag_end(rest).ok_or("an unterminated tag")?;
        let tag = &rest[1..close];
        i += close + 1;
        if let Some(name) = tag.strip_prefix('/') {
            let name = name.trim_end();
            // Only the synthetic `#root` frame is left: this close tag closes
            // nothing. Checked before popping, so `</#root>` can't pop the
            // root and leave the stack empty.
            if stack.len() == 1 {
                return Err(format!("`</{name}>` closes nothing"));
            }
            let open = stack.pop().ok_or("unbalanced close tag")?;
            if open.name != name {
                return Err(format!("`</{name}>` closes `<{}>`", open.name));
            }
            stack.last_mut().unwrap().children.push(open);
            continue;
        }
        let (tag, self_closing) = match tag.strip_suffix('/') {
            Some(t) => (t, true),
            None => (tag, false),
        };
        let name_len = tag
            .char_indices()
            .find(|(_, c)| !is_name_char(*c))
            .map_or(tag.len(), |(n, _)| n);
        let name = &tag[..name_len];
        if !name.starts_with(is_name_start) {
            return Err(format!("a bad tag name in `<{tag}>`"));
        }
        let attrs = parse_attrs(&tag[name_len..], name)?;
        let node = Node {
            name: name.to_string(),
            attrs,
            children: Vec::new(),
            text: String::new(),
        };
        if self_closing {
            stack.last_mut().unwrap().children.push(node);
        } else {
            // `stack[0]` is `#root`, so `stack.len()` is the open depth.
            if stack.len() > MAX_DEPTH {
                return Err(format!(
                    "the body nests too deeply (more than {MAX_DEPTH} levels)"
                ));
            }
            stack.push(node);
        }
    }
    if stack.len() > 1 {
        let open: Vec<String> = stack[1..].iter().map(|n| format!("<{}>", n.name)).collect();
        return Err(format!("unclosed {}", open.join(" ")));
    }
    Ok(stack.pop().unwrap())
}

/// Index of the `>` ending the tag at the start of `s`, skipping quoted values.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote = None;
    for (n, c) in s.char_indices().skip(1) {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => return Some(n),
            // A raw `<` before the tag closed: the tag never ends.
            (None, '<') => return None,
            _ => {}
        }
    }
    None
}

fn parse_attrs(mut s: &str, tag: &str) -> Result<Vec<(String, String)>, String> {
    let mut attrs: Vec<(String, String)> = Vec::new();
    loop {
        let trimmed = s.trim_start();
        if trimmed.is_empty() {
            return Ok(attrs);
        }
        if trimmed.len() == s.len() {
            return Err(format!("attributes of <{tag}> must be separated by spaces"));
        }
        s = trimmed;
        let eq = s
            .find('=')
            .ok_or_else(|| format!("attribute without a value in <{tag}>"))?;
        let name = s[..eq].trim_end();
        if !name.starts_with(is_name_start) || !name.chars().all(is_name_char) {
            return Err(format!("bad attribute name `{name}` in <{tag}>"));
        }
        let after = s[eq + 1..].trim_start();
        let q = after
            .chars()
            .next()
            .filter(|c| *c == '"' || *c == '\'')
            .ok_or_else(|| format!("unquoted value for {name} in <{tag}>"))?;
        let end = after[1..]
            .find(q)
            .ok_or_else(|| format!("unterminated value for {name} in <{tag}>"))?;
        let raw = &after[1..1 + end];
        if raw.contains('<') {
            return Err(format!(
                "a raw `<` in the value of {name} in <{tag}> (escape it as &lt;)"
            ));
        }
        let value = decode(raw)?;
        if attrs.iter().any(|(k, _)| k == name) {
            return Err(format!("duplicate attribute {name} in <{tag}>"));
        }
        attrs.push((name.to_string(), value));
        s = &after[1 + end + 1..];
    }
}

/// `Ok` when `xhtml` is a well-formed XML fragment.
pub fn check_well_formed(xhtml: &str) -> Result<(), String> {
    parse(xhtml).map(|_| ())
}

const INLINE: &[&str] = &[
    "strong", "em", "s", "code", "a", "br", "u", "sub", "sup", "span",
];
const BLOCK: &[&str] = &[
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "table",
    "blockquote",
    "hr",
    "ac:structured-macro",
    "ac:task-list",
];
const STRUCTURAL: &[&str] = &[
    "li",
    "tbody",
    "thead",
    "tr",
    "th",
    "td",
    "colgroup",
    "col",
    "ac:parameter",
    "ac:plain-text-body",
    "ac:rich-text-body",
    "ac:task",
    "ac:task-id",
    "ac:task-status",
    "ac:task-body",
];
const MACROS: &[&str] = &["code", "info", "tip", "note", "warning"];

/// Well-formed AND only the elements / nesting Otto's converter emits.
pub fn validate_storage(xhtml: &str) -> Result<(), String> {
    let root = parse(xhtml)?;
    for c in root.elements() {
        rules(c, "#root", "body")?;
    }
    Ok(())
}

/// Allowed element children of a container (None = any allowed element).
fn only_children(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "ul" | "ol" => &["li"],
        "table" => &["tbody", "thead", "colgroup"],
        "tbody" | "thead" => &["tr"],
        "colgroup" => &["col"],
        "tr" => &["th", "td"],
        "ac:task-list" => &["ac:task"],
        "ac:task" => &["ac:task-id", "ac:task-status", "ac:task-body"],
        "ac:structured-macro" => &["ac:parameter", "ac:plain-text-body", "ac:rich-text-body"],
        _ => return None,
    })
}

fn rules(n: &Node, parent: &str, path: &str) -> Result<(), String> {
    let name = n.name.as_str();
    let here = format!("{path} > {name}");
    if !(INLINE.contains(&name) || BLOCK.contains(&name) || STRUCTURAL.contains(&name)) {
        return Err(format!("{here}: element <{name}> is not allowed"));
    }
    let parent_inline =
        INLINE.contains(&parent) || matches!(parent, "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
    if parent_inline && BLOCK.contains(&name) {
        return Err(format!("{here}: block <{name}> inside <{parent}>"));
    }
    if STRUCTURAL.contains(&name) && only_children(parent).is_none_or(|ok| !ok.contains(&name)) {
        return Err(format!("{here}: <{name}> must not be inside <{parent}>"));
    }
    if let Some(ok) = only_children(name) {
        for c in &n.children {
            if c.name == "#text" && !c.text.trim().is_empty() && name != "ac:structured-macro" {
                return Err(format!("{here}: text directly inside <{name}>"));
            }
            if c.name != "#text" && !ok.contains(&c.name.as_str()) {
                return Err(format!(
                    "{here}: <{}> is not allowed inside <{name}>",
                    c.name
                ));
            }
        }
    }
    match name {
        "br" | "hr" | "col" if !n.children.is_empty() => {
            return Err(format!("{here}: <{name}> must be empty (<{name} />)"));
        }
        "a" if n.attr("href").is_none_or(str::is_empty) => {
            return Err(format!("{here}: a link needs href"));
        }
        "ac:structured-macro" => {
            let m = n.attr("ac:name").unwrap_or("");
            if !MACROS.contains(&m) {
                return Err(format!("{here}: unknown macro \"{m}\""));
            }
            let kids: Vec<&str> = n.elements().map(|c| c.name.as_str()).collect();
            let body = if m == "code" {
                "ac:plain-text-body"
            } else {
                "ac:rich-text-body"
            };
            if kids.iter().filter(|k| **k == body).count() != 1 {
                return Err(format!("{here}: the {m} macro needs exactly one <{body}>"));
            }
        }
        "ac:task" => {
            let status = n
                .elements()
                .find(|c| c.name == "ac:task-status")
                .map(text_of)
                .unwrap_or_default();
            if status != "complete" && status != "incomplete" {
                return Err(format!(
                    "{here}: task status must be complete or incomplete, not \"{status}\""
                ));
            }
        }
        _ => {}
    }
    for c in n.elements() {
        rules(c, name, &here)?;
    }
    Ok(())
}

/// The text inside `n` (its own name is not filtered, unlike [`collect_text`]).
fn text_of(n: &Node) -> String {
    let mut s = String::new();
    for c in &n.children {
        collect_text(c, &mut s);
    }
    s.trim().to_string()
}

fn collect_text(n: &Node, out: &mut String) {
    if n.name == "#text" {
        out.push_str(&n.text);
        return;
    }
    // Macro parameters (code language, …) are settings, not content.
    if n.name == "ac:parameter" || n.name == "ac:task-id" || n.name == "ac:task-status" {
        return;
    }
    for c in &n.children {
        collect_text(c, out);
    }
    if !INLINE.contains(&n.name.as_str()) || n.name == "br" {
        out.push(' ');
    }
}

/// The block skeleton of a storage body in document order: `heading:2`,
/// `bulletList[3]`, `orderedList[2]`, `taskList[2]`, `table[3x4,header]`,
/// `code:rust`, `panel:info`, `blockquote`, `rule`. Paragraph wrappers and
/// Confluence-added attrs are ignored. Unparseable input → `["unparseable"]`.
pub fn storage_outline(xhtml: &str) -> Vec<String> {
    let Ok(root) = parse(xhtml) else {
        return vec!["unparseable".into()];
    };
    let mut out = Vec::new();
    for c in root.elements() {
        outline(c, &mut out);
    }
    out
}

fn outline(n: &Node, out: &mut Vec<String>) {
    let name = n.name.as_str();
    let count = |child: &str| n.elements().filter(|c| c.name == child).count();
    match name {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => out.push(format!("heading:{}", &name[1..])),
        "ul" => out.push(format!("bulletList[{}]", count("li"))),
        "ol" => out.push(format!("orderedList[{}]", count("li"))),
        "ac:task-list" => out.push(format!("taskList[{}]", count("ac:task"))),
        "blockquote" => out.push("blockquote".into()),
        "hr" => out.push("rule".into()),
        "table" => {
            let rows: Vec<&Node> = n
                .elements()
                .flat_map(|c| {
                    if c.name == "tr" {
                        vec![c]
                    } else {
                        c.elements().filter(|r| r.name == "tr").collect()
                    }
                })
                .collect();
            let first: Vec<&Node> = rows.first().map_or(Vec::new(), |r| r.elements().collect());
            let header = !first.is_empty() && first.iter().all(|c| c.name == "th");
            out.push(format!(
                "table[{}x{}{}]",
                rows.len(),
                first.len(),
                if header { ",header" } else { "" }
            ));
        }
        "ac:structured-macro" => {
            let m = n.attr("ac:name").unwrap_or("");
            match m {
                "code" => {
                    let lang = n
                        .elements()
                        .find(|c| c.name == "ac:parameter" && c.attr("ac:name") == Some("language"))
                        .map(text_of)
                        .unwrap_or_default();
                    out.push(format!("code:{}", lang.to_ascii_lowercase()));
                }
                "info" | "tip" | "note" | "warning" => out.push(format!("panel:{m}")),
                other => out.push(format!("macro:{other}")),
            }
        }
        _ => {}
    }
    for c in n.elements() {
        outline(c, out);
    }
}

/// All text (CDATA included, macro parameters excluded), entity-decoded and
/// whitespace-collapsed. Unparseable input → empty.
pub fn storage_plain_text(xhtml: &str) -> String {
    let Ok(root) = parse(xhtml) else {
        return String::new();
    };
    let mut s = String::new();
    collect_text(&root, &mut s);
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_formedness_errors_name_the_problem() {
        for (bad, needle) in [
            ("<p>a<br>b</p>", "`</p>` closes `<br>`"),
            ("<p>open", "unclosed <p>"),
            ("</p>", "closes nothing"),
            ("<p>a & b</p>", "bare `&`"),
            ("<p>&bogus</p>", "bare `&`"),
            ("<a href=x>t</a>", "unquoted value"),
            ("<a href=\"x\" href=\"y\">t</a>", "duplicate attribute"),
            ("<p>x<![CDATA[y</p>", "unterminated CDATA"),
            ("<p>]]></p>", "stray `]]>`"),
            ("<p <b>", "unterminated tag"),
            ("<!DOCTYPE html>", "DOCTYPE"),
            ("<1p/>", "bad tag name"),
            // Used to pop the synthetic root and panic on the empty stack.
            ("</#root>", "closes nothing"),
            ("<p>x</p></#root>", "closes nothing"),
        ] {
            let err = check_well_formed(bad).expect_err(bad);
            assert!(err.contains(needle), "{bad}: want {needle:?}, got {err:?}");
        }
        assert_eq!(
            check_well_formed("<p>a &amp; b &#169; &#x1F680; &nbsp;<br /></p><!-- c -->"),
            Ok(())
        );
        // Caller-supplied storage may use any macro; only well-formedness applies.
        assert_eq!(
            check_well_formed("<ac:structured-macro ac:name=\"toc\" ac:schema-version=\"1\" />"),
            Ok(())
        );
    }

    #[test]
    fn deep_nesting_is_refused_not_a_stack_overflow() {
        // 10k levels: the old parser built a 10k-deep tree whose drop glue and
        // `outline` / `collect_text` walks recursed once per level.
        let deep = format!("{}x{}", "<b>".repeat(10_000), "</b>".repeat(10_000));
        let err = check_well_formed(&deep).expect_err("deep");
        assert!(err.contains("nests too deeply"), "{err}");
        assert_eq!(storage_outline(&deep), vec!["unparseable".to_string()]);
        assert_eq!(storage_plain_text(&deep), "");
        // At the bound is still fine.
        let ok = format!("{}x{}", "<b>".repeat(MAX_DEPTH), "</b>".repeat(MAX_DEPTH));
        assert_eq!(check_well_formed(&ok), Ok(()));
        let over = format!(
            "{}x{}",
            "<b>".repeat(MAX_DEPTH + 1),
            "</b>".repeat(MAX_DEPTH + 1)
        );
        assert!(check_well_formed(&over).is_err());
    }

    #[test]
    fn element_rules_reject_what_confluence_would_mangle() {
        for (bad, needle) in [
            ("<p><ul><li>x</li></ul></p>", "block <ul> inside <p>"),
            ("<strong><p>x</p></strong>", "block <p> inside <strong>"),
            ("<li>x</li>", "<li> must not be inside <#root>"),
            ("<ul><p>x</p></ul>", "<p> is not allowed inside <ul>"),
            ("<ul>text</ul>", "text directly inside <ul>"),
            ("<table><tr><td>x</td></tr></table>", "<tr> is not allowed inside <table>"),
            ("<table><tbody><td>x</td></tbody></table>", "<td> is not allowed inside <tbody>"),
            ("<p>a<br>b</br></p>", "must be empty"),
            ("<div>x</div>", "<div> is not allowed"),
            ("<a>x</a>", "needs href"),
            ("<ac:structured-macro ac:name=\"toc\" />", "unknown macro \"toc\""),
            ("<ac:structured-macro ac:name=\"code\"></ac:structured-macro>", "exactly one <ac:plain-text-body>"),
            (
                "<ac:task-list><ac:task><ac:task-status>done</ac:task-status><ac:task-body>x</ac:task-body></ac:task></ac:task-list>",
                "complete or incomplete",
            ),
        ] {
            let err = validate_storage(bad).expect_err(bad);
            assert!(err.contains(needle), "{bad}: want {needle:?}, got {err:?}");
        }
    }

    #[test]
    fn outline_and_text_ignore_confluence_added_attrs() {
        let sent = "<h2>T</h2><table><tbody><tr><th><p>a</p></th><th><p>b</p></th></tr><tr><td><p>1</p></td><td><p /></td></tr></tbody></table>\
            <ac:structured-macro ac:name=\"code\"><ac:parameter ac:name=\"language\">rust</ac:parameter><ac:plain-text-body><![CDATA[fn x() {}]]></ac:plain-text-body></ac:structured-macro>";
        let stored = "<h2>T</h2><table data-layout=\"default\" ac:local-id=\"q\"><colgroup><col style=\"width: 50%\" /><col /></colgroup><tbody><tr><th><p>a</p></th><th><p>b</p></th></tr><tr><td><p>1</p></td><td><p /></td></tr></tbody></table>\
            <ac:structured-macro ac:name=\"code\" ac:schema-version=\"1\" ac:macro-id=\"m1\"><ac:parameter ac:name=\"language\">rust</ac:parameter><ac:plain-text-body><![CDATA[fn x() {}]]></ac:plain-text-body></ac:structured-macro>";
        assert_eq!(
            storage_outline(sent),
            ["heading:2", "table[2x2,header]", "code:rust"]
        );
        assert_eq!(storage_outline(sent), storage_outline(stored));
        assert_eq!(storage_plain_text(sent), "T a b 1 fn x() {}");
        assert_eq!(storage_plain_text(sent), storage_plain_text(stored));
        assert_eq!(storage_outline("<p>"), ["unparseable"]);
    }
}
