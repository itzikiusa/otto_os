// Contract mirror: git. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';

export type GitProviderKind = 'github' | 'bitbucket' | 'gitlab';

export interface GitAccount {
  id: Id;
  user_id: Id;
  provider: GitProviderKind;
  label: string;
  username: string;
  token_ref: string;
  api_base_url: string | null;
  /** Org / workspace / group to browse repos under (e.g. "your-org"). */
  namespace: string | null;
  /** Token expiry (auto-detected for GitHub/GitLab, user-entered for Bitbucket); null = unknown/none. */
  token_expires_at: string | null;
  created_at: string;
}

/** A repository in a git account's namespace, returned by remote-repos search. */
export interface RemoteRepoSummary {
  full_name: string;
  name: string;
  clone_url: string;
  ssh_url: string;
  description: string;
  private: boolean;
  updated_at: string;
}

export interface Repo {
  id: Id;
  workspace_id: Id;
  name: string;
  path: string;
  remote_url: string | null;
  provider: GitProviderKind | null;
  git_account_id: Id | null;
  created_at: string;
  /** Forge discriminator computed from remote_url: a supported forge,
   *  'unrecognized' (remote exists but isn't GitHub/Bitbucket Cloud/GitLab —
   *  e.g. Bitbucket Server), or null when the repo has no remote. */
  forge?: GitProviderKind | 'unrecognized' | null;
}

/** One row of the agent-facing repo directory (`GET /git/repos/directory`,
 *  `GET /git/repos/resolve`): a repo plus the workspace it is registered in.
 *  `current` marks the caller's own workspace (the session's, or the hint). */
export interface RepoDirectoryEntry {
  id: Id;
  name: string;
  path: string;
  remote_url: string | null;
  provider: GitProviderKind | null;
  workspace_id: Id;
  workspace_name: string;
  current: boolean;
}

/** `GET /git/repos/directory` — every repo in every workspace the caller can
 *  read, current workspace first. */
export interface RepoDirectory {
  repos: RepoDirectoryEntry[];
  current_workspace_id: Id | null;
  workspace_count: number;
}

/** How `GET /git/repos/resolve` matched a friendly repo reference. */
export type RepoMatchedBy = 'id' | 'path' | 'remote' | 'name' | 'session_cwd';

/** `GET /git/repos/resolve?ref=&workspace_id=` — 404 lists what IS available,
 *  409 lists the ambiguous candidates (both in the Problem `message`). */
export interface RepoResolveResp {
  repo: RepoDirectoryEntry;
  matched_by: RepoMatchedBy;
}

/** Kinds the agent discovery routes (`GET /refs/directory`, `GET /refs/resolve`)
 *  know — the same set every agent tool's friendly id argument resolves. */
export type AgentRefKind =
  | 'workspace'
  | 'workflow'
  | 'connection'
  | 'broker_cluster'
  | 'swarm'
  | 'scheduled_task'
  | 'goal_loop'
  | 'agent_room'
  | 'canvas_scene'
  | 'product_story'
  | 'vault'
  | 'api_request'
  | 'api_automation'
  | 'api_environment'
  | 'issue_account'
  | 'aws_account'
  | 'k8s_cluster'
  | 'design_artifact';

/** `GET /refs/directory?kind=&workspace_id=&prefer_workspace_id=` — every
 *  object of `kind` across the workspaces the caller can read (a token's
 *  workspace pin applied). Rows are the kind's own list rows (projected for
 *  heavy kinds — no workflow graph, no connection params); per-workspace kinds
 *  add `workspace_id`, `workspace_name` and `current`, current workspace first. */
export interface AgentRefDirectory {
  kind: AgentRefKind;
  items: Array<Record<string, unknown>>;
  current_workspace_id: Id | null;
  workspace_count: number;
}

