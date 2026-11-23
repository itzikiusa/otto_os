import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fogOf, GROUND_LOOKS, groundLook, halosOf, precipitationOf, settingKey, wallColor, wallsOf } from '../src/lib/design/scene3d/setting.ts';
import { GROUND_KINDS, type Scene3dEnvironment, type Scene3dLight } from '../src/lib/design/scene3d/types.ts';
import { validate } from '../src/lib/design/scene3d/validate.ts';

// The owner's pier as the productions scene writer writes it: water at night in fog, the lantern glowing.
const night: Scene3dEnvironment = {
  preset: 'night',
  intensity: 0.35,
  background: true,
  sky: ['#141925', '#1e2535', '#10141f'],
  fog: { color: '#1e2535', density: 0.012 },
  ground: { kind: 'water', center: [0, 0.64, 0], size: [1200, 1200] },
  exposure: 1,
};
const lantern: Scene3dLight = { id: 'glow-lantern', type: 'point', position: [0.2, 2.3, 0.1], color: '#ffb35c', intensity: 1.5, glow: 0.25 };
const shore = { kind: 'grass' as const, center: [-20, 1.1, -90] as [number, number, number], size: [12, 12] as [number, number] };
const moon: Scene3dLight = { id: 'sun', type: 'directional', position: [10, 20, 5], target: [0, 0, 0], intensity: 0.27 };

test('a setting environment survives the validator, a malformed piece is dropped', () => {
  const doc = {
    type: 'otto-scene3d', version: 2, camera: { position: [9, 5, 14], target: [0, 1, 0], fov: 50, far: 1200 },
    environment: { ...night, shore, room: { height: 3 }, weather: 'rain' }, lights: [lantern, moon], objects: [], groups: [],
  };
  const r = validate(doc);
  assert.ok(r.ok);
  assert.deepEqual(r.doc!.environment, { ...night, shore, room: { height: 3 }, weather: 'rain' });
  assert.equal(r.doc!.lights[0].glow, 0.25);
  const reflected = validate({ ...doc, lights: [{ ...moon, shadow: true, shadow_intensity: 0.76 }] });
  assert.equal(reflected.doc?.lights[0].shadow_intensity, 0.76);
  assert.equal(r.doc!.lights[1].shadow_intensity, undefined);
  const ground = { kind: 'road', center: [4, 0, 0], size: [180, 200], axis: 'z' };
  const wetPane = { kind: 'rain_on_glass' as const, position: [0, 2, -3] as [number, number, number], size: [1, 1.2] as [number, number], rotation: 90 };
  const mist = { ...wetPane, kind: 'steam' as const, position: [4, 1, 0] as [number, number, number] };
  const wet = validate({ ...doc, environment: { ...night, local_effects: [wetPane, mist, { ...mist, kind: 'unknown' }] } });
  assert.deepEqual(wet.doc?.environment?.local_effects, [wetPane, mist]);
  assert.ok(wet.issues.some(i => i.path === 'environment.local_effects[2].kind'));
  assert.notEqual(settingKey(wet.doc!.environment, [], [1, 1, 1], [0, 0, 0]), settingKey(night, [], [1, 1, 1], [0, 0, 0]));
  const road = validate({ ...doc, environment: { preset: 'studio-soft', ground } });
  assert.deepEqual(road.doc?.environment?.ground, ground);
  const bad = validate({ ...doc, environment: { ...night, ground: { kind: 'lava', center: [0, 0, 0], size: [9, 9] }, fog: { color: 'grey', density: 0.1 } } });
  assert.ok(bad.ok);
  assert.equal(bad.doc!.environment!.ground, undefined);
  assert.equal(bad.doc!.environment!.fog, undefined);
  assert.deepEqual(bad.doc!.environment!.sky, night.sky);
  assert.ok(bad.issues.some((i) => i.path === 'environment.ground.kind'));
});

test('each ground kind has its own look, and water is a glossy rippled surface', () => {
  for (const k of GROUND_KINDS) assert.ok(GROUND_LOOKS[k], k);
  const water = groundLook('water');
  assert.equal(water.pattern, 'water');
  assert.ok(water.roughness < 0.2);
  const colours = new Set(GROUND_KINDS.map((k) => groundLook(k).color));
  assert.equal(colours.size, GROUND_KINDS.length, 'no two grounds share a colour');
  assert.ok(groundLook('grass').roughness > 0.8);
  assert.equal(groundLook('wood_floor').pattern, 'planks');
});

test('fog, halos and weather come from the setting; a scene without one draws nothing', () => {
  assert.deepEqual(fogOf(night), night.fog);
  assert.equal(fogOf({ preset: 'studio-soft' }), null);
  assert.deepEqual(halosOf([lantern, moon, { ...lantern, id: 'dark', glow: 0 }, { ...lantern, id: 'off', visible: false }]), [
    { id: 'glow-lantern', position: [0.2, 2.3, 0.1], color: '#ffb35c', radius: 0.25 },
  ]);
  const rain = precipitationOf({ ...night, weather: 'rain' }, [0, 1, 0]);
  assert.equal(rain?.kind, 'rain');
  assert.ok(rain!.speed > 5 && rain!.size[0] <= 40);
  assert.equal(precipitationOf({ ...night, weather: 'snow', room: { height: 3 } }, [0, 1, 0]), null, 'no snow indoors');
  // An old scene (the studio preset, a plane ground) has no setting to draw.
  assert.equal(settingKey({ preset: 'studio-soft' }, [moon], [9, 5, 14], [0, 1, 0]), '');
  const k = settingKey(night, [lantern, moon], [9, 5, 14], [0, 1, 0]);
  assert.notEqual(k, '');
  assert.notEqual(settingKey({ ...night, sky: ['#4f8fd8', '#cfe2f2', '#8f9aa0'] }, [lantern], [9, 5, 14], [0, 1, 0]), k, 'a daytime steer redraws');
});

test('a room puts its two walls on the far sides from the camera, facing it', () => {
  const room: Scene3dEnvironment = { preset: 'studio-soft', ground: { kind: 'wood_floor', center: [1, 0, -1], size: [6, 4] }, room: { height: 3 } };
  const [back, side] = wallsOf(room, [8, 4, 9]);
  assert.deepEqual(back, { position: [1, 1.5, -3], rotationY: 0, width: 6, height: 3 });
  assert.deepEqual(side, { position: [-2, 1.5, -1], rotationY: Math.PI / 2, width: 4, height: 3 });
  const [b2, s2] = wallsOf(room, [-8, 4, -9]);
  assert.equal(b2.position[2], 1);
  assert.equal(b2.rotationY, Math.PI);
  assert.equal(s2.position[0], 4);
  assert.deepEqual(wallsOf(night, [8, 4, 9]), [], 'outdoors has no walls');
  assert.notEqual(wallColor('stage'), wallColor('wood_floor'), 'a stage has curtains, a room plaster');
});
