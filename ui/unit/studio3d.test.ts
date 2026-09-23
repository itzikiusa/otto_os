import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  colorLabel,
  isTokenRef,
  normalizeHex,
  resolveColor,
  tokenName,
} from '../src/modules/product/design/scene3d/tokens.ts';
import { resolveToken } from '../src/modules/design-hall/brand/tokens.ts';
import {
  applyMaterialPreset,
  budgetStatus,
  envPreset,
  formatBytes,
  materialPreset,
  MATERIAL_PRESETS,
  resolveMaterial,
  WEB_BUDGET_BYTES,
} from '../src/modules/product/design/scene3d/presets.ts';
import {
  ease,
  initialState,
  lerpColor,
  lerpPose,
  lerpPoses,
  statePoses,
  stateTiming,
  transitionProgress,
} from '../src/modules/product/design/scene3d/states.ts';
import type { Scene3dDoc } from '../src/modules/product/design/scene3d/types.ts';

// A Brand Kit v1 (`otto-brand/1`) document — Brand Kit owns resolution.
const brand = {
  schema: 'otto-brand/1',
  name: 'Acme',
  color: {
    violet: { $value: '#5B3DF5' },
    amber: { $value: '#fa3' },
  },
};
const kit = (ref: string) => resolveToken(brand, ref);

// ── tokens ───────────────────────────────────────────────────────────────────

test('token references parse and label (Brand Kit name grammar)', () => {
  assert.equal(isTokenRef('token:color.violet'), true);
  assert.equal(isTokenRef('token:color.brand-violet_2'), true);
  assert.equal(isTokenRef('token:violet'), false);
  assert.equal(isTokenRef('token:color.vio let'), false);
  assert.equal(isTokenRef('token:color.brand.violet'), false, 'no dotted names');
  assert.equal(isTokenRef('token:color.-x'), false, 'must start alphanumeric');
  assert.equal(isTokenRef('#5b3df5'), false);
  assert.equal(tokenName('token:color.violet'), 'violet');
  assert.equal(colorLabel('token:color.brand-violet'), 'Brand brand violet');
  assert.equal(colorLabel('#5b3df5'), '#5b3df5');
  assert.equal(colorLabel(undefined), 'Default');
  assert.equal(normalizeHex('#FA3'), '#ffaa33');
  assert.equal(normalizeHex('#111827ff'), '#111827');
  assert.equal(normalizeHex('red'), null);
});

test('resolveColor goes through the kit resolver and falls back instead of breaking the render', () => {
  assert.equal(resolveColor('token:color.violet', kit, '#000000'), '#5b3df5');
  assert.equal(resolveColor('token:color.amber', kit, '#000000'), '#ffaa33');
  assert.equal(resolveColor('token:color.missing', kit, '#123456'), '#123456', 'unknown token → fallback');
  assert.equal(resolveColor('token:color.violet', null, '#94a3b8'), '#94a3b8', 'no kit → fallback');
  assert.equal(resolveColor('#ABCDEF', null, '#000000'), '#abcdef');
  assert.equal(resolveColor(undefined, kit, '#123456'), '#123456');
});

// ── material presets ─────────────────────────────────────────────────────────

const hexOnly = (ref: string | undefined, fallback: string) => resolveColor(ref, kit, fallback);

test('resolveMaterial: explicit field > preset default > renderer default', () => {
  const plain = resolveMaterial(undefined, hexOnly);
  assert.equal(plain.color, '#94a3b8');
  assert.equal(plain.metalness, 0.1);
  assert.equal(plain.roughness, 0.7);
  assert.equal(plain.physical, false);

  const glossy = resolveMaterial({ preset: 'glossy-plastic', color: 'token:color.violet' }, hexOnly);
  assert.equal(glossy.color, '#5b3df5', 'brand token resolved');
  assert.equal(glossy.roughness, 0.35);
  assert.equal(glossy.clearcoat, 1);
  assert.equal(glossy.physical, true);

  const tweaked = resolveMaterial({ preset: 'glossy-plastic', roughness: 0.6 }, hexOnly);
  assert.equal(tweaked.roughness, 0.6, 'explicit field wins over the preset');

  const metal = resolveMaterial({ preset: 'brushed-metal' }, hexOnly);
  assert.equal(metal.metalness, 1);
  assert.equal(metal.color, '#c7cad1', 'preset colour when the object has none');

  const glass = resolveMaterial({ preset: 'frosted-glass', opacity: 0.5 }, hexOnly);
  assert.equal(glass.transmission, 1);
  assert.equal(glass.ior, 1.45);
  assert.equal(glass.transparent, true);

  const clamped = resolveMaterial({ roughness: 7, ior: 9, emissive_intensity: -3 }, hexOnly);
  assert.equal(clamped.roughness, 1);
  assert.equal(clamped.ior, 2.333);
  assert.equal(clamped.emissiveIntensity, 0);
});

test('applyMaterialPreset clears what the preset owns and keeps colour/opacity', () => {
  const m = applyMaterialPreset({ color: 'token:color.violet', roughness: 0.9, metalness: 0.4, opacity: 0.8, clearcoat: 0.2 }, 'brushed-metal');
  assert.deepEqual(m, { color: 'token:color.violet', opacity: 0.8, preset: 'brushed-metal' });
  assert.deepEqual(applyMaterialPreset({ preset: 'satin', roughness: 0.3 }, null), { roughness: 0.3 });
  assert.deepEqual(applyMaterialPreset(undefined, 'matte-paper'), { preset: 'matte-paper' });
  assert.equal(MATERIAL_PRESETS.length, 5);
  assert.equal(materialPreset('frosted-glass')?.label, 'Frosted glass');
  assert.equal(materialPreset('chrome'), null);
  assert.equal(envPreset('studio-soft')?.label, 'Studio soft');
  assert.equal(envPreset('forest'), null);
});

