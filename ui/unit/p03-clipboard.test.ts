import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// ux-design-6-07: the error copyTextOrThrow raises is the toast body every
// `try { copy } catch { toastError(…) }` site shows — it must say what to do.
test('a failed copy tells the user to copy manually', async () => {
  // No navigator and no document: neither the async API nor the legacy path can copy.
  const { copyTextOrThrow } = loadSource(new URL('../src/lib/clipboard.ts', import.meta.url), {}, {
    navigator: undefined,
    document: undefined,
    window: { isSecureContext: false },
  });
  await assert.rejects(() => copyTextOrThrow('x'), (e: Error) => {
    assert.equal(e.message, 'The clipboard isn’t available here. Select the text and copy it manually.');
    return true;
  });
});
