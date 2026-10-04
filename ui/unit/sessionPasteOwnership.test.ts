import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import ts from 'typescript';
import {deferred} from './sourceHarness.ts';

// Execute the production async handler while controlling its mutable pane.
function setup() {
  const text = readFileSync(new URL('../src/lib/components/Terminal.svelte', import.meta.url), 'utf8');
  const start = text.indexOf('<script lang=');
  const script = text.slice(text.indexOf('>', start) + 1, text.indexOf('</script>', start));
  const ast = ts.createSourceFile('terminal.ts', script, ts.ScriptTarget.Latest, true);
  const fn = ast.statements.find(s => ts.isFunctionDeclaration(s) && s.name?.text === 'uploadPastedImage')!;
  const upload = deferred<{path: string}>();
  const sentA: string[] = [], sentB: string[] = [], errors: unknown[] = [];
  const a = {readyState: 1, send: (v: string) => sentA.push(v)};
  const b = {readyState: 1, send: (v: string) => sentB.push(v)};
  const context: Record<string, any> = {socketFactory: null, readOnly: false, sessionId: 'a', sock: a,
    transformFrame: null, WebSocket: {OPEN: 1}, Blob, Uint8Array,
    toPngBytes: async () => new Uint8Array(), snipApi: {uploadPng: () => upload.promise},
    textToBase64: (s: string) => s, toasts: {error: (...args: unknown[]) => errors.push(args)}};
  context.sendJson = (v: unknown) => context.sock.send(JSON.stringify(v));
  runInNewContext(ts.transpileModule(fn.getText(ast), {compilerOptions: {target: ts.ScriptTarget.ES2022}}).outputText, context);
  return {context, upload, a, b, sentA, sentB, errors};
}
for (const change of ['switch', 'disconnect', 'readonly']) test(`image upload cannot cross terminal ownership after ${change}`, async () => {
  const h = setup(); const pending = h.context.uploadPastedImage({}); await Promise.resolve();
  if (change === 'switch') {h.context.sessionId = 'b'; h.context.sock = h.b;}
  else if (change === 'disconnect') h.a.readyState = 3;
  else h.context.readOnly = true;
  h.upload.resolve({path: '/tmp/image.png'}); await pending;
  assert.equal(h.sentB.length, 0); assert.equal(h.sentA.length, 0);
  assert.equal(h.errors.length, 1, 'failed delivery must be visible');
});
test('an image still owned by its connected writable terminal is pasted once', async () => {
  const h = setup(); const pending = h.context.uploadPastedImage({}); await Promise.resolve();
  h.upload.resolve({path: '/tmp/image.png'}); await pending;
  assert.equal(h.sentA.length, 1); assert.match(h.sentA[0], /image.png/); assert.equal(h.errors.length, 0);
});