/** `GET /refs/resolve?kind=&ref=&arg=&workspace_id=&prefer_workspace_id=` —
 *  one reference (id, or name / title / Jira key / label) resolved. 404 lists
 *  near misses or what IS available, 409 the ambiguous candidates (both in the
 *  Problem `message`). `matched_by` is `id`, the matched field name, or
 *  `only_candidate` (an omitted issue account with exactly one account). */
export interface AgentRefResolveResp {
  kind: AgentRefKind;
  id: string;
  label: string;
  workspace_id: Id | null;
  workspace_name: string | null;
  matched_by: string;
  item: Record<string, unknown>;
}

/** One reviewer-typeahead entry from `GET /repos/{id}/collaborators?q=`.
 *  `name` is the provider-native handle to submit in `CreatePrReq.reviewers`. */
export interface Collaborator {
  name: string;
  display_name: string;
}

/** `POST /git/accounts/{id}/test` / `POST /git/accounts/test` result. Auth
 *  failures come back ok:false + error (HTTP 200) for inline rendering. */
export interface GitAccountTestResp {
  ok: boolean;
  login?: string | null;
  scopes?: string[];
  error?: string | null;
}

// ---------------------------------------------------------------------------
// Git
// ---------------------------------------------------------------------------

export interface CreateGitAccountReq {
  provider: GitProviderKind;
  label: string;
  username: string;
  token: string;
  api_base_url?: string | null;
  namespace?: string | null;
  /** Optional user-entered token expiry (ISO); for providers that don't expose it. */
  token_expires_at?: string | null;
}

/** `PATCH /api/v1/git/accounts/{id}` — all fields optional; absent = keep current. */
export interface UpdateGitAccountReq {
  label?: string;
  username?: string;
  /** Empty string clears to null; non-empty sets; absent keeps current. */
  namespace?: string;
  /** Empty string clears to null; non-empty sets; absent keeps current. */
  api_base_url?: string;
  /** Non-empty rotates the Keychain secret; empty/absent keeps existing. */
  token?: string;
  /** User-entered token expiry (ISO): absent keeps current, `null` clears it. */
  token_expires_at?: string | null;
}

/** `PATCH /api/v1/issue/accounts/{id}` — all fields optional; absent = keep current. */
export interface UpdateIssueAccountReq {
  label?: string;
  email?: string;
  base_url?: string;
  /** Non-empty rotates the Keychain secret; empty/absent keeps existing. */
  token?: string;
  /** Set the user-entered token expiry (ISO); absent keeps current. */
  token_expires_at?: string | null;
}

/**
 * Confluence pages. Bodies cross the wire as Markdown in BOTH directions
 * (`body_md`) — the server converts to/from Confluence storage XHTML, so no
 * caller ever handles the storage format.
 */
export interface ConfluencePageResp {
  id: string;
  title: string;
  space_key: string;
  /** Human-friendly browser URL. */
  url: string;
  /** Current version. Updates resolve this server-side; callers never send it. */
  version: number;
  /** Page body converted from storage XHTML to Markdown. */
  body_md: string;
}

/**
 * Response of the Confluence page writes (create / update): the page plus the
 * read-back comparison (title, space, parent, version, body structure + text).
 */
export interface ConfluencePageWriteResp extends ConfluencePageResp {
  verification: IssueVerification;
}

/** Response of POST /issue/confluence/pages/{page_id}/comments. */
export interface ConfluenceCommentWriteResp {
  id: string;
  url?: string | null;
  /** The comment read back: container + body structure + text. */
  verification: IssueVerification;
}

/** POST /issue/confluence/pages?account_id= */
export interface CreateConfluencePageReq {
  space_key: string;
  title: string;
  /** Page body in Markdown; converted server-side. Supply this or body_html. */
  body_md?: string | null;
  /**
   * Page body already in Confluence storage format (XHTML), passed through
   * untouched — the escape hatch for panel/expand/status macros, layouts and
   * anything Markdown cannot express. Wins over body_md when both are present.
   */
  body_html?: string | null;
  /** Create as a child of this page when present. */
  parent_id?: string | null;
}

