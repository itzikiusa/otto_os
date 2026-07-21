// Dump the CURRENT server-side snapshot of an existing session (read-only:
// no claim, and its resize is denied by sticky authority).
import { TermClient, sleep, count } from './client.mjs';
const [sid, cols, rows] = [process.argv[2], 200, 60];
const c = new TermClient(process.env.OTTO_BASE, process.env.OTTO_API_TOKEN, sid, { cols, rows });
await c.connect();
await sleep(500);
const lines = c.bufferLines();
console.log(`buffer rows: ${lines.length}`);
// Regions of interest from the user's screenshot:
const marks = ['healthy and alrea', 'drop(ti', '3457', 'assert!(!mgr.may_resize', 'recently sent `input`', 'Update(docs/contracts/ws.md', 'deploy-last.status'];
for (const m of marks) {
  const hits = lines.map((l, i) => [i, l]).filter(([, l]) => l.includes(m));
  console.log(`\n"${m}": ${hits.length} row(s)`);
  for (const [i, l] of hits.slice(0, 3)) console.log(`  row ${i} (len ${l.length}): ${JSON.stringify(l.slice(0, 180))}`);
}
c.close();
process.exit(0);
