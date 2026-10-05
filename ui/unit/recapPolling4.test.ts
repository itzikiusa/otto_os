import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import { deferred, loadSource } from './sourceHarness.ts';

const path = new URL('../src/modules/rooms/RecapPanel.svelte', import.meta.url);
const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value));
const settle = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };

/** Execute the actual component script and its onMount callback. Only transport
 * and the visible-poll scheduler are adapters: there is no substitute polling
 * implementation and this makes no claim about mounted DOM/Svelte reactivity. */
function panel(eventCount = 100, scheduler?: (run: (signal: AbortSignal) => unknown, opts: {ms: number}) => {stop(): void}) {
  const script = readFileSync(path, 'utf8').split('<script lang="ts">')[1].split('</script>')[0];
  const file = ts.createSourceFile('recap.ts', script, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const source = file.statements.filter(n => !ts.isImportDeclaration(n)).map(n => n.getText(file)).join('\n');
  let poll: (signal: AbortSignal) => unknown;
  let cleanup: () => void;
  let stopped = false;
  const controller = new AbortController();
  const calls: string[] = [];
  let bytes = 0;
  let metadata = { id: 'recap-1', session_title: 'Completed recap', status: 'stopped', last_seq: eventCount, summary_status: 'ready', summary_through_seq: eventCount, bytes_used: 4096, quota_bytes: 1_000_000, speech_available: false };
  let events = Array.from({ length: eventCount }, (_, i) => ({ seq: i + 1, created_at: '2026-10-05T00:00:00Z', payload: { type: 'terminal', data_base64: 'e'.repeat(2048) } }));
  let draft: { overview: string } | null = { overview: 'Original draft' };
  let eventsRevision = `events-${eventCount}`, draftRevision: string | null = 'draft-1';
  let delayNext: ReturnType<typeof deferred<unknown>> | null = null;
  let failRevision = false;
  const context: Record<string, any> = {
    console, Error, AbortController, encodeURIComponent, setTimeout, clearTimeout,
    $props: () => ({ recapId: 'recap-1' }), $state: (value: unknown) => value,
    onMount: (fn: () => () => void) => { cleanup = fn(); },
    pollWhileVisible: (fn: typeof poll, opts: {ms: number}) => { poll = fn; const scheduled = scheduler?.(fn, opts); return { stop() { stopped = true; controller.abort(); scheduled?.stop(); } }; },
    loadErrorText: (e: unknown) => String(e),
    recapRequest: async (url: string) => {
      calls.push(url);
      if (url.endsWith('/revision')) {
        if (failRevision) throw new Error('Revision temporarily unavailable');
        return clone({ metadata, events_revision: eventsRevision, draft_revision: draftRevision });
      }
      assert.match(url, /^\/room-recaps\/recap-1\?after=\d+&limit=100$/);
      const after = Number(new URL(url, 'http://fixture').searchParams.get('after'));
      const remaining = events.filter(event => event.seq > after);
      const page = remaining.slice(0, 100);
      const value = clone({ metadata, events: page, next_cursor: remaining.length > page.length ? page.at(-1)!.seq : null, draft });
      bytes += JSON.stringify(value).length;
      if (delayNext) { const wait = delayNext; delayNext = null; await wait.promise; }
      return value;
    },
  };
  const expose = '\nglobalThis.panelState = () => ({detail, error, loading}); globalThis.manualLoad = () => load();';
  runInNewContext(ts.transpileModule(source + expose, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, context);
  return {
    tick: async () => { await poll!(controller.signal); },
    close: () => cleanup!(),
    state: () => clone(context.panelState()),
    refresh: () => context.manualLoad(),
    get stopped() { return stopped; },
    get bodyCalls() { return calls.filter(p => !p.endsWith('/revision')).length; },
    get revisionCalls() { return calls.filter(p => p.endsWith('/revision')).length; },
    get bytes() { return bytes; },
    delayBody() { const wait = deferred<unknown>(); delayNext = wait; return wait; },
    changeDraft(value: string | null) { draft = value === null ? null : { overview: value }; draftRevision = value === null ? null : `draft-${value}`; },
    changeStatus() { metadata = { ...metadata, summary_status: 'running' }; },
    appendEvent() { metadata = { ...metadata, last_seq: metadata.last_seq + 1 }; eventsRevision = `events-${metadata.last_seq}`; events = [...events, { seq: metadata.last_seq, created_at: '', payload: { type: 'terminal', data_base64: 'new tail' } }]; },
    failStatus(value: boolean) { failRevision = value; },
  };
}

test('Recap unchanged completed archive polls metadata without retransferring its body', async () => {
  const p = panel();
  try {
    await p.tick(); const initialBytes = p.bytes;
    for (let i = 0; i < 15; i++) await p.tick();
    assert.equal(p.state().detail.events.length, 100);
    assert.equal(p.bodyCalls, 1, '15 unchanged four-second ticks must not reread the full event page and draft');
    assert.equal(p.bytes, initialBytes);
    assert.ok(p.revisionCalls >= 15, 'stopped archives still check for external edits and generation');
  } finally { p.close(); }
});

test('Recap summary metadata alone updates without another body request', async () => {
  const p = panel();
  try {
    await p.tick(); p.changeStatus(); await p.tick();
    assert.equal(p.state().detail.metadata.summary_status, 'running');
    assert.equal(p.state().detail.draft.overview, 'Original draft');
    assert.equal(p.bodyCalls, 1);
  } finally { p.close(); }
});

test('Recap append beyond an older complete page updates metadata without rereading that page', async () => {
  const p = panel(200);
  try {
    await p.tick(); p.appendEvent(); await p.tick();
    assert.equal(p.state().detail.metadata.last_seq, 201);
    assert.equal(p.state().detail.events.at(-1).seq, 100);
    assert.equal(p.state().detail.next_cursor, 100);
    assert.equal(p.bodyCalls, 1);
  } finally { p.close(); }
});

test('Recap partial newest page appends after a missed event, then resumes cheap unchanged checks', async () => {
  const p = panel(99);
  try {
    await p.tick(); p.appendEvent(); await p.tick();
    assert.equal(p.state().detail.events.length, 100);
    assert.equal(p.state().detail.events.at(-1).seq, 100);
    assert.equal(p.state().detail.next_cursor, null);
    await p.tick(); assert.equal(p.bodyCalls, 2);
  } finally { p.close(); }
});

test('Recap exact page boundary exposes the next cursor without appending 101 rows or moving the page', async () => {
  const p = panel(100);
  try {
    await p.tick(); assert.equal(p.state().detail.next_cursor, null);
    p.appendEvent(); await p.tick();
    const current = p.state().detail;
    assert.equal(current.metadata.last_seq, 101);
    assert.equal(current.events.length, 100);
    assert.equal(current.events[0].seq, 1, 'a new event must not move the user to a different page');
    assert.equal(current.events.at(-1).seq, 100);
    assert.equal(current.next_cursor, 100, 'the appended event is reachable through Next events');
    const fetched = p.bodyCalls;
    await p.tick(); assert.equal(p.bodyCalls, fetched);
  } finally { p.close(); }
});

test('Recap external draft replacement and removal remain observable after capture stops', async () => {
  const p = panel();
  try {
    await p.tick(); p.changeDraft('Externally regenerated'); await p.tick();
    assert.equal(p.state().detail.draft.overview, 'Externally regenerated');
    p.changeDraft(null); await p.tick(); assert.equal(p.state().detail.draft, null);
    await p.tick(); assert.equal(p.bodyCalls, 3);
  } finally { p.close(); }
});

test('Recap delayed old body cannot be acknowledged as a newer draft revision', async () => {
  const p = panel(); const held = p.delayBody();
  try {
    const loading = p.tick(); await settle();
    p.changeDraft('Edited during body request'); held.resolve(undefined); await loading;
    // A stable revision-body pair may be recovered in the first load or next tick.
    await p.tick(); assert.equal(p.state().detail.draft.overview, 'Edited during body request');
    const fetched = p.bodyCalls;
    await p.tick(); assert.equal(p.bodyCalls, fetched, 'the recovered body becomes cacheable only with its own revision');
  } finally { held.resolve(undefined); p.close(); }
});

test('Recap explicit refresh still reads body and a closed panel ignores pending body completion', async () => {
  const p = panel();
  await p.tick(); await p.refresh(); assert.equal(p.bodyCalls, 2);
  const held = p.delayBody(); p.changeDraft('Should not reopen');
  const pending = p.refresh(); await settle(); const before = p.state(); p.close();
  held.resolve(undefined); await pending;
  assert.equal(p.stopped, true);
  assert.deepEqual(p.state().detail, before.detail);
});

test('Recap revision error retains content and recovery clears error without rereading unchanged body', async () => {
  const p = panel();
  try {
    await p.tick(); p.failStatus(true); await p.tick();
    const failed = p.state(); p.failStatus(false); await p.tick();
    assert.match(failed.error, /Revision temporarily unavailable/);
    assert.equal(failed.detail.draft.overview, 'Original draft');
    assert.equal(p.state().error, ''); assert.equal(p.bodyCalls, 1);
  } finally { p.close(); }
});


test('Recap revision failures back off the real visible poller and recovery restores cadence', async () => {
  const timers = new Map<number, {fn: () => void; ms: number}>(); let timerId = 0;
  const {pollWhileVisible} = loadSource(new URL('../src/lib/poll.ts', import.meta.url), {
    './api/lane': {inLane: (_lane: string, run: () => unknown) => run(), tagSignal() {}},
  }, {
    setTimeout: (fn: () => void, ms: number) => { const id = ++timerId; timers.set(id, {fn, ms}); return id; },
    clearTimeout: (id: number) => timers.delete(id),
  });
  const p = panel(100, (run, opts) => pollWhileVisible(run, {...opts, jitter: 0}));
  const nextDelay = () => { assert.equal(timers.size, 1, 'exactly one poll chain remains scheduled'); return [...timers.values()][0].ms; };
  const fire = async () => {
    const [id, timer] = [...timers][0]; timers.delete(id); timer.fn(); await settle();
  };
  try {
    await settle(); assert.equal(nextDelay(), 4000);
    p.failStatus(true); await fire(); const firstFailureDelay = nextDelay();
    await fire(); const secondFailureDelay = nextDelay();
    p.failStatus(false); await fire();
    assert.equal(firstFailureDelay, 8000, 'revision transport errors must reach the poller failure contract');
    assert.equal(secondFailureDelay, 16000);
    assert.equal(nextDelay(), 4000);
    assert.equal(p.state().error, ''); assert.equal(p.bodyCalls, 1);
  } finally { p.close(); }
  assert.equal(timers.size, 0, 'closing removes the scheduled recovery tick');
});
