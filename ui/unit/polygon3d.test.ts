import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  defaultCameraForAnchor,
  isBackfacing,
  mergeFaceIntoSelection,
  ndcToViewportPx,
  normalizeFaceIndices,
  pin3dLabel,
  pin3dWords,
  type Pin3dAnchor,
} from '../src/lib/design/scene3d/pin3d.ts';
import { formatCount, polyBudgetStatus } from '../src/lib/design/scene3d/meshInspector.ts';
import { formatAnimDuration } from '../src/lib/design/scene3d/rigPlayer.ts';
import { viewportHasWork } from '../src/lib/design/scene3d/viewportLoop.ts';
import { pins3dFor, threadsOf } from '../src/modules/design-hall/comments/pins.ts';
import { formatPinsMarkdown, formatRegenerateFeedbackPrompt } from '../src/modules/design-hall/comments/pinsDispatch.ts';
import {
  HUNYUAN_TERRITORY_WARNING,
  licenseWarningFor,
  pickDefault3dProvider,
  split3dProviders,
} from '../src/modules/design-hall/studio3d/providers.ts';

test('normalizeFaceIndices deduplicates, drops invalid entries, and caps at max', () => {
  assert.deepEqual(normalizeFaceIndices([5, 2, 5, -1, 1.5, 9, 2]), [5, 2, 9]);
  assert.equal(normalizeFaceIndices(Array.from({ length: 3000 }, (_, i) => i)).length, 2048);
});

test('mergeFaceIntoSelection toggles/expands polygon clusters on the same mesh and resets across meshes', () => {
  const f1: Pin3dAnchor = {
    selection_kind: 'polygon',
    mesh_name: 'HeroHelmet',
    object_id: 'model',
    face_index: 42,
    face_indices: [42],
    position: [0.1, 1.4, 0.2],
  };
  const f2: Pin3dAnchor = {
    ...f1,
    face_index: 43,
    face_indices: [43],
  };
  const merged = mergeFaceIntoSelection(f1, f2);
  assert.equal(merged.selection_kind, 'region');
  assert.deepEqual(merged.face_indices, [42, 43]);
  assert.equal(merged.face_index, 42);

  // Clicking an already-selected face removes it and returns to single polygon.
  const toggled = mergeFaceIntoSelection(merged, f2);
  assert.equal(toggled.selection_kind, 'polygon');
  assert.deepEqual(toggled.face_indices, [42]);

  // Clicking a different mesh replaces the selection.
  const otherMesh: Pin3dAnchor = { ...f2, mesh_name: 'Visor' };
  const switched = mergeFaceIntoSelection(merged, otherMesh);
  assert.equal(switched.mesh_name, 'Visor');
  assert.deepEqual(switched.face_indices, [43]);
});

