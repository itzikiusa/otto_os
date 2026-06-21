import { readFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';

// Kill the throwaway daemon launched in global-setup and remove its temp data
// dir. Best-effort: never throw from teardown.
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
