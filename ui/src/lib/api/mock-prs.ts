// Mock fixtures for the PR workspace (split out of mock.ts): a T3-sized PR
// (#51) with many files across languages (a few of them tests), its commits
// and per-commit diffs, plus a couple more open PRs for the list pane.
import type { DiffResp, FileDiff, PrComment, PrDetail, PrSummary } from './types';

const NOW = Date.now();
const ago = (mins: number): string => new Date(NOW - mins * 60_000).toISOString();

/** "You" on the forge, as the forge NAMES you — a display name that matches
 *  none of the mock git accounts' usernames (`dev-otto`, `dev@work.io`), the
 *  Bitbucket shape. Only the daemon's `is_me` / `is_mine` flags say it's you. */
export const MOCK_ME = 'Dev Otto';
/** PRs you authored (author spelled either way). */
export const mockIsMine = (author: string): boolean => author === MOCK_ME || author === 'dev-otto';

/** Detail-only fields per PR: reactions (none on Bitbucket Cloud), the
 *  forge's merge checks and open tasks (#51 has a failing approvals rule). */
export function mockPrExtras(repoId: string, num: number): Partial<PrDetail> {
  if (repoId === 'rep_wallet') return { reactions_supported: false };
  if (num !== 51) return { reactions_supported: true };
  return {
    reactions_supported: true,
    merge_checks: [
      { key: 'minimum_approvals', label: 'At least 2 approvals', passed: false, detail: '1 of 2 approvals' },
      { key: 'all_builds_passed', label: 'All builds passed', passed: false },
      { key: 'no_conflicts', label: 'No merge conflicts', passed: true },
    ],
    task_count: { open: 1, resolved: 2 },
  };
}

/** Simulated push for e2e: `localStorage.otto_mock_pr_pushed = '<number>'`
 *  makes that PR's head a newer commit (after its diff was loaded). */
const PUSHED_SHA = 'c7e57a11';
function pushed(num: string | number): boolean {
  try {
    return localStorage.getItem('otto_mock_pr_pushed') === String(num);
  } catch {
    return false;
  }
}
/** A PR summary as the forge reports it now: `is_mine`, and the head moved
 *  by a simulated push. */
export function mockPrLive(s: PrSummary): PrSummary {
  return { ...s, is_mine: mockIsMine(s.author), head_sha: pushed(s.number) ? PUSHED_SHA : (s.head_sha ?? null) };
}

function fdiff(path: string, body: [('context' | 'add' | 'del'), string][]): FileDiff {
  let o = 1;
  let n = 1;
  const lines = body.map(([origin, content]) => {
    const line = { origin, content, old_line: origin === 'add' ? null : o, new_line: origin === 'del' ? null : n };
    if (origin !== 'add') o++;
    if (origin !== 'del') n++;
    return line;
  });
  const adds = body.filter(([k]) => k === 'add').length;
  const dels = body.filter(([k]) => k === 'del').length;
  return {
    path,
    old_path: null,
    is_binary: false,
    hunks: [{ header: `@@ -1,${body.length - adds} +1,${body.length - dels} @@`, lines }],
  };
}

/** A T3-sized PR (#51): many files across languages, a few of them tests,
 *  so the Code tab's tree, hide-tests and viewed marks have real work to do. */
