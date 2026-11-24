// Shared fixture for the header-verbs specs (desktop-git-header-verbs.spec.ts,
// git-header-verbs-mobile.spec.ts): four real repositories, each a clone of
// its own local bare remote (nothing leaves the machine), on a real-length
// branch name with a tracked upstream, in the four states the Git header must
// read at a glance: clean + synced, dirty, ahead, behind with stashes.
import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, seedWorkspace, tempDir } from './seed';

export type VerbState = 'synced' | 'dirty' | 'ahead' | 'behind';
export const VERB_STATES: VerbState[] = ['synced', 'dirty', 'ahead', 'behind'];
export const VERB_BRANCH = 'feature/PLAT-4821-wallet-gateway-retry-backoff';

const run = (dir: string, ...a: string[]): void => void execFileSync('git', ['-C', dir, ...a], { stdio: 'ignore' });
function ident(dir: string): void {
  run(dir, 'config', 'user.email', 'e2e@otto.local');
  run(dir, 'config', 'user.name', 'E2E');
  run(dir, 'config', 'commit.gpgsign', 'false');
}
function commit(dir: string, file: string, body: string, msg: string): void {
  writeFileSync(join(dir, file), body);
  run(dir, 'add', '-A');
  run(dir, 'commit', '-q', '-m', msg);
}

/** One repository per state, registered in one workspace (so the Git page's
 *  tab strip carries four repos, as a daily user's does). */
export async function seedVerbRepos(tag: string): Promise<Record<VerbState, string>> {
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  const ids = {} as Record<VerbState, string>;
  for (const state of VERB_STATES) {
    const root = tempDir(`otto-e2e-verbs-${state}-`);
    const bare = join(root, 'origin.git');
    const seed = join(root, 'seed');
    const dir = join(root, 'work');
    execFileSync('git', ['init', '-q', '--bare', '-b', 'main', bare]);
    execFileSync('git', ['init', '-q', '-b', 'main', seed]);
    ident(seed);
    for (let i = 0; i < 12; i++) commit(seed, `src_${i}.go`, `package wallet\n\n// v${i}\n`, `feat(wallet): step ${i}`);
    run(seed, 'remote', 'add', 'origin', bare);
    run(seed, 'push', '-q', '-u', 'origin', 'main');
    run(seed, 'checkout', '-q', '-b', VERB_BRANCH);
    commit(seed, 'retry.go', 'package wallet\n', 'feat(wallet): retry with backoff');
    run(seed, 'push', '-q', '-u', 'origin', VERB_BRANCH);
    execFileSync('git', ['clone', '-q', '-b', VERB_BRANCH, bare, dir]);
    ident(dir);
    if (state === 'dirty') {
      writeFileSync(join(dir, 'retry.go'), 'package wallet\n\nconst maxRetries = 5\n');
      writeFileSync(join(dir, 'retry_test.go'), 'package wallet\n');
    } else if (state === 'ahead') {
      commit(dir, 'retry.go', 'package wallet\n\nconst maxRetries = 5\n', 'feat(wallet): cap retries');
      commit(dir, 'backoff.go', 'package wallet\n', 'feat(wallet): exponential backoff');
    } else if (state === 'behind') {
      for (let i = 0; i < 3; i++) commit(seed, `upstream_${i}.go`, 'package wallet\n', `fix(wallet): upstream ${i}`);
      run(seed, 'push', '-q', 'origin', VERB_BRANCH);
      run(dir, 'fetch', '-q', 'origin');
      // Two stashes, the tree clean again.
      for (let i = 0; i < 2; i++) {
        writeFileSync(join(dir, 'retry.go'), `package wallet\n// wip ${i}\n`);
        run(dir, 'stash', 'push', '-q', '-m', `wip ${i}`);
      }
    }
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/repos`, { data: { path: dir, name: `${tag}-${state}` } });
    if (!r.ok()) throw new Error(`repo seed failed: ${r.status()} ${await r.text()}`);
    ids[state] = ((await r.json()) as { id: string }).id;
  }
  await ctx.dispose();
  return ids;
}

/** What each state must show: label text and enabled-ness per verb. */
export const EXPECT: Record<VerbState, Record<'Fetch' | 'Pull' | 'Push' | 'Stash' | 'Pop', { enabled: boolean; text?: RegExp }>> = {
  synced: { Fetch: { enabled: true }, Pull: { enabled: true }, Push: { enabled: false }, Stash: { enabled: false }, Pop: { enabled: false } },
  dirty: { Fetch: { enabled: true }, Pull: { enabled: true }, Push: { enabled: false }, Stash: { enabled: true }, Pop: { enabled: false } },
  ahead: { Fetch: { enabled: true }, Pull: { enabled: true }, Push: { enabled: true, text: /↑\s*2/ }, Stash: { enabled: false }, Pop: { enabled: false } },
  behind: { Fetch: { enabled: true }, Pull: { enabled: true, text: /↓\s*3/ }, Push: { enabled: false }, Stash: { enabled: false }, Pop: { enabled: true, text: /2/ } },
};
export const VERBS = ['Fetch', 'Pull', 'Push', 'Stash', 'Pop'] as const;
