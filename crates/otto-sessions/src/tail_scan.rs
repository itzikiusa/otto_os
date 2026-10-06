//! Rolling-tail needle scan over live PTY output (perf 01 F6).
//!
//! The output scanners (mid-session re-auth detection in otto-server's
//! `AuthScanner`, the trust-prompt [`crate::prompt_guard::PromptGuard`]) look
//! for short lowercase ASCII phrases in a session's output stream. They used
//! to lowercase each chunk (up to 8 KiB) into a `String`, append it to the
//! session's tail, trim that to a few hundred bytes and only THEN search — so
//! a phrase early in a large chunk was trimmed away before anyone looked
//! (a missed re-auth or trust prompt), and the work on the dropped bytes was
//! wasted. [`scan_chunk`] searches the previous tail + the WHOLE chunk, then
//! keeps only enough bytes to catch a phrase that straddles into the next one.

/// Search `tail ++ chunk` (ASCII case-insensitively) for the first of
/// `needles` present; then leave in `tail` the last `keep` bytes of that
/// window (at least `longest needle - 1`, so a phrase split across chunks is
/// still found). `needles` must be lowercase. Works on raw bytes: no UTF-8
/// decoding, and a tail cut can never split a code point into a panic.
pub fn scan_chunk<'n>(
    tail: &mut Vec<u8>,
    chunk: &[u8],
    needles: &[&'n str],
    keep: usize,
) -> Option<&'n str> {
    debug_assert!(needles
        .iter()
        .all(|n| !n.bytes().any(|b| b.is_ascii_uppercase())));
    let longest = needles.iter().map(|n| n.len()).max().unwrap_or(0);
    let keep = keep.max(longest.saturating_sub(1));
    // The tail is never grown by a whole chunk (an 8 KiB chunk used to pin an
    // 8 KiB+ allocation per scanner per session, since `drain` keeps
    // capacity): the window is searched as (a) the old tail plus just enough
    // of the chunk's head to catch a straddling phrase and (b) the chunk
    // itself, in place. The tail then stays within `2 × keep` bytes.
    let bridge = chunk.len().min(longest.saturating_sub(1));
    tail.extend(chunk[..bridge].iter().map(u8::to_ascii_lowercase));
    let hit = needles
        .iter()
        .copied()
        .find(|n| contains(tail, n.as_bytes()) || contains_ci(chunk, n.as_bytes()));
    if chunk.len() >= keep {
        tail.clear();
        tail.extend(
            chunk[chunk.len() - keep..]
                .iter()
                .map(u8::to_ascii_lowercase),
        );
    } else {
        tail.extend(chunk[bridge..].iter().map(u8::to_ascii_lowercase));
        if tail.len() > keep {
            tail.drain(..tail.len() - keep);
        }
    }
    hit
}

/// `hay` (already lowercase) contains `needle` (lowercase).
fn contains(hay: &[u8], needle: &[u8]) -> bool {
    contains_by(hay, needle, |b| b)
}

/// `hay` (any case) contains `needle` (lowercase), ASCII case-insensitively.
fn contains_ci(hay: &[u8], needle: &[u8]) -> bool {
    contains_by(hay, needle, |b| b.to_ascii_lowercase())
}

fn contains_by(hay: &[u8], needle: &[u8], fold: impl Fn(u8) -> u8) -> bool {
    let Some((&first, rest)) = needle.split_first() else {
        return true;
    };
    if needle.len() > hay.len() {
        return false;
    }
    let last_start = hay.len() - needle.len();
    let mut i = 0;
    while i <= last_start {
        match hay[i..=last_start].iter().position(|&b| fold(b) == first) {
            Some(p) => {
                let at = i + p;
                if hay[at + 1..at + needle.len()]
                    .iter()
                    .zip(rest)
                    .all(|(&h, &n)| fold(h) == n)
                {
                    return true;
                }
                i = at + 1;
            }
            None => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A large chunk must not leave the tail holding a chunk-sized buffer.
    #[test]
    fn tail_capacity_shrinks_back_after_a_large_chunk() {
        let mut tail = Vec::new();
        let chunk = vec![b'x'; 64 * 1024];
        scan_chunk(&mut tail, &chunk, &["please log in"], 256);
        assert_eq!(tail.len(), 256);
        assert!(tail.capacity() <= 1024, "capacity {}", tail.capacity());
    }

    /// The old scanners trimmed the tail BEFORE searching: a phrase at the
    /// start of an 8 KiB chunk was cut off and never seen.
    #[test]
    fn a_needle_early_in_a_large_chunk_is_found() {
        let mut chunk = b"Error: Please Log In to continue\n".to_vec();
        chunk.extend(std::iter::repeat_n(b'x', 8000));
        let mut tail = Vec::new();
        assert_eq!(
            scan_chunk(&mut tail, &chunk, &["please log in"], 256),
            Some("please log in")
        );
        assert_eq!(tail.len(), 256, "only the tail is kept");
    }

    /// The pre-fix algorithm, for the record: lowercase + append + trim to
    /// the cap, then search — it misses the same input.
    #[test]
    fn the_old_trim_then_search_order_missed_it() {
        let mut chunk = b"please log in\n".to_vec();
        chunk.extend(std::iter::repeat_n(b'x', 8000));
        let mut buf = String::from_utf8_lossy(&chunk).to_lowercase();
        let cut = buf.len() - 256;
        buf.replace_range(..cut, "");
        assert!(!buf.contains("please log in"));
    }

    #[test]
    fn a_needle_split_across_chunks_is_found() {
        let mut tail = Vec::new();
        let needles = ["do you trust the files in this folder"];
        assert_eq!(
            scan_chunk(&mut tail, b"....Do you trust the fi", &needles, 0),
            None
        );
        assert_eq!(tail.len(), 23, "short output is kept whole");
        assert_eq!(
            scan_chunk(&mut tail, b"les in this folder?\n", &needles, 0),
            Some(needles[0])
        );
    }

    #[test]
    fn multibyte_output_never_panics_and_tail_is_bounded() {
        let mut tail = Vec::new();
        let glyphs = "\u{e0b0}".repeat(3000);
        assert_eq!(
            scan_chunk(&mut tail, glyphs.as_bytes(), &["claude login"], 64),
            None
        );
        assert_eq!(tail.len(), 64);
        assert_eq!(
            scan_chunk(
                &mut tail,
                "run `CLAUDE LOGIN`".as_bytes(),
                &["claude login"],
                64
            ),
            Some("claude login")
        );
        assert!(!contains(b"abc", b"abcd"));
        assert!(contains(b"aab", b"ab"));
    }
}