/** PUT /issue/confluence/pages/{page_id}?account_id= */
export interface UpdateConfluencePageReq {
  /** Absent keeps the page's current title. */
  title?: string | null;
  /** Replacement body in Markdown. Supply this or body_html. */
  body_md?: string | null;
  /** Replacement body in Confluence storage XHTML. Wins over body_md. */
  body_html?: string | null;
  /** The page `version` this edit was based on; the server answers 409 when
   *  the page has changed since (instead of overwriting the newer edit). */
  base_version?: number | null;
}

/**
 * POST /issue/{account_id}/issues. Jira checks it against createmeta first
 * (one 400 lists every problem and the valid values).
 */
export interface CreateIssueReq {
  /** Jira project key; Linear team key (GET /issue/projects). */
  project_key: string;
  /** A name or id from GET /issue/{account_id}/{project_key}/issue-types. */
  issue_type: string;
  /** Single line, at most 255 characters. */
  summary: string;
  /** Markdown — converted to validated ADF for Jira (tables, lists, code, callouts…). */
  description?: string | null;
  /** Jira only; no spaces. */
  labels?: string[];
  /** Jira only: a priority name or id. */
  priority?: string | null;
  /** Jira only: the parent issue key. */
  parent?: string | null;
  /** Jira only: an Epic's key (becomes the parent). */
  epic?: string | null;
  /** Jira only: other fields by id, in Jira's native value shape. */
  fields?: Record<string, unknown>;
}

/** One read-back comparison of a created issue. */
export interface VerificationCheck {
  name: string;
  ok: boolean;
  detail: string;
}

/** `ok` is false when any check failed — the issue still exists. */
export interface IssueVerification {
  ok: boolean;
  checks: VerificationCheck[];
}

/** Response of POST /issue/{account_id}/issues. */
export interface CreateIssueResp {
  key: string;
  /** Browse URL of the new issue. */
  url: string;
  /** Jira only: the issue read back and compared with the request. */
  verification?: IssueVerification;
}

/** POST /issue/confluence/pages/{page_id}/comments?account_id= */
export interface AddConfluenceCommentReq {
  /** Comment body in Markdown. Supply this or body_html. */
  body_md?: string | null;
  /** Comment body in Confluence storage XHTML. */
  body_html?: string | null;
}

/** One footer comment on a Confluence page, body already converted to Markdown. */
export interface PageComment {
  id: string;
  author: string;
  body_md: string;
  created: string;
}

/** Daemon-side directory listing for the folder picker (GET /fs/browse). */
export interface FsEntry {
  name: string;
  path: string;
  is_dir: boolean;
  is_git_repo: boolean;
}

/** Canonical daemon-host listing; access follows the daemon OS account permissions. */
export interface FsBrowse {
  path: string;
  parent: string | null;
  /** True when the browsed directory is itself a git repo. */
  is_git_repo: boolean;
  entries: FsEntry[];
}

/** `GET /fs/stat` — one path's existence and kind, without a listing. */
export interface FsStat {
  /** Canonical path (symlinks, `..` and `~` resolved). */
  path: string;
  is_dir: boolean;
  /** True when the directory is itself a git repo. */
  is_git_repo: boolean;
}

export interface LogFileEntry {
  name: string;
  size: number;
  modified_ms: number;
}

export interface DaemonLogs {
  log_dir: string;
  files: LogFileEntry[];
  selected: string;
  mode: 'all' | 'tail' | 'since';
  content: string;
  /** Byte offset in the live file where `content` starts. */
  offset: number;
  /** Pass back as `offset` with `mode=since`. Refers to the live file: the
   *  selected one, or for `__all__` the newest (`files[last]`). */
  next_offset: number;
  /** Older content was dropped to honour the daemon's per-response byte cap. */
  truncated: boolean;
}

export interface AddRepoReq {
  path?: string | null;
  clone_url?: string | null;
  name?: string | null;
  git_account_id?: Id | null;
  /** Parent directory to clone INTO (repo lands at `<clone_dir>/<name>`; a
   *  leading `~` is expanded). Defaults to the workspace root. Only meaningful
   *  with `clone_url`. */
  clone_dir?: string | null;
}

