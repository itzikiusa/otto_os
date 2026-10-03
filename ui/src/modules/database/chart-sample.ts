// Down-sampling for the inline-SVG dashboard charts (Chart.svelte). A widget run
// returns up to 5,000 rows, but a tile is a 300-unit viewBox: drawing one
// <rect> per row per series (or a 5,000-vertex path) buys nothing visible and
// costs SVG nodes / layout per tile. Pure module — unit-tested.

/** Most bar groups a tile draws; more rows are averaged into this many buckets. */
export const MAX_BARS = 150;
/** Most slices a pie draws; the smallest rest merge into one "Other" slice. */
export const MAX_SLICES = 12;

export interface Series {
  name: string;
  values: number[];
}

/** Bucket bounds [start, end) splitting `n` items into `buckets` near-even groups. */
function bounds(n: number, buckets: number): [number, number][] {
  const out: [number, number][] = [];
  for (let b = 0; b < buckets; b++) {
    const s = Math.floor((b * n) / buckets);
    const e = Math.floor(((b + 1) * n) / buckets);
    if (e > s) out.push([s, e]);
  }
  return out;
}

/** Mean of the finite values in [s, e); NaN when there are none. */
function mean(values: number[], s: number, e: number): number {
  let sum = 0;
  let k = 0;
  for (let i = s; i < e; i++) {
    const v = values[i];
    if (Number.isNaN(v)) continue;
    sum += v;
    k++;
  }
  return k ? sum / k : Number.NaN;
}

/**
 * Bars: past `max` labels, average each series over `max` contiguous buckets
 * (labels become "first – last"). Returns the input unchanged when it fits;
 * `aggregated` is the original row count when bucketing happened, else 0.
 */
export function bucketBars(
  labels: string[],
  series: Series[],
  max = MAX_BARS,
): { labels: string[]; series: Series[]; aggregated: number } {
  const n = labels.length;
  if (n <= max) return { labels, series, aggregated: 0 };
  const bs = bounds(n, max);
  return {
    labels: bs.map(([s, e]) => (e - s > 1 ? `${labels[s]} – ${labels[e - 1]}` : labels[s])),
    series: series.map((sr) => ({ name: sr.name, values: bs.map(([s, e]) => mean(sr.values, s, e)) })),
    aggregated: n,
  };
}

/**
 * Lines / areas: min-max decimation to about `target` points — each bucket
 * keeps its minimum and maximum in their original order, so spikes survive.
 * Returns the input unchanged when it already fits.
 */
export function decimateMinMax(values: number[], target: number): number[] {
  const n = values.length;
  if (n <= target || target < 4) return values;
  const out: number[] = [];
  for (const [s, e] of bounds(n, Math.floor(target / 2))) {
    let lo = -1;
    let hi = -1;
    for (let i = s; i < e; i++) {
      const v = values[i];
      if (Number.isNaN(v)) continue;
      if (lo < 0 || v < values[lo]) lo = i;
      if (hi < 0 || v > values[hi]) hi = i;
    }
    if (lo < 0) {
      out.push(Number.NaN);
      continue;
    }
    if (lo === hi) out.push(values[lo]);
    else if (lo < hi) out.push(values[lo], values[hi]);
    else out.push(values[hi], values[lo]);
  }
  return out;
}

/**
 * Pie: keep the `max - 1` largest slices (in their original order) and merge
 * the rest into one "Other" slice. NaN / negatives count as 0, as in the chart.
 */
export function topSlices(
  labels: string[],
  values: number[],
  max = MAX_SLICES,
): { labels: string[]; values: number[]; merged: number } {
  const clean = values.map((v) => (Number.isNaN(v) || v < 0 ? 0 : v));
  if (clean.length <= max) return { labels, values: clean, merged: 0 };
  const keep = new Set(
    clean
      .map((v, i) => [v, i] as const)
      .sort((a, b) => b[0] - a[0] || a[1] - b[1])
      .slice(0, max - 1)
      .map(([, i]) => i),
  );
  const outL: string[] = [];
  const outV: number[] = [];
  let other = 0;
  clean.forEach((v, i) => {
    if (keep.has(i)) {
      outL.push(labels[i]);
      outV.push(v);
    } else other += v;
  });
  outL.push('Other');
  outV.push(other);
  return { labels: outL, values: outV, merged: clean.length - (max - 1) };
}
