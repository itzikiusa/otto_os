// Fictional Claude Code / Codex transcripts for the throwaway daemon's fake
// HOME: History lists them (resumable conversations) and the usage tailer
// turns their token counts into the Usage page. Deterministic, all synthetic.
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const TOPICS = [
  ['Add rate limiting to the checkout API', 'Token bucket at 60 req/min per client, 429 with Retry-After. Tests cover burst and refill.'],
  ['Fix the flaky cart totals test', 'The test depended on the local timezone. Pinned the clock and the rounding now matches.'],
  ['Refactor payment retries into a helper', 'Moved backoff into withRetry() and stopped retrying hard declines.'],
  ['Write the release notes for 1.5.0', 'Drafted notes grouped by feature, with links to the three merged pull requests.'],
  ['Why is p95 latency up on /orders?', 'An N+1 query on order items. Added a join and an index on (order_id).'],
  ['Migrate the sessions table to UUID keys', 'Wrote an online migration with a backfill job and a rollback script.'],
  ['Review PR #214 for security issues', 'Two findings: an unescaped redirect URL and a missing CSRF check on /refund.'],
  ['Add a dark mode to the admin dashboard', 'Moved colours onto tokens and added a scheme switch that follows the system.'],
  ['Summarize yesterday’s incidents', 'One P2 (payment webhook timeouts, 14 min) and two P4s. Action items listed.'],
  ['Generate OpenAPI docs for the orders service', 'Annotated 18 routes and published the spec to docs/openapi.yaml.'],
  ['Speed up the CI pipeline', 'Cached node_modules and split tests into 4 shards: 11m to 4m.'],
  ['Investigate the Kafka consumer lag', 'The consumer group rebalanced every 30s. Raised session.timeout.ms and fixed the poll loop.'],
  ['Add pagination to the customers endpoint', 'Keyset pagination on (created_at, id) with a stable cursor.'],
  ['Clean up unused feature flags', 'Removed 9 flags that were fully rolled out and their dead branches.'],
];

const MODELS_C = ['claude-opus-4-8', 'claude-sonnet-4-8', 'claude-opus-4-8', 'claude-haiku-4-6'];
const MODELS_X = ['gpt-5.5-codex', 'gpt-5.5'];

function uuid(n) {
  const h = (x) => (x >>> 0).toString(16).padStart(8, '0');
  const a = h(n * 2654435761);
  const b = h(n * 40503 + 12345);
  return `${a}-${b.slice(0, 4)}-4${b.slice(5, 8)}-a${a.slice(1, 4)}-${h(n * 97 + 7)}${b.slice(0, 4)}`;
}

/**
 * Write `count` Claude sessions + `codex` Codex sessions over the last `days`
 * days into `<home>/.claude/projects` and `<home>/.codex/sessions`.
 */
