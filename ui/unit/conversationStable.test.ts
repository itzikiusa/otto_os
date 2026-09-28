// Chat render stability (perf backlog B1): render items keep their identity
// for unchanged turns, the md→html memo is bounded, index items are cached per
// turn object, and the live turn list is capped without breaking "Load earlier".
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { groupTurns, stableGroupTurns, type RenderItem } from '../src/modules/agents/conversation/format.ts';
import { createMdCache } from '../src/modules/agents/conversation/mdCache.ts';
import { indexToRenderItem } from '../src/modules/assistant/chat.ts';
import { loadSource } from './sourceHarness.ts';
import { TranscriptLifecycle } from '../src/lib/stores/transcriptLifecycle.ts';
import * as paneHeader from '../src/lib/paneHeader.ts';
import type { AssistantTurn, Turn } from '../src/lib/api/types.ts';

const user = (id: string, md = `ask ${id}`): Turn => ({
  id, role: 'user', ts: null, blocks: [{ kind: 'text', md }], duration_ms: null, model: null, system: [], reasoning_steps: 0,
});
const reply = (id: string, md = `answer ${id}`): Turn => ({
  id, role: 'assistant', ts: null, blocks: [{ kind: 'text', md }], duration_ms: 5, model: 'm', system: [], reasoning_steps: 0,
});

test('stableGroupTurns reuses the item object of every unchanged group', () => {
  const turns = [user('u1'), reply('a1'), reply('a1b'), user('u2'), reply('a2')];
  const cache = new Map<string, RenderItem>();
  const first = stableGroupTurns(turns, cache);
  assert.deepEqual(first, groupTurns(turns), 'same shape as groupTurns');
  // A live delta: the last response grows (replaced object) and a prompt lands.
  const next = [...turns.slice(0, 4), reply('a2', 'answer a2, longer'), user('u3')];
  const second = stableGroupTurns(next, cache);
  assert.equal(second[0], first[0], 'u1 untouched');
  assert.equal(second[1], first[1], 'the a1 response (two turns) untouched');
  assert.equal(second[2], first[2], 'u2 untouched');
  assert.notEqual(second[3], first[3], 'the grown response is a new item');
  assert.equal(second[3].blocks[0].kind === 'text' && second[3].blocks[0].md, 'answer a2, longer');
  assert.equal(second.length, 5);
  assert.deepEqual([...cache.keys()], ['u1', 'a1', 'u2', 'a2', 'u3'], 'cache holds exactly the current items');
});

test('stableGroupTurns re-groups when a member turn joins or leaves a response', () => {
  const cache = new Map<string, RenderItem>();
  const a = reply('a1');
  const first = stableGroupTurns([user('u1'), a], cache);
  const second = stableGroupTurns([first[0].turns[0], a, reply('a1b')], cache);
  assert.equal(second[0], first[0]);
  assert.notEqual(second[1], first[1], 'a new tool-loop turn extends the response → new item');
  assert.equal(second[1].turns.length, 2);
  // Window slides past the prompt: the response is now the first item, same id, same turns.
  const third = stableGroupTurns(second[1].turns, cache);
  assert.equal(third[0], second[1]);
});

test('md cache: hits return the memo, misses render once, bounds evict oldest', () => {
  let renders = 0;
  const c = createMdCache((md) => (renders++, `<p>${md}</p>`), { maxEntries: 3, maxChars: 1000, maxEntryChars: 100 });
  assert.equal(c.get('a'), '<p>a</p>');
  assert.equal(c.get('a'), '<p>a</p>');
  assert.equal(renders, 1);
  c.get('b');
  c.get('c');
  c.get('a'); // refresh a → b is now oldest
  c.get('d');
  assert.equal(c.size, 3);
  renders = 0;
  c.get('a');
  c.get('c');
  c.get('d');
  assert.equal(renders, 0, 'a, c, d stayed');
  c.get('b');
  assert.equal(renders, 1, 'b was evicted');
  // Not cacheable (e.g. fenced code before hljs loaded) and oversized inputs are never stored.
  renders = 0;
  c.get('x', false);
  c.get('x', false);
  c.get('y'.repeat(200));
  c.get('y'.repeat(200));
  assert.equal(renders, 4);
  // Char budget.
  const small = createMdCache((md) => md, { maxEntries: 100, maxChars: 10, maxEntryChars: 10 });
  small.get('aaaa'); // 8 chars
  small.get('bb'); // 12 → evict aaaa
  assert.equal(small.size, 1);
  assert.equal(small.chars, 4);
});

