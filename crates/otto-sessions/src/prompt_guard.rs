//! Auto-overcome interactive "trust this folder / approve?" prompts so no agent
//! session ever gets stuck waiting for a keystroke.
//!
//! This is the runtime *backstop*. The primary, deterministic mechanism is
//! [`crate::trust::ensure_trusted`], which writes each CLI's trust config
//! before spawn (called for every agent session in [`crate::manager`]). The
//! guard catches what pre-trust can't: providers without a known trust config
//! (e.g. `agy`), and unexpected first-run dialogs.
//!
//! It is an [`OutputScanner`]: it watches each session's PTY output, and when a
//! known approval prompt for the provider appears it writes the accepting
//! keystroke back into the PTY. Detection is intentionally narrow (specific
//! full phrases) so it never injects keys into an agent's real work on a false
//! positive, and it is time-debounced per session.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use otto_core::Id;

use crate::manager::{OutputScanner, SessionManager};

/// A known approval prompt: any `needle` (already lowercased) appearing in the
/// recent screen text means the agent is blocked, and `keys` accepts it.
struct Approval {
    needles: &'static [&'static str],
    keys: &'static [u8],
}

/// Carriage return — accepts the default (usually highlighted "Yes") option in
/// the select-style trust dialogs all three CLIs use.
const ENTER: &[u8] = b"\r";
/// Select the first option ("1. Yes, …") then confirm. Harmless if Enter alone
/// would have sufficed.
const SELECT_YES: &[u8] = b"1\r";

/// Per-provider approval table. Keystrokes are validated by the integration
/// tests in `otto-server/tests`; update here if a CLI changes its wording.
fn approvals_for(provider: &str) -> &'static [Approval] {
    match provider {
        "claude" => &[
            Approval {
                needles: &["do you trust the files in this folder"],
                keys: SELECT_YES,
            },
            CONTINUE,
        ],
        "codex" => &[
            Approval {
                needles: &[
                    "do you trust the files in this folder",
                    "allow codex to work in this folder",
                    "trust this directory",
                ],
                keys: ENTER,
            },
            CONTINUE,
        ],
        "agy" => &[
            Approval {
                needles: &[
                    "do you trust the files in this folder",
                    "trust this folder",
                    "allow access to this folder",
                    "grant access to this directory",
                ],
                keys: ENTER,
            },
            CONTINUE,
        ],
        _ => &[],
    }
}

/// Shared "blocked, waiting for a keystroke" prompts that can stall an unattended
/// session regardless of provider. Kept deliberately narrow — these exact phrases
/// almost never appear in an agent's real streamed output, so accepting them with
/// Enter is safe. Anything NOT matched here is caught by the analysis
/// stuck-detector (idle → retry → notify), so no prompt hangs forever.
const CONTINUE: Approval = Approval {
    needles: &[
        "press enter to continue",
        "press any key to continue",
        "press enter to retry",
    ],
    keys: ENTER,
};

/// Pure: does `screen` (the recent PTY output, already lowercased) contain a
/// known approval prompt for `provider`? Returns the bytes that accept it.
///
/// Kept pure + free of I/O so it is unit-testable without a real CLI.
pub fn detect_approval(provider: &str, screen: &str) -> Option<&'static [u8]> {
    for approval in approvals_for(provider) {
        if approval.needles.iter().any(|n| screen.contains(n)) {
            return Some(approval.keys);
        }
    }
    None
}

/// Max retained tail bytes per session — long enough to hold a multi-line trust
/// dialog, short enough to stay cheap.
const TAIL_CAP: usize = 1024;
/// Don't re-approve the same session more than once per window (avoid spamming
/// keys if the prompt redraws while the CLI processes the first acceptance).
const DEBOUNCE: Duration = Duration::from_secs(5);

