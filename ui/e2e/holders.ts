import { execSync } from 'node:child_process';
import { realpathSync, rmSync } from 'node:fs';

// Detached `runtime-holder` processes (and the agent CLI each one runs)
// outlive the test daemon by design — that is how threads survive a daemon
// restart. The daemon keeps their sockets in `<data dir>/runtime-holders`,
// or, when that path is too long for a Unix socket (a macOS temp dir is),
// in `/tmp/otto-rth-<fnv1a64 of that dir>` (crates/otto-agent-protocol
// src/holder/paths.rs). A holder is the test daemon's when its `--socket`
// is in one of those two directories — matched by path, never by name.

function fnv1a64(s: string): string {
  let h = 0xcbf29ce484222325n;
  for (const b of Buffer.from(s)) {
    h ^= BigInt(b);
    h = (h * 0x100000001b3n) & 0xffffffffffffffffn;
  }
  return h.toString(16).padStart(16, '0');
}

/** The socket directories holders of `dataDir` can use (as written and as resolved). */
export function holderSocketDirs(dataDir: string): string[] {
  const roots = new Set([dataDir]);
  try {
    roots.add(realpathSync(dataDir));
  } catch {
    /* already gone */
  }
  const dirs: string[] = [];
  for (const r of roots) {
    const holders = `${r.replace(/\/+$/, '')}/runtime-holders`;
    dirs.push(holders, `/tmp/otto-rth-${fnv1a64(holders)}`);
  }
  return dirs;
}

type Proc = { pid: number; ppid: number; command: string };
function processes(): Proc[] {
  const out = execSync('ps -axo pid=,ppid=,command=', { encoding: 'utf8' });
  return out.split('\n').flatMap((line) => {
    const m = line.trim().match(/^(\d+)\s+(\d+)\s+(.*)$/);
    return m ? [{ pid: Number(m[1]), ppid: Number(m[2]), command: m[3] }] : [];
  });
}
const socketOf = (command: string): string | null => / runtime-holder .*--socket[ =](\S+)/.exec(command)?.[1] ?? null;

/** SIGKILL the holders of `dataDir` and the agent CLIs they run (children
 *  first, so none is left reparented), then remove the fallback socket dirs.
 *  Returns how many processes were stopped. Never throws. */
export function killHoldersFor(dataDir: string): number {
  let killed = 0;
  try {
    const dirs = holderSocketDirs(dataDir);
    const procs = processes();
    for (const h of procs) {
      const sock = socketOf(h.command);
      if (!sock || !dirs.some((d) => sock.startsWith(`${d}/`))) continue;
      for (const p of [...procs.filter((c) => c.ppid === h.pid).map((c) => c.pid), h.pid]) {
        if (p <= 1) continue;
        try {
          process.kill(p, 'SIGKILL');
          killed++;
        } catch {
          /* gone */
        }
      }
    }
    for (const d of dirs) if (d.startsWith('/tmp/otto-rth-')) rmSync(d, { recursive: true, force: true });
  } catch {
    /* best effort */
  }
  return killed;
}

/** Holders started by `ottod` (this run's binary) that nobody owns any more
 *  (reparented to launchd): what an earlier run left behind. Reported, not
 *  killed — another slot's daemon may be running from the same binary. */
export function orphanHolders(ottod: string): Proc[] {
  try {
    return processes().filter((p) => p.ppid === 1 && p.command.startsWith(`${ottod} runtime-holder `));
  } catch {
    return [];
  }
}
