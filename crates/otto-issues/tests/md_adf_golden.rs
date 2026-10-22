//! Golden tests: Markdown → ADF for every construct the Jira create /
//! comment paths support. Each expected document is spelled out in full, and
//! every output is also run through the structural validator.

use otto_issues::{markdown_to_adf, validate_adf};
use serde_json::{json, Value};

fn adf(md: &str) -> Value {
    let doc = markdown_to_adf(md).unwrap_or_else(|e| panic!("{md:?}: {e}"));
    assert_eq!(validate_adf(&doc), Ok(()), "{doc}");
    doc
}

/// The doc's top-level content.
fn body(md: &str) -> Value {
    adf(md)["content"].clone()
}

fn t(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

fn tm(text: &str, marks: Value) -> Value {
    json!({"type": "text", "text": text, "marks": marks})
}

fn p(content: Value) -> Value {
    json!({"type": "paragraph", "content": content})
}

fn li(content: Value) -> Value {
    json!({"type": "listItem", "content": content})
}

fn link(href: &str) -> Value {
    json!({"type": "link", "attrs": {"href": href}})
}

fn cell(kind: &str, content: Value) -> Value {
    let para = if content.as_array().is_some_and(Vec::is_empty) {
        json!({"type": "paragraph"})
    } else {
        p(content)
    };
    json!({"type": kind, "attrs": {}, "content": [para]})
}

fn table(rows: Vec<Vec<Value>>) -> Value {
    let rows: Vec<Value> = rows
        .into_iter()
        .enumerate()
        .map(|(n, cells)| {
            let kind = if n == 0 { "tableHeader" } else { "tableCell" };
            let cells: Vec<Value> = cells.into_iter().map(|c| cell(kind, c)).collect();
            json!({"type": "tableRow", "content": cells})
        })
        .collect();
    json!({"type": "table", "attrs": {"isNumberColumnEnabled": false, "layout": "default"}, "content": rows})
}

#[test]
fn empty_input_is_an_empty_doc() {
    assert_eq!(adf(""), json!({"type": "doc", "version": 1, "content": []}));
    assert_eq!(body("\n  \n"), json!([]));
}

#[test]
fn headings_h1_to_h6_atx_and_setext() {
    let md = "# One\n## Two ##\n### Three\n#### Four\n##### Five\n###### Six\n####### seven\n\nSet1\n===\n\nSet2\n---";
    let h = |level: u8, text: &str| json!({"type": "heading", "attrs": {"level": level}, "content": [t(text)]});
    assert_eq!(
        body(md),
        json!([
            h(1, "One"),
            h(2, "Two"),
            h(3, "Three"),
            h(4, "Four"),
            h(5, "Five"),
            h(6, "Six"),
            p(json!([t("####### seven")])),
            h(1, "Set1"),
            h(2, "Set2")
        ])
    );
}

#[test]
fn paragraphs_and_hard_breaks() {
    assert_eq!(
        body("line one\nline two  \nline three\\\nfour<br>five\n\nnext para"),
        json!([
            p(
                json!([t("line one"), {"type": "hardBreak"}, t("line two"), {"type": "hardBreak"},
                     t("line three"), {"type": "hardBreak"}, t("four"), {"type": "hardBreak"}, t("five")])
            ),
            p(json!([t("next para")]))
        ])
    );
}

#[test]
fn inline_marks_nest_and_code_keeps_only_link() {
    assert_eq!(
        body("**bold** *it* _it2_ __b2__ ~~gone~~ `x*y` ***both*** **bold *and it***"),
        json!([p(json!([
            tm("bold", json!([{"type": "strong"}])),
            t(" "),
            tm("it", json!([{"type": "em"}])),
            t(" "),
            tm("it2", json!([{"type": "em"}])),
            t(" "),
            tm("b2", json!([{"type": "strong"}])),
            t(" "),
            tm("gone", json!([{"type": "strike"}])),
            t(" "),
            tm("x*y", json!([{"type": "code"}])),
            t(" "),
            tm("both", json!([{"type": "strong"}, {"type": "em"}])),
            t(" "),
            tm("bold ", json!([{"type": "strong"}])),
            tm("and it", json!([{"type": "strong"}, {"type": "em"}]))
        ]))])
    );
    // `code` inside bold drops the bold (ADF: code combines only with link).
    assert_eq!(
        body("**see `cfg`**"),
        json!([p(json!([
            tm("see ", json!([{"type": "strong"}])),
            tm("cfg", json!([{"type": "code"}]))
        ]))])
    );
}

#[test]
fn literal_punctuation_stays_text() {
    assert_eq!(
        body("snake_case_name 2 * 3 * 4 \\*not em\\* a ~ b [1] ``"),
        json!([p(json!([t(
            "snake_case_name 2 * 3 * 4 *not em* a ~ b [1] ``"
        )]))])
    );
}

#[test]
fn links_autolinks_bare_urls_and_images() {
    assert_eq!(
        body("[docs](https://x.io/a \"title\") <https://y.io> see https://z.io/p(1). ![shot](https://i.io/s.png) [**b**](https://b.io)"),
        json!([p(json!([
            tm("docs", json!([link("https://x.io/a")])), t(" "),
            tm("https://y.io", json!([link("https://y.io")])), t(" see "),
            tm("https://z.io/p(1)", json!([link("https://z.io/p(1)")])), t(". "),
            tm("shot", json!([link("https://i.io/s.png")])), t(" "),
            tm("b", json!([{"type": "strong"}, link("https://b.io")]))
        ]))])
    );
    // A code span inside a link keeps the link.
    assert_eq!(
        body("[`run`](https://r.io)"),
        json!([p(json!([tm(
            "run",
            json!([{"type": "code"}, link("https://r.io")])
        )]))])
    );
}

#[test]
fn bullet_ordered_and_nested_lists() {
    let md = "- a\n- b **bold**\n  - b1\n    1. deep\n  - b2\n- c\n\n3. three\n4. four\n   continued\n\n   second para";
    assert_eq!(
        body(md),
        json!([
            {"type": "bulletList", "content": [
                li(json!([p(json!([t("a")]))])),
                li(json!([
                    p(json!([t("b "), tm("bold", json!([{"type": "strong"}]))])),
                    {"type": "bulletList", "content": [
                        li(json!([
                            p(json!([t("b1")])),
                            {"type": "orderedList", "content": [li(json!([p(json!([t("deep")]))]))]}
                        ])),
                        li(json!([p(json!([t("b2")]))]))
                    ]}
                ])),
                li(json!([p(json!([t("c")]))]))
            ]},
            {"type": "orderedList", "attrs": {"order": 3}, "content": [
                li(json!([p(json!([t("three")]))])),
                li(json!([p(json!([t("four"), {"type": "hardBreak"}, t("continued")])), p(json!([t("second para")]))]))
            ]}
        ])
    );
}

#[test]
fn a_loose_list_with_blank_lines_stays_one_list() {
    assert_eq!(
        body("1. one\n\n2. two\n\n* x\n+ y"),
        json!([
            {"type": "orderedList", "content": [li(json!([p(json!([t("one")]))])), li(json!([p(json!([t("two")]))]))]},
            {"type": "bulletList", "content": [li(json!([p(json!([t("x")]))])), li(json!([p(json!([t("y")]))]))]}
        ])
    );
}

#[test]
fn task_lists_top_level_nested_and_degraded() {
    assert_eq!(
        body("- [ ] todo **now**\n- [x] done\n  - [ ] sub"),
        json!([{"type": "taskList", "attrs": {"localId": "otto-tasks-1"}, "content": [
            {"type": "taskItem", "attrs": {"localId": "otto-task-2", "state": "TODO"},
             "content": [t("todo "), tm("now", json!([{"type": "strong"}]))]},
            {"type": "taskItem", "attrs": {"localId": "otto-task-5", "state": "DONE"}, "content": [t("done")]},
            {"type": "taskList", "attrs": {"localId": "otto-tasks-3"}, "content": [
                {"type": "taskItem", "attrs": {"localId": "otto-task-4", "state": "TODO"}, "content": [t("sub")]}
            ]}
        ]}])
    );
    // A mixed list (or one inside a quote) has no taskList: ☐ / ☑ prefixes.
    assert_eq!(
        body("- [x] shipped\n- plain"),
        json!([{"type": "bulletList", "content": [
            li(json!([p(json!([t("☑ "), t("shipped")]))])),
            li(json!([p(json!([t("plain")]))]))
        ]}])
    );
}

#[test]
fn blockquotes_and_callout_panels() {
    assert_eq!(
        body("> quoted *text*\n> - item\n>\n> # not a heading here"),
        json!([{"type": "blockquote", "content": [
            p(json!([t("quoted "), tm("text", json!([{"type": "em"}]))])),
            {"type": "bulletList", "content": [li(json!([p(json!([t("item")]))]))]},
            p(json!([tm("not a heading here", json!([{"type": "strong"}]))]))
        ]}])
    );
    for (marker, panel) in [
        ("NOTE", "info"),
        ("INFO", "info"),
        ("IMPORTANT", "note"),
        ("TIP", "success"),
        ("WARNING", "warning"),
        ("CAUTION", "error"),
    ] {
        assert_eq!(
            body(&format!("> [!{marker}]\n> Be **careful**\n> ## Sub")),
            json!([{"type": "panel", "attrs": {"panelType": panel}, "content": [
                p(json!([t("Be "), tm("careful", json!([{"type": "strong"}]))])),
                {"type": "heading", "attrs": {"level": 2}, "content": [t("Sub")]}
            ]}]),
            "{marker}"
        );
    }
    // Code inside a panel becomes monospace lines (panels hold no codeBlock).
    assert_eq!(
        body("> [!WARNING] Heads up\n> ```\n> a\n> b\n> ```"),
        json!([{"type": "panel", "attrs": {"panelType": "warning"}, "content": [
            p(json!([t("Heads up")])),
            p(json!([tm("a", json!([{"type": "code"}])), {"type": "hardBreak"}, tm("b", json!([{"type": "code"}]))]))
        ]}])
    );
}

#[test]
fn fenced_code_with_language_and_rules() {
    assert_eq!(
        body("```rust\nfn main() {\n    println!(\"*not em*\");\n}\n```\n---\n~~~\nplain\n~~~\n```\n```\n***"),
        json!([
            {"type": "codeBlock", "attrs": {"language": "rust"},
             "content": [t("fn main() {\n    println!(\"*not em*\");\n}")]},
            {"type": "rule"},
            {"type": "codeBlock", "content": [t("plain")]},
            {"type": "codeBlock"},
            {"type": "rule"}
        ])
    );
    // An unclosed fence runs to the end of the document.
    assert_eq!(
        body("```sh\necho hi"),
        json!([{"type": "codeBlock", "attrs": {"language": "sh"}, "content": [t("echo hi")]}])
    );
    // A code block in a list item stays a codeBlock.
    assert_eq!(
        body("- step\n  ```bash\n  make\n  ```"),
        json!([{"type": "bulletList", "content": [li(json!([
            p(json!([t("step")])),
            {"type": "codeBlock", "attrs": {"language": "bash"}, "content": [t("make")]}
        ]))]}])
    );
}

#[test]
fn a_simple_table_has_a_header_row() {
    assert_eq!(
        body("| Name | Value |\n|------|-------|\n| a | 1 |\n| b | 2 |"),
        json!([table(vec![
            vec![json!([t("Name")]), json!([t("Value")])],
            vec![json!([t("a")]), json!([t("1")])],
            vec![json!([t("b")]), json!([t("2")])],
        ])])
    );
}

#[test]
fn complex_tables_marks_links_code_empty_ragged_and_escaped_pipes() {
    let md = "\
Intro line
| **Key** | Desc | Link |
|:--|:-:|--:|
| `a\\|b` and `c` | *em* ~~old~~ new | [site](https://s.io) |
|  | only \\| pipe |
| x | y | z | extra |
after <br> table";
    let doc = body(md);
    assert_eq!(doc[0], p(json!([t("Intro line")])));
    assert_eq!(
        doc[1],
        table(vec![
            vec![
                json!([tm("Key", json!([{"type": "strong"}]))]),
                json!([t("Desc")]),
                json!([t("Link")]),
                json!([])
            ],
            vec![
                json!([
                    tm("a|b", json!([{"type": "code"}])),
                    t(" and "),
                    tm("c", json!([{"type": "code"}]))
                ]),
                json!([
                    tm("em", json!([{"type": "em"}])),
                    t(" "),
                    tm("old", json!([{"type": "strike"}])),
                    t(" new")
                ]),
                json!([tm("site", json!([link("https://s.io")]))]),
                json!([]),
            ],
            vec![json!([]), json!([t("only | pipe")]), json!([]), json!([])],
            vec![
                json!([t("x")]),
                json!([t("y")]),
                json!([t("z")]),
                json!([t("extra")])
            ],
        ]),
        "ragged rows are padded to the widest row (header included); alignment is ignored"
    );
    // A row line ends the table only at a line without a pipe.
    assert_eq!(
        doc[2],
        p(json!([t("after "), {"type": "hardBreak"}, t(" table")]))
    );
    assert_eq!(doc.as_array().unwrap().len(), 3);
}

#[test]
fn tables_without_outer_pipes_and_br_in_cells() {
    assert_eq!(
        body("a | b\n--- | ---\nline1<br>line2 | 2"),
        json!([table(vec![
            vec![json!([t("a")]), json!([t("b")])],
            vec![
                json!([t("line1"), {"type": "hardBreak"}, t("line2")]),
                json!([t("2")])
            ],
        ])])
    );
}

#[test]
fn tables_nested_where_adf_forbids_them_degrade_to_rows() {
    assert_eq!(
        body("- item\n\n  | h1 | h2 |\n  |----|----|\n  | 1 | 2 |"),
        json!([{"type": "bulletList", "content": [li(json!([
            p(json!([t("item")])),
            p(json!([tm("h1 | h2", json!([{"type": "strong"}]))])),
            p(json!([t("1 | 2")]))
        ]))]}])
    );
}

#[test]
fn emoji_and_mentions_pass_through_as_text() {
    assert_eq!(
        body("Ship it 🚀 :tada: @alice — done ✅"),
        json!([p(json!([t("Ship it 🚀 :tada: @alice — done ✅")]))])
    );
}

#[test]
fn a_realistic_issue_description_round_trips_through_the_validator() {
    let md = "\
## Problem
Login fails for **SSO** users after `v2.3`.

### Steps
1. Open https://app.example.com
2. Click *Sign in*
   - with SSO
   - [ ] not with password

> [!WARNING]
> Affects prod.

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
    let doc = adf(md);
    let kinds: Vec<&str> = doc["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "heading",
            "paragraph",
            "heading",
            "orderedList",
            "panel",
            "table",
            "codeBlock",
            "rule",
            "taskList"
        ]
    );
    assert_eq!(
        otto_issues::adf_outline(&doc)
            .iter()
            .find(|e| e.starts_with("table"))
            .unwrap(),
        "table[3x3,header]"
    );
}

#[test]
fn pathological_inline_input_stays_fast() {
    // Thousands of unmatched openers once re-scanned every suffix
    // (exponential); now each closer lookup is memoised.
    let started = std::time::Instant::now();
    for md in [
        "*a ".repeat(2000),
        "**a *b ".repeat(1000),
        "_x ~~y [z ".repeat(1000),
        format!("{}{}", "*".repeat(3000), "a"),
        format!("{}x{}", "*a ".repeat(5000), " a*".repeat(5000)),
        format!("{}deep", "[".repeat(3000)),
    ] {
        let doc = adf(&md);
        assert_eq!(doc["content"][0]["type"], "paragraph");
    }
    // Deep container nesting is cut off, not a stack overflow.
    let doc = adf(&format!("{}bottom", "> ".repeat(5000)));
    assert_eq!(doc["content"][0]["type"], "blockquote");
    let doc = adf(&(0..3000)
        .map(|n| format!("{}- l{n}\n", "  ".repeat(n)))
        .collect::<String>());
    assert_eq!(doc["content"][0]["type"], "bulletList");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "took {:?}",
        started.elapsed()
    );
}