/// Trim `buf` in place to at most `cap` bytes, keeping the most-recent content and
/// NEVER splitting a UTF-8 code point. The tail holds lowercased PTY output, which
/// routinely contains multi-byte glyphs (Powerline prompt separators U+E0B0 ``,
/// emoji, box-drawing, CJK), so a naive `buf[buf.len() - cap..]` byte slice can
/// land mid-char and panic the scan worker. Advance the cut to the next char
/// boundary instead — we keep ≤ `cap` bytes, still well over the longest needle.
fn trim_tail(buf: &mut String, cap: usize) {
    if buf.len() <= cap {
        return;
    }
    let mut cut = buf.len() - cap;
    while cut < buf.len() && !buf.is_char_boundary(cut) {
        cut += 1;
    }
    buf.replace_range(..cut, "");
}

/// Runtime guard that auto-accepts known trust/approval prompts. Wire it into
/// the `SessionManager` (see [`crate::CompositeScanner`]) and call
/// [`PromptGuard::set_manager`] once the manager `Arc` exists.
pub struct PromptGuard {
    /// Set after construction (the manager owns the scanner, so this is a Weak
    /// to avoid a reference cycle).
    manager: OnceLock<Weak<SessionManager>>,
    tails: Mutex<HashMap<Id, String>>,
    last_approved: Mutex<HashMap<Id, Instant>>,
}

impl PromptGuard {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            manager: OnceLock::new(),
            tails: Mutex::new(HashMap::new()),
            last_approved: Mutex::new(HashMap::new()),
        })
    }

    /// Provide the manager handle used to write keystrokes back. Call once,
    /// after the `Arc<SessionManager>` is built.
    pub fn set_manager(&self, manager: Weak<SessionManager>) {
        let _ = self.manager.set(manager);
    }

    /// True if we accepted a prompt for `id` within [`DEBOUNCE`].
    fn recently_approved(&self, id: &Id) -> bool {
        let guard = lock(&self.last_approved);
        guard
            .get(id)
            .is_some_and(|t| t.elapsed() < DEBOUNCE)
    }

    fn mark_approved(&self, id: &Id) {
        lock(&self.last_approved).insert(id.clone(), Instant::now());
    }
}

impl OutputScanner for PromptGuard {
    fn on_output(&self, session_id: &Id, provider: &str, chunk: &[u8]) {
        // Cheap exit for providers we have no approvals for.
        if approvals_for(provider).is_empty() {
            return;
        }
        if self.recently_approved(session_id) {
            return;
        }

        // Append to the rolling tail (prompts can straddle chunk boundaries).
        let combined = {
            let mut tails = lock(&self.tails);
            let buf = tails.entry(session_id.clone()).or_default();
            buf.push_str(&String::from_utf8_lossy(chunk).to_lowercase());
            trim_tail(buf, TAIL_CAP);
            buf.clone()
        };

        let Some(keys) = detect_approval(provider, &combined) else {
            return;
        };

        // Debounce + clear the tail so the same dialog doesn't re-fire.
        self.mark_approved(session_id);
        lock(&self.tails).remove(session_id);

        let Some(weak) = self.manager.get() else {
            return;
        };
        let Some(manager) = weak.upgrade() else {
            return;
        };
        let id = session_id.clone();
        let provider = provider.to_string();
        tokio::spawn(async move {
            match manager.input(&id, keys).await {
                Ok(()) => {
                    tracing::info!(
                        session = %id,
                        provider = %provider,
                        "prompt-guard: auto-approved a trust/permission prompt"
                    );
                    manager.record_approval_trail(&id, &provider);
                }
                Err(e) => tracing::warn!(
                    session = %id,
                    provider = %provider,
                    "prompt-guard: could not send approval keys: {e}"
                ),
            }
        });
    }
}

/// Fans `on_output` out to several scanners (the `SessionManager` exposes a
/// single scanner slot). Use to run [`PromptGuard`] alongside other scanners.
pub struct CompositeScanner {
    scanners: Vec<Arc<dyn OutputScanner>>,
}

impl CompositeScanner {
    pub fn new(scanners: Vec<Arc<dyn OutputScanner>>) -> Arc<Self> {
        Arc::new(Self { scanners })
    }
}

