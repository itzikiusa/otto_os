import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// Values built inside the vm sandbox carry its own Object/Array prototypes.
const plain = <T>(v: T): T => JSON.parse(JSON.stringify(v)) as T;

const tableWindow = () =>
  loadSource(new URL('../src/lib/tableWindow.svelte.ts', import.meta.url), {}, { requestAnimationFrame: (fn: () => void) => { fn(); return 1; } });

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
  const { podContainers } = loadSource(new URL('../src/modules/kubernetes/k8s-util.ts', import.meta.url), {});
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
