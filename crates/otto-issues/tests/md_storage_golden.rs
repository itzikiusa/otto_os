//! Golden tests: Markdown → Confluence storage XHTML for every construct the
//! page / comment write paths support. Each expected body is spelled out in
//! full, and every output also passes the storage validator.

use otto_issues::{markdown_to_storage, storage_outline, storage_plain_text, validate_storage};

fn st(md: &str) -> String {
    let out = markdown_to_storage(md).unwrap_or_else(|e| panic!("{md:?}: {e}"));
    assert_eq!(validate_storage(&out), Ok(()), "{out}");
    out
}

fn code(lang: &str, body: &str) -> String {
    let param = if lang.is_empty() {
        String::new()
    } else {
        format!("<ac:parameter ac:name=\"language\">{lang}</ac:parameter>")
    };
    format!(
        "<ac:structured-macro ac:name=\"code\">{param}<ac:plain-text-body><![CDATA[{body}]]></ac:plain-text-body></ac:structured-macro>"
    )
}

#[test]
fn empty_input_is_an_empty_body() {
    assert_eq!(st(""), "");
    assert_eq!(st("\n \n"), "");
}

#[test]
fn headings_paragraphs_and_hard_breaks() {
    assert_eq!(
        st("# One\n###### Six\n\nSet\n---\n\nline one\nline two  \nthree\\\nfour<br>five"),
        "<h1>One</h1><h6>Six</h6><h2>Set</h2><p>line one<br />line two<br />three<br />four<br />five</p>"
    );
}

#[test]
fn inline_marks_links_and_escaping() {
    assert_eq!(
        st("**b** *i* ~~s~~ `c` [l](https://l.io) ***both*** a & b < c > d \"q\""),
        "<p><strong>b</strong> <em>i</em> <s>s</s> <code>c</code> <a href=\"https://l.io\">l</a> \
         <strong><em>both</em></strong> a &amp; b &lt; c &gt; d \"q\"</p>"
    );
    // Marks nest link-outermost; code keeps only a link; attributes escape quotes.
    assert_eq!(
        st("[**b**](https://b.io) [`run`](https://r.io?a=1&b=\"2\") **see `cfg`**"),
        "<p><a href=\"https://b.io\"><strong>b</strong></a> \
         <a href=\"https://r.io?a=1&amp;b=&quot;2&quot;\"><code>run</code></a> \
         <strong>see </strong><code>cfg</code></p>"
    );
    // Images have no upload path: a link to the image.
    assert_eq!(
        st("![shot](https://i.io/s.png) <https://y.io>"),
        "<p><a href=\"https://i.io/s.png\">shot</a> <a href=\"https://y.io\">https://y.io</a></p>"
    );
}

#[test]
fn nested_ordered_and_loose_lists_keep_text_inline_in_li() {
    assert_eq!(
        st("- a\n- b **bold**\n  - b1\n    1. deep\n- c"),
        "<ul><li>a</li><li>b <strong>bold</strong><ul><li>b1<ol><li>deep</li></ol></li></ul></li><li>c</li></ul>"
    );
    assert_eq!(
        st("3. three\n4. four\n   continued\n\n   second para"),
        "<ol start=\"3\"><li>three</li><li>four<br />continued<p>second para</p></li></ol>"
    );
}

#[test]
fn task_lists_are_native_and_nest() {
    assert_eq!(
        st("- [ ] todo **now**\n- [x] done\n  - [ ] sub"),
        "<ac:task-list>\
         <ac:task><ac:task-id>1</ac:task-id><ac:task-status>incomplete</ac:task-status><ac:task-body>todo <strong>now</strong></ac:task-body></ac:task>\
         <ac:task><ac:task-id>2</ac:task-id><ac:task-status>complete</ac:task-status><ac:task-body>done\
         <ac:task-list><ac:task><ac:task-id>3</ac:task-id><ac:task-status>incomplete</ac:task-status><ac:task-body>sub</ac:task-body></ac:task></ac:task-list>\
         </ac:task-body></ac:task>\
         </ac:task-list>"
    );
    // A list mixing tasks and bullets stays a list, with ☐ / ☑ prefixes.
    assert_eq!(
        st("- [x] shipped\n- plain"),
        "<ul><li>☑ shipped</li><li>plain</li></ul>"
    );
}

#[test]
fn blockquotes_and_callout_panels() {
    assert_eq!(
        st("> quoted *text*\n> # heading\n> - item"),
        "<blockquote><p>quoted <em>text</em></p><h1>heading</h1><ul><li>item</li></ul></blockquote>"
    );
    for (marker, mac) in [
        ("NOTE", "info"),
        ("INFO", "info"),
        ("TIP", "tip"),
        ("IMPORTANT", "note"),
        ("WARNING", "warning"),
        ("CAUTION", "warning"),
    ] {
        assert_eq!(
            st(&format!("> [!{marker}] Heads up\n> Be **careful**\n> ```sh\n> rm -rf x\n> ```")),
            format!(
                "<ac:structured-macro ac:name=\"{mac}\"><ac:rich-text-body><p>Heads up<br />Be <strong>careful</strong></p>{}</ac:rich-text-body></ac:structured-macro>",
                code("sh", "rm -rf x")
            ),
            "{marker}"
        );
    }
}

