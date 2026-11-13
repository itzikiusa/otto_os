// A rewrite written before the newest source version (e.g. the Jira edit a
// conflict-Refresh just pulled in) is marked stale, and every Product publish
// review shows WHERE it goes (site + issue) and the FULL text sent — the old
// confirm showed the first four lines, capped at 400 characters.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { STALE_SUGGESTION, storySite, suggestionIsStale } from '../src/modules/product/publishReview.ts';

test('a suggestion older than the newest source version is stale', () => {
  const suggested = { kind: 'suggested', version_no: 3 };
  const versions = [
    { kind: 'source', version_no: 1 },
    suggested,
    { kind: 'published', version_no: 4 },
  ];
  // Only a newer SOURCE row counts; a publish row does not.
  assert.equal(suggestionIsStale(suggested, versions), false);
  // The Refresh after a 409 recorded source v5.
  assert.equal(suggestionIsStale(suggested, [...versions, { kind: 'source', version_no: 5 }]), true);
  // The story detail's `source` alone is enough (the list may lag).
  assert.equal(suggestionIsStale(suggested, versions, { kind: 'source', version_no: 6 }), true);
  // A suggestion made after the newest source is current.
  assert.equal(suggestionIsStale({ kind: 'suggested', version_no: 7 }, [...versions, { kind: 'source', version_no: 5 }]), false);
  assert.equal(suggestionIsStale(null, versions), false);
});

test('the site comes from the story URL', () => {
  assert.equal(storySite('https://acme.atlassian.net/browse/PROJ-1'), 'acme.atlassian.net');
  assert.equal(storySite(null), '');
  assert.equal(storySite('not a url'), '');
});

function rewriteTab(stale: boolean, asks: any[]) {
  const body = Array.from({ length: 30 }, (_, i) => `Line ${i + 1} of the rewrite`).join('\n');
  return componentFunctions(new URL('../src/modules/product/RewriteTab.svelte', import.meta.url), ['publish'], {
    suggestedVersion: { id: 'v3', story_id: 's1', version_no: 3, body_md: body },
    publishing: false,
    story: { url: 'https://acme.atlassian.net/browse/PROJ-1' },
    source: { kind: 'source', version_no: stale ? 5 : 1 },
    targetShort: 'PROJ-1', targetLabel: 'Jira PROJ-1 “Reset password”', targetNoun: 'description',
    product: { selectedId: 's1', versions: [], captureSelection: () => () => true, publishVersion: async () => {} },
    publishReview: { ask: async (req: any) => { asks.push(req); return false; } },
    suggestionIsStale, STALE_SUGGESTION,
    publishConflictMessage: () => null, confirmer: { ask: async () => false },
    toasts: { success() {}, warn() {} }, toastError() {},
  });
}

test('the rewrite publish review carries the destination and the full text', async () => {
  const asks: any[] = [];
  await rewriteTab(false, asks).publish();
  assert.equal(asks.length, 1);
  const req = asks[0];
  assert.equal(req.owner, 'rewrite');
  assert.equal(req.where, 'Jira PROJ-1 “Reset password”');
  assert.equal(req.url, 'https://acme.atlassian.net/browse/PROJ-1');
  assert.match(req.what, /^Line 1 of the rewrite\n/);
  assert.match(req.what, /Line 30 of the rewrite$/);
  assert.equal(req.note, null);
  assert.equal(req.danger, true);
});

test('a stale suggestion says plainly that it replaces the current Jira description', async () => {
  const asks: any[] = [];
  await rewriteTab(true, asks).publish();
  const note: string = asks[0].note;
  assert.ok(note.startsWith(STALE_SUGGESTION), note);
  assert.match(note, /replaces the current description of PROJ-1/);
});

test('posting questions reviews every question and the site', async () => {
  const asks: any[] = [];
  const long = 'q'.repeat(500);
  const tab = componentFunctions(new URL('../src/modules/product/QuestionsTab.svelte', import.meta.url), ['postSelected'], {
    selectedIds: new Set(['a', 'b']), postingIds: false, postTarget: 'PROJ-1', isJira: true,
    story: { source_key: 'PROJ-1', title: 'Reset password', url: 'https://acme.atlassian.net/browse/PROJ-1' },
    product: { questions: [{ id: 'a', text: long }, { id: 'b', text: 'Last one?' }], captureSelection: () => () => true },
    publishReview: { ask: async (req: any) => { asks.push(req); return false; } },
    plural: (n: number, w: string) => `${n} ${w}s`,
    toasts: { success() {} }, toastError() {},
  });
  await tab.postSelected();
  const req = asks[0];
  assert.equal(req.owner, 'questions');
  assert.equal(req.where, 'Jira PROJ-1 “Reset password” — as one comment');
  assert.equal(req.url, 'https://acme.atlassian.net/browse/PROJ-1');
  assert.equal(req.what, `• ${long}\n• Last one?`);
});
