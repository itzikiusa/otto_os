// A rewrite publish refused because the Jira issue / Confluence page changed
// since the last sync is shown as a conflict with a Refresh offer, not a raw
// error toast. Pins which errors count as that conflict.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { publishConflictMessage } from '../src/modules/product/publishConflict.ts';

test('the Jira and Confluence "changed since sync" 409s are publish conflicts', () => {
  const jira = 'the issue changed in Jira since Otto last synced it; refresh the story, review the changes, then publish again';
  assert.equal(publishConflictMessage({ status: 409, message: jira }), jira);
  const page = 'the Confluence page changed since Otto last synced it (version 3 → 4); refresh the story';
  assert.equal(publishConflictMessage({ status: 409, message: page }), page);
});

test('other failures stay ordinary errors', () => {
  // The lossy-content refusal is a 409 too, but a refresh would not fix it.
  assert.equal(
    publishConflictMessage({ status: 409, message: 'publishing would remove content Otto can’t round-trip' }),
    null,
  );
  assert.equal(publishConflictMessage({ status: 502, message: 'the issue changed in Jira' }), null);
  assert.equal(publishConflictMessage(new Error('boom')), null);
  assert.equal(publishConflictMessage(null), null);
});

test('the Rewrite tab offers Refresh on a publish conflict instead of an error toast', async () => {
  const calls: string[] = [];
  let refreshAnswer = true;
  const conflict = Object.assign(new Error('the issue changed in Jira since Otto last synced it; refresh'), { status: 409 });
  const tab = componentFunctions(new URL('../src/modules/product/RewriteTab.svelte', import.meta.url), ['publish'], {
    suggestedVersion: { id: 'v2', story_id: 's1', version_no: 2, body_md: 'Rewritten' },
    publishing: false,
    targetShort: 'PROJ-1', targetLabel: 'Jira PROJ-1', targetNoun: 'description',
    product: {
      selectedId: 's1',
      captureSelection: () => () => true,
      publishVersion: async () => { calls.push('publish'); throw conflict; },
      refresh: async () => { calls.push('refresh'); },
    },
    confirmOutward: async () => true,
    confirmer: { ask: async (msg: string, opts: { confirmLabel: string }) => { calls.push(`ask:${opts.confirmLabel}:${msg}`); return refreshAnswer; } },
    publishConflictMessage,
    initialLoad: async () => { calls.push('reload'); },
    toasts: { success: () => calls.push('success'), warn: () => calls.push('warn') },
    toastError: () => calls.push('toastError'),
  });
  await tab.publish();
  assert.deepEqual(calls, ['publish', `ask:Refresh:${conflict.message}`, 'refresh', 'reload']);
  assert.equal(tab.publishing, false);

  // Declining the refresh leaves the story alone; still no error toast.
  calls.length = 0;
  refreshAnswer = false;
  await tab.publish();
  assert.deepEqual(calls, ['publish', `ask:Refresh:${conflict.message}`]);
});
