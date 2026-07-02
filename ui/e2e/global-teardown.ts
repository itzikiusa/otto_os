import { execSync } from 'node:child_process';
import { readFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';

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
  try {
    const meta = JSON.parse(readFileSync(metaFile, 'utf8')) as {
      pid?: number;
      dataDir?: string;
    };
    if (meta.pid) {
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
  } catch {
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
