import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  canAutosave,
  isMirrored,
  isMirroredError,
  mergeOlder,
  mirrorSourceLabel,
  saveIndicatorText,
  statusPillOf,
  totalCountOf,
  versionKindLabel,
  versionRows,
} from '../src/modules/design-hall/versionRows.ts';
import { blenderSourceOf, sourceExt } from '../src/modules/design-hall/exportNames.ts';

// Design Hall artifact view (round 2): the Versions panel's rows, the status
// pill of an approved-then-edited design, mirror detection, paging and the
// autosave gate — the pure halves of ArtifactView / VersionsPanel.

type V = { id: string; seq: number; kind: string };
const v = (seq: number, kind = 'autosave'): V => ({ id: `v${seq}`, seq, kind });
const ids = (rows: ReturnType<typeof versionRows>) =>
  rows.map((r) => (r.kind === 'fold' ? `fold(${r.versions.map((x) => x.seq).join(',')})` : `v${r.version.seq}`));

test('versionRows: newest first, runs of 2+ autosaves fold, a lone autosave stays a row', () => {
  const vs = [v(1, 'import'), v(2), v(3), v(4), v(5, 'named'), v(6)] as never[];
  assert.deepEqual(ids(versionRows(vs)), ['v6', 'v5', 'fold(4,3,2)', 'v1']);
});

test('versionRows: head, approved and the viewed version never hide inside a fold', () => {
  const vs = [v(1), v(2), v(3), v(4), v(5)] as never[];
  assert.deepEqual(ids(versionRows(vs, { headId: 'v5', approvedId: 'v3' })), ['v5', 'v4', 'v3', 'fold(2,1)']);
  assert.deepEqual(ids(versionRows(vs, { headId: 'v5', keep: ['v2'] })), ['v5', 'fold(4,3)', 'v2', 'v1']);
});

test('versionRows: an expanded fold lists its versions', () => {
  const vs = [v(1, 'named'), v(2), v(3)] as never[];
  const folded = versionRows(vs);
  const key = folded.find((r) => r.kind === 'fold')!;
  assert.equal(key.kind, 'fold');
  const open = versionRows(vs, { expanded: new Set([(key as { key: string }).key]) });
  assert.deepEqual(ids(open), ['v3', 'v2', 'v1']);
});

test('versionKindLabel names every version kind', () => {
  assert.deepEqual(['named', 'autosave', 'agent', 'restore', 'sync', 'import'].map(versionKindLabel), ['Named', 'Autosave', 'Otto', 'Restore', 'Sync', 'Import']);
});

test('statusPillOf: an approved design edited afterwards says “Approved v5 · edited”, not Draft', () => {
  assert.deepEqual(statusPillOf({ status: 'approved', approved_version_id: 'v5' }, 'v7', 5), { status: 'approved', suffix: ' v5 · edited' });
  assert.deepEqual(statusPillOf({ status: 'approved', approved_version_id: 'v5' }, 'v5', 5), { status: 'approved', suffix: ' v5' });
  assert.deepEqual(statusPillOf({ status: 'draft', approved_version_id: null }, 'v2', null), { status: 'draft', suffix: '' });
  assert.deepEqual(statusPillOf({ status: 'approved', approved_version_id: 'v5' }, 'v6', null), { status: 'approved', suffix: ' · edited' });
});

test('isMirrored: a source without adoption is read-only; adopted or unsourced is not', () => {
  assert.equal(isMirrored({ source_kind: 'product_attachment', meta: {} }), true);
  assert.equal(isMirrored({ source_kind: 'canvas_scene', meta: { adopted: true } }), false);
  assert.equal(isMirrored({ source_kind: null, meta: {} }), false);
  assert.equal(isMirrored(null), false);
  assert.equal(mirrorSourceLabel('canvas_scene'), 'a Canvas board');
  assert.equal(mirrorSourceLabel('product_attachment'), 'a story attachment');
});

test('isMirroredError: only a 409 with code `mirrored`', () => {
  assert.equal(isMirroredError({ status: 409, code: 'mirrored' }), true);
  assert.equal(isMirroredError({ status: 409, code: 'conflict' }), false);
  assert.equal(isMirroredError({ status: 400, code: 'mirrored' }), false);
  assert.equal(isMirroredError(new Error('x')), false);
  assert.equal(isMirroredError(null), false);
});

test('paging: X-Total-Count parses; an older page merges under the held versions without duplicates', () => {
  assert.equal(totalCountOf('1234'), 1234);
  assert.equal(totalCountOf(null), null);
  assert.equal(totalCountOf(''), null);
  assert.equal(totalCountOf('-1'), null);
  assert.equal(totalCountOf('x'), null);
  const merged = mergeOlder([v(5), v(4)] as never[], [v(4), v(3)] as never[]) as unknown as V[];
  assert.deepEqual(merged.map((x) => x.seq), [5, 4, 3]);
});

test('canAutosave: never for a mirror, an archived or read-only design, or while viewing a version', () => {
  const ok = { readonly: false, mirrored: false, archived: false, viewing: false };
  assert.equal(canAutosave(ok), true);
  for (const k of ['readonly', 'mirrored', 'archived', 'viewing'] as const) assert.equal(canAutosave({ ...ok, [k]: true }), false, k);
});

test('saveIndicatorText: Saved / Saving… / Couldn’t save', () => {
  assert.equal(saveIndicatorText('saved', false), 'Saved');
  assert.equal(saveIndicatorText('idle', false), 'Saved');
  assert.equal(saveIndicatorText('pending', true), 'Saving…');
  assert.equal(saveIndicatorText('saving', true), 'Saving…');
  assert.equal(saveIndicatorText('error', true), 'Couldn’t save');
  // A save that ended without storing the edits (a dismissed conflict) is not “Saved”.
  assert.equal(saveIndicatorText('saved', true), 'Couldn’t save');
});

test('export: the Blender script only for a scene that came from a Product story', () => {
  const scene = { format: 'scene3d', source_kind: 'product_attachment', source_id: 'att1', meta: { imported_from: { story_id: 'st1' } } };
  assert.deepEqual(blenderSourceOf(scene as never), { storyId: 'st1', attachmentId: 'att1' });
  assert.equal(blenderSourceOf({ ...scene, source_kind: null } as never), null);
  assert.equal(blenderSourceOf({ ...scene, format: 'html' } as never), null);
  assert.equal(blenderSourceOf({ ...scene, meta: {} } as never), null);
  assert.equal(sourceExt('mermaid'), 'mmd');
  assert.equal(sourceExt('scene3d'), 'scene3d.json');
  assert.equal(sourceExt('otto-site'), 'json');
});