const BIG_PR_PATHS: [string, number, number, 'add' | 'mod' | 'del'][] = [
  ['apps/cbo/src/app/modules/marketing/vip-program/vip-program.routes.ts', 8, 3, 'mod'],
  ['apps/cbo/src/app/modules/marketing/vip-program/specs/vip-program.routes.spec.ts', 6, 1, 'mod'],
  ['apps/cbo/src/styles/_components_override.scss', 2, 1, 'mod'],
  ['apps/cbo/src/environments/config.spec.ts', 1, 2, 'mod'],
  ['libs/common-styles/includes/_vip-status.scss', 43, 0, 'add'],
  ['libs/data-models/facade-account-managers.ts', 1, 1, 'mod'],
  ['libs/data-models/facade-vip-program.ts', 45, 7, 'mod'],
  ['libs/data-models/facade-vip-player-card.ts', 36, 0, 'add'],
  ['libs/data-models/player-details.ts', 0, 14, 'mod'],
  ['libs/facades/players/src/lib/+state/players.effects.ts', 1, 21, 'mod'],
  ['libs/facades/players/src/lib/+state/players.facade.ts', 20, 1, 'mod'],
  ['libs/facades/players/src/lib/+state/vip/players-vip.actions.ts', 32, 0, 'add'],
  ['libs/facades/players/src/lib/+state/vip/players-vip.effects.ts', 41, 0, 'add'],
  ['libs/facades/players/src/lib/+state/vip/players-vip.reducer.ts', 32, 0, 'add'],
  ['libs/facades/players/src/lib/+state/vip/players-vip.reducer.spec.ts', 28, 0, 'add'],
  ['services/wallet/internal/vip/levels.go', 24, 6, 'mod'],
  ['services/wallet/internal/vip/levels_test.go', 30, 0, 'add'],
  ['services/scoring/vip_score.py', 12, 4, 'mod'],
  ['services/scoring/tests/test_vip_score.py', 18, 0, 'add'],
  ['services/legacy/src/main/java/com/acme/VipService.java', 9, 9, 'mod'],
  ['services/legacy/src/test/java/com/acme/VipServiceTest.java', 14, 2, 'mod'],
  ['docs/vip-program.md', 22, 0, 'add'],
  ['libs/data-models/legacy-vip.ts', 0, 18, 'del'],
];
function bigFile(path: string, adds: number, dels: number, kind: 'add' | 'mod' | 'del'): FileDiff {
  const body: [('context' | 'add' | 'del'), string][] = [];
  const isPy = path.endsWith('.py');
  const isGo = path.endsWith('.go');
  const cmt = isPy ? '#' : '//';
  if (kind === 'mod') body.push(['context', isGo ? 'package vip' : isPy ? 'from typing import Optional' : "import { Injectable } from '@angular/core';"], ['context', '']);
  for (let i = 0; i < dels; i++) body.push(['del', `${cmt} legacy line ${i + 1}: computeLevel(player, legacyThresholds)`]);
  for (let i = 0; i < adds; i++) {
    body.push([
      'add',
      isGo
        ? `\tlevels = append(levels, Level{ID: ${i + 1}, MinPoints: ${(i + 1) * 500}})`
        : isPy
          ? `    score += weights[${i}] * signals.get("s${i}", 0.0)`
          : `  readonly vipLevel${i + 1} = signal<VipLevel | null>(null);`,
    ]);
  }
  if (kind === 'mod') body.push(['context', ''], ['context', isPy ? 'return score' : '}']);
  const f = fdiff(path, body);
  return {
    ...f,
    status: kind === 'add' ? 'added' : kind === 'del' ? 'deleted' : 'modified',
    added: adds,
    deleted: dels,
  } as FileDiff;
}
const bigPrDiff: DiffResp = (() => {
  const files = BIG_PR_PATHS.map(([p, a, d, k]) => bigFile(p, a, d, k));
  return {
    files,
    total_added: files.reduce((n, f) => n + (f.added ?? 0), 0),
    total_deleted: files.reduce((n, f) => n + (f.deleted ?? 0), 0),
  };
})();
/** PR #51's commits, newest first (the Code tab's "All commits" picker). */
export const bigPrCommits = [
  ['c1fac680', 'fix(vip program): keep the selected level after a refresh', 'dev-otto', 35],
  ['c13d1dfe', 'feat(player details): vip settings history popup', 'dev-otto', 140],
  ['c3e0ffb2', 'feat(vip program): active pins page', 'maya', 300],
  ['c87aff03', "Merge branch 'develop' into feature/GS-16909", 'dev-otto', 600],
  ['c61df12d', 'feat(player details): loyalty section adjustments', 'maya', 900],
  ['c0c3c6ca', 'feat(player details): vip settings management', 'dev-otto', 1600],
].map(([sha, subject, author, mins]) => ({
  sha: `${sha as string}${'0'.repeat(32)}`,
  short_sha: (sha as string).slice(0, 7),
  author: author as string,
  date: ago(mins as number),
  subject: subject as string,
}));

interface ExtraPr {
  repo_id: string;
  summary: PrSummary;
  description_md: string;
  comments: PrComment[];
  approved_by: string[];
  /** Reviewers whose current opinion is "changes requested". */
  changes_by?: string[];
  mergeable: boolean | null;
}

