#!/usr/bin/env node
// Run every populated workload in the inventory, serially and in its required
// browser. Full App pages collect application telemetry; standalone component
// fixtures keep their own direct performance probes. The transcript is synthetic.
import { TELEMETRY_LOAD_SCENARIOS } from '../../ui/e2e/telemetry-load-manifest.ts';
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

if (process.argv.includes('--list')) {
  for (const row of TELEMETRY_LOAD_SCENARIOS) console.log(`${row.project ?? 'desktop-browser'} ${row.spec}: ${row.workload}`);
  process.exit(0);
}
if (!process.env.OTTO_E2E_BIN) throw new Error('Set OTTO_E2E_BIN to this worktree’s freshly built target/debug/ottod');
const output = process.argv[2] ? resolve(process.argv[2]) : mkdtempSync(join(tmpdir(), 'otto-telemetry-components-'));
mkdirSync(output, { recursive: true });
const transcript = join(output, 'synthetic-transcript.jsonl');
const records = [];
for (let i = 0; i < 2000; i++) {
  const common = { sessionId: 'telemetry-load-fixture', timestamp: new Date(Date.UTC(2026, 8, 29, 12, 0, i)).toISOString() };
  records.push({ ...common, type: 'user', uuid: `user-${i}`, message: { role: 'user', content: `Fixture request ${i}: explain this operation.` } });
  records.push({ ...common, type: 'assistant', uuid: `assistant-${i}`, message: { id: `message-${i}`, role: 'assistant', model: 'fixture', stop_reason: 'end_turn', content: [{ type: 'text', text: `Fixture answer ${i}.\n\n` + 'Synthetic performance evidence. '.repeat(60) }] } });
}
writeFileSync(transcript, records.map((r) => JSON.stringify(r)).join('\n') + '\n');
const env = { ...process.env, OTTO_E2E_BIN: resolve(process.env.OTTO_E2E_BIN),
  OTTO_E2E_SLOT: process.env.OTTO_E2E_SLOT ?? 'telemetry-components',
  OTTO_E2E_PORT: process.env.OTTO_E2E_PORT ?? '7843', OTTO_E2E_PW_PORT: process.env.OTTO_E2E_PW_PORT ?? '5343',
  OTTO_E2E_SWEEP_ORPHANS: '0', OTTO_E2E_TELEMETRY: '1', OTTO_CONV_FIXTURE: transcript };
console.log('Telemetry collection: enabled for full App pages and daemon operations.');
console.log('Standalone component fixtures retain their own direct performance probes; they do not boot App telemetry instrumentation.');
writeFileSync(join(output, 'collection-scope.json'), JSON.stringify({
  applicationTelemetry: true,
  fullAppPages: 'UI and server telemetry enabled; collector readiness required before workloads',
  standaloneComponents: 'Direct fixture performance probes; no App telemetry bootstrap',
}, null, 2) + '\n');
let failed = false;
for (const project of ['desktop-browser', 'desktop-webkit']) {
  const specs = TELEMETRY_LOAD_SCENARIOS.filter((row) => (row.project ?? 'desktop-browser') === project).map((row) => `e2e/${row.spec}`);
  const run = spawnSync('npx', ['playwright', 'test', `--project=${project}`, '--workers=1', `--output=${join(output, project)}`, ...specs],
    { cwd: fileURLToPath(new URL('../../ui', import.meta.url)), env, stdio: 'inherit' });
  if (run.error) throw run.error;
  if (run.signal) { failed = true; break; }
  failed ||= run.status !== 0;
}
console.log(`Component workload evidence: ${output}`);
process.exitCode = failed ? 1 : 0;
