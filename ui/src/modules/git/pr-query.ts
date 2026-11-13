// PR list search language + sort + grouping (pure — unit/prQuery.test.ts runs
// it under node --test). The search box takes free text plus GitHub-style
// qualifiers; anything it doesn't recognise stays free text, so a typo never
// silently hides every row.
//
//   free text      title / author / branch / number, every word must match
//   #123 | 123     that PR number (a bare number also matches as text)
//   author:x       author contains x (author:@me = "is:mine")
//   label:x        a label equals x (case-insensitive); repeatable (AND)
//   base:x head:x  target / source branch contains x
//   is:open|merged|declined|closed|all   server-side state (closed = declined)
//   is:draft  -is:draft                  drafts only / no drafts
//   is:mine                              authored by one of your git accounts
//   ci:passing|failing|pending           CI state
// Values may be quoted: label:"needs review".

import type { PrState, PrSummary } from '../../lib/api/types';

export type CiFilter = 'passing' | 'failing' | 'pending';

export interface PrQuery {
  text: string[];
  number: number | null;
  authors: string[];
  labels: string[];
  base: string[];
  head: string[];
  state: PrState | null;
  draft: boolean | null;
  mine: boolean;
  ci: CiFilter | null;
}

export const emptyQuery = (): PrQuery => ({
  text: [],
  number: null,
  authors: [],
  labels: [],
  base: [],
  head: [],
  state: null,
  draft: null,
  mine: false,
  ci: null,
});

/** Split on whitespace, keeping `"quoted runs"` (also after `key:`) whole. */
export function tokenize(input: string): string[] {
  const out: string[] = [];
  const re = /(-?[A-Za-z]+:)?"([^"]*)"?|\S+/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(input)) !== null) {
    if (m[2] !== undefined) out.push(`${m[1] ?? ''}${m[2]}`);
    else out.push(m[0]);
  }
  return out.filter((t) => t !== '');
}

const STATES: Record<string, PrState> = {
  open: 'open',
  merged: 'merged',
  declined: 'declined',
  closed: 'declined',
  all: 'all',
};

export function parsePrQuery(input: string): PrQuery {
  const q = emptyQuery();
  for (const tok of tokenize(input)) {
    const m = /^(-?)([A-Za-z]+):(.*)$/.exec(tok);
    if (!m || m[3] === '') {
      const num = /^#?(\d+)$/.exec(tok);
      if (num && tok.startsWith('#')) q.number = Number(num[1]);
      else q.text.push(tok.toLowerCase());
      continue;
    }
    const neg = m[1] === '-';
    const key = m[2].toLowerCase();
    const val = m[3];
    const low = val.toLowerCase();
    if (key === 'is' && !neg && STATES[low]) q.state = STATES[low];
    else if (key === 'is' && low === 'draft') q.draft = !neg;
    else if (key === 'is' && low === 'mine' && !neg) q.mine = true;
    else if (key === 'author' && !neg) {
      if (low === '@me') q.mine = true;
      else q.authors.push(low);
    } else if (key === 'label' && !neg) q.labels.push(low);
    else if (key === 'base' && !neg) q.base.push(low);
    else if (key === 'head' && !neg) q.head.push(low);
    else if (key === 'ci' && !neg && (low === 'passing' || low === 'failing' || low === 'pending')) q.ci = low;
    else q.text.push(tok.toLowerCase());
  }
  return q;
}

/** True when the query narrows rows client-side (state alone doesn't). */
export function hasClientFilter(q: PrQuery): boolean {
  return (
    q.text.length > 0 ||
    q.number !== null ||
    q.authors.length > 0 ||
    q.labels.length > 0 ||
    q.base.length > 0 ||
    q.head.length > 0 ||
    q.draft !== null ||
    q.mine ||
    q.ci !== null
  );
}

/** Normalised CI state of a row (providers spell it two ways). */
export function ciOf(pr: PrSummary): CiFilter | null {
  const s = pr.ci_status;
  if (s === 'passing' || s === 'success') return 'passing';
  if (s === 'failing' || s === 'failure') return 'failing';
  if (s === 'pending') return 'pending';
  return null;
}

/** The row's review chip: approved (with the count), changes requested, or
 *  waiting on a reviewer; `null` when the provider says nothing. */
export function reviewChip(pr: PrSummary): { tone: 'ok' | 'bad' | 'warn'; text: string; label: string } | null {
  const r = pr.review;
  // A merged or declined PR is past review: only open rows say where it stands.
  if (!r || pr.state !== 'open') return null;
  if (r.status === 'changes_requested') return { tone: 'bad', text: 'Changes requested', label: 'Changes requested' };
  if (r.status === 'approved') {
    const text = `${r.approvals} ${r.approvals === 1 ? 'approval' : 'approvals'}`;
    return { tone: 'ok', text, label: `Approved — ${text}` };
  }
  if (r.status === 'waiting') return { tone: 'warn', text: 'Awaiting review', label: 'Awaiting review' };
  return null;
}

