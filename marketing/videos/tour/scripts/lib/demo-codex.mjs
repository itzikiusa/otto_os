#!/usr/bin/env node
// The tour capture's stand-in for `codex app-server` (the structured runtime
// that Threads and Personal Agents run on). Same wire shapes as
// otto-agent-protocol's `otto-fake-codex` test double — JSON-RPC without the
// "jsonrpc" member, one message per line — but it streams believable, scripted
// turns for the film (fictional Acme project) instead of `echo:`. Pure local
// script: nothing leaves the machine and no account is used.
import { createInterface } from 'node:readline';

const out = (v) => process.stdout.write(`${JSON.stringify(v)}\n`);
const notify = (method, params) => out({ method, params });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let thread = '';
let turns = 0;
let asking = null; // { rid, turn, resume }
let nextServerId = 100;

const turnObj = (id, status) => ({ id, items: [], status });

async function stream(t, id, text, { reasoning = false, pace = 38 } = {}) {
  const type = reasoning ? 'reasoning' : 'agentMessage';
  notify('item/started', { threadId: thread, turnId: t, startedAtMs: 0, item: { type, id, text: '', ...(reasoning ? { summary: [] } : {}) } });
  const parts = text.match(/\S+\s*|\s+/g) ?? [];
  for (let i = 0; i < parts.length; i += 2) {
    const delta = parts.slice(i, i + 2).join('');
    notify(reasoning ? 'item/reasoning/summaryTextDelta' : 'item/agentMessage/delta', { threadId: thread, turnId: t, itemId: id, delta });
    await sleep(pace);
  }
  const done = reasoning ? { type, id, summary: [text] } : { type, id, text };
  notify('item/completed', { threadId: thread, turnId: t, completedAtMs: 0, item: done });
}

const cmdItem = (id, command, status, output, exitCode) => ({
  type: 'commandExecution', id, command, cwd: '~/Code/acme-checkout', commandActions: [], status,
  aggregatedOutput: output ?? null, exitCode: exitCode ?? null,
});

async function command(t, id, cmd, lines, { pause = 140 } = {}) {
  notify('item/started', { threadId: thread, turnId: t, startedAtMs: 0, item: cmdItem(id, cmd, 'inProgress') });
  let all = '';
  for (const l of lines) {
    await sleep(pause);
    all += `${l}\n`;
    notify('item/commandExecution/outputDelta', { threadId: thread, turnId: t, itemId: id, delta: `${l}\n` });
  }
  notify('item/completed', { threadId: thread, turnId: t, completedAtMs: 0, item: cmdItem(id, cmd, 'completed', all, 0) });
}

const plan = (t, steps) => notify('turn/plan/updated', { threadId: thread, turnId: t, plan: steps.map(([step, status]) => ({ step, status })) });

function usage(t, input, output) {
  const u = { inputTokens: input, outputTokens: output, cachedInputTokens: Math.round(input * 0.6), reasoningOutputTokens: 0, totalTokens: input + output };
  notify('thread/tokenUsage/updated', { threadId: thread, turnId: t, tokenUsage: { last: u, total: u, modelContextWindow: 272000 } });
}
const finish = (t) => notify('turn/completed', { threadId: thread, turn: turnObj(t, 'completed') });

const TEST_DIFF = `--- /dev/null
+++ b/tests/rateLimit.test.ts
@@ -0,0 +1,18 @@
+import { describe, expect, it } from 'vitest';
+import { hit, resetWindow } from '../src/middleware/rateLimit';
+
+describe('checkout rate limit', () => {
+  it('allows 60 requests a minute', () => {
+    for (let i = 0; i < 60; i++) expect(hit('cart:42').ok).toBe(true);
+  });
+  it('answers 429 with Retry-After on the 61st', () => {
+    const r = hit('cart:42');
+    expect(r.status).toBe(429);
+    expect(r.headers['Retry-After']).toBe('60');
+  });
+  it('resets after the window', () => {
+    resetWindow('cart:42');
+    expect(hit('cart:42').ok).toBe(true);
+  });
+});`;

