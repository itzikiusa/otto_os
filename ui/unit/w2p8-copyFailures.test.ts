import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import { componentFunctions } from './componentFunctions.ts';

// ux-design-6w1-02: every clipboard failure toasts, and its body says what to
// do next. Drives the components' own copy handlers against the REAL
// lib/clipboard on an origin with no `navigator.clipboard` (LAN http, the
// self-signed listener) and no legacy path either.

const NEXT_STEP = /Select the (text|path) and copy it manually/;

function noClipboard() {
  const shown: [string, string, string | undefined][] = [];
  const toasts = {
    success: (t: string, b?: string) => shown.push(['success', t, b]),
    info: (t: string, b?: string) => shown.push(['info', t, b]),
    warn: (t: string, b?: string) => shown.push(['warn', t, b]),
    error: (t: string, b?: string) => shown.push(['error', t, b]),
  };
  const clipboard = loadSource(new URL('../src/lib/clipboard.ts', import.meta.url), { './toast.svelte': { toasts } }, {
    navigator: {},
    window: { isSecureContext: false },
    document: undefined,
  });
  return { clipboard, toasts, shown };
}

test('Markdown “Copy link” without navigator.clipboard toasts the failure (was a silent no-op)', async () => {
  const { clipboard, toasts, shown } = noClipboard();
  const { copy } = componentFunctions(new URL('../src/modules/agents/conversation/Markdown.svelte', import.meta.url), ['copy'], {
    copyText: clipboard.copyText,
    toasts,
    navigator: {},
  });
  await copy('https://example.test/x', 'https://example.test/x');
  assert.equal(shown.length, 1, 'exactly one toast');
  assert.equal(shown[0][0], 'error');
  assert.equal(shown[0][1], 'Couldn’t copy');
  assert.match(shown[0][2] ?? '', NEXT_STEP);
});

test('Notification “Copy details” failure goes through copyWithToast', async () => {
  const { clipboard, toasts, shown } = noClipboard();
  const { copyDetail } = componentFunctions(new URL('../src/shell/NotificationBell.svelte', import.meta.url), ['copyDetail'], {
    copyWithToast: clipboard.copyWithToast,
    toasts,
  });
  await copyDetail({ body: 'build failed on main' });
  assert.deepEqual(shown, [['error', 'Couldn’t copy', 'The clipboard isn’t available here. Select the text and copy it manually.']]);
});