/** Authored by the forge account in use: the daemon's `is_mine` (a stable-id
 *  match — a Bitbucket author is a display name, the username an email, so a
 *  name match misses), else — older daemon, or an identity it couldn't look
 *  up — the author matched against your handles. */
export function isMine(pr: PrSummary, handles: ReadonlySet<string>): boolean {
  return (pr.is_mine ?? false) || handles.has(pr.author.toLowerCase());
}

export function matchesPr(pr: PrSummary, q: PrQuery, handles: ReadonlySet<string>): boolean {
  if (q.number !== null && pr.number !== q.number) return false;
  if (q.draft !== null && !!pr.draft !== q.draft) return false;
  if (q.mine && !isMine(pr, handles)) return false;
  if (q.ci !== null && ciOf(pr) !== q.ci) return false;
  const author = pr.author.toLowerCase();
  if (!q.authors.every((a) => author.includes(a))) return false;
  const labels = (pr.labels ?? []).map((l) => l.toLowerCase());
  if (!q.labels.every((l) => labels.includes(l))) return false;
  const base = pr.target_branch.toLowerCase();
  if (!q.base.every((b) => base.includes(b))) return false;
  const head = pr.source_branch.toLowerCase();
  if (!q.head.every((h) => head.includes(h))) return false;
  if (q.text.length > 0) {
    const hay = [pr.title, pr.author, pr.source_branch, pr.target_branch, `#${pr.number}`, String(pr.number), ...(pr.labels ?? [])]
      .join('\n')
      .toLowerCase();
    if (!q.text.every((t) => hay.includes(t))) return false;
  }
  return true;
}

export type PrSort = 'updated' | 'newest' | 'oldest' | 'changed' | 'number' | 'title';

export const SORT_LABEL: Record<PrSort, string> = {
  updated: 'Recently updated',
  newest: 'Newest',
  oldest: 'Oldest',
  changed: 'Most changed',
  number: 'Number',
  title: 'Title',
};

const time = (s: string | null | undefined): number => {
  const t = s ? Date.parse(s) : NaN;
  return Number.isNaN(t) ? 0 : t;
};

/** A new sorted array; `size` gives a PR's changed-line count when known. */
export function sortPrs(
  prs: readonly PrSummary[],
  sort: PrSort,
  size: (pr: PrSummary) => number | null = () => null,
): PrSummary[] {
  const out = prs.slice();
  const byNum = (a: PrSummary, b: PrSummary) => b.number - a.number;
  switch (sort) {
    case 'updated':
      out.sort((a, b) => time(b.updated_at) - time(a.updated_at) || byNum(a, b));
      break;
    // Without created_at the PR number is the creation order.
    case 'newest':
      out.sort((a, b) => time(b.created_at) - time(a.created_at) || byNum(a, b));
      break;
    case 'oldest':
      out.sort((a, b) => time(a.created_at) - time(b.created_at) || a.number - b.number);
      break;
    case 'changed':
      out.sort((a, b) => (size(b) ?? -1) - (size(a) ?? -1) || byNum(a, b));
      break;
    case 'number':
      out.sort(byNum);
      break;
    case 'title':
      out.sort((a, b) => a.title.localeCompare(b.title) || byNum(a, b));
      break;
  }
  return out;
}

export interface PrGroup {
  id: 'mine' | 'others' | 'all';
  label: string;
  items: PrSummary[];
}

/** "Authored by you" first, then everyone else; one flat group when nothing
 *  can say who you are (no handles and no PR flagged `is_mine`). */
export function groupPrs(prs: readonly PrSummary[], handles: ReadonlySet<string>): PrGroup[] {
  if (handles.size === 0 && !prs.some((p) => p.is_mine)) return [{ id: 'all', label: 'Pull requests', items: prs.slice() }];
  const mine: PrSummary[] = [];
  const others: PrSummary[] = [];
  for (const p of prs) (isMine(p, handles) ? mine : others).push(p);
  const out: PrGroup[] = [];
  if (mine.length > 0) out.push({ id: 'mine', label: 'Authored by you', items: mine });
  if (others.length > 0) out.push({ id: mine.length > 0 ? 'others' : 'all', label: mine.length > 0 ? 'Others' : 'Pull requests', items: others });
  return out;
}

/** Merge a refreshed page 1 over the rows already on screen: the fresh page
 *  replaces the old page 1 (so a PR that left this state disappears), and
 *  rows from later pages (`prev` past `page1Count`) stay unless page 1 now
 *  has them. */
export function mergeRefreshed(
  prev: readonly PrSummary[],
  fresh: readonly PrSummary[],
  page1Count: number,
): PrSummary[] {
  const seen = new Set(fresh.map((p) => p.number));
  return [...fresh, ...prev.slice(page1Count).filter((p) => !seen.has(p.number))];
}