async function regressionTest(t) {
  await stream(t, `${t}-r`, 'Reading the rate-limit middleware and the existing checkout tests to see what is covered.', { reasoning: true, pace: 30 });
  plan(t, [['Read src/middleware/rateLimit.ts', 'inProgress'], ['Write tests/rateLimit.test.ts', 'pending'], ['Run the suite', 'pending']]);
  await command(t, `${t}-c1`, 'rg -n "rateLimit" src tests', ['src/middleware/rateLimit.ts:12:export function hit(key: string)', 'src/middleware/rateLimit.ts:31:export function resetWindow(key: string)', 'src/routes/cart.ts:8:app.use(rateLimit({ perMinute: 60 }))']);
  plan(t, [['Read src/middleware/rateLimit.ts', 'completed'], ['Write tests/rateLimit.test.ts', 'inProgress'], ['Run the suite', 'pending']]);
  await sleep(500);
  const fc = { type: 'fileChange', id: `${t}-f`, status: 'completed', changes: [{ path: 'tests/rateLimit.test.ts', kind: { type: 'add' }, diff: TEST_DIFF }] };
  notify('item/started', { threadId: thread, turnId: t, startedAtMs: 0, item: { ...fc, status: 'inProgress' } });
  await sleep(700);
  notify('item/completed', { threadId: thread, turnId: t, completedAtMs: 0, item: fc });
  plan(t, [['Read src/middleware/rateLimit.ts', 'completed'], ['Write tests/rateLimit.test.ts', 'completed'], ['Run the suite', 'inProgress']]);
  await command(t, `${t}-c2`, 'npm test -- rateLimit', [' RUN  v3.2.4 ~/Code/acme-checkout', ' ✓ tests/rateLimit.test.ts (3 tests) 9ms', '   ✓ allows 60 requests a minute', '   ✓ answers 429 with Retry-After on the 61st', '   ✓ resets after the window', ' Test Files  1 passed (1)', '      Tests  3 passed (3)'], { pause: 220 });
  plan(t, [['Read src/middleware/rateLimit.ts', 'completed'], ['Write tests/rateLimit.test.ts', 'completed'], ['Run the suite', 'completed']]);
  await stream(t, `${t}-m`, 'Added **tests/rateLimit.test.ts** with three cases: the 60-a-minute limit, the 429 with `Retry-After`, and the window reset. All three pass. Want me to open a draft PR?');
  usage(t, 18400, 1260);
  finish(t);
}

async function draftPr(t) {
  await stream(t, `${t}-m0`, 'Pushing the branch and opening a draft PR against `main`.', { pace: 32 });
  const cmd = 'gh pr create --draft --title "test: checkout rate limit regression"';
  notify('item/started', { threadId: thread, turnId: t, startedAtMs: 0, item: cmdItem(`${t}-c`, cmd, 'inProgress') });
  const rid = ++nextServerId;
  asking = {
    rid,
    turn: t,
    resume: async (accepted) => {
      if (!accepted) {
        notify('item/completed', { threadId: thread, turnId: t, completedAtMs: 0, item: cmdItem(`${t}-c`, cmd, 'declined') });
        await stream(t, `${t}-m`, 'Okay, I left the branch local.');
        return finish(t);
      }
      const o = 'https://github.com/acme/checkout/pull/218\n';
      await sleep(400);
      notify('item/commandExecution/outputDelta', { threadId: thread, turnId: t, itemId: `${t}-c`, delta: o });
      notify('item/completed', { threadId: thread, turnId: t, completedAtMs: 0, item: cmdItem(`${t}-c`, cmd, 'completed', o, 0) });
      await stream(t, `${t}-m`, 'Draft PR **#218** is up with the new test. CI is running; I will tell you when it is green.');
      usage(t, 21900, 420);
      finish(t);
    },
  };
  out({ id: rid, method: 'item/commandExecution/requestApproval', params: { threadId: thread, turnId: t, itemId: `${t}-c`, startedAtMs: 0, command: cmd, reason: 'push the branch and open a draft PR' } });
}

