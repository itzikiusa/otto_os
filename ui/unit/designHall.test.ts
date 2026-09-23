import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  STUDIOS,
  buildLineage,
  brandColors,
  contrastRatio,
  epicTree,
  filterProjects,
  formatForFile,
  formatOttoUri,
  parseDesignRoute,
  parseOttoUri,
  policyLabel,
  projectStats,
  projectSummary,
  renderKind,
  signalSummary,
  splitLinks,
  titleFromPrompt,
  versionAuthor,
} from '../src/modules/design-hall/model.ts';

// Minimal fixtures in the wire shape (docs/contracts/api.md § Design Hall).
function art(id: string, over: Record<string, unknown> = {}): any {
  return {
    id,
    project_id: null,
    workspace_id: 'w',
    studio: 'frames',
    format: 'html',
    mime: 'text/html',
    title: id,
    status: 'draft',
    head_version_id: `${id}-v1`,
    head_seq: 1,
    approved_version_id: null,
    tags: [],
    thumb_blob: null,
    meta: {},
    source_kind: null,
    source_id: null,
    created_by: 'u1',
    created_by_kind: 'user',
    created_session_id: null,
    created_at: '2026-09-01T00:00:00Z',
    updated_at: '2026-09-01T00:00:00Z',
    ...over,
  };
}
function link(id: string, src: string, rel: string, dst: string, over: Record<string, unknown> = {}): any {
  return {
    id,
    src_artifact_id: src,
    src_version_id: null,
    src_node: null,
    dst_kind: 'artifact',
    dst_id: dst,
    dst_node: null,
    rel,
    policy: 'follow_approved',
    pinned_version_id: null,
    origin: 'explicit',
    broken: false,
    meta: {},
    created_by: 'u1',
    created_at: '2026-09-01T00:00:00Z',
    ...over,
  };
}

test('otto:// URIs parse every selector and reject malformed input', () => {
  assert.deepEqual(parseOttoUri('otto://design/a_7f3c'), { artifactId: 'a_7f3c', selector: { kind: 'default' }, node: null });
  assert.deepEqual(parseOttoUri('otto://design/A1@approved#view:hero'), {
    artifactId: 'A1',
    selector: { kind: 'approved' },
    node: 'view:hero',
  });
  assert.deepEqual(parseOttoUri('otto://design/A1@latest')?.selector, { kind: 'latest' });
  assert.deepEqual(parseOttoUri('otto://design/A1@v12')?.selector, { kind: 'version', seq: 12 });
  for (const bad of ['otto://design/', 'otto://design/A1@v0', 'otto://design/A1@vx', 'otto://design/A1@', 'otto://design/A1#', 'https://x/y', 'otto://vault/A1']) {
    assert.equal(parseOttoUri(bad), null, bad);
  }
  for (const u of ['otto://design/A1', 'otto://design/A1@approved', 'otto://design/A1@v3#n1', 'otto://design/A1@latest#x']) {
    assert.equal(formatOttoUri(parseOttoUri(u)!), u);
  }
});

test('links split into Uses / Used in and lineage orders provenance first with citations', () => {
  const me = 'A';
  const artifacts = [art('B', { title: 'Card 3D' }), art('C', { title: 'Spring promo' }), art('D', { title: 'Social kit' })];
  const links = [
    link('l1', me, 'embeds', 'B'),
    link('l2', me, 'derived_from', 'C', { policy: 'pinned', pinned_version_id: 'C-v12' }),
    link('l3', me, 'implements', 'S1', { dst_kind: 'story', policy: 'follow_latest' }),
    link('l4', 'D', 'references', me, { policy: 'pinned' }),
    link('l5', me, 'references', 'GONE', { broken: true, policy: 'pinned' }),
  ];
  const { uses, usedIn } = splitLinks(me, links, artifacts, (k, id) => (k === 'story' ? `LOY-142 (${id})` : id));
  assert.deepEqual(uses.map((u) => u.link.id), ['l2', 'l1', 'l5', 'l3']);
  assert.equal(uses.find((u) => u.link.id === 'l3')!.label, 'LOY-142 (S1)');
  assert.deepEqual(usedIn.map((u) => u.label), ['Social kit']);

  const lineage = buildLineage(uses, (v) => (v === 'C-v12' ? 12 : null));
  assert.deepEqual(
    lineage.map((l) => [l.relText, l.title, l.version, l.cite, l.broken]),
    [
      ['derived from', 'Spring promo', 'v12', 'R1', false],
      ['embeds', 'Card 3D', 'follows approved', null, false],
      // A broken reference has no artifact on the other end: listed, not cited.
      ['references', 'GONE', 'pinned', null, true],
      ['implements', 'LOY-142 (S1)', 'always latest', null, false],
    ],
  );
  assert.equal(policyLabel('pinned', 9), 'pinned v9');
});

