// Contract mirror: git-merge. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';
import type {
  GitOpInProgress, RepoStatusResp,
} from './git';

// --- Local merge + conflict resolution (#4) ---

/** Strategy for a local branch merge. */
export type LocalMergeStrategy = 'merge_commit' | 'ff' | 'ff_only' | 'squash';

/** `POST /repos/{id}/merge` — merge `source` into `target`. */
export interface MergeBranchReq {
  source: string;
  target: string;
  strategy: LocalMergeStrategy;
  /** When true and the tree is dirty, stash → merge → pop. Default false. */
  auto_stash?: boolean;
}

/** Outcome of a local merge / merge-completion. Conflicts are a normal result. */
export interface MergeResult {
  status: 'merged' | 'conflicts' | 'up_to_date' | 'paused';
  commit: string | null;
  conflicted_files: string[];
  repo_status: RepoStatusResp;
  /** Optional note (e.g. auto-stash outcome) to surface as a toast. */
  note?: string | null;
}

/** `POST /repos/{id}/merge/preview` — dry-run conflict check (no tree mutation). */
export interface MergePreviewReq {
  source: string;
  target: string;
}

/** Result of a merge dry-run. */
export interface MergePreview {
  conflicts: boolean;
  conflicted_files: string[];
  up_to_date: boolean;
}

/** `GET /repos/{id}/merge/status` */
export interface MergeConflictStatus {
  /** True when any resolvable operation is underway OR conflicted files exist. */
  merging: boolean;
  /** Which operation; absent/undefined for conflicts with no state file
   *  (a conflicting stash pop / squash). */
  op?: GitOpInProgress | null;
  source: string | null;
  conflicted_files: string[];
}

/** One segment of a conflicted file. */
export type ConflictSegment =
  | { kind: 'context'; lines: string[] }
  | { kind: 'conflict'; ours: string[]; theirs: string[]; base: string[] };

/** `GET /repos/{id}/conflict?path=<p>` */
export interface ConflictFile {
  ours_present: boolean;
  theirs_present: boolean;
  worktree_present: boolean;
  trailing_newline: boolean;
  path: string;
  is_binary: boolean;
  segments: ConflictSegment[];
}

/** `POST /repos/{id}/conflict/resolve` — send `content` (the rebuilt file), or
 *  `side` to take one side wholesale (`content` is ignored then). `content` is
 *  400 for a binary working file, and when empty for an absent one. */
export interface ResolveConflictReq {
  path: string;
  content: string;
  side?: 'ours' | 'theirs' | 'keep' | 'delete';
}

/** `GET /repos/{id}/head/remotes` — remote-tracking refs containing HEAD. */
export interface HeadRemotesResp {
  remotes: string[];
}

/** `POST /repos/{id}/merge/commit` */
export interface MergeCommitReq {
  message: string | null;
}

export type PrState = 'open' | 'merged' | 'declined' | 'all';

export interface PrSummary {
  number: number;
  title: string;
  author: string;
  state: PrState;
  source_branch: string;
  target_branch: string;
  updated_at: string;
  url: string;
  draft?: boolean | null;
  ci_status?: string | null;
  labels?: string[];
  /** Create-response only: e.g. a reviewer request that failed after the PR
   *  was opened. Empty/omitted everywhere else. */
  reviewer_warnings?: string[];
  /** Source-branch head commit as the provider reports it (Bitbucket
   *  abbreviates). Opaque — pass back as the PR diff's `rev=`. */
  head_sha?: string | null;
}

/** `GET /repos/{id}/prs?state=&page=&per_page=` — one page. `has_more` is the
 *  provider's own next-page signal; `page`/`per_page` echo the clamped request
 *  (`per_page` 1..=100, `page` 1..=10000). */
export interface PrListResp {
  items: PrSummary[];
  has_more: boolean;
  page: number;
  per_page: number;
}

export interface PrComment {
  id: string;
  author: string;
  body: string;
  path: string | null;
  line: number | null;
  created_at: string;
  replies: PrComment[];
  /** Thread resolution status (thread heads only; replies stay false). */
  resolved: boolean;
  /** Id for the resolve/unresolve endpoint (thread heads only) — Bitbucket
   *  comment id, GitLab discussion id, GitHub GraphQL reviewThread node id. */
  thread_id?: string | null;
  /** Diff side `line` counts on: `old` = a deleted line (old number); absent
   *  or `new` = the head side. The UI renders the comment under one row. */
  side?: PrCommentSide | null;
  /** The forge no longer maps the comment onto the current diff; `line` is
   *  the original line. Rendered under "File comments" with a chip. */
  outdated?: boolean;
}

/** Side of a unified diff an inline PR comment anchors to. */
export type PrCommentSide = 'old' | 'new';

/** A PR reviewer with approval state; avatar/timestamp are best-effort. */
export interface PrReviewer {
  name: string;
  approved: boolean;
  avatar_url: string | null;
  reviewed_at: string | null;
}