async function brief(t) {
  await stream(t, `${t}-r`, 'Pulling the last day of Slack mentions, DMs and open threads.', { reasoning: true, pace: 30 });
  const call = { type: 'mcpToolCall', id: `${t}-s`, server: 'otto', tool: 'slack_digest', arguments: { since: '24h', include: ['mentions', 'dms', 'unanswered'] }, status: 'inProgress' };
  notify('item/started', { threadId: thread, turnId: t, startedAtMs: 0, item: call });
  await sleep(1100);
  notify('item/completed', { threadId: thread, turnId: t, completedAtMs: 0, item: { ...call, status: 'completed', result: '14 messages · 3 need a reply · 2 action items' } });
  await stream(
    t,
    `${t}-m`,
    "Here's your morning brief.\n\n**Needs you**\n- Alex asked for your review on PR #214 (checkout retries), waiting 3 h\n- #release: confirm the 2.4 cut by noon\n\n**Unanswered**\n- #payments: Sam's question about webhook retries\n\n**Action items**\n- Approve Friday's deploy window\n- Reply to the design review notes\n\nShall I post this to your digest channel and ask the Bitbucket helper for the PR summary?",
    { pace: 34 },
  );
  usage(t, 9600, 610);
  finish(t);
}

async function generic(t) {
  await stream(t, `${t}-m`, 'On it. I will report back here when it is done.');
  usage(t, 2100, 40);
  finish(t);
}

function pick(text) {
  const s = text.toLowerCase();
  if (s.includes('regression test') || s.includes('rate limit') || s.includes('rate-limit')) return regressionTest;
  if (s.includes('draft pr') || s.includes('open a pr') || s.includes('open the pr')) return draftPr;
  if (s.includes('needs me') || s.includes('brief') || s.includes('digest')) return brief;
  return generic;
}

const rl = createInterface({ input: process.stdin });
rl.on('line', (line) => {
  let msg;
  try {
    msg = JSON.parse(line);
  } catch {
    return;
  }
  const { id = null, method = '', params = {} } = msg;
  switch (method) {
    case 'initialize':
      return out({ id, result: { userAgent: 'otto-tour/0.166.0 (demo)', codexHome: '/tmp', platformFamily: 'unix', platformOs: 'macos' } });
    case 'initialized':
      return;
    case 'thread/start':
      thread = `demo-th-${process.pid}`;
      return out({ id, result: { thread: { id: thread }, model: 'gpt-5.5-codex', approvalPolicy: params.approvalPolicy, sandbox: params.sandbox } });
    case 'thread/resume': {
      const tid = params.threadId ?? '';
      if (!tid.startsWith('demo-th-')) return out({ id, error: { code: -32600, message: `no rollout found for thread id ${tid}` } });
      thread = tid;
      return out({ id, result: { thread: { id: thread }, model: 'gpt-5.5-codex' } });
    }
    case 'turn/start': {
      const t = `turn-${++turns}`;
      out({ id, result: { turn: turnObj(t, 'inProgress') } });
      notify('turn/started', { threadId: thread, turn: turnObj(t, 'inProgress') });
      const text = (params.input ?? []).map((i) => i.text ?? '').join('\n');
      setTimeout(() => void pick(text)(t), 350);
      return;
    }
    case 'turn/interrupt': {
      out({ id, result: {} });
      if (asking) {
        notify('serverRequest/resolved', { threadId: thread, requestId: asking.rid });
        notify('turn/completed', { threadId: thread, turn: turnObj(asking.turn, 'interrupted') });
        asking = null;
      }
      return;
    }
    case '':
      if (id !== null && asking && id === asking.rid) {
        const a = asking;
        asking = null;
        const d = msg.result?.decision ?? '';
        void a.resume(d === 'accept' || d === 'acceptForSession');
      }
      return;
    default:
      if (id !== null) out({ id, error: { code: -32601, message: 'unsupported' } });
  }
});
rl.on('close', () => process.exit(0));
