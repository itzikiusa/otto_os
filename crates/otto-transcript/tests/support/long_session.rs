//! A Claude Code transcript shaped like a real long daily session, for the
//! "long conversation opens" tests (otto-transcript `tests/window.rs`, the
//! otto-server route suite). Record shapes and size mix follow the owner's
//! own long sessions (sizes and record types only): ~1 in 10 tool results is
//! a multi-hundred-KB dump duplicated in `toolUseResult`, screenshots ride as
//! base64 images, the CLI's sidecar/telemetry lines (`last-prompt`,
//! `permission-mode`, `api-request`, `ai-title`, `queue-operation`) sit
//! between turns, `Agent` calls spawn sub-agents with sidecar transcripts,
//! compactions write a boundary plus a summary prompt, and the final turn is
//! still running (tool calls, no closing answer).

use std::io::Write;
use std::path::Path;

/// What was written, for the assertions.
#[allow(dead_code)]
pub struct LongSession {
    pub bytes: u64,
    pub records: usize,
    pub exchanges: usize,
    /// The prompt of the final, still-running turn.
    pub last_prompt: String,
    /// The tool call of that turn whose result is the 2 MB dump.
    pub big_tool_id: String,
}

const SID: &str = "5e55e55e-0000-4000-8000-000000000001";

