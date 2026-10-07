import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

const engine = '/fixture/ms-playwright/webkit-2359/';
const start = 'Wed Oct  7 12:50:36 2026';
function row(pid: number, parent: number, cpu: number, rss: number, path: string, identity = start) {
  return `${pid} ${parent} ${identity} ${cpu} ${rss * 1024} ${path}`;
}
function fixture() {
  let lines = [
    row(10, 1, 90, 500, '/node'), // driver never enters total
    row(20, 1, 1, 100, '/owned/ottod'),
    row(30, 10, 2, 120, `${engine}Playwright.app/Contents/MacOS/Playwright`),
    row(31, 1, 3, 50, `${engine}com.apple.WebKit.Networking.xpc/Contents/MacOS/com.apple.WebKit.Networking.Development`),
    row(32, 1, 4, 80, `${engine}com.apple.WebKit.GPU.xpc/Contents/MacOS/com.apple.WebKit.GPU.Development`),
    row(33, 1, 5, 210, `${engine}com.apple.WebKit.WebContent.xpc/Contents/MacOS/com.apple.WebKit.WebContent.Development`),
    row(34, 1, 60, 900, `${engine}com.apple.WebKit.WebContent.xpc/Contents/MacOS/com.apple.WebKit.WebContent.Development`), // another test
    row(35, 1, 70, 1000, '/System/Library/Frameworks/WebKit.framework/com.apple.WebKit.WebContent'),
  ];
  const coalitions = new Map([[30, 431541], [31, 431541], [32, 431541], [33, 431541], [34, 999999], [35, 431541]]);
  const lookups: number[][] = [];
  const module = loadSource(new URL('../e2e/process-resources.ts', import.meta.url), {
    'node:child_process': { execFileSync(command: string, args: string[]) {
      if (command === 'ps') return lines.join('\n');
      assert.equal(command, 'python3');
      const candidates = JSON.parse(args.at(-1)!) as { pid: number; start: string }[];
      lookups.push(candidates.map((p) => p.pid));
      return JSON.stringify(candidates.map((p) => ({ ...p, coalition: coalitions.get(p.pid) ?? null })));
    } },
  }, { process: { pid: 10, platform: 'darwin', env: {} } });
  return { sample: () => module.processResources(20) as { process: string; cpu_percent: number; rss_mb: number; pids: number[] }[], lookups,
    replacePid() { lines = lines.map((line) => line.startsWith('33 ') ? row(33, 1, 80, 2000, `${engine}com.apple.WebKit.WebContent.xpc/Contents/MacOS/com.apple.WebKit.WebContent.Development`, 'Wed Oct  7 12:51:36 2026') : line); coalitions.set(33, 777777); },
    removeRoot() { lines = lines.filter((line) => !line.startsWith('30 ')); } };
}

test('resource attribution includes reparented owned WebKit XPCs, excludes foreign coalitions/system WebKit and driver', () => {
  const f = fixture();
  const rows = f.sample();
  const browser = rows.find((r) => r.process === 'browser')!;
  assert.equal(browser.cpu_percent, 14);
  assert.equal(browser.rss_mb, 460);
  assert.deepEqual(Array.from(browser.pids), [30, 31, 32, 33]);
  assert.equal(rows.find((r) => r.process === 'total')!.rss_mb, 560);
  assert.equal(rows.find((r) => r.process === 'total')!.cpu_percent, 15);
  assert.ok(!f.lookups.flat().includes(35), 'unrelated system WebKit is not even inspected for coalition');
});

test('coalition cache reuses stable identities, invalidates reused PIDs, and never keeps an exited owner', () => {
  const f = fixture();
  f.sample(); f.sample();
  assert.equal(f.lookups.length, 1, 'stable samples do not spawn Python every second');
  f.replacePid();
  const next = f.sample().find((r) => r.process === 'browser')!;
  assert.deepEqual(Array.from(next.pids), [30, 31, 32]);
  assert.deepEqual(f.lookups[1], [33], 'only changed process incarnation is looked up');
  f.removeRoot();
  assert.equal(f.sample().some((r) => r.process === 'browser'), false, 'orphan XPCs require a currently owned browser root');
});

test('Chromium keeps descendant attribution without a macOS coalition lookup', () => {
  const module = loadSource(new URL('../e2e/process-resources.ts', import.meta.url), {
    'node:child_process': { execFileSync(command: string) {
      assert.equal(command, 'ps', 'Chromium never launches the macOS coalition reader');
      return [
        row(10, 1, 90, 500, '/node'),
        row(20, 1, 1, 100, '/owned/ottod'),
        row(30, 10, 2, 120, '/fixture/chromium/chrome'),
        row(31, 30, 3, 50, '/fixture/chromium/chrome'),
        row(32, 1, 50, 800, '/fixture/chromium/chrome'),
      ].join('\n');
    } },
  }, { process: { pid: 10, platform: 'darwin', env: {} } });
  const rows = module.processResources(20) as { process: string; rss_mb: number; pids: number[] }[];
  assert.deepEqual(Array.from(rows.find((r) => r.process === 'browser')!.pids), [30, 31]);
  assert.equal(rows.find((r) => r.process === 'total')!.rss_mb, 270);
});