impl OutputScanner for CompositeScanner {
    fn on_output(&self, session_id: &Id, provider: &str, chunk: &[u8]) {
        for s in &self.scanners {
            s.on_output(session_id, provider, chunk);
        }
    }
}

/// Lock helper that survives a poisoned mutex (a panicked holder shouldn't take
/// the whole guard down — worst case we miss/repeat one approval).
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_claude_trust_prompt_and_selects_yes() {
        let screen = "\n  Do you trust the files in this folder?\n  1. Yes, proceed\n  2. No, exit\n";
        assert_eq!(
            detect_approval("claude", &screen.to_lowercase()),
            Some(SELECT_YES)
        );
    }

    #[test]
    fn detects_codex_and_agy_folder_prompts() {
        assert_eq!(
            detect_approval("codex", "allow codex to work in this folder? (y/n)"),
            Some(ENTER)
        );
        assert_eq!(
            detect_approval("agy", "trust this folder to continue"),
            Some(ENTER)
        );
    }

    #[test]
    fn ignores_normal_output_and_unknown_providers() {
        assert_eq!(detect_approval("claude", "running the test suite now…"), None);
        assert_eq!(detect_approval("claude", "the folder structure looks fine"), None);
        // A provider with no approval table never matches.
        assert_eq!(detect_approval("shell", "do you trust the files in this folder"), None);
    }

    #[test]
    fn detects_shared_continue_prompts_per_provider() {
        for p in ["claude", "codex", "agy"] {
            assert_eq!(
                detect_approval(p, "press enter to continue"),
                Some(ENTER),
                "provider {p} should accept a 'press enter to continue' prompt"
            );
            assert_eq!(
                detect_approval(p, "  press any key to continue  "),
                Some(ENTER),
                "provider {p} should accept a 'press any key to continue' prompt"
            );
        }
        // Conservative: ordinary prose mentioning 'continue' must NOT match.
        assert_eq!(
            detect_approval("claude", "i will continue analyzing the codebase"),
            None
        );
    }

    #[test]
    fn composite_fans_out_to_each_scanner() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct Counter(Arc<AtomicUsize>);
        impl OutputScanner for Counter {
            fn on_output(&self, _id: &Id, _provider: &str, _chunk: &[u8]) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let n = Arc::new(AtomicUsize::new(0));
        let c = CompositeScanner::new(vec![
            Arc::new(Counter(Arc::clone(&n))),
            Arc::new(Counter(Arc::clone(&n))),
        ]);
        c.on_output(&"s1".to_string(), "claude", b"hi");
        assert_eq!(n.load(Ordering::SeqCst), 2);
    }

    /// Regression: a tail full of multi-byte glyphs must not panic when trimmed.
    /// The Powerline separator U+E0B0 is 3 bytes, so the byte cut point lands
    /// inside a code point — the old `buf[len-cap..]` slice panicked here.
    #[test]
    fn trim_tail_handles_multibyte_glyphs() {
        let glyph = '\u{e0b0}';
        let mut s: String = std::iter::repeat_n(glyph, TAIL_CAP).collect(); // 3×cap bytes
        trim_tail(&mut s, TAIL_CAP); // must not panic on a mid-char byte index
        assert!(s.len() <= TAIL_CAP);
        assert!(s.chars().all(|c| c == glyph), "no split/garbled code points");
    }

    /// A multi-byte trust dialog drives the full scanner path without panicking
    /// and still matches once the prompt text arrives.
    #[test]
    fn on_output_survives_multibyte_output() {
        let guard = PromptGuard::new();
        let id = "s1".to_string();
        // 2 KB of Powerline glyphs (over TAIL_CAP) → forces a mid-char trim.
        let glyphs: String = std::iter::repeat_n('\u{e0b0}', 700).collect();
        guard.on_output(&id, "claude", glyphs.as_bytes()); // must not panic
        // The real prompt then arrives; with no manager wired the guard simply
        // returns, but it must process the tail without panicking.
        guard.on_output(&id, "claude", b"\n  Do you trust the files in this folder?\n  1. Yes, proceed\n");
    }
}
