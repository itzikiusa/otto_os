//! One text-in/text-out Claude turn run HEADLESS (`claude -p`, stream-json)
//! — the commit and PR drafts. No TUI, so no start-up screen (an MCP approval
//! dialog, a login notice, onboarding) can swallow the prompt; no MCP
//! servers, tools, hooks, plugins or settings (`--strict-mcp-config`,
//! `--tools=`, `--setting-sources=`); no session file; and it runs in the
//! temp dir, so nothing is read from or written into the repository. The
//! prompt carries everything the model needs (the diff).

use std::process::Stdio;
use std::time::Duration;

use otto_core::{Error, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

/// A draft that has not answered by now is not coming: say so instead of
/// leaving the card spinning (the CLI answers a one-file draft in seconds).
pub const DRAFT_TIMEOUT: Duration = Duration::from_secs(30);

/// The argv after the program: print mode, streamed, nothing attached.
pub fn draft_args(model: &str) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--strict-mcp-config",
        "--tools=",
        "--setting-sources=",
        "--no-session-persistence",
        "--disable-slash-commands",
    ]
    .map(String::from)
    .into();
    if !model.trim().is_empty() {
        args.extend(["--model".to_string(), model.trim().to_string()]);
    }
    args
}

/// Draft with the `claude` provider as Settings › Providers configures it
/// (its command and env; a shell alias goes through the login shell, as
/// sessions do). `on_text` gets each streamed piece of the reply.
pub async fn claude_draft(
    providers: &otto_sessions::ProviderRegistry,
    model: &str,
    prompt: &str,
    on_text: impl FnMut(&str) + Send,
) -> Result<String> {
    let spec = providers.get("claude");
    let program = spec
        .as_ref()
        .map_or("claude", |s| s.cmd.as_str())
        .to_string();
    // `~` expanded as a session spawn does (`CLAUDE_CONFIG_DIR=~/.claude-work`).
    let home = otto_transcript::roots::home_dir();
    let env: Vec<(String, String)> = spec
        .as_ref()
        .map(|s| {
            s.env
                .iter()
                .map(|(k, v)| {
                    let v = (v == "~" || v.starts_with("~/"))
                        .then(|| otto_transcript::claude_config::expand_home_path(v, &home))
                        .flatten()
                        .map_or_else(|| v.clone(), |p| p.to_string_lossy().into_owned());
                    (k.clone(), v)
                })
                .collect()
        })
        .unwrap_or_default();
    let (program, args) = if otto_sessions::providers::resolves_on_path(&program) {
        (program, draft_args(model))
    } else {
        otto_sessions::providers::shell_wrapped(&program, draft_args(model))
    };
    text_turn(&program, &args, &env, prompt, DRAFT_TIMEOUT, on_text).await
}

