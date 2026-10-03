import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// Values built inside the vm sandbox carry its own Object/Array prototypes.
const plain = <T>(v: T): T => JSON.parse(JSON.stringify(v)) as T;

const tableWindow = () =>
  loadSource(new URL('../src/lib/tableWindow.svelte.ts', import.meta.url), { svelte: { tick: async () => {} }, './findProviders': { registerFindProvider: () => () => {} } }, { requestAnimationFrame: (fn: () => void) => { fn(); return 1; } });


/** k8s-util reads the shared clock (`rowAge`). */
const K8S_UTIL_FIXTURES = { '../../lib/stores/now.svelte': { now: () => Date.now() } };

test('TableWindow renders small lists whole and windows big ones', () => {
  const { TableWindow } = tableWindow();
  const tw = new TableWindow(400, 15);
  assert.deepEqual(plain(tw.range(300)), { start: 0, end: 300, top: 0, bottom: 0 });

  tw.rowH = 30;
  tw.viewH = 600;
  tw.scrollTop = 0;
  const top = tw.range(50_000);
  assert.equal(top.start, 0);
  assert.ok(top.end - top.start <= 20 + 30 + 1, 'only the viewport plus overscan is mounted');
  assert.equal(top.top + (top.end - top.start) * 30 + top.bottom, 50_000 * 30, 'spacers keep the full scroll height');

  tw.scrollTop = 30 * 10_000;
  const mid = tw.range(50_000);
  assert.equal(mid.start, 10_000 - 15);
  assert.equal(mid.top, mid.start * 30);
  assert.equal(mid.top + (mid.end - mid.start) * 30 + mid.bottom, 50_000 * 30);

  tw.scrollTop = 30 * 60_000; // past the end after a filter shrank the list
  const end = tw.range(50_000);
  assert.ok(end.start <= end.end && end.end === 50_000);
});

test('TableWindow measures the real row height and resets on a new listing', () => {
  const { TableWindow } = tableWindow();
  const tw = new TableWindow();
  const row = { getBoundingClientRect: () => ({ height: 27.5 }) };
  const container = { scrollTop: 900, querySelector: () => row };
  tw.measure(container as never);
  assert.equal(tw.rowH, 27.5);
  tw.scrollTop = 900;
  tw.reset(container as never);
  assert.equal(tw.scrollTop, 0);
  assert.equal(container.scrollTop, 0);
});

test('podContainers mirrors the daemon (init first, live state + restarts)', () => {
  const { podContainers } = loadSource(new URL('../src/modules/kubernetes/k8s-util.ts', import.meta.url), K8S_UTIL_FIXTURES);
  const got = podContainers({
    spec: {
      initContainers: [{ name: 'init', image: 'busybox' }],
      containers: [{ name: 'app', image: 'app:1' }, { name: 'side', image: 'envoy' }],
    },
    status: {
      initContainerStatuses: [{ name: 'init', ready: false, restartCount: 0, state: { terminated: { reason: 'Completed' } } }],
      containerStatuses: [
        { name: 'app', ready: true, restartCount: 3, state: { running: {} } },
        { name: 'side', ready: false, restartCount: 0, state: { waiting: { reason: 'CrashLoopBackOff' } } },
      ],
    },
  });
  assert.deepEqual(plain(got), [
    { name: 'init', image: 'busybox', ready: false, state: 'terminated:Completed', restarts: 0, init: true },
    { name: 'app', image: 'app:1', ready: true, state: 'running', restarts: 3, init: false },
    { name: 'side', image: 'envoy', ready: false, state: 'waiting:CrashLoopBackOff', restarts: 0, init: false },
  ]);
  assert.deepEqual(plain(podContainers({ spec: { containers: [{ name: 'x' }] } })), [
    { name: 'x', image: '', ready: false, state: 'unknown', restarts: 0, init: false },
  ]);
  assert.deepEqual(plain(podContainers(null)), []);
});

test('TableWindow.active gates windowing; a 5000-message peek mounts ≤ 150 rows (SC-08/SC-22)', () => {
  const { TableWindow } = tableWindow();
  const tw = new TableWindow();
  assert.equal(tw.active(400), false);
  assert.equal(tw.active(401), true);
  tw.rowH = 29;
  tw.viewH = 900;
  for (const top of [0, 29 * 2500, 29 * 4990]) {
    tw.scrollTop = top;
    const r = tw.range(5000);
    assert.ok(r.end - r.start <= 150, `mounted ${r.end - r.start} rows at scrollTop ${top}`);
    assert.equal(r.top + (r.end - r.start) * 29 + r.bottom, 5000 * 29);
  }
  // 12k offsets (a 600-partition × 20-topic group) stay bounded too.
  tw.scrollTop = 29 * 6000;
  const big = tw.range(12_000);
  assert.ok(big.end - big.start <= 150);
});

