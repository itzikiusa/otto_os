//! Per-provider "new folder access" robustness tests.
//!
//! Each test creates a FRESH (untrusted) temp dir, launches a provider CLI
//! there in a real PTY, and drives the same `prompt_guard::detect_approval`
//! logic the daemon uses to auto-accept the "trust this folder?" dialog — then
//! asks the agent to print a sentinel and asserts it gets through.
//!
//! These spawn real `claude` / `codex` / `agy` binaries (network + auth), so
//! they are `#[ignore]`d and never run in normal `cargo test`. Run them in an
//! environment where the CLIs are installed and logged in:
//!
//!     cargo test -p otto-sessions --test folder_trust_overcome -- --ignored --nocapture
//!
//! They double as the spike that validates/refines the `detect_approval`
//! needles + keystrokes for each CLI: run with `--nocapture` and read the
//! captured screen on failure to see the real prompt wording.
//!
//! SPIKE FINDING (2026-06-13): `claude` launched with `--dangerously-skip-
//! permissions` (the provider's default args) does NOT show a "trust this
//! folder?" dialog in a fresh dir — it goes straight into its TUI. So the
//! trust-overcome path mainly matters for codex/agy, or for claude when that
//! flag is absent; the `PromptGuard` needle for claude is a harmless backstop.
//! The claude run currently times out on the REQUEST round-trip (not a trust
//! prompt): the simple inject below needs the daemon bridge's dispatch-retry
//! pacing (`submit_to_agent`) to reliably land a prompt. Refine before relying
//! on these as green; they are provided as a runnable harness, not a gate.

use std::time::{Duration, Instant};

use otto_pty::PtyHandle;
use otto_sessions::prompt_guard::detect_approval;
use otto_sessions::ProviderRegistry;

/// Valid UUID-v4 shape for claude's `--session-id` flag (value is irrelevant).
const SID: &str = "11111111-1111-4111-8111-111111111111";
/// Distinctive sentinel the agent is asked to echo once it's past the prompt.
const SENTINEL: &str = "OTTO-TRUST-OK-7F3A";
const OVERALL_TIMEOUT: Duration = Duration::from_secs(90);
const POLL: Duration = Duration::from_millis(500);

/// Drive one provider end-to-end in a fresh temp dir. Returns Ok(()) when the
/// sentinel appears (the agent got past any trust prompt and answered), Err
/// with the last screen otherwise.
fn run_provider(provider: &str) -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("otto-trust-{provider}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir: {e}"))?;
    let cwd = dir.to_string_lossy().to_string();

    let registry = ProviderRegistry::new(None);
    let spec = registry
        .build_spec(provider, SID, &cwd, false)
        .map_err(|e| format!("build_spec: {e}"))?;
    let handle = PtyHandle::spawn(&spec).map_err(|e| format!("spawn: {e}"))?;

    let request = format!("Reply with exactly this token and nothing else: {SENTINEL}");
    let mut injected = false;
    let mut last_input_at = Instant::now();
    let deadline = Instant::now() + OVERALL_TIMEOUT;

    loop {
        std::thread::sleep(POLL);
        let screen = String::from_utf8_lossy(&handle.scrollback(400)).to_string();
        let lower = screen.to_lowercase();

        // 1. Auto-accept any trust/approval prompt (the behaviour under test).
        if let Some(keys) = detect_approval(provider, &lower) {
            let _ = handle.write(keys);
            last_input_at = Instant::now();
        }

        // 2. Success: the agent echoed the sentinel → it's past the prompt.
        if screen.contains(SENTINEL) && injected {
            let _ = handle.kill();
            return Ok(());
        }

        // 3. Once the TUI has drawn and gone briefly quiet, inject the request
        //    once (bracketed paste + Enter, like the daemon does).
        if !injected
            && !screen.trim().is_empty()
            && handle.last_output_at().elapsed() >= Duration::from_millis(800)
            && last_input_at.elapsed() >= Duration::from_secs(1)
        {
            let mut paste = Vec::new();
            paste.extend_from_slice(b"\x1b[200~");
            paste.extend_from_slice(request.as_bytes());
            paste.extend_from_slice(b"\x1b[201~");
            let _ = handle.write(&paste);
            std::thread::sleep(Duration::from_millis(200));
            let _ = handle.write(b"\r");
            injected = true;
            last_input_at = Instant::now();
        }

        if Instant::now() >= deadline {
            let tail: String = screen.chars().rev().take(600).collect::<String>().chars().rev().collect();
            let _ = handle.kill();
            return Err(format!(
                "timed out before sentinel (injected={injected}). last screen tail:\n{tail}"
            ));
        }
    }
}

#[test]
#[ignore = "spawns real claude; run with --ignored in an env where claude is installed + logged in"]
fn claude_overcomes_new_folder_access() {
    run_provider("claude").unwrap();
}

#[test]
#[ignore = "spawns real codex; run with --ignored in an env where codex is installed + logged in"]
fn codex_overcomes_new_folder_access() {
    run_provider("codex").unwrap();
}

#[test]
#[ignore = "spawns real agy; run with --ignored in an env where agy is installed + logged in"]
fn agy_overcomes_new_folder_access() {
    run_provider("agy").unwrap();
}
