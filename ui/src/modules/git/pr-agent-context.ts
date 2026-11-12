// The messages the PR’s embedded agent thread (PrAgentPanel) sends: every ask
// carries enough context for the agent to work without the UI — repo name and
// folder, PR number / title / branches / head, and for a file or line ask the
// paths, the line range and the diff text itself, bounded (INLINE_CAP). Pure,
// unit-tested in unit/prAgentContext.test.ts.
import type { FileDiff, PrDetail, Repo, RuntimeAccessMode } from '../../lib/api/types';
import { prAgentPrompt, prRef } from './pr-agent';

/** A run of diff rows picked in the Code tab ("Explain these lines"). */
export interface LinePick {
  path: string;
  /** First / last line number on `side` (null when no row carried one). */
  start: number | null;
  end: number | null;
  /** `new` unless every picked row is a deletion. */
  side: 'new' | 'old';
  /** The rows as diff text, one per line ("+  foo()"). */
  text: string;
}

/** One request to the PR’s thread. `ask` only opens the panel (and its ask
 *  box) — nothing is sent, nothing is created. */
export type PrAgentAsk =
  | { kind: 'ask' }
  | { kind: 'explain' }
  | { kind: 'fix' }
  | { kind: 'question'; text: string }
  | { kind: 'files'; paths: string[] }
  | { kind: 'lines'; pick: LinePick };
export type PrAgentSend = Exclude<PrAgentAsk, { kind: 'ask' }>;

/** What a message may inline. Past these the agent is told what was cut and
 *  reads the rest from the clone (it has the path and the head commit). */
export const INLINE_CAP = {
  /** Picked lines of one "Explain these lines". */
  lineChars: 6_000,
  /** One file's diff in "Explain the selected files". */
  fileChars: 4_000,
  /** All files' diffs together. */
  totalChars: 12_000,
  /** Files whose diff is inlined at all (the rest are named only). */
  files: 8,
  /** Paths listed by name. */
  paths: 60,
} as const;

export interface PrRepoCtx {
  name: string;
  /** "owner/repo" (or the local name when the remote can't be read). */
  slug: string;
  /** The local clone. */
  path: string;
}

/** "owner/repo" from the remote URL (falls back to the local name). */
export function repoSlugOf(repo: Pick<Repo, 'name' | 'remote_url'>): string {
  const m = (repo.remote_url ?? '').match(/[:/]([^/:]+\/[^/]+?)(?:\.git)?\/?$/);
  return m?.[1] ?? repo.name;
}

/** Read-only for every question; Fix is the one ask that edits files, and
 *  even it asks before running commands (the CLI's own prompts reach the
 *  person). `approval-required` is the most restrictive mode — reads still
 *  run without asking (Claude's default mode; Codex's read-only sandbox). */
export function accessModeFor(kind: PrAgentSend['kind']): RuntimeAccessMode {
  return kind === 'fix' ? 'auto-accept-edits' : 'approval-required';
}

/** `s` cut to `max` characters (at a line break when one is near), with a
 *  note naming the cut so the agent knows to read the rest. */
export function capText(s: string, max: number): string {
  if (s.length <= max) return s;
  const nl = s.lastIndexOf('\n', max);
  const end = nl > max * 0.8 ? nl : max;
  return `${s.slice(0, end)}\n… [${(s.length - end).toLocaleString('en-US')} more characters cut — read the rest from the clone]`;
}

const SIGN: Record<string, string> = { add: '+', del: '-', context: ' ' };

/** One file's hunks as unified-diff text. */
export function hunkText(file: FileDiff): string {
  const out: string[] = [];
  for (const h of file.hunks) {
    out.push(h.header);
    for (const l of h.lines) out.push(`${SIGN[l.origin] ?? ' '}${l.content.replace(/\n$/, '')}`);
  }
  return out.join('\n');
}

/** The rows of a selection, read from the diff (sign, numbers, text). */
export interface PickedRow {
  sign: string;
  oldLine: number | null;
  newLine: number | null;
  code: string;
}

export function linePickFrom(path: string, rows: readonly PickedRow[]): LinePick | null {
  if (rows.length === 0) return null;
  const news = rows.map((r) => r.newLine).filter((n): n is number => n !== null);
  const olds = rows.map((r) => r.oldLine).filter((n): n is number => n !== null);
  const side = news.length > 0 ? 'new' : 'old';
  const nums = side === 'new' ? news : olds;
  return {
    path,
    start: nums.length ? Math.min(...nums) : null,
    end: nums.length ? Math.max(...nums) : null,
    side,
    text: rows.map((r) => `${r.sign || ' '}${r.code}`).join('\n'),
  };
}

/** "lines 40–58" / "line 12" / "old lines 3–9" / "the selected lines". */
export function lineLabel(p: Pick<LinePick, 'start' | 'end' | 'side'>): string {
  if (p.start === null || p.end === null) return 'the selected lines';
  const old = p.side === 'old' ? 'old ' : '';
  return p.start === p.end ? `${old}line ${p.start}` : `${old}lines ${p.start}–${p.end}`;
}