/** `PATCH /repos/{id}` — (re)bind the repo's hosting account. The field is
 *  authoritative: an id binds, `null` unbinds. */
export interface UpdateRepoReq {
  git_account_id?: Id | null;
}

export interface FileChange {
  path: string;
  orig_path: string | null;
  kind: 'modified' | 'added' | 'deleted' | 'renamed' | 'untracked' | 'conflicted';
  staged: boolean;
  unstaged: boolean;
}

/** Multi-step git operation a worktree can be in the middle of. */
export type GitOpInProgress = 'merge' | 'rebase' | 'cherry_pick' | 'revert';

export interface RepoStatusResp {
  branch: string;
  upstream: string | null;
  ahead: number;
  behind: number;
  changes: FileChange[];
  /** Operation currently in progress (from the git dir's state files); absent/
   *  undefined when none. Conflicted files can exist without one (stash pop). */
  op_in_progress?: GitOpInProgress | null;
  /** Total untracked paths git reported — present only when the untracked
   *  rows in `changes` were capped (see `untracked_truncated`). */
  untracked_total?: number | null;
  /** True when `changes` carries only the first 5,000 untracked rows (tracked,
   *  staged and conflicted rows are never capped). Absent/false otherwise. */
  untracked_truncated?: boolean;
}

/** `POST /repos/{id}/pull` response: the fresh status plus an optional human
 *  note (what happened to auto-stashed changes). */
export interface PullResp {
  status: RepoStatusResp;
  note?: string | null;
}

/** How `POST /repos/{id}/pull` reconciles: merge, rebase, or refuse anything
 *  that isn't a fast-forward. Absent on the request → the repo's own
 *  `pull.rebase` / `pull.ff` config decides. */
export type PullMode = 'merge' | 'rebase' | 'ff_only';

/** `POST /repos/{id}/pull` body (all optional — an empty body is a plain pull). */
export interface PullReq {
  auto_stash?: boolean;
  mode?: PullMode;
}

/** `POST /repos/{id}/push` body (all optional — an empty body pushes the
 *  current branch, publishing it with `--set-upstream` when it has none). */
export interface PushReq {
  /** Push THIS branch explicitly instead of the checked-out one. */
  branch?: string;
  /** Rewrite the captured target with an explicit remote OID lease and
   *  integration check. Changed source/destination or missing target is 409. */
  force_with_lease?: boolean;
  /** Required for force: source/destination captured before the original push. */
  expected_target?: PushTarget;
}

/** Local-only snapshot from `GET /repos/{id}/push-target`; URLs stay secret. */
export interface PushTarget {
  branch: string;
  source_sha: string;
  remote: string;
  destination_ref: string;
  destination_hash: string;
  remote_sha: string | null;
}

/** `POST /repos/{id}/discard` body. `keep_staged` reverts only the unstaged
 *  side (worktree → index), keeping staged hunks; absent = revert to HEAD. */
export interface DiscardReq {
  paths: string[];
  keep_staged?: boolean;
}

/** `GET /repos/{id}/pull-mode` — the mode a plain pull would use. */
export interface PullModeResp {
  mode: PullMode;
}

/** `POST /repos/{id}/rebase` — replay the current branch onto `onto`. */
export interface RebaseReq {
  onto: string;
  auto_stash?: boolean;
}

/** `GET /repos/{id}/rebase-preview?onto=` — what a rebase would replay. */
export interface RebasePreview {
  commits: number;
  onto_sha: string;
}

export interface BranchInfo {
  name: string;
  is_current: boolean;
  upstream: string | null;
}

export interface CommitInfo {
  sha: string;
  short_sha: string;
  author: string;
  date: string;
  subject: string;
  parents: string[];
  refs: string[];
}

/** One RUN of consecutive lines attributed to the same commit
 *  (`GET /repos/{id}/blame`) — `count` lines starting at `line_start`. */
