/// UTF-8-safe splitting: every byte appears in exactly one model input.
pub(super) fn split(mut text: &str, limit: usize) -> Vec<String> {
    assert!(limit >= 4);
    let mut result = Vec::new();
    while !text.is_empty() {
        let mut end = text.len().min(limit);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        result.push(text[..end].into());
        text = &text[end..];
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_unicode_or_tail_is_lost() {
        let text = "שלום, room!\n".repeat(5000);
        let chunks = split(&text, 24000);
        assert!(chunks.iter().all(|c| c.len() <= 24000));
        assert_eq!(chunks.concat(), text);
        assert!(split("", 24000).is_empty());
    }
}
