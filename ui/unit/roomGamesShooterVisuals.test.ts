import { test } from 'node:test';
import assert from 'node:assert/strict';
import { emptyPose, GRIP_NONE, GRIP_ONE, GRIP_PICK, GRIP_TWO, solvePose, STRIDE, type Pose, type PoseInput } from '../src/modules/rooms/games/shooter/render/pose.ts';
import { GUN_PARTS, GUNS, gunFor } from '../src/modules/rooms/games/shooter/render/guns.ts';
import { isWeather, WEATHER, WEATHERS, weatherFor, weatherTheme } from '../src/modules/rooms/games/shooter/render/weather.ts';
import { CONSUMABLE_IDS, WEAPON_IDS } from '../src/modules/rooms/games/shooter/weapons.ts';

const still = (over: Partial<PoseInput> = {}): PoseInput => ({
  speed: 0, phase: 0, air: false, sprint: false, crouch: 0, slide: 0, aim: 0, kick: 0, swing: 0, grip: GRIP_NONE, pitch: 0, down: false, dead: 0, infected: false, time: 0, ...over,
});
const pose = (over: Partial<PoseInput> = {}): Pose => solvePose(still(over), emptyPose());

// ---------------------------------------------------------------- poses

test('pose: walking swings the legs and arms in opposition, faster strides swing wider', () => {
  const step = pose({ speed: 5, phase: Math.PI / 2 });
  assert.ok(step.hipL > 0.3 && step.hipR < -0.3, `left leg forward (${step.hipL.toFixed(2)}), right back (${step.hipR.toFixed(2)})`);
  assert.ok(Math.abs(step.hipL + step.hipR) < 1e-9);
  assert.ok(step.shR > 0 && step.shL < 0, 'arms counter-swing');
  const other = pose({ speed: 5, phase: Math.PI * 1.5 });
  assert.ok(other.hipR > 0.3 && other.hipL < -0.3, 'half a stride later the legs swap');
  assert.ok(pose({ speed: 7, phase: Math.PI / 2, sprint: true }).hipL > pose({ speed: 2, phase: Math.PI / 2 }).hipL, 'sprint swings wider');
  assert.ok(pose({ speed: 7, sprint: true }).spine > pose({ speed: 2 }).spine, 'sprint leans in');
  const idle = pose();
  assert.ok(Math.abs(idle.hipL) < 1e-9 && idle.drop === 0 && idle.tilt === 0, 'standing still');
  assert.ok(STRIDE > 1 && STRIDE < 2.5);
});

test('pose: crouch and slide lower the body; a slide leans back on a straight front leg', () => {
  const stand = pose(), crouch = pose({ crouch: 1 }), slide = pose({ slide: 1, speed: 10 });
  assert.ok(crouch.drop > 0.3 && crouch.kneeL > 1.2 && crouch.kneeR > 1, `crouch drop ${crouch.drop}`);
  assert.ok(slide.drop > crouch.drop, 'a slide is lower than a crouch');
  assert.ok(slide.tilt < -0.3 && stand.tilt === 0, 'leans back');
  assert.ok(slide.hipL > 1.2 && slide.kneeL < 0.3 && slide.kneeR > 1.5, 'front leg out, back leg folded');
  const half = pose({ crouch: 0.5 });
  assert.ok(half.drop > 0 && half.drop < crouch.drop, 'blends');
});

test('pose: a long gun comes up into the shoulder when aiming and follows the view pitch; recoil kicks it', () => {
  const ready = pose({ grip: GRIP_TWO }), ads = pose({ grip: GRIP_TWO, aim: 1 });
  assert.ok(ads.shR > ready.shR && ads.gunY > ready.gunY, 'sights up');
  assert.ok(Math.abs(ads.gunPitch) < 1e-9 && ready.gunPitch < -0.3, 'level when aimed, muzzle down at the ready');
  const up = pose({ grip: GRIP_TWO, aim: 1, pitch: 0.5 }), down = pose({ grip: GRIP_TWO, aim: 1, pitch: -0.5 });
  assert.ok(up.gunPitch > 0.4 && down.gunPitch < -0.4 && up.shR > down.shR, 'follows the pitch');
  assert.ok(up.head > 0 && down.head < 0, 'head follows');
  const kicked = pose({ grip: GRIP_TWO, aim: 1, kick: 1 });
  assert.ok(kicked.gunPitch > ads.gunPitch && kicked.shR < ads.shR, 'recoil');
  // Sprinting carries it across the chest; a pistol is held out in one hand, a pickaxe on the shoulder.
  const sprint = pose({ grip: GRIP_TWO, sprint: true, speed: 7 });
  assert.ok(sprint.gunYaw > 0.5 && sprint.gunPitch > 0.3);
  const pistol = pose({ grip: GRIP_ONE, aim: 1 });
  assert.ok(pistol.shR > 1.2 && pistol.gunZ > ready.gunZ);
  const swing = [0, 0.25, 0.5, 0.75, 1].map(t => pose({ grip: GRIP_PICK, swing: t }).gunPitch);
  assert.ok(swing[1] > swing[4] && swing[4] < 0, 'the pickaxe chops down');
});

