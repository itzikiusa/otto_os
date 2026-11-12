import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  NO_FILTERS,
  activeFilterCount,
  cursorAfter,
  gridQuery,
  groupHits,
  matchesGrid,
  mirrorLabel,
  sameCard,
} from '../src/modules/design-hall/designGrid.ts';

function art(id: string, over: Record<string, unknown> = {}): any {
  return {
    id,
    project_id: null,
    workspace_id: 'w',
    studio: 'frames',
    format: 'html',
    title: id,
    status: 'draft',
    head_version_id: `${id}-v1`,
    thumb_blob: null,
    source_kind: null,
    created_by: 'u1',
    updated_at: '2026-11-01T00:00:00Z',
    ...over,
  };
}
const hit = (a: any, story_ids: string[] = []) => ({ artifact: a, snippet: '', score: 0, reference_count: 0, story_ids }) as any;
const plain = (x: unknown) => JSON.parse(JSON.stringify(x));

test('the Archived chip asks the server for archived designs only', () => {
  assert.deepEqual(plain(gridQuery({}, NO_FILTERS)), {});
  assert.deepEqual(plain(gridQuery({}, { ...NO_FILTERS, archived: true, status: 'review' })), {
    status: 'archived',
    include_archived: true,
  });
  assert.deepEqual(plain(gridQuery({}, { ...NO_FILTERS, status: 'review', studio: '3d' })), { studio: '3d', status: 'review' });
});

test('a scope wins over a chip that contradicts it, and Mine never reaches the query', () => {
  assert.deepEqual(plain(gridQuery({ studio: 'site' }, { ...NO_FILTERS, studio: '3d', mine: true })), { studio: 'site' });
  assert.deepEqual(plain(gridQuery({ story_id: 'S', include_children: true, project_id: undefined }, NO_FILTERS)), {
    story_id: 'S',
    include_children: true,
  });
  assert.equal(activeFilterCount(NO_FILTERS), 0);
  assert.equal(activeFilterCount({ studio: 'site', status: 'draft', mine: true, archived: true }), 4);
});

test('a live-patched card leaves the grid when it no longer matches', () => {
  // Archived out of a live list; unarchived out of the Archived list.
  assert.equal(matchesGrid(art('a', { status: 'archived' }), {}, NO_FILTERS, 'u1'), false);
  assert.equal(matchesGrid(art('a', { status: 'draft' }), {}, { ...NO_FILTERS, archived: true }, 'u1'), false);
  assert.equal(matchesGrid(art('a', { status: 'archived' }), {}, { ...NO_FILTERS, archived: true }, 'u1'), true);
  // Moved to another project; a status chip; a studio scope; Mine.
  assert.equal(matchesGrid(art('a', { project_id: 'Q' }), { project_id: 'P' }, NO_FILTERS, 'u1'), false);
  assert.equal(matchesGrid(art('a', { status: 'approved' }), {}, { ...NO_FILTERS, status: 'review' }, 'u1'), false);
  assert.equal(matchesGrid(art('a', { studio: '3d' }), { studio: 'frames' }, NO_FILTERS, 'u1'), false);
  assert.equal(matchesGrid(art('a', { created_by: 'u2' }), {}, { ...NO_FILTERS, mine: true }, 'u1'), false);
  assert.equal(matchesGrid(art('a'), {}, { ...NO_FILTERS, mine: true }, 'u1'), true);
});

test('the next page starts after the last loaded row, newest first', () => {
  assert.equal(cursorAfter([]), undefined);
  assert.equal(cursorAfter([hit(art('a')), hit(art('b', { updated_at: '2026-10-01T00:00:00Z' }))]), '2026-10-01T00:00:00Z|b');
});

test('group by story lists a design under each story it implements, then No story', () => {
  const hits = [hit(art('1'), ['S1', 'S2', 'S1']), hit(art('2'), ['S2']), hit(art('3'))];
  const labels = {
    story: (id: string) => (id === 'S1' ? { key: 'LOY-1', title: 'Landing' } : undefined),
    project: () => undefined,
  };
  const g = groupHits(hits, 'story', labels);
  assert.deepEqual(
    g.map((x) => [x.label, x.href, x.hits.map((h) => h.artifact.id)]),
    [
      ['LOY-1 · Landing', '#/design/story/S1', ['1']],
      ['A linked story', '#/design/story/S2', ['1', '2']],
      ['No story', null, ['3']],
    ],
  );
});

test('group by project keeps first-seen order and ends with Unfiled; Recent is one flat group', () => {
  const hits = [hit(art('1', { project_id: 'P' })), hit(art('2')), hit(art('3', { project_id: 'Q' })), hit(art('4', { project_id: 'P' }))];
  const labels = { story: () => undefined, project: (id: string) => (id === 'P' ? 'Rewards+' : undefined) };
  const g = groupHits(hits, 'project', labels);
  assert.deepEqual(g.map((x) => [x.label, x.hits.length]), [['Rewards+', 2], ['A project', 1], ['Unfiled', 1]]);
  assert.equal(g[0].href, '#/design/p/P');
  const r = groupHits(hits, 'recent', labels);
  assert.equal(r.length, 1);
  assert.equal(r[0].label, null);
  assert.deepEqual(r[0].hits.map((h) => h.artifact.id), ['1', '2', '3', '4']);
});

test('a mirrored design is labelled by where it comes from, never by colour alone', () => {
  assert.deepEqual(plain(mirrorLabel('product_attachment')), { label: 'From a story', icon: 'ticket' });
  assert.deepEqual(plain(mirrorLabel('canvas_scene')), { label: 'From Canvas', icon: 'shapes' });
  assert.equal(mirrorLabel(null), null);
  assert.equal(mirrorLabel('something_new')?.label, 'Mirrored');
});

test('sameCard ignores fields a card does not show and catches the ones it does', () => {
  const a = art('a');
  assert.equal(sameCard(a, { ...a, meta: { x: 1 } }), true);
  assert.equal(sameCard(a, { ...a, title: 'Renamed' }), false);
  assert.equal(sameCard(a, { ...a, status: 'archived' }), false);
  assert.equal(sameCard(a, { ...a, thumb_blob: 'sha' }), false);
});
