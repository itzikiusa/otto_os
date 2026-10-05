import { execFileSync, execSync } from 'node:child_process';
import { readFileSync, readdirSync, realpathSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { request } from '@playwright/test';
import type { TelemetryConfig, TelemetryStatus } from '../src/lib/api/types';

// Kill the throwaway daemon launched in global-setup and remove its temp data
// dir. Best-effort: never throw from teardown.
//
// The daemon's usage engine spawns a `clickhouse server` CHILD whose config
// lives inside the temp data dir. SIGKILLing the daemon orphans that server
// (it reparents to launchd and runs forever — a dozen piled up before this),
// so kill it BY DATA-DIR MATCH before deleting the dir. The clickhouse
// watchdog respawns its child on abnormal exit, so the watchdog (parent pid)
// dies first.
export default async function globalTeardown(): Promise<void> {
  const SLOT = process.env.OTTO_E2E_SLOT ?? '0';
  const metaFile = join(process.cwd(), 'e2e', `.auth-${SLOT}`, 'daemon.json');
  let telemetry = false;
  try {
    const meta = JSON.parse(readFileSync(metaFile, 'utf8')) as {
      pid?: number;
      dataDir?: string;
      port?: string;
      telemetry?: boolean;
      command?: string;
      startedAt?: string;
    };
    telemetry = meta.telemetry === true;
    if (telemetry) {
      await stopTelemetryFixture(meta, join(process.cwd(), 'e2e', `.auth-${SLOT}`, 'state.json'));
      // The asynchronous shutdown must not authorize deleting a replacement
      // fixture's data, even when it reused the old port and root credentials.
      const currentMeta = JSON.parse(readFileSync(metaFile, 'utf8')) as typeof meta;
      if (currentMeta.pid !== meta.pid || currentMeta.command !== meta.command || currentMeta.startedAt !== meta.startedAt || currentMeta.dataDir !== meta.dataDir || (meta.pid && processIdentity(meta.pid) !== null)) {
        throw new Error('Fixture ownership changed during shutdown; preserving fixture data');
      }
    } else if (meta.pid) {
      try {
        process.kill(meta.pid, 'SIGKILL');
      } catch {
        /* already gone */
      }
    }
    if (meta.dataDir) {
      killClickhouseFor(meta.dataDir);
      try {
        rmSync(meta.dataDir, { recursive: true, force: true });
      } catch {
        /* ignore */
      }
    }
  } catch (error) {
    if (telemetry) console.warn('[e2e] Telemetry cleanup incomplete; preserving fixture data:', error instanceof Error ? error.message : String(error));
    /* no meta file — nothing to clean up */
  }
}

/** SIGKILL any clickhouse process whose command line references `dataDir`
 *  (the server), plus its parent (the watchdog — killed first so it can't
 *  respawn the server). Never touches pid ≤ 1. */
export function killClickhouseFor(dataDir: string): void {
  try {
    const out = execSync('ps -axo pid=,ppid=,command=', { encoding: 'utf8' });
    for (const line of out.split('\n')) {
      if (!line.includes(dataDir) || !line.includes('clickhouse')) continue;
      const m = line.trim().match(/^(\d+)\s+(\d+)/);
      if (!m) continue;
      const pid = Number(m[1]);
      const ppid = Number(m[2]);
      for (const p of [ppid, pid]) {
        if (p > 1) {
          try {
            process.kill(p, 'SIGKILL');
          } catch {
            /* gone / not ours */
          }
        }
      }
    }
  } catch {
    /* ps unavailable — skip */
  }
}

type OwnedProcess = { pid: number; command: string; startedAt: string };
function processIdentity(pid: number): OwnedProcess | null {
  try {
    return {
      pid,
      command: execFileSync('ps', ['-p', String(pid), '-o', 'command='], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim(),
      startedAt: execFileSync('ps', ['-p', String(pid), '-o', 'lstart='], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim(),
    };
  } catch { return null; }
}
function stillOwned(expected: OwnedProcess): boolean {
  const current = processIdentity(expected.pid);
  return current?.command === expected.command && current?.startedAt === expected.startedAt;
}
function signalOwned(process: OwnedProcess, signal: NodeJS.Signals): void {
  if (!stillOwned(process)) return;
  try { globalThis.process.kill(process.pid, signal); } catch { /* exited */ }
}
function collectorChildren(daemon: OwnedProcess, dataDir: string): OwnedProcess[] {
  if (!stillOwned(daemon)) return [];
  try {
    const dir = join(realpathSync(dataDir), 'telemetry');
    const commands = new Set(readdirSync(dir).filter((name) => /^collector-\d+\.\d+\.\d+$/.test(name))
      .map((name) => `${join(dir, name, 'otelcol-contrib')} --config ${join(dir, 'collector.json')}`));
    // Only direct children of this verified fixture daemon. No global process
    // scan, name match, or data-directory substring is used for collectors.
    return execFileSync('pgrep', ['-P', String(daemon.pid)], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] })
      .trim().split(/\s+/).map(Number).filter((pid) => Number.isInteger(pid) && pid > 1)
      .map(processIdentity).filter((child): child is OwnedProcess => child !== null && commands.has(child.command));
  } catch { return []; }
}
async function waitForExit(process: OwnedProcess, timeout: number): Promise<void> {
  const deadline = Date.now() + timeout;
  while (stillOwned(process) && Date.now() < deadline) await new Promise((resolve) => setTimeout(resolve, 100));
}
async function stopTelemetryFixture(meta: {
  pid?: number; dataDir?: string; port?: string; command?: string; startedAt?: string;
}, stateFile: string): Promise<void> {
  const daemon = meta.pid && meta.command && meta.startedAt ? { pid: meta.pid, command: meta.command, startedAt: meta.startedAt } : null;
  if (daemon === null || !stillOwned(daemon)) throw new Error('Daemon ownership unavailable; preserving fixture data');
  const collectors = meta.dataDir ? collectorChildren(daemon, meta.dataDir) : [];
  const ctx = await request.newContext({ timeout: 5_000 });
  try {
    const state = JSON.parse(readFileSync(stateFile, 'utf8')) as { origins: Array<{ localStorage: Array<{ name: string; value: string }> }> };
    const token = state.origins.flatMap((origin) => origin.localStorage).find((entry) => entry.name === 'otto_token')?.value;
    if (token && meta.port) {
      const api = `http://127.0.0.1:${meta.port}/api/v1/telemetry`;
      const headers = { Authorization: `Bearer ${token}` };
      const original = await ctx.get(`${api}/config`, { headers });
      if (original.ok()) {
        const config = await original.json() as TelemetryConfig;
        await original.dispose();
        if (!stillOwned(daemon)) throw new Error('Daemon ownership changed before disabling telemetry');
        const disabled = await ctx.put(`${api}/config`, { headers, data: { ...config, enabled: false, native_profiling: false } });
        if (!disabled.ok()) throw new Error(`disable telemetry returned HTTP ${disabled.status()}`);
        await disabled.dispose();
        const deadline = Date.now() + 10_000;
        while (Date.now() < deadline) {
          const response = await ctx.get(`${api}/status`, { headers });
          if (!response.ok()) break;
          const status = await response.json() as TelemetryStatus;
          await response.dispose();
          if (!status.enabled && !status.collector_ready) break;
          await new Promise((resolve) => setTimeout(resolve, 100));
        }
      }
    }
  } catch {
    if (!stillOwned(daemon)) throw new Error('Daemon ownership changed during telemetry API cleanup; preserving fixture data');
    // Startup may have failed mid-download or the API may already be down.
    // Owned-process shutdown below remains responsible for this fixture.
  } finally { await ctx.dispose(); }
  if (daemon) {
    if (meta.dataDir) collectors.push(...collectorChildren(daemon, meta.dataDir));
    signalOwned(daemon, 'SIGTERM');
    await waitForExit(daemon, 10_000);
    if (meta.dataDir) collectors.push(...collectorChildren(daemon, meta.dataDir));
    signalOwned(daemon, 'SIGKILL');
    await waitForExit(daemon, 2_000);
    if (stillOwned(daemon)) throw new Error('Owned telemetry daemon did not stop');
  }
  for (const collector of new Map(collectors.map((item) => [item.pid, item])).values()) {
    signalOwned(collector, 'SIGTERM');
    await waitForExit(collector, 2_000);
    signalOwned(collector, 'SIGKILL');
    await waitForExit(collector, 1_000);
    if (stillOwned(collector)) throw new Error('Owned telemetry collector did not stop; preserving fixture data');
  }
}