/// Run `program args` as one stream-json turn: send `prompt` as the user
/// message, forward text deltas to `on_text`, return the reply. Dropping the
/// future kills the CLI (Stop).
pub async fn text_turn(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    prompt: &str,
    timeout: Duration,
    mut on_text: impl FnMut(&str) + Send,
) -> Result<String> {
    let mut child = tokio::process::Command::new(program)
        .args(args)
        .envs(env.iter().map(|(k, v)| (k, v)))
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| Error::Upstream(format!("couldn’t start the Claude CLI ({program}): {e}")))?;
    let (Some(mut stdin), Some(stdout), Some(mut stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        return Err(Error::Internal("the Claude CLI has no stdio".into()));
    };
    // Drained alongside stdout: a CLI blocked on a full stderr pipe would
    // never finish its stdout.
    let stderr = tokio::spawn(async move {
        let mut err = Vec::new();
        let _ = stderr.read_to_end(&mut err).await;
        String::from_utf8_lossy(&err).into_owned()
    });
    let turn = async {
        let user = json!({
            "type": "user",
            "session_id": "",
            "message": {"role": "user", "content": [{"type": "text", "text": prompt}]},
            "parent_tool_use_id": null,
        });
        // A CLI that died at start-up closes stdin; its stderr says why.
        let _ = stdin.write_all(format!("{user}\n").as_bytes()).await;
        let _ = stdin.flush().await;
        let mut streamed = String::new();
        // Bytes, not `lines()`: one non-UTF-8 byte must not end the read.
        let (mut out, mut buf) = (BufReader::new(stdout), Vec::new());
        while out.read_until(b'\n', &mut buf).await.is_ok_and(|n| n > 0) {
            let msg = serde_json::from_str::<Value>(&String::from_utf8_lossy(&buf));
            buf.clear();
            let Ok(msg) = msg else {
                continue;
            };
            match msg.get("type").and_then(Value::as_str) {
                Some("stream_event") => {
                    let delta = msg.pointer("/event/delta");
                    if delta.and_then(|d| d.get("type")).and_then(Value::as_str)
                        == Some("text_delta")
                    {
                        if let Some(t) = delta.and_then(|d| d.get("text")).and_then(Value::as_str) {
                            streamed.push_str(t);
                            on_text(t);
                        }
                    }
                }
                Some("result") => {
                    let text = msg
                        .get("result")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if msg.get("is_error").and_then(Value::as_bool) == Some(true) {
                        return Err(Error::Upstream(format!("Claude: {}", one_line(text))));
                    }
                    return Ok(if text.trim().is_empty() {
                        streamed
                    } else {
                        text.to_string()
                    });
                }
                _ => {}
            }
        }
        let err = stderr.await.unwrap_or_default();
        let why = err
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("it exited without a reply");
        Err(Error::Upstream(format!(
            "the Claude CLI stopped: {}",
            one_line(why)
        )))
    };
    match tokio::time::timeout(timeout, turn).await {
        Ok(r) => r,
        Err(_) => Err(Error::Upstream(format!(
            "Claude didn’t answer within {} s",
            timeout.as_secs()
        ))),
    }
}

fn one_line(s: &str) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    s.chars().take(240).collect()
}

