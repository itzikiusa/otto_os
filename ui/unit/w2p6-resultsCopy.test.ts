import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// The Database results' copy helper (modules/database/results-format.ts) keeps
// its silent-by-default signature, but its failure toast is the shared one from
// lib/clipboard — what to do next, the same words as every other Copy.

function setup(copies: boolean) {
  const toasts: Array<[kind: string, title: string, body?: string]> = [];
  const recorded: string[] = [];
  const { COPY_FAILED_BODY } = loadSource(new URL('../src/lib/clipboard.ts', import.meta.url), {});
  const fmt = loadSource(
    new URL('../src/modules/database/results-format.ts', import.meta.url),
    {
      'lib/toast.svelte': {
        toasts: {
          error: (t: string, b?: string) => toasts.push(['error', t, b]),
          success: (t: string, b?: string) => toasts.push(['success', t, b]),
        },
      },
      'lib/clipboard': { copyText: async () => copies, COPY_FAILED_BODY },
      'lib/stores/clipHistory.svelte': { clipHistory: { record: (s: string) => recorded.push(s) } },
    },
    {},
    { relative: true },
  );
  return { fmt, toasts, recorded, COPY_FAILED_BODY };
}

test('a blocked clipboard shows the shared next-step body and records nothing', async () => {
  const h = setup(false);
  assert.equal(await h.fmt.copyWithToast('secret'), false);
  assert.deepEqual(h.toasts, [['error', 'Couldn’t copy', h.COPY_FAILED_BODY]]);
  assert.match(h.COPY_FAILED_BODY, /isn’t available here\. Select the text and copy it manually\./);
  assert.deepEqual(h.recorded, []);
});

test('a menu copy stays silent; a Copy button confirms with its own words', async () => {
  const h = setup(true);
  assert.equal(await h.fmt.copyWithToast('42'), true);
  assert.deepEqual(h.toasts, [], 'the "Copy value" menu items never toast on success');
  assert.equal(await h.fmt.copyWithToast('{}', ['Copied', 'Record JSON copied']), true);
  assert.deepEqual(h.toasts, [['success', 'Copied', 'Record JSON copied']]);
  assert.deepEqual(h.recorded, ['42', '{}'], 'both land in the clip history');
});
