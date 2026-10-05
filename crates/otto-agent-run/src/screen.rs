//! Terminal-screen text helpers shared by the agent watchers.

/// Drop ESC/CSI/OSC sequences from a screen snapshot, keeping printable text.
pub fn strip_ansi(bytes: &[u8]) -> String {
    otto_core::text::strip_ansi_bytes(bytes)
        .chars()
        .filter_map(|c| match c {
            '\n' | '\r' | '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}
