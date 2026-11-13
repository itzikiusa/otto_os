#!/usr/bin/env node
// The tour capture's stand-in for `claude` on the structured runtime (the
// Agent SDK's stream-json control protocol: `--input-format stream-json
// --output-format stream-json`). Same frame shapes as otto-agent-protocol's
// `otto-fake-claude` test double, but it streams believable, scripted answers
// for the film (fictional Acme project): a Council seat, a judge's synthesis,
// and a child thread asked from inside another thread. Pure local script:
// nothing leaves the machine and no account is used.
import { createInterface } from 'node:readline';

const out = (v) => process.stdout.write(`${JSON.stringify(v)}\n`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const arg = (flag) => {
  const i = process.argv.indexOf(flag);
  return i >= 0 ? process.argv[i + 1] : undefined;
};
const session = arg('--resume') ?? arg('--session-id') ?? 'demo-claude';
const model = arg('--model') ?? 'claude-opus-4-8';
let n = 0;
let busy = false;

/** One assistant text block, streamed word by word, then the full message. */
async function say(text, pace = 34) {
  const id = `msg_${session.slice(0, 8)}_${++n}`;
  const ev = (event) => out({ type: 'stream_event', event, parent_tool_use_id: null });
  ev({ type: 'message_start', message: { id } });
  ev({ type: 'content_block_start', index: 0, content_block: { type: 'text', text: '' } });
  const parts = text.match(/\S+\s*|\s+/g) ?? [];
  for (let i = 0; i < parts.length; i += 2) {
    ev({ type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: parts.slice(i, i + 2).join('') } });
    await sleep(pace);
  }
  ev({ type: 'content_block_stop', index: 0 });
  out({ type: 'assistant', message: { id, model, content: [{ type: 'text', text }] }, parent_tool_use_id: null });
}

/** A read-only tool call and its result (shown as a work row). */
async function tool(name, input, result, ms = 600) {
  const id = `toolu_${session.slice(0, 6)}_${++n}`;
  out({ type: 'assistant', message: { id: `msg_t${n}`, model, content: [{ type: 'tool_use', id, name, input }] }, parent_tool_use_id: null });
  await sleep(ms);
  out({ type: 'user', message: { content: [{ type: 'tool_result', tool_use_id: id, content: result }] }, parent_tool_use_id: null });
}

// Usage → Remaining: structured Claude sessions report the plan windows as
// `rate_limit_event` frames (fictional numbers).
const limits = () => {
  const now = Math.floor(Date.now() / 1000);
  out({ type: 'rate_limit_event', rate_limit_info: { status: 'allowed', resetsAt: now + 2 * 3600 + 840, rateLimitType: 'five_hour', utilization: 0.34 }, uuid: `rl-${n}a`, session_id: session });
  out({ type: 'rate_limit_event', rate_limit_info: { status: 'allowed', resetsAt: now + 3 * 86400 + 3600, rateLimitType: 'seven_day', utilization: 0.58 }, uuid: `rl-${n}b`, session_id: session });
};
const done = (input, output) => {
  limits();
  out({ type: 'result', subtype: 'success', is_error: false, stop_reason: 'end_turn', usage: { input_tokens: input, output_tokens: output } });
};

async function seat() {
  await tool('Grep', { pattern: 'retry', path: 'src/payments' }, 'src/payments/charge.ts:14:  for (let attempt = 0; attempt < 3; attempt++) {\nsrc/payments/charge.ts:22:    await sleep(500);');
  await say(
    'Use **exponential backoff with full jitter**, not a fixed delay.\n\n- Start at 200 ms, double each time, cap at 5 s.\n- Retry only soft declines and timeouts; a hard decline fails at once.\n- Send the same idempotency key on every attempt so a retry never charges twice.\n\nA fixed 500 ms delay makes every client retry in step when the card network is slow.',
  );
  done(16200, 540);
}

async function synthesis() {
  await say(
    '**Agreement.** Both seats pick exponential backoff over a fixed delay, retry only soft declines, and reuse one idempotency key.\n\n**Disagreement.** Seat 1 adds full jitter; seat 2 caps attempts at 4. Both are right: jitter stops clients retrying in step, the cap bounds checkout latency.\n\n**Pick.** Merge them: 200 ms base, doubling, full jitter, at most 4 attempts, 5 s cap, one idempotency key.\n\n**Open question.** Should a retried charge show the customer a "still working" state after 2 s?',
    30,
  );
  done(4100, 380);
}

async function reviewTests() {
  await tool('Read', { file_path: 'tests/rateLimit.test.ts' }, '18 lines');
  await say(
    'The three cases are right. Two gaps:\n\n1. **Separate keys**: nothing checks that `cart:42` hitting the limit leaves `cart:43` alone.\n2. **Window edge**: no test sends request 61 exactly when the window resets.\n\nBoth are one `it()` each with fake timers.',
  );
  done(9400, 310);
}

// A Council follow-up (round 2), sent to every seat.
async function followUp() {
  await say(
    'After 2 s, swap the spinner for **"Still confirming your payment…"** and keep Pay disabled, so nobody taps twice. If the last attempt fails, show the bank\'s reason and a **Try another card** button.',
  );
  done(7200, 260);
}

async function generic() {
  await say('Done. Nothing else needs changing here.');
  done(900, 20);
}

function pick(text) {
  const s = text.toLowerCase();
  if (s.includes('you are the judge of a council')) return synthesis;
  if (s.includes('customer see')) return followUp;
  if (s.includes('retries') || s.includes('backoff')) return seat;
  if (s.includes('review') && s.includes('test')) return reviewTests;
  return generic;
}

const userText = (msg) => {
  const c = msg?.message?.content;
  if (typeof c === 'string') return c;
  return Array.isArray(c) ? c.map((p) => p.text ?? '').join('\n') : '';
};

const rl = createInterface({ input: process.stdin });
rl.on('line', (line) => {
  let msg;
  try {
    msg = JSON.parse(line);
  } catch {
    return;
  }
  if (msg.type === 'control_request') {
    out({ type: 'control_response', response: { subtype: 'success', request_id: msg.request_id ?? null, response: {} } });
    if (msg.request?.subtype === 'initialize') out({ type: 'system', subtype: 'init', session_id: session, model });
    return;
  }
  if (msg.type === 'user' && !busy) {
    busy = true;
    const run = pick(userText(msg));
    setTimeout(() => void run().finally(() => (busy = false)), 400);
  }
});
rl.on('close', () => process.exit(0));
