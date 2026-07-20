// Per-resize-step server-history duplication probe for codex.
// After each settled resize, side-attach a fresh client and measure how many
// copies of each response line the SERVER snapshot holds.
import {
  bootstrap, makeApi, ensureWorkspace, TermClient,
  sleep, count, waitFor, waitBufferStable,
} from './client.mjs';

const env = await bootstrap();
const api = makeApi(env.base, env.token);
const wsId = await ensureWorkspace(api);
const s = await api('POST', `/api/v1/workspaces/${wsId}/sessions`, {
  kind: 'agent', provider: 'codex', title: 'dupprobe', cwd: process.env.HOME,
});
const c = new TermClient(env.base, env.token, s.id, { cols: 120, rows: 32 });
const NL = 40;
const maxCount = (text) => {
  let m = 0;
  for (let i = 1; i <= NL; i++) m = Math.max(m, count(text, `CODL_${String(i).padStart(2, '0')}`));
  return m;
};

async function sideCount(cols, rows) {
  const sc = new TermClient(env.base, env.token, s.id, { cols, rows });
  await sc.connect();
  await sleep(400);
  const m = maxCount(sc.bufferText());
  const lines = sc.bufferLines();
  sc.close();
  return { m, lines };
}

await c.connect();
await waitFor(() => c.bufferText().trim().length > 80, 45_000);
await waitBufferStable(c, { quietMs: 3500, timeoutMs: 90_000 });
for (let attempt = 0; attempt < 4; attempt++) {
  c.type(`Output the lines CODL_01 through CODL_${NL}, one per line, no other text, no tools.`);
  if (await waitFor(() => c.bufferText().includes('CODL_01'), 5000, 250)) break;
  await sleep(2000);
}
await sleep(600);
c.type('\r');
await waitFor(() => count(c.bufferText(), `CODL_${NL}`) >= 1, 180_000, 500);
await waitBufferStable(c, { quietMs: 5000, timeoutMs: 90_000 });

let side = await sideCount(120, 32);
console.log(`baseline: live=${maxCount(c.bufferText())} server=${side.m}`);

const steps = [[100, 28], [78, 24], [140, 40], [120, 32]];
let final;
for (const [cols, rows] of steps) {
  await c.resizeSettled(cols, rows, 1500);
  side = await sideCount(cols, rows);
  console.log(`after ${cols}x${rows}: live=${maxCount(c.bufferText())} server=${side.m}`);
  final = side;
}

// Show the pattern around the duplicated region in the server rebuild.
const marks = final.lines
  .map((l, i) => ({ l, i }))
  .filter(({ l }) => l.includes('CODL_03'));
console.log(`\nCODL_03 appears on rebuilt rows: ${marks.map((m) => m.i).join(', ')}`);
for (const { i } of marks.slice(0, 6)) {
  console.log(`--- context @${i}:`);
  console.log(final.lines.slice(Math.max(0, i - 2), i + 3).map((x) => '   |' + x).join('\n'));
}

await api('DELETE', `/api/v1/sessions/${s.id}`).catch(() => {});
c.close();
await env.stop();
process.exit(0);