export type PrDetail = PrSummary & {
  description_md: string;
  comments: PrComment[];
  /** Approver display names (back-compat; prefer `reviewers`). */
  approved_by: string[];
  reviewers: PrReviewer[];
  mergeable: boolean | null;
};

export interface CreatePrReq {
  title: string;
  description: string;
  source_branch: string;
  target_branch: string;
  /** Proof pack to gate this PR on: Otto refuses to open the PR unless the pack
   *  is `passed`/`waived` (or `allow_unproven` is set). */
  proof_pack_id?: string | null;
  /** Open the PR even over an unproven pack — records an audit `approval`
   *  artifact on the pack. */
  allow_unproven?: boolean | null;
  /** Open as a draft (GitHub native flag; GitLab `Draft:` title prefix;
   *  Bitbucket Cloud draft field). Absent = ready for review. */
  draft?: boolean;
  /** Provider-native reviewer handles to request at creation. Failures surface
   *  in `PrSummary.reviewer_warnings`, never as a creation error. */
  reviewers?: string[];
}

export interface DraftPrReq {
  base: string;
  /** The PR's Source branch. When set the draft describes
   *  `merge-base(base, head)..head` and `DraftPrResp.source_branch` echoes it;
   *  absent ⇒ the checked-out branch (+ working tree). */
  head?: string;
}

export interface DraftPrResp {
  title: string;
  description: string;
  source_branch: string;
  target_branch: string;
  /** The Otto session that drafted this. Drafting runs as a REAL session —
   *  it appears in Agents the moment it spawns, so it can be opened in a pane
   *  and watched (or talked to) while the modal is still spinning. */
  session_id?: Id;
}

export interface DraftCommitMessageResp {
  message: string;
  /** True when drafted from the staged diff; false when it fell back to the working diff. */
  from_staged: boolean;
  /** The `commit-draft` session that drafted this (same path as `DraftPrResp.session_id`). */
  session_id?: Id | null;
}

export interface UpdatePrReq {
  title?: string | null;
  description?: string | null;
}

export interface NewPrCommentReq {
  body: string;
  path?: string | null;
  line?: number | null;
  in_reply_to?: string | null;
  /** Diff side `line` counts on (default `new`); a deleted line sends `old`
   *  with its OLD number. */
  side?: PrCommentSide | null;
  /** The anchored row's old number when it has one (GitLab needs both
   *  numbers for an unchanged context line). */
  old_line?: number | null;
  /** Head sha of the diff the reviewer saw (anchors the comment; GitHub
   *  skips a PR round-trip). */
  commit_id?: string | null;
}

/** Body for POST /repos/{id}/prs/{number}/comments/{cid}/resolve — `{cid}` is
 *  `PrComment.thread_id`; `resolved: false` reopens the thread. */
export interface ResolvePrThreadReq {
  resolved: boolean;
}

export type MergeStrategy = 'merge' | 'squash' | 'rebase';

export interface MergePrReq {
  strategy: MergeStrategy;
  /** Ask the provider to delete the PR's source branch as part of the merge. */
  delete_source_branch?: boolean;
  /** The `PrSummary.head_sha` the user reviewed. A PR whose head moved since
   *  is refused with 409 "PR changed — re-check" instead of merging. */
  expected_head_sha?: string | null;
}

/** `POST /repos/{id}/branch/delete` body. */
export interface DeleteBranchReq {
  name: string;
  /** Also delete the branch on `remote_name` (default `origin`). */
  remote?: boolean;
  /** Delete the local branch (default true); `false` = remote-only. */
  local?: boolean;
  /** `-D` instead of the safe `-d` — only after the user's explicit confirm. */
  force?: boolean;
  /** Which remote a `remote:true` delete targets (default `origin`). A
   *  remote-ref row like `upstream/x` must send `remote_name: 'upstream'`. */
  remote_name?: string;
}

/** One CI check / job / commit-status row behind the PR's aggregate status
 *  (GET /repos/{id}/prs/{number}/checks). */
export interface PrCheck {
  name: string;
  state: 'success' | 'failure' | 'pending' | 'skipped' | 'neutral';
  url?: string | null;
  started_at?: string | null;
  completed_at?: string | null;
}

export interface PrChecksResp {
  ci: { state: string; total: number; passed: number; failed: number; url: string | null };
  checks: PrCheck[];
}

/** GET /repos/{id}/prs/{number}/readiness — PR-keyed twin of
 *  /reviews/{id}/merge-readiness. `review` is null when no review ever ran;
 *  `unpushed`/`branch_freshness` come from the local checkout. */
export interface PrReadiness {
  ci_status: string;
  approvals: number;
  mergeable: boolean | null;
  conflicts: boolean;
  review: {
    review_id: string;
    unresolved_total: number;
    unresolved_blocker_count: number;
    total_findings: number;
  } | null;
  unpushed: number | null;
  branch_freshness: 'fresh' | 'behind' | 'unknown';
}

export interface Problem {
  code: string;
  message: string;
}

export interface PrCommit {
  sha: string;
  short_sha: string;
  author: string;
  date: string;
  subject: string;
}

export interface RequestChangesReq {
  body?: string | null;
}