export interface BlameLine {
  sha: string;
  short_sha: string;
  author: string;
  /** RFC3339 UTC. */
  at: string;
  orig_line: number;
  line_start: number;
  count: number;
  summary: string;
  /** The run's source lines (`count` of them, each capped at 1000 chars). */
  text: string[];
  /** Porcelain `previous`: the parent commit + the file's path there —
   *  "Blame before this change" re-blames `previous.path` at `previous.sha`.
   *  Absent for a root commit. */
  previous?: { sha: string; path: string } | null;
}

export interface BlameResp {
  path: string;
  rev: string;
  lines: BlameLine[];
}

/** `GET /repos/{id}/remotes` row — URLs come back with any `user:password@`
 *  userinfo stripped. */
export interface RemoteInfo {
  name: string;
  fetch_url: string;
  push_url: string;
}

export interface RemoteOpReq {
  op: 'add' | 'set_url' | 'remove';
  name: string;
  url?: string;
}

export interface RefBranch {
  name: string;
  is_current: boolean;
  upstream: string | null;
  remote: boolean;
  /** Commits relative to this local branch's upstream; zero without a live
   *  upstream and for remote refs. Optional for older daemon responses. */
  ahead?: number;
  behind?: number;
  /** True when this branch's tip is already contained in the repo's cleanup base
   *  branch — a hint that it's safe to delete. The base branch itself is never
   *  flagged. Absent on responses that predate the field → treat as false. */
  merged_into_base?: boolean;
  /** Every integration branch (cleanup base + develop/main/master when present)
   *  already holding this tip — a git-flow release shows "develop" before main
   *  has it. Integration branches and their remote twins are never listed.
   *  Absent when empty or on older responses. */
  merged_into?: string[];
  /** Full sha this branch points at, so the graph can locate the branch even when
   *  its tip is outside the currently loaded page of history. Absent on responses
   *  that predate the field. */
  sha?: string;
}

export interface RefTag {
  name: string;
  /** Full sha of the COMMIT the tag resolves to (annotated tags are already
   *  dereferenced server-side). Absent on responses that predate the field. */
  sha?: string;
}

export interface RefsResp {
  local: RefBranch[];
  remote: RefBranch[];
  tags: RefTag[];
  /** Base branch `merged_into_base` was computed against (per-repo
   *  `cleanup_base_branch` override, else the detected default). `null` when the
   *  repo has no resolvable base — then nothing is flagged merged. */
  base_branch?: string | null;
}

/** GET/PUT /repos/{id}/cleanup-base — the per-repo base branch driving the
 *  "safe to delete (merged)" indicators. `base_branch` is the stored override
 *  (`null` = follow the detected default); `resolved` is what it currently
 *  resolves to. */
export interface CleanupBaseResp {
  base_branch: string | null;
  resolved: string | null;
}

/** PUT /repos/{id}/cleanup-base body. Empty/null clears the override. */
export interface SetCleanupBaseReq {
  base_branch?: string | null;
}

/** One `git stash list` entry. `ref` is the `stash@{N}` selector; `parents` are
 *  `[base, index, (untracked)]`. Read-only — fed by GET /repos/{id}/stashes. */
export interface StashInfo {
  index: number;
  ref: string;
  sha: string;
  parents: string[];
  date: string;
  message: string;
  branch: string | null;
}

/** One `git worktree list --porcelain` entry (first = main worktree). Fed by
 *  GET /repos/{id}/worktrees; POST …/worktrees/remove `{path, force?}` and
 *  POST …/worktrees/prune both return the refreshed list. */
export interface WorktreeInfo {
  /** Absolute path of the worktree. */
  path: string;
  /** HEAD commit SHA ("" for a bare entry). */
  head: string;
  /** Checked-out branch (short name); null when detached. */
  branch: string | null;
  /** True for the main worktree — never removable. */
  is_main: boolean;
  locked: boolean;
  lock_reason: string | null;
  /** Stale registration (directory gone) — cleared by prune, not remove. */
  prunable: boolean;
  /** Best-effort uncommitted-changes probe (false when unknown). */
  dirty: boolean;
  /** False/absent when status could not be determined; never force-safe. */
  dirty_known?: boolean;
}

