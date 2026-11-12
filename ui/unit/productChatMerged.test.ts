// The merged story Chat (former Chat + Refine tabs): old tab ids land on
// `chat` (ProductPage maps `product.tab` through `canonicalProductTab`), an
// agent message's cards and suggestion
// are read from its meta, earlier Discovery Chat replies get a read-only
// summary, and a card applies through the conversation's own route.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

const posts: [string, unknown][] = [];
const mod = loadSource(new URL('../src/modules/product/chatActions.ts', import.meta.url), {
  '../../lib/api/client': {
    api: {
      post: async (path: string, body: unknown) => {
        posts.push([path, JSON.parse(JSON.stringify(body))]);
        return { story_updated: false, created_question_ids: ['q'], created_note_ids: [], canvas_id: null };
      },
    },
  },
});
const plain = (v: unknown) => JSON.parse(JSON.stringify(v));

test('old tab ids (refine, discovery) map to the merged chat tab; others are untouched', () => {
  assert.equal(mod.canonicalProductTab('refine'), 'chat');
  assert.equal(mod.canonicalProductTab('discovery'), 'chat');
  for (const tab of ['chat', 'rewrite', 'overview', 'questions']) assert.equal(mod.canonicalProductTab(tab), tab);
});

test('cards come from meta.actions and only appliable kinds render', () => {
  const meta = JSON.stringify({
    story_updated: true,
    version_no: 7,
    actions: [
      { type: 'add_questions', questions: [{ text: 'SLA?' }] },
      { type: 'apply_draft', title: 'T', body_md: 'B' },
      { type: 'create_canvas', title: 'Flow', mermaid: 'flowchart TD' },
      null,
      { type: 'rm_rf' },
    ],
  });
  const cards = plain(mod.chatMessageCards({ meta_json: meta }));
  assert.deepEqual(cards.map((c: { type: string }) => c.type), ['add_questions', 'create_canvas']);
  assert.equal(mod.chatMessageSuggestion({ meta_json: meta }), 7);
});

test('a message without a suggestion or with bad meta shows nothing', () => {
  assert.equal(mod.chatMessageSuggestion({ meta_json: '{"story_updated":false,"version_no":null}' }), null);
  assert.equal(mod.chatMessageSuggestion({ meta_json: 'not json' }), null);
  assert.deepEqual(plain(mod.chatMessageCards({ meta_json: null })), []);
});

test('an earlier chat reply gets a read-only proposal summary', () => {
  const s = mod.proposalSummary(JSON.stringify([
    { type: 'apply_draft', title: 'T', body_md: 'B' },
    { type: 'add_questions', questions: [{}, {}] },
    { type: 'create_canvas', title: 'D' },
  ]));
  assert.equal(s, 'Proposed a story draft, 2 question(s), a diagram');
  assert.equal(mod.proposalSummary(null), '');
  assert.equal(mod.proposalSummary('oops'), '');
});

test('a card applies through the conversation route', async () => {
  const res = await mod.applyChatAction('T1', { type: 'add_notes', notes: [{ body: 'n' }] });
  assert.deepEqual(posts, [['/product/refinement-threads/T1/apply', { action: { type: 'add_notes', notes: [{ body: 'n' }] } }]]);
  assert.deepEqual(plain(res.created_question_ids), ['q']);
});
