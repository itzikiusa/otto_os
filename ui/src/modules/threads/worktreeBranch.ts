// The branch a new thread's worktree gets when the person did not name one
// (Orca's blank-name-is-fine create): a short kebab slug of the prompt's
// first words, under the repo's own branch prefix when its branches clearly
// follow one ("feat/…"), with -2, -3… on a collision; without a prompt, a
// dated name. Pure, import-free: unit/threadsWorktreeBranch.test.ts.

const STOP = new Set(['a', 'an', 'the', 'to', 'of', 'in', 'on', 'for', 'and', 'or', 'please', 'can', 'you', 'we', 'i', 'it', 'is', 'be', 'with', 'this', 'that']);
const WORDS = 5;
const MAX_SLUG = 40;

/** "Fix the flaky checkout test!" → "fix-flaky-checkout-test". */
export function branchSlug(prompt: string): string {
  const words = prompt
    .normalize('NFKD')
    .replace(/[\u0300-\u036f]/g, '')
    .toLowerCase()
    .replace(/[@`'"]/g, '')
    .split(/[^a-z0-9]+/)
    .filter((w) => w && !STOP.has(w))
    .slice(0, WORDS);
  let slug = words.join('-');
  if (slug.length > MAX_SLUG) slug = slug.slice(0, MAX_SLUG).replace(/-[^-]*$/, '') || slug.slice(0, MAX_SLUG);
  return slug;
}

/** The prefix most of the repo's branches use ("feat/"), when one is clear:
 *  at least two branches and over half of those not on the default branch. */
export function branchPrefix(branches: readonly string[], defaultBranch: string | null): string {
  const rest = branches.filter((b) => b !== defaultBranch && !['main', 'master', 'develop'].includes(b));
  const counts = new Map<string, number>();
  for (const b of rest) {
    const i = b.indexOf('/');
    if (i > 0) counts.set(b.slice(0, i + 1), (counts.get(b.slice(0, i + 1)) ?? 0) + 1);
  }
  let best = '';
  let n = 0;
  for (const [p, c] of counts) if (c > n) [best, n] = [p, c];
  return n >= 2 && n * 2 > rest.length ? best : '';
}

/** "thread-2026-11-23-1805" (local time). */
export function datedSlug(now: Date): string {
  const p = (x: number): string => String(x).padStart(2, '0');
  return `thread-${now.getFullYear()}-${p(now.getMonth() + 1)}-${p(now.getDate())}-${p(now.getHours())}${p(now.getMinutes())}`;
}

/** The branch for a new worktree: prefix + slug (dated without a prompt),
 *  made unique against `taken` (local branches, other worktrees'). */
export function worktreeBranch(o: { prompt: string; branches: readonly string[]; defaultBranch: string | null; now: Date }): string {
  const base = `${branchPrefix(o.branches, o.defaultBranch)}${branchSlug(o.prompt) || datedSlug(o.now)}`;
  const taken = new Set(o.branches);
  if (!taken.has(base)) return base;
  for (let i = 2; ; i++) if (!taken.has(`${base}-${i}`)) return `${base}-${i}`;
}