test('clipLongScalars cuts only >64 KiB strings and keeps untouched subtrees (SC-19)', () => {
  const { clipLongScalars, MANIFEST_SCALAR_MAX } = loadSource(new URL('../src/modules/kubernetes/k8s-util.ts', import.meta.url), K8S_UTIL_FIXTURES);
  const huge = 'x'.repeat(MANIFEST_SCALAR_MAX + 10 * 1024);
  const meta = { name: 'grafana-dashboards', labels: { app: 'grafana' } };
  const manifest = { kind: 'ConfigMap', metadata: meta, data: { 'big.json': huge, small: 'ok' }, list: [huge, 1] };
  const { value, clipped } = clipLongScalars(manifest);
  assert.equal(clipped, 2);
  const v = value as { metadata: unknown; data: Record<string, string>; list: unknown[] };
  assert.equal(v.metadata, meta, 'unchanged subtree keeps identity');
  assert.equal(v.data.small, 'ok');
  assert.ok(v.data['big.json'].length < MANIFEST_SCALAR_MAX + 100);
  assert.ok(v.data['big.json'].startsWith('x'.repeat(100)));
  assert.match(v.data['big.json'], /10 KiB more — use Copy/);
  assert.equal(v.list[1], 1);
  assert.equal(manifest.data['big.json'], huge, 'the input is not mutated');
  const same = clipLongScalars(meta);
  assert.equal(same.value, meta);
  assert.equal(same.clipped, 0);
});

test('mergeS3Head refreshes the head and keeps the loaded pages (I9)', () => {
  const { mergeS3Head } = loadSource(new URL('../src/modules/aws/util.ts', import.meta.url), {});
  const obj = (key: string, size = 1) => ({ key, size, last_modified: '2026-01-01T00:00:00Z' });
  // Loaded: page 1 (a..c) + a "Load more" page (d..f).
  const loaded = { prefixes: ['b/', 'e/'], objects: ['a', 'c', 'd', 'f'].map((k) => obj(k)) };
  // Fresh page 1: `a` changed size, `c` deleted, a new `bb` arrived; the page ends at `c2`.
  const fresh = { prefixes: ['b/'], objects: [obj('a', 9), obj('bb'), obj('c2')], is_truncated: true, next_token: 't' };
  const m = plain(mergeS3Head(loaded, fresh)) as { prefixes: string[]; objects: { key: string; size: number }[] };
  assert.deepEqual(m.objects.map((o) => o.key), ['a', 'bb', 'c2', 'd', 'f']);
  assert.equal(m.objects[0].size, 9, 'fresh page wins');
  assert.deepEqual(m.prefixes, ['b/', 'e/'], 'loaded prefixes past the head survive');
  // A complete fresh listing replaces everything (deletions beyond page 1 drop).
  const all = plain(mergeS3Head(loaded, { prefixes: [], objects: [obj('a')], is_truncated: false })) as typeof m;
  assert.deepEqual(all.objects.map((o) => o.key), ['a']);
  assert.deepEqual(all.prefixes, []);
});

test('rowAge ticks from created_at, falls back to age_seconds (perf R1)', () => {
  let nowMs = 1_790_848_800_000 + 90_000;
  const { rowAge } = loadSource(new URL('../src/modules/kubernetes/k8s-util.ts', import.meta.url), {
    '../../lib/stores/now.svelte': { now: () => nowMs },
  });
  const row = { age_seconds: 5, created_at: 1_790_848_800 };
  assert.equal(rowAge(row), 90);
  nowMs += 60_000; // a 304 kept the same row: the age still moves
  assert.equal(rowAge(row), 150);
  assert.equal(rowAge({ age_seconds: 42 }), 42);
  assert.equal(rowAge({ age_seconds: 42, created_at: null }), 42);
  assert.equal(rowAge({ age_seconds: 0, created_at: 1_790_848_800 + 10_000 }), 0, 'clock skew never goes negative');
});
