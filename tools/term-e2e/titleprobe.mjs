// Auto-title probe: create an UNTITLED provider session, prompt it, and watch
// provider_session_id + title evolve through the daemon's title sweep.
//   node titleprobe.mjs codex
import {
  bootstrap, makeApi, ensureWorkspace, TermClient,
  sleep, waitFor, waitBufferStable,
} from './client.mjs';

const provider = process.argv[2] ?? 'codex';
const env = await bootstrap();
const api = makeApi(env.base, env.token);
const wsId = await ensureWorkspace(api);
const s = await api('POST', `/api/v1/workspaces/${wsId}/sessions`, {
  kind: 'agent', provider, cwd: process.env.HOME,
});
console.log(`created untitled ${provider} session ${s.id} title="${s.title}" meta=${JSON.stringify(s.meta ?? {})}`);
const c = new TermClient(env.base, env.token, s.id, { cols: 120, rows: 32 });
await c.connect();
await waitFor(() => c.bufferText().trim().length > 80, 45_000);
await waitBufferStable(c, { quietMs: 3500, timeoutMs: 90_000 });
for (let attempt = 0; attempt < 4; attempt++) {
  c.type('Say hi about the moon landing in one short line.');
  if (await waitFor(() => c.bufferText().includes('moon landing'), 5000, 250)) break;
  await sleep(2000);
}
await sleep(600);
c.type('\r');

for (let t = 0; t <= 90; t += 10) {
  const cur = await api('GET', `/api/v1/sessions/${s.id}`);
  console.log(`t=${t}s  provider_session_id=${cur.provider_session_id ?? 'NULL'}  title="${cur.title}"  title_source=${cur.meta?.title_source ?? '-'}`);
  if (cur.meta?.title_source === 'provider') break;
  await sleep(10_000);
}

await api('DELETE', `/api/v1/sessions/${s.id}`).catch(() => {});
c.close();
await env.stop();
process.exit(0);