test('indexToRenderItem is memoized per index turn object', () => {
  const t = { id: 'i1', role: 'assistant', text: 'hi', created_at: '2026-01-01T00:00:00Z', model: 'm' } as unknown as AssistantTurn;
  const a = indexToRenderItem(t);
  assert.equal(indexToRenderItem(t), a);
  const replaced = { ...t, text: 'hi, edited' } as AssistantTurn;
  const b = indexToRenderItem(replaced);
  assert.notEqual(b, a);
  assert.equal(b.blocks[0].kind === 'text' && b.blocks[0].md, 'hi, edited');
});

function loadStore() {
  return loadSource(new URL('../src/lib/stores/transcript.svelte.ts', import.meta.url), {
    './transcriptLifecycle': { TranscriptLifecycle },
    '../paneHeader': paneHeader,
    '../win': { winKey: (key: string) => key },
    '../api/client': { api: {}, isAbortError: () => false },
  });
}

test('trimTurnHead only cuts in front of a turn with a known cursor', () => {
  const { trimTurnHead } = loadStore();
  const turns = Array.from({ length: 12 }, (_, i) => reply(`t${i}`));
  assert.equal(trimTurnHead(turns, new Map(), 10, 6), null, 'under the cap → untouched');
  assert.equal(trimTurnHead(turns.slice(0, 10), new Map([['t4', '40']]), 10, 6), null);
  assert.equal(trimTurnHead(turns, new Map(), 10, 6), null, 'no bound → nothing dropped');
  const bounds = new Map([['t1', '10'], ['t7', '70'], ['t8', '80']]);
  const r = trimTurnHead(turns, bounds, 10, 6);
  assert.ok(r);
  assert.equal(r.dropped, 7, 'first bound at or after len-keep (index 6) is t7');
  assert.equal(r.turns[0].id, 't7');
  assert.equal(r.cursor, '70');
  assert.deepEqual([...bounds.keys()], ['t7', 't8'], 'dropped ids leave the bound map');
  // A bound further than keep/2 from the tail is too deep a cut.
  assert.equal(trimTurnHead(turns, new Map([['t10', '100']]), 10, 6), null);
});

test('a long live chat is capped and "Load earlier" resumes exactly at the dropped head', () => {
  const { transcript, TURN_CAP, TURN_KEEP } = loadStore();
  const c = transcript.conversation({ sessionId: 's' });
  c.transcript = { cursor: '0', has_earlier: false, stats: { turns: 0 }, unavailable_reason: null, turns: [] };
  // Turn k occupies records [10(k-1)+1, 10k]; each delta folds through 10k.
  for (let k = 1; k <= TURN_CAP + 1; k++) c.applyDelta(String(10 * k), [reply(`t${k}`)]);
  const dropped = TURN_CAP + 1 - TURN_KEEP;
  assert.equal(c.turns.length, TURN_KEEP);
  assert.equal(c.headDropped, dropped);
  const head = c.turns[0];
  assert.equal(head.id, `t${dropped + 1}`);
  assert.equal(c.transcript.has_earlier, true);
  // `before` is exclusive on the first record: it must page every dropped turn
  // (first record ≤ 10·dropped − 9) and nothing from the kept head onward.
  const firstOf = (k: number) => 10 * (k - 1) + 1;
  assert.equal(Number(c.transcript.cursor), firstOf(dropped + 1));
  assert.equal(c.transcript.stats.turns, TURN_CAP + 1, 'server total still counts every turn');
  // Growing a kept turn in place never trims again.
  c.applyDelta(String(10 * (TURN_CAP + 1) + 1), [reply(`t${TURN_CAP + 1}`, 'grew')]);
  assert.equal(c.turns.length, TURN_KEEP);
  assert.equal(c.turns.at(-1).blocks[0].md, 'grew');
});