test('web budget readout', () => {
  assert.equal(formatBytes(512), '512 B');
  assert.equal(formatBytes(1536), '1.5 KB');
  assert.equal(formatBytes(3 * 1024 * 1024), '3.00 MB');
  assert.equal(budgetStatus(WEB_BUDGET_BYTES / 2).tone, 'ok');
  assert.equal(budgetStatus(WEB_BUDGET_BYTES * 0.9).tone, 'warn');
  const over = budgetStatus(WEB_BUDGET_BYTES * 2);
  assert.equal(over.tone, 'over');
  assert.equal(over.pct, 200);
});

// ── states + tweening ────────────────────────────────────────────────────────

const doc: Scene3dDoc = {
  type: 'otto-scene3d',
  version: 2,
  camera: { position: [3, 2, 4], target: [0, 1, 0], fov: 35 },
  lights: [],
  groups: [],
  objects: [
    { id: 'card', name: 'Card', type: 'box', position: [0, 1, 0], rotation: [0, 0, 0], scale: [1.6, 1, 0.03] },
    { id: 'chip', name: 'Chip', type: 'box', position: [0.4, 1.1, 0.02], rotation: [0, 0, 0], scale: [0.2, 0.15, 0.01], visible: false },
  ],
  states: [
    { id: 'idle', name: 'Idle' },
    { id: 'flipped', name: 'Flipped', duration_ms: 500, easing: 'ease-in-out', overrides: { card: { rotation: [0, 180, 0], color: 'token:color.amber' }, chip: { visible: true, opacity: 0.5 } } },
  ],
  default_state: 'idle',
};

test('easing curves start at 0, end at 1 and keep their shape', () => {
  for (const e of ['linear', 'ease-in', 'ease-out', 'ease-in-out', 'spring'] as const) {
    assert.equal(ease(e, 0), 0, e);
    assert.equal(ease(e, 1), 1, e);
  }
  assert.equal(ease('linear', 0.25), 0.25);
  assert.ok(ease('ease-in', 0.5) < 0.5, 'ease-in starts slow');
  assert.ok(ease('ease-out', 0.5) > 0.5, 'ease-out starts fast');
  assert.equal(ease('ease-in-out', 0.5), 0.5, 'symmetric');
  assert.ok(Math.max(...[0.2, 0.3, 0.4].map((t) => ease('spring', t))) > 1, 'spring overshoots');
  assert.equal(ease(undefined, 2), 1, 'clamped');
  assert.equal(transitionProgress(250, 500, 'linear'), 0.5);
  assert.equal(transitionProgress(10, 0), 1, 'zero duration is already done');
});

test('stateTiming defaults, clamps and honours reduced motion', () => {
  const flipped = doc.states![1];
  assert.deepEqual(stateTiming(flipped), { duration: 500, easing: 'ease-in-out' });
  assert.deepEqual(stateTiming(flipped, true), { duration: 0, easing: 'ease-in-out' });
  assert.deepEqual(stateTiming(null), { duration: 400, easing: 'ease-in-out' });
  assert.equal(stateTiming({ id: 'x', duration_ms: 99_999 }).duration, 10_000);
});

test('statePoses = base document + that state’s overrides', () => {
  assert.equal(initialState(doc), 'idle');
  assert.equal(initialState({ ...doc, default_state: undefined }), 'idle', 'first state');
  assert.equal(initialState({ ...doc, states: [] }), null);
  const idle = statePoses(doc, 'idle');
  assert.deepEqual(idle.get('card')!.rotation, [0, 0, 0]);
  assert.equal(idle.get('chip')!.visible, false);
  const flipped = statePoses(doc, 'flipped', (c) => resolveColor(c, kit, '#000000'));
  assert.deepEqual(flipped.get('card')!.rotation, [0, 180, 0]);
  assert.deepEqual(flipped.get('card')!.position, [0, 1, 0], 'untouched fields stay base');
  assert.equal(flipped.get('card')!.color, '#ffaa33', 'override token resolved');
  assert.equal(flipped.get('chip')!.visible, true);
  assert.deepEqual(statePoses(doc, 'nope').get('card')!.rotation, [0, 0, 0], 'unknown state = base');
});

test('lerpPose tweens transforms, colours, opacity and visibility', () => {
  const a = statePoses(doc, 'idle');
  const b = statePoses(doc, 'flipped', (c) => resolveColor(c, kit, '#000000'));
  const mid = lerpPoses(a, b, 0.5);
  assert.deepEqual(mid.get('card')!.rotation, [0, 90, 0], 'Flipped tweens Y 0 → 180 through 90');
  assert.equal(mid.get('chip')!.visible, true, 'appearing shows immediately');
  assert.equal(mid.get('chip')!.opacity, 0.75);
  // colour: null (material colour) → override snaps at the end only.
  assert.equal(mid.get('card')!.color, null);
  assert.equal(lerpPoses(a, b, 1).get('card')!.color, '#ffaa33');
  const back = lerpPose(b.get('chip')!, a.get('chip')!, 0.5);
  assert.equal(back.visible, true, 'disappearing stays until the end');
  assert.equal(lerpPose(b.get('chip')!, a.get('chip')!, 1).visible, false);
  assert.equal(lerpColor('#000000', '#ffffff', 0.5), '#808080');
  assert.equal(lerpColor('#000000', '#ffffff', 0), '#000000');
  // Spring overshoot extrapolates transforms but never colours past the target.
  const over = lerpPose(a.get('card')!, b.get('card')!, 1.1);
  assert.ok(over.rotation[1] > 180);
});
