// Agent UI control — every module's handlers, registered at import. The shell
// (App.svelte) imports this once per document, before the events socket sends
// its `hello`, so `capabilities` lists them all. Add a module here when you add
// `uiCommands/<module>.ts` (unit/uiCommands.test.ts fails on a file that
// registers handlers but isn't imported here, and on catalog drift).

import { catalogDrift } from './catalog';
import { uiCapabilities, uiCommandModule } from '../uiCommands';

// ── Shell: state / open / focus (Agent 3) ──
import './nav';
// ── Database Explorer (Agent 4) ──
import './database';
// ── API client, Git, Browser (Agent 2) ──
import './api';
import './git';
import './browser';
// ── Kubernetes, AWS, Brokers, Vault, Workflows, … (Agent 5) ──
import './k8s';
import './aws';
import './brokers';
import './vault';
import './workflows';
import './scheduled';
import './home';
import './swarm';
import './loops';

if (import.meta.env.DEV) {
  const drift = catalogDrift(uiCapabilities().map((name) => ({ name, module: uiCommandModule(name) ?? '' })));
  if (drift.missing.length || drift.extra.length || drift.misplaced.length) {
    console.warn('[uiCommands] handlers and docs/contracts/ui-commands.json disagree', drift);
  }
}