test('pose: knocked crawls on hands and knees, death falls back flat, the infected reach forward', () => {
  const down = pose({ down: true, speed: 1.5, phase: 1 });
  assert.ok(down.tilt > 0.8 && down.drop > 0.4 && down.kneeL > 1.3 && down.shL > 1, 'on all fours');
  const dying = [0, 0.3, 0.7, 1].map(d => pose({ dead: d }));
  assert.ok(dying[3].tilt < -1.4 && dying[3].drop > 0.7, 'flat on the back');
  for (let i = 1; i < dying.length; i++) assert.ok(dying[i].tilt <= dying[i - 1].tilt, 'falls steadily');
  const z = pose({ infected: true, speed: 2.5, phase: 2, time: 3 });
  assert.ok(z.shL > 1.1 && z.shR > 1.1 && z.spine > 0.3, 'arms out, hunched');
  const claw = pose({ infected: true, kick: 1, time: 3 });
  assert.ok(claw.shL > z.shL, 'a swing lifts the arms');
});

test('pose: every output stays finite for any input', () => {
  const keys: (keyof Pose)[] = Object.keys(emptyPose()) as (keyof Pose)[];
  let seed = 7;
  const rnd = () => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff);
  for (let i = 0; i < 400; i++) {
    const p = pose({
      speed: rnd() * 14, phase: rnd() * 100, air: rnd() < 0.2, sprint: rnd() < 0.3, crouch: rnd(), slide: rnd() < 0.3 ? rnd() : 0, aim: rnd(), kick: rnd(), swing: rnd(),
      grip: [GRIP_NONE, GRIP_ONE, GRIP_TWO, GRIP_PICK][Math.floor(rnd() * 4)] as PoseInput['grip'], pitch: (rnd() - 0.5) * 4, down: rnd() < 0.1, dead: rnd() < 0.1 ? rnd() : 0, infected: rnd() < 0.2, time: rnd() * 100,
    });
    for (const k of keys) assert.ok(Number.isFinite(p[k]), `${k} finite`);
    assert.ok(p.drop >= 0 && p.drop < 1.2);
  }
});

// ---------------------------------------------------------------- weapon silhouettes

test('guns: every weapon and consumable has a silhouette that fits the instance budget, muzzle in front', () => {
  for (const id of [...WEAPON_IDS, ...CONSUMABLE_IDS, 'pickaxe']) {
    const g = gunFor(id);
    assert.ok(g, id);
    assert.ok(g.parts.length > 0 && g.parts.length <= GUN_PARTS, `${id} parts ${g.parts.length}`);
    for (const [x, y, z, w, h, d, sw] of g.parts) assert.ok([x, y, z].every(Number.isFinite) && w > 0 && h > 0 && d > 0 && sw >= 0 && sw <= 2, id);
    const front = Math.max(...g.parts.map(p => p[2] + p[5] / 2));
    assert.ok(g.muzzle[2] >= front - 0.02, `${id} muzzle at the front (${g.muzzle[2]} vs ${front})`);
    assert.ok(g.sight > 0 && g.colors.length === 3);
  }
  assert.equal(gunFor(null), null);
  assert.equal(gunFor('laser-blaster'), GUNS.rifle, 'unknown guns fall back to a rifle');
  assert.ok(GUNS.sniper.muzzle[2] > GUNS.rifle.muzzle[2] && GUNS.rifle.muzzle[2] > GUNS.pistol.muzzle[2], 'long guns are longer');
});

// ---------------------------------------------------------------- weather

test('weather: Dead City burns under ash by default, overrides must be known presets, themes merge', () => {
  assert.equal(weatherFor('deadcity'), 'ash');
  assert.equal(weatherFor('isle'), 'day');
  assert.equal(weatherFor('mesa', 'night'), 'night');
  assert.equal(weatherFor('isle', 'tornado'), 'day');
  assert.equal(weatherFor('isle', null), 'day');
  assert.deepEqual(WEATHERS.map(w => w.id).sort(), Object.keys(WEATHER).sort());
  assert.ok(isWeather('rain') && !isWeather('snow') && !isWeather(3));
  const base = { skyTop: 1, skyHorizon: 2, fog: 3, fogNear: 100, fogFar: 600, sun: 4, sunIntensity: 2, ambientSky: 5, ambientGround: 6, ambient: 1 };
  assert.deepEqual(weatherTheme(base, 'day'), base);
  const night = weatherTheme(base, 'night'), fog = weatherTheme(base, 'fog');
  assert.ok(night.sunIntensity < 1 && night.ambient < 0.6 && WEATHER.night.flashlight, 'night is dark and lit by flashlights');
  assert.ok(fog.fogFar < 200 && fog.fogNear < 20, 'fog closes in');
  assert.ok(WEATHER.rain.rain > 0 && WEATHER.ash.ash > 0 && WEATHER.rain.lightning);
  // The toxic wall over Dead City is green; the storm elsewhere purple.
  assert.ok(WEATHER.ash.wall[1] > WEATHER.ash.wall[0] && WEATHER.day.wall[2] > WEATHER.day.wall[1]);
  for (const w of Object.values(WEATHER)) assert.ok(w.exposure > 0.8 && w.exposure < 1.5 && w.sun[1] > 0, 'sun above the horizon');
});
