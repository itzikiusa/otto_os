// Render every Otto walkthrough to out/<Id>.mp4.
//   node render-all.mjs            → render all
//   node render-all.mjs Intro Git  → render just those ids
import { execFileSync } from 'node:child_process';
import { mkdirSync } from 'node:fs';

// Order = the recommended viewing order (Intro → features → Outro).
const ALL = [
  'Intro',
  'Sessions',
  'Git',
  'Review',
  'Product',
  'Connections',
  'Database',
  'Brokers',
  'Swarm',
  'Channels',
  'UsageInsights',
  'Skills',
  'Workflows',
  'Plugins',
  'Vault',
  'TeamMobile',
  'Platform',
  'Outro',
];

const ids = process.argv.slice(2).length ? process.argv.slice(2) : ALL;
mkdirSync('out', { recursive: true });

let failed = [];
for (const id of ids) {
  process.stdout.write(`\n=== rendering ${id} ===\n`);
  try {
    execFileSync(
      'npx',
      ['remotion', 'render', 'src/index.ts', id, `out/${id}.mp4`, '--log=error', '--jpeg-quality=92'],
      { stdio: 'inherit' },
    );
  } catch {
    failed.push(id);
    process.stdout.write(`!!! FAILED ${id}\n`);
  }
}

process.stdout.write(`\n=== done — ${ids.length - failed.length}/${ids.length} ok ===\n`);
if (failed.length) {
  process.stdout.write(`failed: ${failed.join(', ')}\n`);
  process.exit(1);
}
