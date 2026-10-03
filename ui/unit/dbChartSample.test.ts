import { test } from 'node:test';
import assert from 'node:assert/strict';
import { bucketBars, decimateMinMax, topSlices, MAX_BARS } from '../src/modules/database/chart-sample.ts';

test('bars under the cap are untouched', () => {
  const labels = ['a', 'b', 'c'];
  const series = [{ name: 'n', values: [1, 2, 3] }];
  const out = bucketBars(labels, series);
  assert.equal(out.aggregated, 0);
  assert.equal(out.labels, labels);
  assert.equal(out.series, series);
});

test('5,000 bars average into MAX_BARS buckets with range labels', () => {
  const n = 5000;
  const labels = Array.from({ length: n }, (_, i) => `d${i}`);
  const values = Array.from({ length: n }, (_, i) => (i % 2 === 0 ? 10 : Number.NaN));
  const out = bucketBars(labels, [{ name: 'v', values }, { name: 'w', values: values.map(() => 4) }]);
  assert.equal(out.aggregated, n);
  assert.equal(out.labels.length, MAX_BARS);
  assert.equal(out.series[0].values.length, MAX_BARS);
  // NaN is ignored by the mean, so every bucket of the alternating series is 10.
  assert.ok(out.series[0].values.every((v) => v === 10));
  assert.ok(out.series[1].values.every((v) => v === 4));
  assert.equal(out.labels[0].startsWith('d0 – '), true);
  assert.equal(out.labels.at(-1)?.endsWith(` – d${n - 1}`), true);
});

test('min-max decimation keeps spikes and stays near the target', () => {
  const values = Array.from({ length: 5000 }, () => 1);
  values[1234] = 99;
  values[4321] = -50;
  const out = decimateMinMax(values, 600);
  assert.ok(out.length <= 600, `len ${out.length}`);
  assert.ok(out.includes(99));
  assert.ok(out.includes(-50));
  assert.ok(out.indexOf(99) < out.indexOf(-50));
  assert.equal(decimateMinMax([1, 2, 3], 600).length, 3);
});

test('pie keeps the 11 largest slices plus Other', () => {
  const labels = Array.from({ length: 40 }, (_, i) => `s${i}`);
  const values = labels.map((_, i) => i);
  const out = topSlices(labels, values);
  assert.equal(out.labels.length, 12);
  assert.equal(out.labels.at(-1), 'Other');
  assert.equal(out.merged, 29);
  const total = values.reduce((a, b) => a + b, 0);
  assert.equal(out.values.reduce((a, b) => a + b, 0), total);
  // Original order kept for the survivors.
  assert.deepEqual(out.labels.slice(0, 3), ['s29', 's30', 's31']);
  const small = topSlices(['a', 'b'], [Number.NaN, -2]);
  assert.deepEqual(small.values, [0, 0]);
});
