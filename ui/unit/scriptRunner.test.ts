import test from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

function fixture() {
  let created = 0, posted = 0, terminated = 0;
  const worker: any = { onmessage: null, onerror: null, onmessageerror: null,
    postMessage() { posted++; }, terminate() { terminated++; } };
  const protocol = loadSource(new URL('../src/lib/api/scriptProtocol.ts', import.meta.url), {});
  const runner = loadSource(new URL('../src/lib/api/scriptRunner.ts', import.meta.url), {
    './scriptProtocol': protocol,
    './scriptWorkerFactory': { createScriptWorker: () => { created++; return worker; } },
  });
  const input = { kind: 'pre', code: 'while(true){}', request: { method:'GET',url:'https://example.test',headers:[],body:'' }, vars:{} };
  return { worker, runner, input, counts: () => ({ created, posted, terminated }) };
}

test('abort terminates pending worker and ignores late messages', async () => {
  const f = fixture(); const ctrl = new AbortController();
  const pending = f.runner.runScript(f.input, ctrl.signal);
  const late = f.worker.onmessage;
  ctrl.abort(); await assert.rejects(pending, { name:'AbortError' });
  late?.({data:{run:{logs:[],tests:[]},vars:{late:'no'}}});
  assert.equal(f.counts().terminated, 1); assert.equal(f.worker.onmessage, null);
});

test('elapsed deadline terminates a script that never replies', async () => {
  const f=fixture();
  await assert.rejects(f.runner.runScript(f.input, undefined, 10), /timed out/i);
  assert.equal(f.counts().terminated, 1);
});

test('oversized input rejects before any worker message', async () => {
  const f=fixture(); f.input.request.body='x'.repeat(8*1024*1024);
  await assert.rejects(f.runner.runScript(f.input), /payload.*budget/i);
  assert.equal(f.counts().posted, 0); assert.equal(f.counts().created, 0);
});

test('postMessage clone failure terminates and settles exactly once', async () => {
  const f=fixture(); f.worker.postMessage=()=>{ throw new DOMException('Cannot clone','DataCloneError'); };
  await assert.rejects(f.runner.runScript(f.input), /clone/i);
  assert.equal(f.counts().terminated, 1);
});

test('messageerror terminates and success cleans its worker', async () => {
  const f=fixture(); const pending=f.runner.runScript(f.input);
  f.worker.onmessageerror({}); await assert.rejects(pending, /decode|message/i);
  assert.equal(f.counts().terminated,1);
  const g=fixture();const success=g.runner.runScript(g.input);
  g.worker.onmessage({data:{run:{logs:['ok'],tests:[]},request:g.input.request,vars:{next:'yes'}}});
  assert.equal((await success).vars.next,'yes');assert.equal(g.counts().terminated,1);
});

test('already aborted script creates no worker', async () => {
  const f=fixture();const c=new AbortController();c.abort();
  await assert.rejects(f.runner.runScript(f.input,c.signal),{name:'AbortError'});
  assert.equal(f.counts().created,0);
});
