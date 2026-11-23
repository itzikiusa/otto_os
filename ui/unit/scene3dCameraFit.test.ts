import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fitCamera, fitCameraPosition, type FitInput, type V3 } from '../src/lib/design/scene3d/cameraFit.ts';

// The lantern from the mobile evidence: a 2.7 m tall stack seen from the
// authored hero camera (fov 30) — its cap was cut off in a portrait viewport.
const lantern: Omit<FitInput, 'aspect'> = {
  position: [3.4, 2.1, 5.2],
  target: [0, 0.8, 0],
  fov: 30,
  min: [-0.6, 0, -0.55],
  max: [0.6, 2.7, 0.55],
  topReserve: 0.08,
  bottomReserve: 0.1,
};

/** Project a world point with a camera at `pos` looking at `target`; NDC x/y. */
function ndc(p: V3, pos: V3, target: V3, fov: number, aspect: number): [number, number] {
  const w = norm([pos[0] - target[0], pos[1] - target[1], pos[2] - target[2]]);
  const r = norm([w[2], 0, -w[0]]); // cross(up, w)
  const u: V3 = [w[1] * r[2] - w[2] * r[1], w[2] * r[0] - w[0] * r[2], w[0] * r[1] - w[1] * r[0]];
  const q: V3 = [p[0] - pos[0], p[1] - pos[1], p[2] - pos[2]];
  const z = -(q[0] * w[0] + q[1] * w[1] + q[2] * w[2]);
  const t = Math.tan((fov * Math.PI) / 360);
  return [(q[0] * r[0] + q[1] * r[1] + q[2] * r[2]) / (z * t * aspect), (q[0] * u[0] + q[1] * u[1] + q[2] * u[2]) / (z * t)];
}
function norm(a: V3): V3 {
  const l = Math.hypot(a[0], a[1], a[2]);
  return [a[0] / l, a[1] / l, a[2] / l];
}
function corners(i: { min: V3; max: V3 }): V3[] {
  return Array.from({ length: 8 }, (_, c) => [c & 1 ? i.max[0] : i.min[0], c & 2 ? i.max[1] : i.min[1], c & 4 ? i.max[2] : i.min[2]] as V3);
}

for (const aspect of [390 / 600, 834 / 1000, 1.6, 2.4]) {
  test(`fitCameraPosition keeps every corner inside the frame and under the top band (aspect ${aspect.toFixed(2)})`, () => {
    const pos = fitCameraPosition({ ...lantern, aspect });
    for (const c of corners(lantern)) {
      const [x, y] = ndc(c, pos, lantern.target, lantern.fov, aspect);
      assert.ok(Math.abs(x) <= 1, `corner ${c} leaves the frame sideways (x=${x.toFixed(2)})`);
      assert.ok(y >= -1 + 2 * lantern.bottomReserve! && y <= 1 - 2 * lantern.topReserve!, `corner ${c} is cut at the top/bottom (y=${y.toFixed(2)})`);
    }
  });
}

test('fitCameraPosition keeps direction and target, and only ever dollies out', () => {
  const pos = fitCameraPosition({ ...lantern, aspect: 0.6 });
  const a = norm([lantern.position[0] - lantern.target[0], lantern.position[1] - lantern.target[1], lantern.position[2] - lantern.target[2]]);
  const b = norm([pos[0] - lantern.target[0], pos[1] - lantern.target[1], pos[2] - lantern.target[2]]);
  for (let k = 0; k < 3; k++) assert.ok(Math.abs(a[k] - b[k]) < 1e-9);
  const d0 = Math.hypot(...(lantern.position.map((v, k) => v - lantern.target[k]) as [number, number, number]));
  const d1 = Math.hypot(...(pos.map((v, k) => v - lantern.target[k]) as [number, number, number]));
  assert.ok(d1 > d0, 'a portrait viewport needs the camera further back');
});

test('fitCameraPosition leaves a framing that already fits alone', () => {
  const small = { ...lantern, min: [-0.1, 0.7, -0.1] as V3, max: [0.1, 0.9, 0.1] as V3, aspect: 1.6 };
  assert.deepEqual(fitCameraPosition(small), lantern.position);
});

