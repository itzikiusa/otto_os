// One-off: spawn a provider session, wait, dump the screen. Debug aid.
//   node probe.mjs claude [secondsToWait] [textToType]
import { bootstrap, makeApi, ensureWorkspace, TermClient, sleep } from './client.mjs';

const provider = process.argv[2] ?? 'claude';
const waitS = Number(process.argv[3] ?? 20);
const toType = process.argv[4];

const env = await bootstrap();
const api = makeApi(env.base, env.token);
const wsId = await ensureWorkspace(api);
const s = await api('POST', `/api/v1/workspaces/${wsId}/sessions`, {
  kind: 'agent', provider, title: `probe-${provider}`, cwd: process.env.HOME,
});
const c = new TermClient(env.base, env.token, s.id, { cols: 120, rows: 32 });
await c.connect();
const born = Date.now();
let died = null;
for (let t = 0; t < waitS; t++) {
  await sleep(1000);
  if (c.exitCode !== null && died === null) {
    died = (Date.now() - born) / 1000;
    console.log(`>>> exited after ${died.toFixed(1)}s with code ${c.exitCode}`);
    break;
  }
}
c.dump(`${provider} after ${((Date.now() - born) / 1000).toFixed(0)}s`);
const info = await api('GET', `/api/v1/sessions/${s.id}`);
console.log(`status=${info.status} exitCode(ws)=${c.exitCode} meta=${JSON.stringify(info.meta ?? {})}`);
if (toType) {
  c.type(toType);
  await sleep(1000);
  c.type('\r');
  await sleep(15_000);
  c.dump(`${provider} after typing`);
}
await api('DELETE', `/api/v1/sessions/${s.id}`).catch(() => {});
c.close();
await env.stop();
process.exit(0);