/** The thread's title (also what Threads lists it as). */
export function prThreadTitle(pr: Pick<PrDetail, 'number' | 'title'>): string {
  const t = `PR #${pr.number} · ${pr.title}`.replace(/\s+/g, ' ').trim();
  return t.length > 80 ? `${t.slice(0, 79)}…` : t;
}

/** Where the PR lives, so the agent can work without the UI. */
export function prContextBlock(pr: PrDetail, repo: PrRepoCtx): string {
  const head = (pr.head_sha ?? '').trim();
  const read = head
    ? `The clone’s working tree may be on another branch: read the PR’s version of a file with \`git show ${head}:<path>\` and the PR diff with \`git diff origin/${pr.target_branch}...${head}\` (run \`git fetch\` first if the commit is missing).`
    : `The clone’s working tree may be on another branch: read the PR’s version from \`origin/${pr.source_branch}\` (run \`git fetch\` first).`;
  return [
    'Context (sent from Otto’s pull-request view):',
    `- Pull request: #${pr.number} “${pr.title}”, ${pr.source_branch} → ${pr.target_branch}${pr.url ? `, ${pr.url}` : ''}`,
    `- Repository: ${repo.slug} (Otto repo “${repo.name}”), cloned at ${repo.path}`,
    head ? `- PR head commit: ${head}` : null,
    `- ${read}`,
  ].filter((l): l is string => l !== null).join('\n');
}

const READ_ONLY = 'This is a read-only request: do not edit files, commit, push, or post anything to the pull request.';

function filesSection(paths: readonly string[], diffs: readonly FileDiff[]): string {
  const byPath = new Map(diffs.map((f) => [f.path, f]));
  const listed = paths.slice(0, INLINE_CAP.paths);
  const out: string[] = [listed.map((p) => `- ${p}`).join('\n')];
  if (paths.length > listed.length) out.push(`- … and ${paths.length - listed.length} more (list them with \`git diff --name-only\`)`);
  let budget: number = INLINE_CAP.totalChars;
  let inlined = 0;
  const skipped: string[] = [];
  for (const p of paths) {
    const f = byPath.get(p);
    const body = f && f.hunks.length > 0 ? hunkText(f) : '';
    if (!body || inlined >= INLINE_CAP.files || budget < 200) {
      skipped.push(p);
      continue;
    }
    const text = capText(body, Math.min(INLINE_CAP.fileChars, budget));
    budget -= text.length;
    inlined += 1;
    out.push(`\nDiff of ${p}:\n\`\`\`diff\n${text}\n\`\`\``);
  }
  if (skipped.length > 0) {
    out.push(`\nNot inlined (over the ${INLINE_CAP.totalChars.toLocaleString('en-US')}-character / ${INLINE_CAP.files}-file cap, binary, or not loaded): ${skipped.length === paths.length ? 'every file' : skipped.slice(0, 10).join(', ')}${skipped.length > 10 && skipped.length !== paths.length ? ', …' : ''} — read them from the clone.`);
  }
  return out.join('\n');
}

/** The message, its access mode and the thread title for one send. */
export function buildAskMessage(
  ask: PrAgentSend,
  pr: PrDetail,
  repo: PrRepoCtx,
  diffs: readonly FileDiff[] = [],
): { text: string; access: RuntimeAccessMode; title: string } {
  const ref = prRef(pr, repo.slug);
  const ctx = prContextBlock(pr, repo);
  let body: string;
  if (ask.kind === 'fix') {
    body = `${prAgentPrompt('fix', pr, repo.slug)}\n\n${ctx}\n\nYou may edit files in ${repo.path} only. Do not commit, push, or post to the pull request: I review your diff and apply it myself.`;
  } else if (ask.kind === 'explain') {
    body = `${prAgentPrompt('explain', pr, repo.slug)}\n\n${ctx}\n\n${READ_ONLY}`;
  } else if (ask.kind === 'question') {
    body = `${prAgentPrompt('ask', pr, repo.slug, ask.text)}\n\n${ctx}\n\n${READ_ONLY}`;
  } else if (ask.kind === 'files') {
    const one = ask.paths.length === 1;
    body = `Explain the changes ${ref} makes to ${one ? ask.paths[0] : `these ${ask.paths.length} files`}: what changed and why, how ${one ? 'it fits' : 'they fit'} the rest of the PR, and what a reviewer should check. Cite file:line.\n\n${filesSection(ask.paths, diffs)}\n\n${ctx}\n\n${READ_ONLY}`;
  } else {
    const p = ask.pick;
    const label = lineLabel(p);
    body = `In ${ref}, explain ${label} of ${p.path}${p.side === 'old' ? ' (lines the PR deletes)' : ''}: what this code does, why the PR changes it, and anything risky. Cite file:line.\n\nThe picked diff rows (capped at ${INLINE_CAP.lineChars.toLocaleString('en-US')} characters):\n\`\`\`diff\n${capText(p.text, INLINE_CAP.lineChars)}\n\`\`\`\nThe whole file is on disk at ${repo.path}/${p.path} (the PR’s version: see below).\n\n${ctx}\n\n${READ_ONLY}`;
  }
  return { text: body, access: accessModeFor(ask.kind), title: prThreadTitle(pr) };
}