test('route parser maps every Design Hall path and falls back to the lobby', () => {
  assert.deepEqual(parseDesignRoute(['design']), { view: 'lobby' });
  assert.deepEqual(parseDesignRoute(['design', 'spatial']), { view: 'spatial' });
  assert.deepEqual(parseDesignRoute(['design', 'a', 'X1']), { view: 'artifact', id: 'X1' });
  assert.deepEqual(parseDesignRoute(['design', 'p', 'P1']), { view: 'project', id: 'P1' });
  assert.deepEqual(parseDesignRoute(['design', 'studio', '3d']), { view: 'studio', id: '3d' });
  assert.deepEqual(parseDesignRoute(['design', 'studio', 'brand']), { view: 'brand' });
  assert.deepEqual(parseDesignRoute(['design', 'studio', 'nope']), { view: 'lobby' });
  assert.deepEqual(parseDesignRoute(['design', 'learned', 'rules']), { view: 'learned', tab: 'rules' });
  assert.deepEqual(parseDesignRoute(['design', 'learned', 'x']), { view: 'learned', tab: 'pending' });
  assert.deepEqual(parseDesignRoute(['design', 'learned', 'signals']), { view: 'learned', tab: 'signals' });
  assert.deepEqual(parseDesignRoute(['design', 'learned', 'settings']), { view: 'learned', tab: 'settings' });
  assert.deepEqual(parseDesignRoute(['design', 'a']), { view: 'lobby' });
});

test('there are exactly seven studios with unique ids and every format renders', () => {
  assert.equal(STUDIOS.length, 7);
  assert.equal(new Set(STUDIOS.map((s) => s.id)).size, 7);
  assert.equal(renderKind('otto-canvas'), 'canvas');
  assert.equal(renderKind('glb'), 'model');
  assert.equal(renderKind('webp'), 'image');
  assert.equal(renderKind('weird'), 'other');
  assert.equal(formatForFile('hero.GLB'), 'glb');
  assert.equal(formatForFile('x.jpg'), 'jpeg');
  assert.equal(formatForFile('page', 'text/html'), 'html');
  assert.equal(formatForFile('notes.txt'), null);
});

test('project stats, summary and filters', () => {
  const p = { id: 'P', name: 'Rewards+', created_by: 'u2', archived: false, epic_story_id: 'E', updated_at: '2026-09-01T00:00:00Z' } as any;
  const q = { id: 'Q', name: 'Other', created_by: 'u2', archived: false, epic_story_id: null, updated_at: '2026-09-01T00:00:00Z' } as any;
  const arts = [
    art('1', { project_id: 'P', studio: 'site', status: 'review', updated_at: '2026-09-03T00:00:00Z' }),
    art('2', { project_id: 'P', studio: '3d', status: 'approved' }),
    art('3', { project_id: 'P', studio: '3d', status: 'draft', created_by: 'me' }),
    art('4', { project_id: 'P', status: 'archived' }),
    art('5', { project_id: 'Q', status: 'shipped' }),
  ];
  const s = projectStats(p, arts);
  assert.equal(s.artifacts, 3);
  assert.deepEqual(s.studios, ['site', '3d']);
  assert.equal(s.mosaic[0].id, '1');
  assert.equal(s.updatedAt, '2026-09-03T00:00:00Z');
  assert.deepEqual(projectSummary(s), { text: '1 in review · 1 draft', tone: 'warn' });
  assert.deepEqual(projectSummary(projectStats(q, arts)), { text: 'All shipped', tone: 'info' });
  assert.deepEqual(filterProjects([p, q], arts, 'mine', 'me').map((x) => x.id), ['P']);
  assert.deepEqual(filterProjects([p, q], arts, 'epic', 'me').map((x) => x.id), ['P']);
  assert.deepEqual(filterProjects([p, q], arts, 'shipped', 'me').map((x) => x.id), ['Q']);
});

