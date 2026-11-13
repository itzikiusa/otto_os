import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  legacyEditorNote,
  opensInHall,
  pendingSourceVersion,
  sideVersionNote,
  sourceNoun,
  syncErrorOf,
} from '../src/modules/design-hall/mirrorSync.ts';
import { conflictHead } from '../src/modules/design-hall/versionRows.ts';

// Round 3: a Hall copy that falls behind its Canvas board / story attachment
// (`meta.sync_error`), the change that arrives from the source after "Edit
// here", where `#/canvas/<id>` opens a board, and the version a save conflict
// is resolved against.

type V = { id: string; seq: number; kind: string; branch: string; created_at: string; provenance: Record<string, unknown> };
const v = (seq: number, branch = 'main', kind = 'autosave', extra: Partial<V> = {}): V => ({
  id: `v${seq}`, seq, kind, branch, created_at: `2026-11-13T10:${String(seq).padStart(2, '0')}:00Z`, provenance: {}, ...extra,
});

test('conflictHead: the real head, even when a side version is numbered newer', () => {
  // The head moved to v3 (an agent commit), then a variant / source sync v4.
  const head = v(3, 'main', 'agent');
  const side = v(4, 'variant/sync-20261113T100400000Z/1', 'sync');
  const detail = { artifact: { head_version_id: 'v3' }, head };
  assert.equal(conflictHead(detail as never, [side, head] as never[])?.id, 'v3');
  // No head row in the detail: the head among the recent ones…
  assert.equal(conflictHead({ artifact: { head_version_id: 'v3' }, head: null } as never, [side, head, v(2)] as never[])?.id, 'v3');
  // …else the newest version on main, never the side one.
  assert.equal(conflictHead({ artifact: { head_version_id: 'gone' }, head: null } as never, [side, v(2), head] as never[])?.id, 'v3');
  assert.equal(conflictHead({ artifact: { head_version_id: null }, head: null } as never, [side] as never[]), null);
  // A detail head that is not the artifact's head (stale) is not trusted.
  assert.equal(conflictHead({ artifact: { head_version_id: 'v3' }, head: side } as never, [head] as never[])?.id, 'v3');
});

test('syncErrorOf: a copy behind its source, with a fallback reason', () => {
  assert.deepEqual(syncErrorOf({ meta: { sync_error: { reason: 'one of its images is missing', at: '2026-11-13T00:00:00Z' } } }), {
    reason: 'one of its images is missing', at: '2026-11-13T00:00:00Z',
  });
  assert.equal(syncErrorOf({ meta: { sync_error: {} } })?.reason, 'its latest change couldn’t be copied');
  assert.equal(syncErrorOf({ meta: {} }), null);
  assert.equal(syncErrorOf({ meta: { sync_error: 'x' } }), null);
  assert.equal(syncErrorOf(null), null);
  assert.equal(sourceNoun('canvas_scene'), 'Canvas board');
  assert.equal(sourceNoun('product_attachment'), 'story attachment');
});

test('pendingSourceVersion: the newest unaccepted source change after the take-over', () => {
  const adopted = { source_kind: 'canvas_scene', meta: { adopted: true, adopted_at: '2026-11-13T10:02:30Z' } } as never;
  const before = v(2, 'variant/sync-a/1', 'sync');
  const after = v(4, 'variant/sync-b/1', 'sync');
  const variant = v(5, 'variant/run1/1', 'agent');
  const versions = [variant, after, v(3), before, v(1, 'main', 'import')];
  assert.equal(pendingSourceVersion(versions as never[], adopted)?.id, 'v4');
  // Accepted already (a main version names it) → nothing pending.
  const accepted = v(6, 'main', 'named', { provenance: { accepted_variant: { version_id: 'v4' } } });
  assert.equal(pendingSourceVersion([accepted, ...versions] as never[], adopted), null);
  // Not adopted (a mirror follows its source) or no source → never.
  assert.equal(pendingSourceVersion(versions as never[], { source_kind: 'canvas_scene', meta: {} } as never), null);
  assert.equal(pendingSourceVersion(versions as never[], { source_kind: null, meta: { adopted: true } } as never), null);
});

test('sideVersionNote: names a source sync and a variant; main rows say nothing', () => {
  assert.equal(sideVersionNote(v(4, 'variant/sync-b/1', 'sync') as never, 'canvas_scene'), 'from the Canvas board');
  assert.equal(sideVersionNote(v(4, 'variant/sync-b/1', 'sync') as never, 'product_attachment'), 'from the story attachment');
  assert.equal(sideVersionNote(v(5, 'variant/run1/2', 'agent') as never, 'canvas_scene'), 'variant');
  assert.equal(sideVersionNote(v(3) as never, 'canvas_scene'), null);
});

test('#/canvas: a board without an up-to-date Hall copy opens in the Canvas editor, saying why', () => {
  assert.equal(opensInHall({ id: 'A', adopted: false, behind: null }), true);
  assert.equal(opensInHall({ id: 'A', adopted: false, behind: 'one of its images is missing' }), false);
  // Adopted: the Hall owns it (its banner says it is behind).
  assert.equal(opensInHall({ id: 'A', adopted: true, behind: 'one of its images is missing' }), true);
  assert.equal(opensInHall(null), false);
  assert.equal(opensInHall('unavailable'), false);
  assert.match(legacyEditorNote(null), /isn’t in Design Hall yet.*Canvas editor/);
  assert.match(legacyEditorNote({ id: 'A', adopted: false, behind: 'with its images it is 31 MB, over the 25 MB a design can hold' }), /behind \(with its images it is 31 MB/);
  assert.match(legacyEditorNote('unavailable'), /can’t show this board/);
});
