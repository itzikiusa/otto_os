import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// copyWithToast (lib/clipboard.ts) is the one Copy-button helper: it copies,
// then toasts the outcome. A failure says what to do next (ux-design-6w1-02).

function harness(globals: Record<string, unknown>) {
  const shown: [string, string, string | undefined][] = [];
  const toasts = {
    success: (t: string, b?: string) => shown.push(['success', t, b]),
    error: (t: string, b?: string) => shown.push(['error', t, b]),
  };
  const mod = loadSource(new URL('../src/lib/clipboard.ts', import.meta.url), { './toast.svelte': { toasts } }, globals);
  return { ...mod, shown } as Record<string, any>;
}

const secureClipboard = (written: string[]) => ({
  navigator: { clipboard: { writeText: async (t: string) => void written.push(t) } },
  window: { isSecureContext: true },
  document: { hasFocus: () => true },
});

test('a copy toasts “Copied {what}” with the text as the body', async () => {
  const written: string[] = [];
  const h = harness(secureClipboard(written));
  assert.equal(await h.copyWithToast('main', 'the branch name'), true);
  assert.deepEqual(written, ['main']);
  assert.deepEqual(h.shown, [['success', 'Copied the branch name', 'main']]);
});

test('a long copied text is not repeated in the toast body', async () => {
  const h = harness(secureClipboard([]));
  await h.copyWithToast('x'.repeat(81), 'the log');
  assert.deepEqual(h.shown, [['success', 'Copied the log', undefined]]);
});

test('no clipboard at all: “Couldn’t copy” and the manual next step, and false', async () => {
  // An insecure origin with no async API and no document for the legacy path.
  const h = harness({ navigator: {}, window: { isSecureContext: false }, document: undefined });
  assert.equal(await h.copyWithToast('secret', 'the token'), false);
  assert.deepEqual(h.shown, [['error', 'Couldn’t copy', 'The clipboard isn’t available here. Select the text and copy it manually.']]);
});