test('pin3dLabel and pin3dWords format polygon, region, mesh and point anchors', () => {
  const poly: Pin3dAnchor = {
    selection_kind: 'polygon',
    mesh_name: 'Shield',
    face_index: 412,
    face_indices: [412],
    vertices: [10, 11, 12],
    position: [0.1234, 1.4567, -0.0891],
  };
  assert.equal(pin3dLabel(poly), 'Shield · Face #412');
  assert.match(pin3dWords(poly), /3D polygon on "Shield" · face #412 · verts \[10, 11, 12\] · at \(0\.123, 1\.457, -0\.089\)/);

  const region: Pin3dAnchor = { ...poly, selection_kind: 'region', face_indices: [412, 413, 414] };
  assert.equal(pin3dLabel(region), 'Shield · 3 faces (#412…)');
  assert.match(pin3dWords(region), /\(\+2 faces\)/);

  const mesh: Pin3dAnchor = { selection_kind: 'mesh', mesh_name: 'Pauldron', position: [0, 1, 0] };
  assert.equal(pin3dLabel(mesh), 'Mesh: Pauldron');
});

test('ndcToViewportPx and isBackfacing compute viewport coordinates and backface visibility', () => {
  const center = ndcToViewportPx(0, 0, 0.2, 800, 600);
  assert.deepEqual(center, { x: 400, y: 300, inFront: true, inBounds: true });

  const behind = ndcToViewportPx(0, 0, 1.5, 800, 600);
  assert.equal(behind.inFront, false);
  assert.equal(behind.inBounds, false);

  assert.equal(isBackfacing([0, 0, 0], [0, 0, 1], [0, 0, 5]), false);
  assert.equal(isBackfacing([0, 0, 0], [0, 0, -1], [0, 0, 5]), true);
});

test('defaultCameraForAnchor preserves saved camera or derives one from position + normal', () => {
  const saved = { pos: [1, 2, 3] as [number, number, number], target: [0, 0, 0] as [number, number, number], fov: 50 };
  assert.deepEqual(
    defaultCameraForAnchor({ selection_kind: 'point', position: [0, 0, 0], camera: saved }),
    saved,
  );
  const derived = defaultCameraForAnchor({ selection_kind: 'polygon', position: [0, 1, 0], normal: [0, 0, 1] }, 2);
  assert.ok(derived);
  assert.deepEqual(derived.target, [0, 1, 0]);
  assert.ok(derived.pos[2] > 1.5);
});

test('polyBudgetStatus and formatCount report game polygon budget thresholds accurately', () => {
  assert.equal(formatCount(850), '850');
  assert.equal(formatCount(12_400), '12.4k');
  assert.equal(formatCount(1_250_000), '1.25M');

  assert.equal(polyBudgetStatus(3000, 5000).tone, 'ok');
  assert.equal(polyBudgetStatus(4200, 5000).tone, 'warn');
  assert.equal(polyBudgetStatus(6500, 5000).tone, 'over');
});

test('pins3dFor, formatPinsMarkdown, and formatRegenerateFeedbackPrompt include 3D polygon metadata', () => {
  const c1 = {
    id: 'c1',
    artifact_id: 'a1',
    workspace_id: 'w1',
    version_id: 'v2',
    version_seq: 2,
    parent_id: null,
    author_kind: 'user' as const,
    author_id: 'u1',
    author_name: 'Maya',
    session_id: null,
    body: 'Bevel this sharp visor edge',
    pin: null,
    pin_3d: {
      selection_kind: 'polygon' as const,
      mesh_name: 'HeroHelmet',
      face_index: 412,
      face_indices: [412],
      position: [0.1, 1.5, 0.2] as [number, number, number],
    },
    resolved: false,
    resolved_by: null,
    resolved_by_name: null,
    resolved_at: null,
    edited_at: null,
    created_at: '2026-11-19T10:00:00Z',
    updated_at: '2026-11-19T10:00:00Z',
  };
  const threads = threadsOf([c1]);
  const drawn = pins3dFor(threads, 'v2');
  assert.equal(drawn.length, 1);
  assert.equal(drawn[0].muted, false);

  const md = formatPinsMarkdown({ artifactId: 'a1', artifactTitle: 'Knight', versionSeq: 2, threads });
  assert.match(md, /3D Pin #1/);
  assert.match(md, /HeroHelmet/);
  assert.match(md, /face #412/);

  const regen = formatRegenerateFeedbackPrompt('Knight', threads);
  assert.match(regen, /Refine 3D model "Knight"/);
  assert.match(regen, /Bevel this sharp visor edge/);
});

test('pickDefault3dProvider never auto-selects hunyuan3d-omni and licenseWarningFor surfaces territory notice', () => {
  const onlyHunyuan = [{ id: 'hunyuan3d-omni', kind: 'hunyuan3d_omni', enabled: true, ready: true }];
  assert.equal(pickDefault3dProvider(onlyHunyuan), 'otto-forge');
  assert.equal(licenseWarningFor(onlyHunyuan[0]), HUNYUAN_TERRITORY_WARNING);

  const withForgeAndTrellis = [
    { id: 'hunyuan3d-omni', kind: 'hunyuan3d_omni', enabled: true, ready: true },
    { id: 'otto-forge', kind: 'custom_cli', enabled: true, ready: true },
    { id: 'trellis-2', kind: 'trellis_2', enabled: true, ready: true },
  ];
  assert.equal(pickDefault3dProvider(withForgeAndTrellis), 'otto-forge');
  assert.equal(licenseWarningFor(withForgeAndTrellis[2]), null);
});

test('split3dProviders shows Otto Forge alone until another provider is set up; the rest wait under Advanced', () => {
  const forgeMissing = { id: 'otto-forge', kind: 'custom_cli', enabled: true, ready: false };
  const trellis = { id: 'trellis-2', kind: 'trellis_2', enabled: true, ready: false };
  const hunyuan = { id: 'hunyuan3d-omni', kind: 'hunyuan3d_omni', enabled: true, ready: false };
  const seeded = split3dProviders([forgeMissing, trellis, hunyuan]);
  assert.deepEqual(seeded.shown.map((p) => p.id), ['otto-forge'], 'Forge shows even before it is installed');
  assert.deepEqual(seeded.advanced.map((p) => p.id), ['trellis-2', 'hunyuan3d-omni']);
  assert.equal(pickDefault3dProvider([forgeMissing, trellis, hunyuan]), 'otto-forge');

  const configured = split3dProviders([forgeMissing, { ...trellis, ready: true }, { ...hunyuan, ready: true, enabled: false }]);
  assert.deepEqual(configured.shown.map((p) => p.id), ['otto-forge', 'trellis-2'], 'a set-up provider shows normally');
  assert.deepEqual(configured.advanced.map((p) => p.id), ['hunyuan3d-omni'], 'a disabled one stays under Advanced');
});

test('rig playback keeps viewport render loop active and formats clip durations', () => {
  assert.equal(viewportHasWork({ tween: false, camTween: false, turntable: false, dragging: false, animating: true }), true);
  assert.equal(viewportHasWork({ tween: false, camTween: false, turntable: false, dragging: false, animating: false }), false);
  assert.equal(formatAnimDuration(2.35), '2.4s');
  assert.equal(formatAnimDuration(0), '0.0s');
});
