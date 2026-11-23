// The making screen at real-life pace, for desktop-create and mobile-create,
// on the REAL productions server of the isolated E2E daemon: its fixed
// planner plans "a chest of gold" as the man, the chest, the coins in it and
// his line (crates/otto-media/src/productions/e2e_progress_plan.json), and the
// fake engine (create-fixture.ts) makes them from REAL Otto Forge outputs of
// the second Smart acceptance run: the man before his skeleton (man-v1), with
// a skeleton but no moves (man-v2) and with stand/walk/run (man-v3), the
// chest and the coins, their turnaround renders as the pictures it draws
// first. A step lasts as long as the test says: the fake engine holds each
// model's step after its progress line until the test lets it go, so the
// server's percent stands still for the whole step, exactly like the real
// engine (40 % for three minutes), and a bar or a clock that only follows
// the server is seen to freeze. Planning is held the same way (the daemon's
// planner thinks while `$OTTO_DATA_DIR/e2e-hold-planning` exists).
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect } from '@playwright/test';
import type { CreateSeed } from './create-fixture';

export { progressSamplesPresent } from './create-fixture';

/** The E2E daemon's data dir (global-setup writes it beside the root token). */
function dataDir(): string {
  const slot = process.env.OTTO_E2E_SLOT ?? '0';
  return (JSON.parse(readFileSync(join(process.cwd(), 'e2e', `.auth-${slot}`, 'daemon.json'), 'utf8')) as { dataDir: string }).dataDir;
}

/** The planner keeps thinking until `release`. */
export function holdPlanning(): { release: () => void } {
  const file = join(dataDir(), 'e2e-hold-planning');
  writeFileSync(file, 'held by the progress E2E\n');
  return { release: () => rmSync(file, { force: true }) };
}

/** The fake engine's steps, let go one at a time, by piece (man, chest, coins; the man's skeleton and moves are his too). */
export class Pace {
  private dir: string;
  constructor(seed: CreateSeed) {
    this.dir = join(seed.forgeDir, 'gates');
    mkdirSync(this.dir, { recursive: true });
  }

  /** The step the engine holds for `key` (its stage key), once it holds one. */
  async at(key: string): Promise<string> {
    const file = join(this.dir, `at-${key}`);
    let stage = '';
    await expect.poll(() => (stage = existsSync(file) ? readFileSync(file, 'utf8').trim() : ''), { message: `the engine holds ${key}`, timeout: 30_000 }).not.toBe('');
    return stage;
  }

  /** Let the held step of `key` end (the next one begins, or the run ends). */
  async next(key: string): Promise<string> {
    const stage = await this.at(key);
    const go = join(this.dir, `go-${key}`);
    writeFileSync(go, '');
    await expect.poll(() => existsSync(go), { message: `${key} went on from ${stage}`, timeout: 30_000 }).toBe(false);
    return stage;
  }

  /** Let `key` go on until the engine holds it at `stage`. */
  async until(key: string, stage: string): Promise<void> {
    for (let k = 0; k < 12 && (await this.at(key)) !== stage; k++) await this.next(key);
    expect(await this.at(key)).toBe(stage);
  }

  /** Let `key` go on to its end (its last step is `last`). */
  async through(key: string, last: string): Promise<void> {
    await this.until(key, last);
    await this.next(key);
  }

  /** Nothing is held any more (the engine runs on at its own pace). */
  stop(): void {
    rmSync(this.dir, { recursive: true, force: true });
  }
}
