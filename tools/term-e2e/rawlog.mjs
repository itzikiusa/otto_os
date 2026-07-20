// Capture codex's RAW PTY byte stream around one resize, to see exactly which
// control sequences carry the transcript re-emission.
import {
  bootstrap, makeApi, ensureWorkspace, TermClient,
  sleep, count, waitFor, waitBufferStable,
} from './client.mjs';

const env = await bootstrap();
const api = makeApi(env.base, env.token);
const wsId = await ensureWorkspace(api);
const s = await api('POST', `/api/v1/workspaces/${wsId}/sessions`, {
  kind: 'agent', provider: 'codex', title: 'rawlog', cwd: process.env.HOME,
});
const c = new TermClient(env.base, env.token, s.id, { cols: 120, rows: 32 });

const chunks = [];
let recording = false;
const origWrite = c.term.write.bind(c.term);
// Tap binary frames at the socket level: wrap connect's message path by
// monkey-patching term.write (all PTY bytes go through it).
c.term.write = (data, cb) => {
  if (recording && (data instanceof Uint8Array)) chunks.push(Buffer.from(data));
  return origWrite(data, cb);
};

await c.connect();
await waitFor(() => c.bufferText().trim().length > 80, 45_000);
await waitBufferStable(c, { quietMs: 3500, timeoutMs: 90_000 });
for (let attempt = 0; attempt < 4; attempt++) {
  c.type('Output the lines CODL_01 through CODL_20, one per line, no other text, no tools.');
  if (await waitFor(() => c.bufferText().includes('CODL_01'), 5000, 250)) break;
  await sleep(2000);
}
await sleep(600);
c.type('\r');
await waitFor(() => count(c.bufferText(), 'CODL_20') >= 1, 180_000, 500);
await waitBufferStable(c, { quietMs: 5000, timeoutMs: 90_000 });

console.log('=== recording one resize 120x32 -> 100x28 ===');
recording = true;
await c.resizeSettled(100, 28, 2500);
recording = false;

const raw = Buffer.concat(chunks).toString('latin1');
// Escape for reading: show CSI/OSC introducers symbolically.
const vis = raw
  .replaceAll('\x1b', '⟪ESC⟫')
  .replaceAll('\r', '⟪CR⟫')
  .replaceAll('\n', '⟪LF⟫\n');
console.log(`raw bytes: ${raw.length}`);
console.log(vis.slice(0, 12_000));
console.log('=== occurrences ===');
for (const seq of [
  ['DECSTBM set', /⟪ESC⟫\[\d*;\d*r/g],
  ['DECSTBM reset', /⟪ESC⟫\[r/g],
  ['ED2 clear', /⟪ESC⟫\[2J/g],
  ['ED0/J', /⟪ESC⟫\[0?J/g],
  ['SU scroll-up', /⟪ESC⟫\[\d*S/g],
  ['sync2026 on', /⟪ESC⟫\[\?2026h/g],
  ['sync2026 off', /⟪ESC⟫\[\?2026l/g],
  ['CODL lines', /CODL_\d\d/g],
]) {
  const m = vis.match(seq[1]);
  console.log(`${seq[0]}: ${m ? m.length : 0}`);
}

await api('DELETE', `/api/v1/sessions/${s.id}`).catch(() => {});
c.close();
await env.stop();
process.exit(0);