test('epic tree groups implementing artifacts under their epic with per-studio counts', () => {
  const stories = [
    { id: 'E', source_key: 'LOY-120', title: 'Loyalty relaunch', parent_id: null },
    { id: 'S1', source_key: 'LOY-142', title: 'Landing', parent_id: 'E' },
    { id: 'S2', source_key: 'LOY-147', title: 'Tier ladder', parent_id: 'E' },
    { id: 'S3', source_key: 'LOY-151', title: 'Social kit', parent_id: 'E' },
    { id: 'T', source_key: 'CHK-1', title: 'Solo', parent_id: null },
  ];
  const hit = (a: any, story_ids: string[]) => ({ artifact: a, snippet: '', score: 0, reference_count: 0, story_ids });
  const rows = epicTree(
    [
      hit(art('1', { studio: 'site' }), ['S1']),
      hit(art('2', { studio: '3d' }), ['S1']),
      hit(art('3', { studio: 'frames' }), ['S2']),
      hit(art('4', { studio: 'frames', status: 'archived' }), ['S2']),
      hit(art('5', { studio: 'whiteboard' }), ['T', 'UNKNOWN']),
    ],
    stories,
  );
  // Busiest epic first.
  assert.deepEqual(rows.map((r) => r.story.source_key), ['LOY-120', 'CHK-1']);
  const loy = rows[0];
  assert.equal(loy.childCount, 3);
  assert.deepEqual(loy.children.map((c) => [c.story.source_key, c.counts]), [
    ['LOY-142', { site: 1, '3d': 1 }],
    ['LOY-147', { frames: 1 }],
  ]);
  assert.deepEqual(rows[1].own, { whiteboard: 1 });
});

test('signals read as one line from the bounded server payloads', () => {
  const sig = (kind: string, payload: Record<string, unknown>) => ({ kind, payload }) as any;
  assert.equal(signalSummary(sig('status_change', { from: 'review', to: 'approved', seq: 8 })), 'Approved v8');
  assert.equal(signalSummary(sig('status_change', { from: 'draft', to: 'review' })), 'Draft → In review');
  assert.equal(
    signalSummary(sig('edit_after_draft', { summary: { kind: 'json', changed_total: 3, changed_paths: ['a', 'b', 'c'] } })),
    'Edited 3 fields after an agent draft',
  );
  assert.equal(
    signalSummary(sig('edit_after_draft', { summary: { kind: 'text', lines_added: 2, lines_removed: 1 } })),
    'Changed +2 −1 lines after an agent draft',
  );
  assert.equal(signalSummary(sig('variant_chosen', { chosen_seq: 12, over_seq: 15 })), 'Chose v12 over v15');
  assert.equal(signalSummary(sig('variant_rejected', { rejected_seq: 4, reason: 'kept mine' })), 'Rejected v4 · kept mine');
  assert.equal(signalSummary(sig('review_comment', { summary: 'Tier cards feel crowded' })), 'Tier cards feel crowded');
});

test('small helpers: titles, authors, brand colours and contrast', () => {
  assert.equal(titleFromPrompt('  a launch page for Rewards+. With a 3D hero'), 'A launch page for Rewards+');
  assert.equal(titleFromPrompt(''), 'Untitled design');
  assert.equal(titleFromPrompt('x'.repeat(80)).length, 60);
  const v = (author_kind: string, author_id: string, kind = 'autosave') => ({ author_kind, author_id, kind }) as any;
  assert.equal(versionAuthor(v('agent', 's1'), 'me'), 'Otto');
  assert.equal(versionAuthor(v('user', 'me'), 'me'), 'you');
  assert.equal(versionAuthor(v('system', 'system:import', 'sync'), 'me'), 'sync');
  assert.equal(versionAuthor(v('user', '01J9ZZZZ'), 'me'), 'teammate');
  assert.deepEqual(brandColors({ color: { primary: { $value: '#5B3DF5' }, bad: { $value: 'red' }, ink: '#14122B' } }), [
    { name: 'primary', value: '#5B3DF5' },
    { name: 'ink', value: '#14122B' },
  ]);
  assert.equal(contrastRatio('#ffffff', '#000000'), 21);
  assert.equal(contrastRatio('#fff', '#fff'), 1);
  assert.equal(contrastRatio('nope', '#fff'), null);
});