/// Write the session to `path` (plus `<sid>/subagents/` beside it) until it
/// holds at least `min_bytes` and `min_records`.
pub fn write_long_claude_session(path: &Path, min_bytes: u64, min_records: usize) -> LongSession {
    let mut out = Out {
        file: std::io::BufWriter::new(std::fs::File::create(path).unwrap()),
        bytes: 0,
        records: 0,
    };
    let sub_dir = path.with_extension("").join("subagents");
    std::fs::create_dir_all(&sub_dir).unwrap();
    let dump = "lorem ipsum dolor sit amet 0123456789 ".repeat(8 * 1024); // ~300 KB
    let small = "ok: line of tool output\\n".repeat(80); // ~2 KB
    let png = "iVBORw0KGgo".repeat(14 * 1024); // ~150 KB of base64
    let mut n = 0usize;
    let ts = |n: usize, k: usize| {
        format!(
            "2026-11-{:02}T{:02}:{:02}:{:02}.000Z",
            1 + n / 2000 % 28,
            n / 80 % 24,
            n % 60,
            k % 60
        )
    };
    let meta = |n: usize, k: usize| {
        format!(
            r#""sessionId":"{SID}","cwd":"/work/repo","version":"2.1.0","gitBranch":"main","timestamp":"{}""#,
            ts(n, k)
        )
    };
    while out.bytes < min_bytes || out.records < min_records {
        let m = meta(n, 0);
        out.put(format!(r#"{{"type":"user","uuid":"u-{n}","parentUuid":null,{m},"message":{{"role":"user","content":"prompt {n}: look at the next failing check and fix it"}}}}"#));
        out.put(format!(r#"{{"type":"permission-mode","permissionMode":"bypassPermissions","sessionId":"{SID}"}}"#));
        out.put(format!(
            r#"{{"type":"last-prompt","lastPrompt":"prompt {n}","sessionId":"{SID}"}}"#
        ));
        out.put(format!(r#"{{"type":"api-request","requestId":"req-{n}-0","model":"claude-opus-5-5","sessionId":"{SID}"}}"#));
        out.put(format!(r#"{{"type":"assistant","uuid":"a-{n}-t",{m},"requestId":"req-{n}-0","message":{{"id":"msg-{n}-0","role":"assistant","model":"claude-opus-5-5","content":[{{"type":"thinking","thinking":"","signature":"sig"}}],"usage":{{"input_tokens":12,"output_tokens":40,"cache_read_input_tokens":90000}}}}}}"#));
        for k in 0..3 {
            let id = format!("toolu-{n}-{k}");
            let rid = format!("req-{n}-{k}");
            let m = meta(n, k + 1);
            let agent = n % 40 == 7 && k == 0;
            let (name, input) = if agent {
                ("Agent", r#"{"description":"scan the module","prompt":"read the module and report","subagent_type":"Explore"}"#.to_string())
            } else {
                (
                    "Bash",
                    format!(
                        r#"{{"command":"cargo test -p crate-{k} -- case_{n}","description":"run the case"}}"#
                    ),
                )
            };
            out.put(format!(r#"{{"type":"assistant","uuid":"a-{n}-{k}",{m},"requestId":"{rid}","message":{{"id":"msg-{n}-{k}","role":"assistant","model":"claude-opus-5-5","content":[{{"type":"tool_use","id":"{id}","name":"{name}","input":{input}}}],"usage":{{"input_tokens":3,"output_tokens":120,"cache_read_input_tokens":91000}}}}}}"#));
            let text = if n % 25 == 3 && k == 1 {
                dump.as_str()
            } else {
                small.as_str()
            };
            if agent {
                let aid = format!("a{n:06}");
                std::fs::write(sub_dir.join(format!("agent-{aid}.meta.json")), format!(r#"{{"agentType":"Explore","description":"scan the module","toolUseId":"{id}"}}"#)).unwrap();
                std::fs::write(sub_dir.join(format!("agent-{aid}.jsonl")), format!("{{\"type\":\"user\",\"isSidechain\":true,\"agentId\":\"{aid}\",\"uuid\":\"s-{n}\",{m},\"message\":{{\"role\":\"user\",\"content\":\"read the module and report\"}}}}\n")).unwrap();
                out.put(format!(r#"{{"type":"user","uuid":"r-{n}-{k}",{m},"message":{{"role":"user","content":[{{"tool_use_id":"{id}","type":"tool_result","content":[{{"type":"text","text":"Module report."}}]}}]}},"toolUseResult":{{"status":"completed","agentId":"{aid}","content":[{{"type":"text","text":"Module report."}}]}}}}"#));
            } else if n % 25 == 11 && k == 2 {
                out.put(format!(r#"{{"type":"user","uuid":"r-{n}-{k}",{m},"message":{{"role":"user","content":[{{"tool_use_id":"{id}","type":"tool_result","content":[{{"type":"image","source":{{"type":"base64","media_type":"image/png","data":"{png}{n}"}}}}]}}]}}}}"#));
            } else {
                out.put(format!(r#"{{"type":"user","uuid":"r-{n}-{k}",{m},"message":{{"role":"user","content":[{{"tool_use_id":"{id}","type":"tool_result","content":"{text}","is_error":false}}]}},"toolUseResult":{{"stdout":"{text}","stderr":"","interrupted":false}}}}"#));
            }
        }
        out.put(format!(r#"{{"type":"assistant","uuid":"a-{n}-x",{m},"requestId":"req-{n}-9","message":{{"id":"msg-{n}-9","role":"assistant","model":"claude-opus-5-5","content":[{{"type":"text","text":"Done with step {n}: the check passes now."}}],"usage":{{"input_tokens":4,"output_tokens":60,"cache_read_input_tokens":92000}}}}}}"#));
        out.put(format!(
            r#"{{"type":"ai-title","aiTitle":"Long running session","sessionId":"{SID}"}}"#
        ));
        if n % 9 == 4 {
            out.put(format!(r#"{{"type":"queue-operation","operation":"enqueue","content":"and then the next one","sessionId":"{SID}","timestamp":"{}"}}"#, ts(n, 9)));
        }
        if n % 500 == 499 {
            out.put(format!(r#"{{"type":"system","subtype":"compact_boundary","content":"Conversation compacted","uuid":"c-{n}",{m},"compactMetadata":{{"trigger":"auto","preTokens":180000}}}}"#));
            out.put(format!(r#"{{"type":"user","uuid":"cs-{n}",{m},"isCompactSummary":true,"isVisibleInTranscriptOnly":true,"message":{{"role":"user","content":"This session is being continued from a previous conversation that ran out of context. Summary: steps 0..{n} fixed."}}}}"#));
        }
        n += 1;
    }
    // The final turn: a prompt, then a long run of tool calls (one 2 MB dump)
    // and no closing answer yet — the agent is still working.
    let last_prompt = format!("prompt {n}: final sweep, still running");
    let m = meta(n, 0);
    out.put(format!(r#"{{"type":"user","uuid":"u-{n}","parentUuid":null,{m},"message":{{"role":"user","content":"{last_prompt}"}}}}"#));
    let big_tool_id = format!("toolu-{n}-big");
    for k in 0..120 {
        let id = if k == 60 {
            big_tool_id.clone()
        } else {
            format!("toolu-{n}-r{k}")
        };
        let m = meta(n, k);
        out.put(format!(r#"{{"type":"assistant","uuid":"a-{n}-r{k}",{m},"requestId":"req-{n}-r{k}","message":{{"id":"msg-{n}-r{k}","role":"assistant","model":"claude-opus-5-5","content":[{{"type":"tool_use","id":"{id}","name":"Read","input":{{"file_path":"/work/repo/src/file_{k}.rs"}}}}],"usage":{{"input_tokens":2,"output_tokens":30,"cache_read_input_tokens":93000}}}}}}"#));
        let text = if k == 60 {
            dump.repeat(7)
        } else {
            small.clone()
        };
        out.put(format!(r#"{{"type":"user","uuid":"r-{n}-r{k}",{m},"message":{{"role":"user","content":[{{"tool_use_id":"{id}","type":"tool_result","content":"{text}"}}]}}}}"#));
    }
    out.file.flush().unwrap();
    LongSession {
        bytes: out.bytes,
        records: out.records,
        exchanges: n,
        last_prompt,
        big_tool_id,
    }
}

struct Out {
    file: std::io::BufWriter<std::fs::File>,
    bytes: u64,
    records: usize,
}

impl Out {
    fn put(&mut self, line: String) {
        self.file.write_all(line.as_bytes()).unwrap();
        self.file.write_all(b"\n").unwrap();
        self.bytes += line.len() as u64 + 1;
        self.records += 1;
    }
}
