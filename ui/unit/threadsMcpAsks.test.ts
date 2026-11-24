// An approval-gated MCP call from a thread's own agent asks in that thread:
// the card joins the attention dock, names the tool, offers Deny / Allow once
// (and "Always" only where a workspace rule can apply), and is answered there.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { attentionHeadline, attentionItems } from '../src/modules/threads/runtime/attentionDock.ts';
import { askAsPending, askDecision, askSentence, askTool, canAlways, isMcpAsk, threadAsks } from '../src/modules/threads/runtime/mcpAsks.ts';

const card = (id: string, extra: Record<string, unknown> = {}) =>
  ({
    id, workspace_id: 'ws1', kind: 'tool_call', server_id: null, server_name: 'otto', tool: 'otto.create_pr',
    title: 'otto MCP server → otto.create_pr', detail: 'Open a PR from feat into main', args_redacted_json: '{"head":"feat"}',
    risk_label: 'dangerous', status: 'pending', requested_by: 'u1', requested_by_kind: 'mcp_server',
    requested_by_session_id: 'thread-1', decided_by: null, decision_note: null, created_at: '2026-11-23T10:00:01Z',
    decided_at: null, consumed_at: null, expires_at: null, ...extra,
  }) as never;

test("only this thread's pending cards, oldest first", () => {
  const all = [card('b', { created_at: '2026-11-23T10:00:05Z' }), card('a'), card('other', { requested_by_session_id: 'thread-2' }), card('done', { status: 'approved' })];
  assert.deepEqual(threadAsks(all, 'thread-1').map((a: { id: string }) => a.id), ['a', 'b']);
});

test('a card is a dock approval that names the tool and sorts with the agent requests', () => {
  const live = { type: 'live' } as const;
  const items = attentionItems(
    [{ id: 'r1', kind: 'command', status: 'pending', created_at: '2026-11-23T10:00:03Z', response_capability: live } as never],
    null,
    [card('a')],
  );
  assert.deepEqual(items.map((i) => i.id), ['mcp:a', 'r1']);
  assert.ok(isMcpAsk(items[0].id) && !isMcpAsk(items[1].id));
  assert.equal(attentionHeadline(items[0], 'Claude'), 'otto · create_pr wants to create pr');
  assert.equal(askTool(card('g', { server_name: 'github', tool: 'create_issue' })), 'github · create_issue');
});

test('the row: Deny / Allow once, the summary as the reason, the redacted args as the detail', () => {
  const row = askAsPending(card('a'), { canDecide: true, always: false });
  assert.equal(row.requestId, 'mcp:a');
  assert.deepEqual(row.options.map((o) => o.label), ['Deny', 'Allow once']);
  assert.equal(row.reason, 'Open a PR from feat into main');
  assert.equal(row.prompt, '{\n  "head": "feat"\n}');
  assert.equal(askAsPending(card('a', { args_redacted_json: '{}' }), { canDecide: false, always: false }).prompt, null);
  assert.equal(askAsPending(card('a'), { canDecide: false, always: false }).canRespond, false);
  assert.deepEqual(askAsPending(card('a'), { canDecide: true, always: true }).options.map((o) => o.decision), ['decline', 'accept', 'accept_always']);
});

test('"Always" only for a reversible otto tool in a workspace, for an admin', () => {
  const none = new Set<string>();
  assert.equal(canAlways(card('a'), none, true), true);
  assert.equal(canAlways(card('a'), none, false), false);
  assert.equal(canAlways(card('a'), new Set(['create_pr']), true), false);
  assert.equal(canAlways(card('a', { server_id: 'srv' }), none, true), false, 'governed servers have no auto-approve rules');
  assert.equal(canAlways(card('a', { workspace_id: null }), none, true), false);
  assert.deepEqual(askDecision('accept'), { approved: true, always: false });
  assert.deepEqual(askDecision('accept_always'), { approved: true, always: true });
  assert.deepEqual(askDecision('decline'), { approved: false, always: false });
});

test('a long governed call shows every argument, not a 400-character cut (security 5)', () => {
  const body = 'x'.repeat(1200);
  const row = askAsPending(card('long', { args_redacted_json: JSON.stringify({ body, to: 'attacker@example.com' }) }), { canDecide: true, always: false });
  assert.ok(row.prompt!.includes('attacker@example.com'), 'the part after the long body is visible');
  assert.ok(row.prompt!.includes(body));
});

test('the card leads with a plain sentence; the arguments are details (I15)', () => {
  assert.equal(askSentence(card('a')), 'otto · create_pr wants to create pr');
  assert.equal(askSentence(card('h', { kind: 'human_ask', tool: 'otto.ask_human_approval', title: 'Deploy to staging?' })), 'otto asks: Deploy to staging?');
  assert.equal(askAsPending(card('a'), { canDecide: true, always: false }).folded, true);
});