export function writeTranscripts(home, { cwds, count = 46, codex = 14, days = 28, now = Date.now() } = {}) {
  const claudeRoot = join(home, '.claude', 'projects');
  const codexRoot = join(home, '.codex', 'sessions');
  let n = 0;
  for (let i = 0; i < count; i++) {
    const [ask, answer] = TOPICS[i % TOPICS.length];
    const cwd = cwds[i % cwds.length];
    const sid = uuid(1000 + i);
    // Busier weekdays, more recent days denser.
    const dayAgo = Math.floor(((i * 7) % (days * 10)) / 10);
    const t0 = now - dayAgo * 86400000 - ((i * 37) % 600) * 60000 - 3600000;
    const model = MODELS_C[i % MODELS_C.length];
    const ts = (s) => new Date(t0 + s * 1000).toISOString();
    const inTok = 18000 + ((i * 7919) % 90000);
    const outTok = 1200 + ((i * 104729) % 9000);
    const lines = [
      { type: 'user', uuid: `u${i}a`, timestamp: ts(0), sessionId: sid, cwd, message: { role: 'user', content: ask } },
      {
        type: 'assistant', uuid: `a${i}a`, requestId: `r${i}a`, timestamp: ts(4), sessionId: sid, cwd,
        message: { id: `m${i}a`, model, content: [{ type: 'text', text: 'Let me look at the relevant code first.' }, { type: 'tool_use', id: `toolu_${i}a`, name: 'Grep', input: { pattern: 'checkout', path: 'src' } }], usage: { input_tokens: Math.round(inTok * 0.4), output_tokens: Math.round(outTok * 0.2), cache_read_input_tokens: Math.round(inTok * 2) } },
      },
      { type: 'user', uuid: `u${i}b`, timestamp: ts(6), sessionId: sid, cwd, message: { role: 'user', content: [{ type: 'tool_result', tool_use_id: `toolu_${i}a`, content: 'src/router.ts\nsrc/cart/totals.ts\nsrc/payments/retry.ts' }] } },
      {
        type: 'assistant', uuid: `a${i}b`, requestId: `r${i}b`, timestamp: ts(40), sessionId: sid, cwd,
        message: { id: `m${i}b`, model, content: [{ type: 'tool_use', id: `toolu_${i}b`, name: 'Bash', input: { command: 'npm test -- --silent', description: 'Run the test suite' } }], usage: { input_tokens: Math.round(inTok * 0.3), output_tokens: Math.round(outTok * 0.3) } },
      },
      { type: 'user', uuid: `u${i}c`, timestamp: ts(58), sessionId: sid, cwd, message: { role: 'user', content: [{ type: 'tool_result', tool_use_id: `toolu_${i}b`, content: `Tests: ${30 + (i % 17)} passed, ${30 + (i % 17)} total` }] }, toolUseResult: { stdout: `Tests: ${30 + (i % 17)} passed`, stderr: '', interrupted: false } },
      {
        type: 'assistant', uuid: `a${i}c`, requestId: `r${i}c`, timestamp: ts(70), sessionId: sid, cwd,
        message: { id: `m${i}c`, model, content: [{ type: 'text', text: answer }], usage: { input_tokens: Math.round(inTok * 0.3), output_tokens: Math.round(outTok * 0.5) } },
      },
      { type: 'ai-title', aiTitle: ask, sessionId: sid },
    ];
    const slug = '-' + cwd.replace(/[^a-zA-Z0-9]/g, '-').replace(/^-+/, '');
    const dir = join(claudeRoot, slug);
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, `${sid}.jsonl`), lines.map((l) => JSON.stringify(l)).join('\n') + '\n');
    n++;
  }
  for (let i = 0; i < codex; i++) {
    const [ask, answer] = TOPICS[(i * 5 + 3) % TOPICS.length];
    const cwd = cwds[(i + 1) % cwds.length];
    const dayAgo = (i * 2) % days;
    const t0 = now - dayAgo * 86400000 - ((i * 53) % 500) * 60000 - 7200000;
    const d = new Date(t0);
    const p2 = (x) => String(x).padStart(2, '0');
    const dir = join(codexRoot, String(d.getUTCFullYear()), p2(d.getUTCMonth() + 1), p2(d.getUTCDate()));
    mkdirSync(dir, { recursive: true });
    const id = uuid(5000 + i);
    const stamp = d.toISOString().slice(0, 19).replace(/:/g, '-');
    const ts = (s) => new Date(t0 + s * 1000).toISOString();
    const model = MODELS_X[i % MODELS_X.length];
    const inTok = 30000 + ((i * 7919) % 60000);
    const outTok = 2000 + ((i * 3571) % 6000);
    const lines = [
      { timestamp: ts(0), type: 'session_meta', payload: { session_id: id, id, cwd, cli_version: '0.150.0', model } },
      { timestamp: ts(0), type: 'turn_context', payload: { cwd, model } },
      { timestamp: ts(1), type: 'event_msg', payload: { type: 'task_started', turn_id: 'T1' } },
      { timestamp: ts(1), type: 'response_item', payload: { type: 'message', role: 'user', content: [{ type: 'input_text', text: ask }] } },
      { timestamp: ts(20), type: 'event_msg', payload: { type: 'item_completed', turn_id: 'T1', item: { type: 'CommandExecution', id: 'e1', command: ['/bin/zsh', '-lc', 'npm test'], cwd: `file://${cwd}`, status: 'completed', aggregated_output: 'ok\n', exit_code: 0 } } },
      { timestamp: ts(30), type: 'event_msg', payload: { type: 'token_count', info: { total_token_usage: { input_tokens: inTok, cached_input_tokens: Math.round(inTok * 0.6), output_tokens: outTok, reasoning_output_tokens: Math.round(outTok * 0.4), total_tokens: inTok + outTok }, last_token_usage: { input_tokens: inTok, cached_input_tokens: 0, output_tokens: outTok, reasoning_output_tokens: 0, total_tokens: inTok + outTok }, model_context_window: 272000 } } },
      { timestamp: ts(31), type: 'event_msg', payload: { type: 'item_completed', turn_id: 'T1', item: { type: 'AgentMessage', id: 'm1', content: [{ type: 'Text', text: answer }] } } },
      { timestamp: ts(32), type: 'event_msg', payload: { type: 'task_complete', turn_id: 'T1', last_agent_message: answer } },
    ];
    writeFileSync(join(dir, `rollout-${stamp}-${id}.jsonl`), lines.map((l) => JSON.stringify(l)).join('\n') + '\n');
    n++;
  }
  return { claudeRoot, codexRoot, files: n };
}