export const mockExtraPrs: ExtraPr[] = [
{
    repo_id: 'rep_otto',
    summary: {
      number: 51,
      title: 'Feature/GS-16909 — VIP program: levels, pins and player card',
      author: 'dev-otto',
      state: 'open',
      source_branch: 'feature/GS-16909',
      target_branch: 'develop',
      updated_at: ago(16),
      created_at: ago(60 * 26),
      url: 'https://github.com/dev-otto/otto/pull/51',
      ci_status: 'passing',
      labels: ['feature', 'vip'],
      head_sha: 'c1fac680',
    },
    description_md:
      '## Summary\n\nAdds the **VIP program** to the back office: level thresholds, active pins and the player VIP card.\n\n## Changes\n\n- New `players-vip` NgRx slice (actions, effects, reducer)\n- VIP level computation moved to the wallet service (`levels.go`)\n- Scoring weights for the VIP score\n\n## Testing\n\n- Unit tests for the reducer, levels and scoring\n- Manually verified on staging',
    comments: [
      {
        id: 'c51_1', author: 'maya', body: 'Can the level thresholds come from config instead of being hard-coded?',
        path: null, line: null, created_at: ago(60 * 20), replies: [
          { id: 'c51_2', author: 'dev-otto', body: 'Good call — moved them to the wallet config in the latest push.', path: null, line: null, created_at: ago(60 * 18), replies: [], resolved: false, in_reply_to: 'c51_1' },
          // A reply to the reply (Bitbucket flattens the chain under the root).
          { id: 'c51_4', author: 'maya', body: 'Thanks — can the config be per brand?', path: null, line: null, created_at: ago(60 * 17), replies: [], resolved: false, in_reply_to: 'c51_2' },
        ], resolved: false, thread_id: 'c51_1',
        reactions: [{ key: 'thumbs_up', count: 2, me: false }, { key: 'eyes', count: 1, me: false }],
      },
      {
        // A GitHub review summary: verdict chip, Reply only.
        id: 'review-901', author: 'maya', body: 'Looks good once the effect handles its errors.', review_state: 'approved',
        path: null, line: null, created_at: ago(60 * 4), replies: [], resolved: false, can_edit: false,
      },
      {
        // Yours (MOCK_ME): Edit / Delete in its menu.
        id: 'c51_5', author: MOCK_ME, body: 'Rebased on develop — ready for another look.', can_edit: true,
        path: null, line: null, created_at: ago(60 * 2), replies: [], resolved: false, thread_id: 'c51_5',
        reactions: [{ key: 'rocket', count: 1, me: true }],
      },
      {
        id: 'c51_3', author: 'maya', body: 'Nit: this effect swallows the error — add a `catchError` that dispatches a failure action.',
        path: 'libs/facades/players/src/lib/+state/vip/players-vip.effects.ts', line: 4, created_at: ago(60 * 3), replies: [], resolved: false, thread_id: 'c51_3',
      },
    ],
    approved_by: ['maya'],
    mergeable: true,
  },
  {
    repo_id: 'rep_otto',
    summary: {
      number: 50,
      title: 'GS-16933 Add layered chance reward and VIP slot screens to the CBO',
      author: 'itzik',
      state: 'open',
      source_branch: 'feature/GS-16933',
      target_branch: 'develop',
      updated_at: ago(60 * 19),
      created_at: ago(60 * 40),
      url: 'https://github.com/dev-otto/otto/pull/50',
      ci_status: 'pending',
      labels: ['feature'],
      head_sha: 'b50aa11',
    },
    description_md: 'Adds the layered chance reward and VIP slot screens.',
    comments: [],
    approved_by: [],
    mergeable: true,
  },
  {
    repo_id: 'rep_otto',
    summary: {
      number: 49,
      title: 'fix(wallet): round bonus balance before conversion',
      author: MOCK_ME,
      state: 'open',
      draft: true,
      source_branch: 'fix/bonus-rounding',
      target_branch: 'develop',
      updated_at: ago(60 * 30),
      created_at: ago(60 * 50),
      url: 'https://github.com/dev-otto/otto/pull/49',
      labels: ['bug'],
      head_sha: 'b49bb22',
    },
    description_md: 'Work in progress.',
    comments: [],
    approved_by: [],
    mergeable: null,
  },
];

/** PR #51's diff (one file with `path`). */
export function mockBigPrDiff(path: string | null): DiffResp {
  return path ? { files: bigPrDiff.files.filter((f) => f.path === path) } : bigPrDiff;
}

/** A PR's commits: #51 has six, any other PR two. */
export function mockPrCommits(num: string): typeof bigPrCommits {
  const list = num === '51' ? bigPrCommits : bigPrCommits.slice(0, 2).map((c) => ({ ...c, sha: `x${c.sha.slice(1)}` }));
  if (!pushed(num)) return list;
  const sha = `${PUSHED_SHA}${'0'.repeat(32)}`;
  return [{ sha, short_sha: sha.slice(0, 7), author: 'maya', date: new Date().toISOString(), subject: 'fix: late push after review' }, ...list];
}

/** `commit:<sha>` of one of #51's commits → the slice of files it touched. */
export function mockCommitDiff(target: string | null, path: string | null): DiffResp | null {
  const ci = bigPrCommits.findIndex((c) => target === `commit:${c.sha}`);
  if (ci < 0) return null;
  const files = bigPrDiff.files.filter((_f, i) => i % bigPrCommits.length === ci);
  return { files: path ? files.filter((f) => f.path === path) : files };
}
