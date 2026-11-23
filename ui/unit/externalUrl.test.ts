import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

function loadExternal(opts: { isEmbedded?: boolean; tauri?: boolean; windowOpen?: (...args: unknown[]) => void; postToHost?: (m: unknown) => void; invoke?: (cmd: string, args: unknown) => Promise<unknown>; toastError?: (title: string, e: unknown) => void } = {}) {
  const win: Record<string, unknown> = {
    open: opts.windowOpen ?? (() => {}),
  };
  if (opts.tauri) win.__TAURI_INTERNALS__ = {};
  return loadSource(
    new URL('../src/lib/external.ts', import.meta.url),
    {
      './desktop': { isEmbedded: !!opts.isEmbedded },
      './embedGuest': { postToHost: opts.postToHost ?? (() => {}) },
      '@tauri-apps/api/core': { invoke: opts.invoke ?? (async () => {}) },
      './toastError': { toastError: opts.toastError ?? (() => {}) },
    },
    { window: win },
  );
}

test('safeHttpUrl admits only http(s) URLs without control characters', () => {
  const { safeHttpUrl, isExternalUrl } = loadExternal();
  assert.equal(safeHttpUrl(' https://example.com/a?b=1 '), 'https://example.com/a?b=1');
  assert.equal(safeHttpUrl('http://127.0.0.1:7700/health'), 'http://127.0.0.1:7700/health');
  assert.equal(isExternalUrl('HTTPS://OTTO.DEV'), true);

  for (const bad of [
    '',
    '   ',
    null,
    undefined,
    'mailto:user@example.com',
    'file:///etc/passwd',
    'javascript:alert(1)',
    'smb://fileserver/share',
    'ssh://host',
    'vscode://file/tmp/a',
    'data:text/html,<b>hi</b>',
    'https://example.com/\nEvil',
    'https://example.com/\u0000x',
  ]) {
    assert.equal(safeHttpUrl(bad), undefined, `expected reject: ${String(bad)}`);
    assert.equal(isExternalUrl(bad), false, `expected non-external: ${String(bad)}`);
  }
});

test('safeExternalUrl allows http(s) and mailto while rejecting local/arbitrary schemes', () => {
  const { safeExternalUrl } = loadExternal();
  assert.equal(safeExternalUrl('https://github.com/org/repo'), 'https://github.com/org/repo');
  assert.equal(safeExternalUrl('mailto:team@example.com'), 'mailto:team@example.com');

  for (const bad of [
    'file:///System/Applications/Calculator.app',
    'javascript:alert(1)',
    'vbscript:msgbox(1)',
    'smb://10.0.0.1/share',
    'ssh://root@prod',
    'x-apple.systempreferences:com.apple.preference',
    'mailto:a@b.com\r\nbcc:c@d.com',
  ]) {
    assert.equal(safeExternalUrl(bad), undefined, `expected reject: ${bad}`);
  }
});

test('openExternal blocks unsafe schemes and never invokes plugin:shell|open', async () => {
  const opened: unknown[][] = [];
  const invoked: string[] = [];
  const { openExternal } = loadExternal({
    tauri: false,
    windowOpen: (...args) => opened.push(args),
    invoke: async (cmd) => { invoked.push(cmd); },
  });

  await openExternal('file:///etc/passwd');
  await openExternal('javascript:alert(1)');
  await openExternal('smb://evil/share');
  assert.equal(opened.length, 0);
  assert.equal(invoked.length, 0);

  await openExternal('https://example.com/docs');
  assert.deepEqual(opened, [['https://example.com/docs', '_blank', 'noopener,noreferrer']]);
});

test('desktop: opens through the opener plugin and toasts a refusal instead of a dead window.open', async () => {
  const opened: unknown[][] = [];
  const invoked: [string, unknown][] = [];
  const toasted: [string, unknown][] = [];
  let refuse = false;
  const { openExternal } = loadExternal({
    tauri: true,
    windowOpen: (...args) => opened.push(args),
    invoke: async (cmd, args) => {
      invoked.push([cmd, args]);
      if (refuse) throw 'Not allowed to open url https://example.com/docs';
    },
    toastError: (title, e) => toasted.push([title, e]),
  });

  await openExternal('https://example.com/docs');
  assert.equal(JSON.stringify(invoked), JSON.stringify([['plugin:opener|open_url', { url: 'https://example.com/docs' }]]));
  assert.deepEqual(toasted, []);

  refuse = true;
  await openExternal('https://example.com/docs');
  assert.deepEqual(toasted, [['Couldn’t open the link', 'Not allowed to open url https://example.com/docs']]);
  // The WKWebView has no new-window handler: window.open would silently do nothing.
  assert.equal(opened.length, 0);
});