#[test]
fn code_macros_with_language_and_cdata_terminators() {
    assert_eq!(
        st("```Rust\nfn main() {}\n```"),
        code("rust", "fn main() {}")
    );
    assert_eq!(st("```\n```"), code("", ""));
    // Markup inside code is CDATA, not escaped text.
    assert_eq!(
        st("```html\n<p>&amp; \"x\"</p>\n```"),
        code("html", "<p>&amp; \"x\"</p>")
    );
    // A `]]>` inside the code would end the CDATA early: it is split.
    let out = st("```js\nconst a = b[c[0]]>1;\nx]]>]]>y\n```");
    assert_eq!(
        out,
        code(
            "js",
            "const a = b[c[0]]]]><![CDATA[>1;\nx]]]]><![CDATA[>]]]]><![CDATA[>y"
        )
    );
    assert_eq!(storage_plain_text(&out), "const a = b[c[0]]>1; x]]>]]>y");
    assert_eq!(storage_outline(&out), ["code:js"]);
}

#[test]
fn rules() {
    assert_eq!(st("a\n\n---\n\n***\nb"), "<p>a</p><hr /><hr /><p>b</p>");
}

#[test]
fn a_simple_table_has_a_th_header_row() {
    assert_eq!(
        st("| A | B |\n|---|---|\n| 1 | 2 |"),
        "<table><tbody><tr><th><p>A</p></th><th><p>B</p></th></tr><tr><td><p>1</p></td><td><p>2</p></td></tr></tbody></table>"
    );
}

#[test]
fn complex_tables_marks_links_code_empty_ragged_escaped() {
    let md = "\
| **Key** | Desc | Link |
|:--|:-:|--:|
| `a\\|b` and `c` | *em* ~~old~~ new | [site](https://s.io?x=1&y=2) |
|  | only \\| pipe & <x> |
| x | y | z | extra |
| line1<br>line2 | 🚀 |";
    let th = |c: &str| {
        if c.is_empty() {
            "<th><p /></th>".to_string()
        } else {
            format!("<th><p>{c}</p></th>")
        }
    };
    let td = |c: &str| {
        if c.is_empty() {
            "<td><p /></td>".to_string()
        } else {
            format!("<td><p>{c}</p></td>")
        }
    };
    let row = |cells: Vec<String>| format!("<tr>{}</tr>", cells.concat());
    let expected = format!(
        "<table><tbody>{}{}{}{}{}</tbody></table>",
        row(vec![
            th("<strong>Key</strong>"),
            th("Desc"),
            th("Link"),
            th("")
        ]),
        row(vec![
            td("<code>a|b</code> and <code>c</code>"),
            td("<em>em</em> <s>old</s> new"),
            td("<a href=\"https://s.io?x=1&amp;y=2\">site</a>"),
            td(""),
        ]),
        row(vec![
            td(""),
            td("only | pipe &amp; &lt;x&gt;"),
            td(""),
            td("")
        ]),
        row(vec![td("x"), td("y"), td("z"), td("extra")]),
        row(vec![td("line1<br />line2"), td("🚀"), td(""), td("")]),
    );
    let out = st(md);
    assert_eq!(out, expected);
    assert_eq!(storage_outline(&out), ["table[5x4,header]"]);
}

#[test]
fn tables_nest_inside_lists_and_quotes_in_storage() {
    // Storage (unlike ADF) holds a table inside a list item or a quote.
    assert_eq!(
        st("- item\n\n  | h |\n  |---|\n  | v |"),
        "<ul><li>item<table><tbody><tr><th><p>h</p></th></tr><tr><td><p>v</p></td></tr></tbody></table></li></ul>"
    );
}

#[test]
fn control_characters_and_emoji() {
    assert_eq!(
        st("red \u{1b}[31mtext\u{0} 🚀 :tada: @alice"),
        "<p>red [31mtext 🚀 :tada: @alice</p>"
    );
}

#[test]
fn a_realistic_test_case_document_has_the_expected_outline() {
    let md = "\
## TC-1 Login
Precondition: **SSO** user.

### Steps
1. Open https://app.example.com
2. Click *Sign in*
   - with SSO

> [!WARNING]
> NOT READY

| Env | Status | Notes |
|-----|--------|-------|
| prod | ❌ | see [log](https://log.io/1) |
| stg | ✅ | |

```json
{\"error\": \"invalid_grant\"}
```
---
- [ ] fix
- [x] triage";
    let out = st(md);
    assert_eq!(
        storage_outline(&out),
        [
            "heading:2",
            "heading:3",
            "orderedList[2]",
            "bulletList[1]",
            "panel:warning",
            "table[3x3,header]",
            "code:json",
            "rule",
            "taskList[2]"
        ]
    );
}

#[test]
fn pathological_input_stays_fast_and_valid() {
    let started = std::time::Instant::now();
    for md in [
        "*a ".repeat(2000),
        format!("{}bottom", "> ".repeat(3000)),
        "| a |\n|---|\n".to_string() + &"| x |\n".repeat(3000),
    ] {
        st(&md);
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}