test('fitCameraPosition handles a straight-down camera and degenerate input', () => {
  const down = fitCameraPosition({ ...lantern, position: [0, 9, 0], target: [0, 0, 0], aspect: 0.5 });
  assert.ok(down.every(Number.isFinite));
  assert.deepEqual(fitCameraPosition({ ...lantern, position: [0, 0.8, 0], aspect: 1 }), [0, 0.8, 0]);
  assert.deepEqual(fitCameraPosition({ ...lantern, aspect: 0 }), lantern.position);
});

test('closer: a small standalone model far from the default model camera is framed tightly, every corner still inside', () => {
  // A 0.45 m character seen from the model stage's 10 m camera: the old fit kept that distance.
  const knight = { min: [-0.2, 0, -0.1] as V3, max: [0.2, 0.45, 0.1] as V3 };
  const target: V3 = [0, 0.225, 0];
  const position: V3 = [6, 4.225, 8];
  const aspect = 390 / 600;
  const far = fitCameraPosition({ position, target, fov: 50, aspect, ...knight });
  assert.deepEqual(far, position, 'without closer the authored distance is kept');
  const pos = fitCameraPosition({ position, target, fov: 50, aspect, ...knight, closer: true });
  const dist = Math.hypot(pos[0] - target[0], pos[1] - target[1], pos[2] - target[2]);
  assert.ok(dist < 2, `came close (${dist.toFixed(2)} m)`);
  let tallest = 0;
  for (const c of corners(knight)) {
    const [x, y] = ndc(c, pos, target, 50, aspect);
    assert.ok(Math.abs(x) <= 1 && Math.abs(y) <= 1, `corner ${c} stays in frame`);
    tallest = Math.max(tallest, Math.abs(y));
  }
  assert.ok(tallest > 0.5, 'the model fills a good part of the frame height');
});

// Create review v3: a spread group (a tall monster at one side, a sword in front, a chest
// behind) filled about a third of the picture because the fit framed its joint box.
test('fitCamera frames a spread group by its pieces: closer, every corner still inside', () => {
  const pieces: [V3, V3][] = [
    [[1.2, 0, -0.6], [2.6, 2.7, 0.6]],
    [[-0.5, 0, -0.2], [0.0, 1.8, 0.2]],
    [[-0.3, 0, 1.4], [-0.2, 1.0, 1.5]],
    [[0.2, 0, -2.0], [0.8, 0.4, -1.6]],
  ];
  const all = { min: [-0.5, 0, -2.0] as V3, max: [2.6, 2.7, 1.5] as V3 };
  const base = { position: [2.6, 2.4, 4.4] as V3, target: [1, 1, 0] as V3, fov: 50, aspect: 1, closer: true, ...all };
  const joint = fitCameraPosition(base);
  const fit = fitCamera({ ...base, boxes: pieces });
  const dist = (p: V3, t: V3) => Math.hypot(p[0] - t[0], p[1] - t[1], p[2] - t[2]);
  assert.ok(dist(fit.position, fit.target) < dist(joint, base.target) * 0.9, 'the pieces are framed closer than their joint box');
  const xs: number[] = [];
  const ys: number[] = [];
  for (const [min, max] of pieces) {
    for (const c of corners({ min, max })) {
      const [x, y] = ndc(c, fit.position, fit.target, 50, 1);
      assert.ok(Math.abs(x) <= 1 && Math.abs(y) <= 1, `corner ${c} is out of view (${x.toFixed(2)}, ${y.toFixed(2)})`);
      xs.push(x);
      ys.push(y);
    }
  }
  // In the middle of the picture, and filling it one way (the margin leaves 12 % free).
  const span = (v: number[]) => Math.max(...v) - Math.min(...v);
  for (const v of [xs, ys]) assert.ok(Math.abs(Math.min(...v) + Math.max(...v)) < 0.05, `centred: ${Math.min(...v).toFixed(2)}..${Math.max(...v).toFixed(2)}`);
  assert.ok(Math.max(span(xs), span(ys)) > 1.7, 'the group fills the picture');
  // Without pieces it is the plain fit, at the given target.
  assert.deepEqual(fitCamera(base), { position: joint, target: base.target });
});