/// An NDJSON body for a streamed draft: `{"delta": …}` per piece of text,
/// then the single line `run` returns (`{"done": …}` / `{"error": …}`).
/// Dropping the stream (the client went away, Stop) drops `run`, which
/// kills the CLI.
pub fn ndjson_lines<F, Fut>(run: F) -> impl futures_util::Stream<Item = String> + Send
where
    F: FnOnce(Box<dyn FnMut(&str) + Send>) -> Fut,
    Fut: std::future::Future<Output = Value> + Send + 'static,
{
    use futures_util::StreamExt;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let deltas = tx.clone();
    let turn = run(Box::new(move |t: &str| {
        let _ = deltas.send(format!("{}\n", json!({ "delta": t })));
    }));
    let last = async move {
        let line = turn.await;
        let _ = tx.send(format!("{line}\n"));
        None
    };
    let rx = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|l| (Some(l), rx))
    });
    futures_util::stream::select(rx, futures_util::stream::once(last))
        .filter_map(std::future::ready)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(dir: &std::path::Path, body: &str) -> String {
        let path = dir.join("claude");
        std::fs::write(&path, format!("#!/bin/sh\nread -r _prompt\n{body}\n")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[tokio::test]
    async fn streams_the_reply_and_returns_the_result() {
        let dir = tempfile::tempdir().unwrap();
        let p = fake(
            dir.path(),
            r#"printf '%s\n' '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"fix: a"}}}'
printf '%s\n' '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"\n\n- b"}}}'
printf '%s\n' '{"type":"result","is_error":false,"result":"fix: a\n\n- b"}'"#,
        );
        let mut got = Vec::new();
        let reply = text_turn(&p, &[], &[], "draft", DRAFT_TIMEOUT, |t| {
            got.push(t.to_string())
        })
        .await
        .unwrap();
        assert_eq!(reply, "fix: a\n\n- b");
        assert_eq!(got, ["fix: a", "\n\n- b"]);
    }

    #[tokio::test]
    async fn an_error_result_or_a_dead_cli_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let p = fake(
            dir.path(),
            r#"echo '{"type":"result","is_error":true,"result":"Not logged in · Please run /login"}'"#,
        );
        let e = text_turn(&p, &[], &[], "x", DRAFT_TIMEOUT, |_| {})
            .await
            .unwrap_err();
        assert!(e.to_string().contains("Not logged in"), "{e}");
        let p = fake(
            dir.path(),
            "echo 'error: unknown option --tools=' >&2; exit 1",
        );
        let e = text_turn(&p, &[], &[], "x", DRAFT_TIMEOUT, |_| {})
            .await
            .unwrap_err();
        assert!(e.to_string().contains("unknown option --tools="), "{e}");
    }

    /// R1-076: dropping the turn (Stop, the client went away) kills the CLI.
    #[tokio::test]
    async fn dropping_the_turn_kills_the_cli() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let p = fake(
            dir.path(),
            &format!("echo $$ > '{}'; exec sleep 30", pid_file.display()),
        );
        let turn = text_turn(&p, &[], &[], "x", DRAFT_TIMEOUT, |_| {});
        let wait_pid = async {
            loop {
                if let Some(pid) = std::fs::read_to_string(&pid_file)
                    .ok()
                    .filter(|s| s.ends_with('\n'))
                {
                    break pid.trim().to_string();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        let pid = tokio::select! {
            _ = turn => panic!("the turn ended on its own"),
            pid = wait_pid => pid,
        };
        let alive = || {
            let out = std::process::Command::new("ps")
                .args(["-o", "stat=", "-p", &pid])
                .output()
                .unwrap();
            let stat = String::from_utf8_lossy(&out.stdout).trim().to_string();
            !stat.is_empty() && !stat.starts_with('Z')
        };
        for _ in 0..200 {
            if !alive() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the CLI (pid {pid}) outlived its dropped turn");
    }

    /// R1-038: a CLI that writes more than a pipe holds (64 KiB) to stderr
    /// before its reply no longer deadlocks the turn, and a non-UTF-8 byte on
    /// stdout no longer drops the reply.
    #[tokio::test]
    async fn a_loud_stderr_or_a_non_utf8_line_still_returns_the_reply() {
        let dir = tempfile::tempdir().unwrap();
        let p = fake(
            dir.path(),
            r#"head -c 200000 /dev/zero | tr '\0' x >&2
printf '\377\n'
printf '%s\n' '{"type":"result","is_error":false,"result":"fix: a"}'"#,
        );
        let reply = text_turn(&p, &[], &[], "x", Duration::from_secs(5), |_| {})
            .await
            .unwrap();
        assert_eq!(reply, "fix: a");
    }

    /// R1-025: `~` in the provider's env is expanded as a session spawn does.
    #[tokio::test]
    async fn the_draft_expands_home_in_the_provider_env() {
        let dir = tempfile::tempdir().unwrap();
        let p = fake(
            dir.path(),
            r#"printf '{"type":"result","is_error":false,"result":"%s"}\n' "$CLAUDE_CONFIG_DIR""#,
        );
        let providers = otto_sessions::ProviderRegistry::new(Some(&json!({
            "claude": { "cmd": p, "env": { "CLAUDE_CONFIG_DIR": "~/.claude-work" } }
        })));
        let reply = claude_draft(&providers, "", "x", |_| {}).await.unwrap();
        let home = otto_transcript::roots::home_dir();
        assert_eq!(reply, home.join(".claude-work").to_string_lossy());
    }

    #[tokio::test]
    async fn a_silent_cli_times_out_with_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = fake(dir.path(), "sleep 30");
        let started = std::time::Instant::now();
        let e = text_turn(&p, &[], &[], "x", Duration::from_millis(300), |_| {})
            .await
            .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(e.to_string().contains("didn’t answer within"), "{e}");
    }

    #[tokio::test]
    async fn ndjson_keeps_deltas_before_the_last_line() {
        use futures_util::StreamExt;
        let lines: Vec<String> = ndjson_lines(|mut on_text| async move {
            on_text("a");
            tokio::task::yield_now().await;
            on_text("b");
            json!({"done": {"message": "ab"}})
        })
        .collect()
        .await;
        assert_eq!(
            lines,
            [
                "{\"delta\":\"a\"}\n",
                "{\"delta\":\"b\"}\n",
                "{\"done\":{\"message\":\"ab\"}}\n"
            ]
        );
    }
}
