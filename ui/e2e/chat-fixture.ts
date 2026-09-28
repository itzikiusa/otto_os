// A realistic Claude Code transcript for the chat specs (and the design
// screenshots): prompts, prose with a code block and a table, a plan
// (TodoWrite), reads, a search, a failing and a passing command, an edit with a
// structured patch, a long command output and a final summary. The redacted
// Track A fixtures (crates/otto-transcript/fixtures) keep the parser honest but
// read as "Xxxx xxx"; this one reads like a real session so layout, hierarchy
// and density can be judged. Written to a temp dir and fed to a seeded session
// through the `OTTO_E2E=1` `meta.e2e_transcript_path` hook.
import { mkdtempSync, writeFileSync, appendFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const SID = '00000000-0000-4000-8000-00000000c4a7';
const T0 = Date.parse('2026-09-28T09:12:00Z');
let clock = 0;
const ts = (): string => new Date(T0 + (clock += 4_000)).toISOString();
let seq = 0;
const uid = (): string => `00000000-0000-4000-8000-${String(++seq).padStart(12, '0')}`;

const usage = { input_tokens: 1200, output_tokens: 180, cache_read_input_tokens: 24000, cache_creation_input_tokens: 800 };

export function userPrompt(text: string): string {
  return JSON.stringify({ type: 'user', uuid: uid(), sessionId: SID, timestamp: ts(), cwd: '/repo/otto', message: { role: 'user', content: text } }) + '\n';
}

export function assistantText(req: string, text: string): string {
  return (
    JSON.stringify({
      type: 'assistant', uuid: uid(), requestId: `req_${req}`, sessionId: SID, timestamp: ts(),
      message: { id: `msg_${req}`, role: 'assistant', model: 'claude-opus-5', content: [{ type: 'text', text }], stop_reason: 'end_turn', usage },
    }) + '\n'
  );
}

export function toolUse(req: string, id: string, name: string, input: Record<string, unknown>): string {
  return (
    JSON.stringify({
      type: 'assistant', uuid: uid(), requestId: `req_${req}`, sessionId: SID, timestamp: ts(),
      message: { id: `msg_${req}`, role: 'assistant', model: 'claude-opus-5', content: [{ type: 'tool_use', id, name, input }], stop_reason: 'tool_use', usage },
    }) + '\n'
  );
}

export function toolResult(id: string, content: string, extra: Record<string, unknown> = {}, isError = false): string {
  return (
    JSON.stringify({
      type: 'user', uuid: uid(), sessionId: SID, timestamp: ts(),
      message: { role: 'user', content: [{ type: 'tool_result', tool_use_id: id, content, is_error: isError }] },
      ...(Object.keys(extra).length ? { toolUseResult: extra } : {}),
    }) + '\n'
  );
}

function thinking(req: string): string {
  return (
    JSON.stringify({
      type: 'assistant', uuid: uid(), requestId: `req_${req}`, sessionId: SID, timestamp: ts(),
      message: { id: `msg_${req}`, role: 'assistant', model: 'claude-opus-5', content: [{ type: 'thinking', thinking: '', signature: 'sig' }], stop_reason: 'tool_use', usage },
    }) + '\n'
  );
}

function duration(ms: number): string {
  return JSON.stringify({ type: 'system', subtype: 'turn_duration', durationMs: ms, uuid: uid(), timestamp: ts(), sessionId: SID }) + '\n';
}

const TEST_LOG = [
  'running 14 tests',
  'test retry::tests::backoff_doubles ... ok',
  'test retry::tests::backoff_caps_at_max ... ok',
  'test retry::tests::jitter_stays_in_range ... FAILED',
  'test retry::tests::gives_up_after_budget ... ok',
  '',
  'failures:',
  '',
  '---- retry::tests::jitter_stays_in_range stdout ----',
  "thread 'retry::tests::jitter_stays_in_range' panicked at crates/otto-net/src/retry.rs:88:9:",
  'assertion failed: d <= max',
  '  left: 1034ms',
  ' right: 1000ms',
  '',
  'test result: FAILED. 13 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out',
].join('\n');

const LONG_LOG = Array.from({ length: 60 }, (_, i) => `   Compiling crate-${String(i).padStart(2, '0')} v0.${i}.0 (/repo/otto/crates/crate-${i})`)
  .concat(['    Finished `test` profile [unoptimized + debuginfo] target(s) in 41.2s', '     Running unittests src/lib.rs', 'test result: ok. 14 passed; 0 failed'])
  .join('\n');

const RETRY_RS = [
  'pub fn backoff(attempt: u32, base: Duration, max: Duration) -> Duration {',
  '    let exp = base.saturating_mul(1 << attempt.min(16));',
  '    let capped = exp.min(max);',
  '    capped + jitter(capped / 10)',
  '}',
].map((l, i) => `${String(80 + i).padStart(6)}\t${l}`).join('\n');

/** The session, as records. `extra` lines are appended by live specs. */
export function chatTranscript(): string {
  clock = 0;
  seq = 0;
  let s = JSON.stringify({ type: 'ai-title', aiTitle: 'Fix the flaky retry jitter test', sessionId: SID }) + '\n';
  s += userPrompt('The `jitter_stays_in_range` test in otto-net fails about once in 20 runs. Can you find out why and fix it? Keep the public API the same.');
  s += thinking('1');
  s += assistantText('1', "I'll start by reproducing the failure and reading the retry module.");
  s += toolUse('1', 'toolu_plan', 'TodoWrite', {
    todos: [
      { content: 'Reproduce the flaky test', status: 'completed', activeForm: 'Reproducing the flaky test' },
      { content: 'Find where jitter can exceed the cap', status: 'in_progress', activeForm: 'Finding where jitter can exceed the cap' },
      { content: 'Fix backoff and add a property test', status: 'pending', activeForm: 'Fixing backoff' },
      { content: 'Run the crate tests', status: 'pending', activeForm: 'Running the crate tests' },
    ],
  });
  s += toolResult('toolu_plan', 'Todos have been modified successfully.');
  s += toolUse('1', 'toolu_run1', 'Bash', { command: 'cargo test -p otto-net retry -- --test-threads=1', description: 'Run the retry tests' });
  s += toolResult('toolu_run1', TEST_LOG, { stdout: TEST_LOG, stderr: '', interrupted: false }, true);
  s += toolUse('1', 'toolu_grep', 'Grep', { pattern: 'fn jitter|fn backoff', path: 'crates/otto-net/src' });
  s += toolResult('toolu_grep', 'crates/otto-net/src/retry.rs:80:pub fn backoff(\ncrates/otto-net/src/retry.rs:97:fn jitter(');
  s += toolUse('1', 'toolu_read', 'Read', { file_path: '/repo/otto/crates/otto-net/src/retry.rs', offset: 80, limit: 5 });
  s += toolResult('toolu_read', RETRY_RS, { type: 'text', file: { filePath: '/repo/otto/crates/otto-net/src/retry.rs', content: RETRY_RS, numLines: 5, startLine: 80, totalLines: 140 } });
  s += assistantText(
    '1',
    [
      'Found it. `backoff` caps the exponential delay **before** adding jitter, so the result can overshoot `max` by up to 10%:',
      '',
      '```rust',
      'let capped = exp.min(max);',
      'capped + jitter(capped / 10) // can exceed `max`',
      '```',
      '',
      'The fix is to clamp after the jitter is applied. I will also add a property test that runs the function 10 000 times.',
    ].join('\n'),
  );
  s += toolUse('1', 'toolu_edit', 'Edit', {
    file_path: '/repo/otto/crates/otto-net/src/retry.rs',
    old_string: '    let capped = exp.min(max);\n    capped + jitter(capped / 10)',
    new_string: '    let capped = exp.min(max);\n    (capped + jitter(capped / 10)).min(max)',
  });
  s += toolResult('toolu_edit', 'The file /repo/otto/crates/otto-net/src/retry.rs has been updated successfully.', {
    filePath: '/repo/otto/crates/otto-net/src/retry.rs',
    structuredPatch: [
      { oldStart: 81, oldLines: 4, newStart: 81, newLines: 4, lines: [
        '     let exp = base.saturating_mul(1 << attempt.min(16));',
        '     let capped = exp.min(max);',
        '-    capped + jitter(capped / 10)',
        '+    (capped + jitter(capped / 10)).min(max)',
        ' }',
      ] },
    ],
  });
  s += toolUse('1', 'toolu_run2', 'Bash', { command: 'cargo test -p otto-net', description: 'Run the otto-net tests' });
  s += toolResult('toolu_run2', LONG_LOG, { stdout: LONG_LOG, stderr: '', interrupted: false });
  s += assistantText(
    '1',
    [
      '## Fixed',
      '',
      '`backoff` now clamps **after** adding jitter, so a delay never exceeds `max`. All 14 tests pass, including 200 repeated runs of the jitter test.',
      '',
      '| Case | Before | After |',
      '|---|---|---|',
      '| `attempt = 12`, `max = 1s` | up to 1.1s | ≤ 1s |',
      '| `attempt = 0` | unchanged | unchanged |',
      '',
      'Next, I can add the property test to CI, or look at the other two callers that assume the old overshoot.',
    ].join('\n'),
  );
  s += duration(127_000);
  s += userPrompt('Great — add the property test, and tell me which callers assume the overshoot.');
  s += assistantText('2', 'On it. Two callers use `backoff` directly:\n\n1. `crates/otto-sessions/src/lifecycle.rs` — reconnect loop\n2. `crates/otto-channels/src/slack.rs` — rate-limit retry\n\nNeither depends on the overshoot; both only need a monotone delay.');
  s += duration(9_000);
  return s;
}

/** Write the transcript to a fresh temp dir; returns its path. */
export function writeChatTranscript(): { dir: string; path: string } {
  const dir = mkdtempSync(join(tmpdir(), 'otto-chat-e2e-'));
  const path = join(dir, `${SID}.jsonl`);
  writeFileSync(path, chatTranscript());
  return { dir, path };
}

/** Append records (a live turn) to a transcript written by `writeChatTranscript`. */
export function appendRecords(path: string, records: string): void {
  appendFileSync(path, records);
}
