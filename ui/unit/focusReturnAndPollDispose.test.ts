// Focus return when an inline panel closes from its own ✕, and the Logs
// Insights poll that must not re-arm after the view unmounted.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { componentFunctions } from './componentFunctions.ts';
import { deferred } from './sourceHarness.ts';

const src = (rel: string) => new URL(`../src/${rel}`, import.meta.url);

test('a Logs Insights poll in flight at unmount does not re-arm itself', async () => {
  const pending = deferred<any>();
  let armed = 0;
  const c = componentFunctions(src('modules/aws/LogsView.svelte'), ['pollResults'], {
    disposed: false,
    qid: 'q1',
    iResult: undefined,
    iError: '',
    iRunning: true,
    iRegion: 'us-east-1',
    iPoll: null,
    iPollN: 0,
    account: { id: 'acc' },
    awsApi: { logsInsightsResults: () => pending.promise },
    statusPollMs: () => 1000,
    document: { visibilityState: 'visible' },
    setTimeout: () => { armed++; return 1; },
  });
  const polling = c.pollResults('q1');
  c.disposed = true; // onDestroy while the `aws` call is still running
  pending.resolve({ done: false, rows: [], columns: [] });
  await polling;
  assert.equal(armed, 0, 'no further poll after unmount');
  assert.equal(c.iResult, undefined, 'the late result is dropped');

  // Control: a mounted view keeps polling an unfinished query.
  const live = deferred<any>();
  c.disposed = false;
  c.awsApi = { logsInsightsResults: () => live.promise };
  const again = c.pollResults('q1');
  live.resolve({ done: false, rows: [], columns: [] });
  await again;
  assert.equal(armed, 1);
});

test('closing the event detail returns focus to the selected row', () => {
  let focused = '';
  const row = { isConnected: true, focus: () => { focused = 'row'; } };
  const pane = { parentElement: { querySelector: (sel: string) => (sel === 'tr.trow.sel' ? row : null) } };
  const button = { closest: (sel: string) => (sel === '.detail' ? pane : null) };
  const c = componentFunctions(src('modules/aws/LogsView.svelte'), ['closeDetail'], { detail: { id: 'e1' } });
  c.closeDetail(button);
  assert.equal(c.detail, null);
  assert.equal(focused, 'row');
});

test('closing a Workbench side panel hands focus back to its toolbar toggle', () => {
  const calls: string[] = [];
  const toggles: Record<string, any> = {};
  for (const kind of ['history', 'placeholders']) {
    toggles[`[data-testid="wb-${kind}-toggle"]`] = {
      isConnected: true,
      getClientRects: () => [{}],
      focus: () => calls.push(`focus:${kind}`),
    };
  }
  const workbench = { panel: 'history', setPanel(p: string) { calls.push(`set:${p}`); this.panel = this.panel === p ? 'none' : p; } };
  const c = componentFunctions(src('modules/workbench/WorkbenchPage.svelte'), ['closeSidePanel'], {
    workbench,
    document: { querySelector: (sel: string) => toggles[sel] ?? null },
  });
  c.closeSidePanel('history');
  assert.deepEqual(calls, ['set:history', 'focus:history']);
  assert.equal(workbench.panel, 'none');
  // Already closed: never toggles it back OPEN.
  c.closeSidePanel('history');
  assert.equal(workbench.panel, 'none');
});

test('the inline DockedDrawer column restores focus to its opener on close', () => {
  const s = readFileSync(src('lib/components/DockedDrawer.svelte'), 'utf8');
  assert.match(s, /dialogReturnTarget\(\)/, 'records the opener when the column opens');
  assert.match(s, /queueMicrotask\(/, 'restores after the column unmounts');
  assert.match(s, /target\?\.isConnected/, 'only an opener still in the document');
});

test('Workbench live validation and autosave are not assertive live regions', () => {
  const s = readFileSync(src('modules/workbench/WorkbenchPage.svelte'), 'utf8');
  assert.doesNotMatch(s, /class="wb-issue" role="alert"/);
  assert.match(s, /class="wb-issue" role=\{issueFromFormat \? 'status' : undefined\}/);
  assert.doesNotMatch(s, /data-testid="wb-save-state" aria-live/);
});

test('Quick Open is a combobox whose options are not tab stops', () => {
  const s = readFileSync(src('modules/workbench/QuickOpen.svelte'), 'utf8');
  assert.match(s, /role="combobox"/);
  assert.match(s, /aria-activedescendant=\{results\.length > 0 \? `wb-qo-\$\{index\}` : undefined\}/);
  assert.match(s, /id="wb-qo-\{i\}"/);
  assert.doesNotMatch(s, /<li role="option"/, 'no button nested inside an option');
});