/** One `git submodule status` entry + `.gitmodules` config. Fed by
 *  GET /repos/{id}/submodules; POST …/submodules/update `{path?}` returns the
 *  refreshed list. */
export interface SubmoduleInfo {
  /** Path relative to the repo root. */
  path: string;
  sha: string;
  /** "ok" | "uninitialized" | "modified" | "conflict". */
  state: string;
  describe: string | null;
  url: string | null;
  branch: string | null;
}

export type LineOrigin = 'context' | 'add' | 'del';

export interface DiffLine {
  origin: LineOrigin;
  content: string;
  old_line: number | null;
  new_line: number | null;
}

export interface Hunk {
  header: string;
  lines: DiffLine[];
}

/** How a file changed (git's A/D/M/R/C/T status letters). */
export type FileChangeStatus = 'added' | 'deleted' | 'modified' | 'renamed' | 'copied' | 'typechange';

export interface FileDiff {
  fingerprint?: string;
  path: string;
  old_path: string | null;
  is_binary: boolean;
  hunks: Hunk[];
  /** Per-file cap hit (rendered text > 200 KB or > 5,000 lines): `hunks` is `[]`; `?full=true` loads it. */
  too_large?: boolean | null;
  /** Hunks not included (summary mode, per-file cap, or response budget): fetch via `?path=` (+`old_path=`). */
  hunks_omitted?: boolean | null;
  /** Always populated by the server (numstat); null only for binary files. */
  added?: number | null;
  deleted?: number | null;
  status?: FileChangeStatus | null;
  language?: string | null;
}

export interface DiffResp {
  files: FileDiff[];
  /** Response budget hit (20,000 lines / 4 MB of hunks); later files carry `hunks_omitted`. */
  truncated?: boolean | null;
  total_added?: number | null;
  total_deleted?: number | null;
  /** git skipped rename detection (past `-l1000`): some renames show as delete + add. */
  renames_incomplete?: boolean | null;
}

export interface StagePathsReq {
  paths: string[];
}

/** What `POST /repos/{id}/stage-hunk` does with the addressed hunk. */
export type HunkOp = 'stage' | 'unstage' | 'discard';

/**
 * Stage / unstage / discard ONE hunk — or a line selection inside it.
 * `hunk_header` is the `@@ … @@` line that was rendered: the server rebuilds
 * the patch from its OWN fresh diff, so a stale `hunk_index` would silently
 * act on a different hunk; a header mismatch is a 409 with nothing applied.
 * `lines` indexes `Hunk.lines` (markers excluded); absent = the whole hunk.
 */
export interface StageHunkReq {
  path: string;
  hunk_index: number;
  hunk_header: string;
  fingerprint: string;
  lines?: number[];
  op: HunkOp;
  /** Required for `discard` — it rewrites the working file. */
  confirm?: boolean;
}

export interface StageHunkResp {
  status: RepoStatusResp;
  diff: DiffResp;
  /** `discard` only: the stash the pre-discard snapshot was stored under. */
  backup_stash?: string | null;
}

/** `GET /repos/{id}/commit-config` — the repo's signing defaults. */
export interface CommitConfig {
  gpgsign: boolean;
  format: 'openpgp' | 'ssh' | 'x509' | null;
  signing_key: string | null;
}

export interface CommitReq {
  message: string;
  amend: boolean;
  /** `true` → `-S`, `false` → `--no-gpg-sign`, absent → repo config. */
  sign?: boolean | null;
}

export interface CheckoutReq {
  branch: string;
  create?: boolean;
  /** Stash -u → checkout → pop around a dirty tree. NEVER pulls or merges. */
  auto_stash?: boolean;
}
