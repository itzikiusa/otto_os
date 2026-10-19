import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';

test('Git collection pull failure keeps the dialog open and retry closes after success', async () => {
  let fail = true, closed = 0;
  const dialog = componentFunctions(new URL('../src/modules/api/GitSyncDialog.svelte', import.meta.url), ['pull'], {
    busy: false, pullError: '', repoId: 'repo',
    apiClient: { gitPullCollections: async () => { if (fail) throw new Error('Unavailable'); } },
    onclose: () => { closed++; }, loadErrorText: (e: Error) => e.message,
  });
  await dialog.pull();
  assert.equal(closed, 0);
  assert.equal(dialog.pullError, 'Unavailable');
  assert.equal(dialog.busy, false);
  fail = false;
  await dialog.pull();
  assert.equal(closed, 1);
  assert.equal(dialog.pullError, '');
});

test('cancelled mark deletion preserves the mark and its note', async () => {
  let confirmed = false, removed = 0;
  const rail = componentFunctions(new URL('../src/modules/browser/NotesRail.svelte', import.meta.url), ['remove'], {
    annotations: [{ id: 'mark', text: 'Important excerpt' }],
    confirmer: { ask: async () => confirmed },
    browser: { deleteAnnotation: async () => { removed++; } }, toastError() {},
  });
  await rail.remove('mark');
  assert.equal(removed, 0);
  confirmed = true;
  await rail.remove('mark');
  assert.equal(removed, 1);
});

test('send mark previews the target and sends only after confirmation', async () => {
  let confirmed = false; const sent: string[][] = []; const notices: string[] = [];
  const view = componentFunctions(new URL('../src/modules/browser/SendToSession.svelte', import.meta.url), ['eligible', 'sendTo'], {
    annotationId: 'mark', sending: false,
    ws: { agentSessions: [{ id: 'session', title: 'Ronaldo', provider: 'agy', status: 'working' }], statusMap: {} },
    browser: { annotations: [{ id: 'mark', text: 'Excerpt' }], sendAnnotation: async (...args: string[]) => sent.push(args) },
    confirmer: { ask: async (body: string) => { assert.match(body, /Ronaldo \(agy\)/); assert.match(body, /Excerpt/); return confirmed; } },
    toasts: { success: (title: string) => notices.push(title) }, toastError() {},
  });
  await view.sendTo('session');
  assert.equal(sent.length, 0);
  confirmed = true;
  await view.sendTo('session');
  assert.deepEqual(sent, [['mark', 'session']]);
  assert.deepEqual(notices, ['Sent to Ronaldo']);
});
