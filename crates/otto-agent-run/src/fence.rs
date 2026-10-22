//! Fenced-block fallback: an agent asked to edit a file sometimes prints the
//! result as a fenced code block instead (and the offline E2E stub always
//! does). Shared by every assist engine (canvas, Design Hall, product
//! mockups) so the rule is the same everywhere.

/// The trimmed contents of the first ```` ```<lang> ```` fenced block in `raw`,
/// or `None` when there is none or it is empty.
pub fn extract_fenced(raw: &str, lang: &str) -> Option<String> {
    let open = format!("```{lang}");
    let start = raw.find(&open)?;
    let after = &raw[start + open.len()..];
    let after = after.strip_prefix('\n').unwrap_or(after);
    let end = after.find("```")?;
    let body = after[..end].trim();
    (!body.is_empty()).then(|| body.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_block_of_the_language_trimmed() {
        let raw = "Built it.\n\n```html\n<!doctype html><body>hi</body>\n```\n```html\nx\n```";
        assert_eq!(
            extract_fenced(raw, "html").as_deref(),
            Some("<!doctype html><body>hi</body>")
        );
        assert_eq!(
            extract_fenced("```mermaid\nflowchart TD\n  A-->B\n```", "mermaid").as_deref(),
            Some("flowchart TD\n  A-->B")
        );
        assert!(extract_fenced("no fence", "html").is_none());
        assert!(
            extract_fenced("```json\n  \n```", "json").is_none(),
            "empty"
        );
        assert!(
            extract_fenced("```json\n{\"a\":1}", "json").is_none(),
            "unclosed"
        );
    }
}
