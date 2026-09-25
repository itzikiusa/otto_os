// Agent UI control — Git handlers (`otto.ui_git_*`). They drive the Git page's
// shared store (stores/git.svelte.ts) in the document showing the Git module:
// the repo tab opens, the WIP panel shows the status / the file's diff / the
// commit message the agent proposes, and the toolbar's ahead/behind moves —
// so the user watches what the agent does to their repository.
//
// Risk (docs/contracts/ui-commands.json): status, diff and fetch are `read`
// (fetch only moves remote-tracking refs); stage, commit and pull change the
// working tree or the branch — `local_write`, an attributed confirm the user
// may remember for this repo for the session; push is `outward` and ALWAYS
// confirms (where, how many commits, who sees them).

import { registerUiCommands, registerUiState, UiCommandError, type UiCommandCtx } from '../uiCommands';
import { git } from '../stores/git.svelte';
import { router } from '../router.svelte';
import { api, isDirtyGitRefusal } from '../api/client';
import { toasts } from '../toast.svelte';
import type { DiffResp, FileChange, PullMode, Repo, RepoStatusResp } from '../api/types';
import { reportPull } from '../../modules/git/pullFlow';
import { asUiError, capList, highlightWhenReady } from './pagePort';

const SUB_TABS = ['graph', 'prs', 'review', 'focus'] as const;
type SubTab = (typeof SUB_TABS)[number];
const PULL_MODES: PullMode[] = ['merge', 'rebase', 'ff_only'];
const DIFF_MAX_LINES = 800;

/** The repo list is loaded and the open tabs restored in this document. */
async function ready(): Promise<void> {
  await git.initializeOpenTabs();
  if (!git.allReposLoaded) {
    throw new UiCommandError('failed', `Couldn't load the repositories${git.allReposError ? `: ${git.allReposError}` : ''}.`);
  }
}

/** Resolve `key` (id, exact path, or unique case-insensitive name); the
 *  active repo tab when omitted. */
function repoFor(key: unknown): Repo {
  const k = typeof key === 'string' ? key.trim() : '';
  if (!k) {
    const active = git.allRepos.find((r) => r.id === git.activeRepoId);
    if (active) return active;
    throw new UiCommandError('invalid_args', 'No repository is open — pass `repo_id` (otto.ui_git_list_repos).');
  }
  const hit = git.allRepos.find((r) => r.id === k) ?? git.allRepos.find((r) => r.path === k);
  if (hit) return hit;
  const byName = git.allRepos.filter((r) => r.name.toLowerCase() === k.toLowerCase());
  if (byName.length === 1) return byName[0];
  if (byName.length > 1) throw new UiCommandError('invalid_args', `More than one repository is named “${k}” — pass its id.`);
  const known = git.allRepos.slice(0, 20).map((r) => `${r.name} (${r.id})`).join(', ');
  throw new UiCommandError('not_found', `No repository “${k}”.${known ? ` Known: ${known}` : ''}`);
}

/** Show `repo` as the active tab on `tab` (dropping a PR deep link, which
 *  would otherwise keep the repo view hidden). */
function show(repo: Repo, tab?: SubTab): void {
  git.openRepoTab(repo.id, tab);
  if (tab && git.subTabFor(repo.id) !== tab) git.setSubTab(repo.id, tab);
  if (router.parts[0] === 'git' && router.parts.length > 1) router.go('git');
}

/** Fresh status for `repo` (throws `failed` with the reason when it can't load). */
async function freshStatus(repo: Repo): Promise<RepoStatusResp> {
  await git.refreshStatus(repo.id);
  const err = git.statusErrorById[repo.id];
  const s = git.statusById[repo.id];
  if (err || !s) throw new UiCommandError('failed', `Couldn't read the status of ${repo.name}${err ? `: ${err}` : ''}.`);
  return s;
}

function changeRow(c: FileChange) {
  return { path: c.path, orig_path: c.orig_path, kind: c.kind, staged: c.staged, unstaged: c.unstaged };
}

function statusResult(repo: Repo, s: RepoStatusResp) {
  return {
    repo_id: repo.id,
    repo: repo.name,
    branch: s.branch,
    upstream: s.upstream,
    ahead: s.ahead,
    behind: s.behind,
    op_in_progress: s.op_in_progress ?? null,
    changes: capList(s.changes.map(changeRow), 500),
  };
}

/** Open the WIP panel (graph tab) and outline it once it renders. */
async function showWip(repo: Repo, ctx: UiCommandCtx, opts: Parameters<typeof git.requestWip>[1] = {}, target = '.wip-panel'): Promise<void> {
  show(repo, 'graph');
  git.requestWip(repo.id, opts);
  await highlightWhenReady(ctx, target);
}

/** A DiffResp as unified-diff text, capped at `maxLines` lines. */
export function diffText(d: DiffResp, maxLines: number): { text: string; truncated: boolean; binary: boolean } {
  const out: string[] = [];
  let binary = false;
  for (const f of d.files) {
    out.push(`--- a/${f.old_path ?? f.path}`, `+++ b/${f.path}`);
    if (f.is_binary) {
      binary = true;
      out.push('(binary file)');
      continue;
    }
    if (f.too_large) {
      out.push('(diff too large to show)');
      continue;
    }
    for (const h of f.hunks) {
      out.push(h.header);
      for (const l of h.lines) out.push(`${l.origin === 'add' ? '+' : l.origin === 'del' ? '-' : ' '}${l.content.replace(/\n$/, '')}`);
    }
  }
  const truncated = out.length > maxLines;
  return { text: (truncated ? out.slice(0, maxLines) : out).join('\n'), truncated, binary };
}

registerUiState('git', () => {
  const repo = git.allRepos.find((r) => r.id === git.activeRepoId) ?? null;
  const s = repo ? git.statusById[repo.id] : null;
  return {
    active_repo: repo ? { id: repo.id, name: repo.name } : null,
    sub_tab: repo ? git.subTabFor(repo.id) : null,
    branch: s?.branch ?? null,
    changes: s?.changes.length ?? null,
    open_repos: git.openRepoIds.length,
  };
});

registerUiCommands('git', {
  async git_list_repos(args: { query?: string }) {
    await ready();
    const q = (args.query ?? '').trim().toLowerCase();
    const rows = git.allRepos
      .filter((r) => !q || r.name.toLowerCase().includes(q) || r.path.toLowerCase().includes(q))
      .map((r) => ({
        id: r.id,
        name: r.name,
        path: r.path,
        workspace_id: r.workspace_id,
        remote_url: r.remote_url,
        open: git.openRepoIds.includes(r.id),
        active: git.activeRepoId === r.id,
      }));
    return capList(rows);
  },

  async git_open_repo(args: { repo_id: string; tab?: SubTab }, ctx: UiCommandCtx) {
    await ready();
    if (args.tab !== undefined && !SUB_TABS.includes(args.tab)) throw new UiCommandError('invalid_args', `Unknown tab “${args.tab}”.`);
    const repo = repoFor(args.repo_id);
    show(repo, args.tab);
    const s = await freshStatus(repo);
    await highlightWhenReady(ctx, '.git-tablist .git-tab[aria-selected="true"]');
    return { ...statusResult(repo, s), tab: git.subTabFor(repo.id) };
  },

  async git_tab(args: { repo_id?: string; tab: SubTab }) {
    await ready();
    if (!SUB_TABS.includes(args.tab)) throw new UiCommandError('invalid_args', `Unknown tab “${String(args.tab)}”.`);
    const repo = repoFor(args.repo_id);
    show(repo, args.tab);
    return { repo_id: repo.id, tab: git.subTabFor(repo.id) };
  },

  async git_status(args: { repo_id?: string }, ctx: UiCommandCtx) {
    await ready();
    const repo = repoFor(args.repo_id);
    show(repo, 'graph');
    const s = await freshStatus(repo);
    if (s.changes.length) await showWip(repo, ctx);
    return statusResult(repo, s);
  },

  async git_diff(args: { repo_id?: string; path: string; staged?: boolean; max_lines?: number }, ctx: UiCommandCtx) {
    await ready();
    const path = args.path?.trim();
    if (!path) throw new UiCommandError('invalid_args', '`path` is empty.');
    const repo = repoFor(args.repo_id);
    const s = await freshStatus(repo);
    const ch = s.changes.find((c) => c.path === path);
    if (!ch) throw new UiCommandError('not_found', `${path} has no changes in ${repo.name} (otto.ui_git_status lists them).`);
    // The same target WipPanel shows for this file.
    const target =
      ch.kind === 'untracked' || ch.kind === 'conflicted'
        ? 'working'
        : ch.staged && ch.unstaged
          ? args.staged ? 'staged' : 'worktree'
          : ch.unstaged ? 'worktree' : ch.staged ? 'staged' : 'working';
    await showWip(repo, ctx, { path, staged: target === 'staged' }, '.wip-panel .wp-file.selected');
    let d: DiffResp;
    try {
      d = await api.get<DiffResp>(`/repos/${repo.id}/diff?target=${target}&path=${encodeURIComponent(path)}`, ctx.signal);
    } catch (e) {
      throw asUiError(e);
    }
    const max = Math.min(Math.max(1, Math.trunc(args.max_lines ?? DIFF_MAX_LINES)), 5000);
    return { repo_id: repo.id, path, kind: ch.kind, target, ...diffText(d, max) };
  },

  async git_stage(args: { repo_id?: string; paths: string[]; unstage?: boolean }, ctx: UiCommandCtx) {
    await ready();
    const paths = Array.isArray(args.paths) ? args.paths.filter((p) => typeof p === 'string' && p.trim()) : [];
    if (!paths.length) throw new UiCommandError('invalid_args', '`paths` is empty.');
    const repo = repoFor(args.repo_id);
    const s = await freshStatus(repo);
    const conflicted = paths.filter((p) => s.changes.some((c) => c.path === p && c.kind === 'conflicted'));
    if (conflicted.length && !args.unstage) {
      // Staging a conflicted file would mark it resolved with its markers in.
      throw new UiCommandError('invalid_args', `Resolve the conflicts first: ${conflicted.slice(0, 5).join(', ')}.`);
    }
    await showWip(repo, ctx);
    const verb = args.unstage ? 'Unstage' : 'Stage';
    const list = paths.slice(0, 20).join('\n') + (paths.length > 20 ? `\n… and ${paths.length - 20} more` : '');
    const ok = await ctx.confirmWrite({
      what: `${verb} ${paths.length} file${paths.length === 1 ? '' : 's'}:\n${list}`,
      where: `${repo.name} (${s.branch})`,
      verb,
      connId: `git:${repo.id}`,
    });
    if (!ok) throw new UiCommandError('cancelled_by_user', `The user declined to ${verb.toLowerCase()} the files.`);
    try {
      const next = await git.stage(repo.id, paths, !args.unstage);
      return statusResult(repo, next);
    } catch (e) {
      throw asUiError(e);
    }
  },

  async git_commit(args: { repo_id?: string; message: string; amend?: boolean }, ctx: UiCommandCtx) {
    await ready();
    const message = args.message?.trim();
    if (!message) throw new UiCommandError('invalid_args', '`message` is empty.');
    const repo = repoFor(args.repo_id);
    const s = await freshStatus(repo);
    if (s.changes.some((c) => c.kind === 'conflicted')) {
      throw new UiCommandError('invalid_args', `${repo.name} has unresolved conflicts — resolve them before committing.`);
    }
    const staged = s.changes.filter((c) => c.staged);
    if (!staged.length && !args.amend) {
      throw new UiCommandError('invalid_args', 'Nothing is staged — stage files first (otto.ui_git_stage).');
    }
    // Show the message in the composer the user would commit from.
    const nl = message.indexOf('\n');
    const subject = nl === -1 ? message : message.slice(0, nl).trim();
    const body = nl === -1 ? '' : message.slice(nl + 1).replace(/^\s*\n/, '').trimEnd();
    await showWip(repo, ctx, { subject, body }, '.wip-panel .wp-composer');
    const files = staged.slice(0, 15).map((c) => c.path).join('\n') + (staged.length > 15 ? `\n… and ${staged.length - 15} more` : '');
    const ok = await ctx.confirmWrite({
      what: `${args.amend ? 'Amend the last commit' : `Commit ${staged.length} staged file${staged.length === 1 ? '' : 's'}`} on ${s.branch}:\n\n${message}${files ? `\n\nFiles:\n${files}` : ''}`,
      where: `${repo.name} (${s.branch})`,
      verb: args.amend ? 'Amend' : 'Commit',
      connId: `git:${repo.id}`,
    });
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined the commit (the message stays in the composer).');
    try {
      const r = await git.commit(repo.id, { message, amend: args.amend ?? false });
      // Clear the composer the agent filled (a no-op once the panel closed).
      git.requestWip(repo.id, { subject: '', body: '' });
      toasts.success('Committed', r.sha.slice(0, 8));
      return { sha: r.sha, ...statusResult(repo, r.status) };
    } catch (e) {
      throw asUiError(e);
    }
  },

  async git_fetch(args: { repo_id?: string }, ctx: UiCommandCtx) {
    await ready();
    const repo = repoFor(args.repo_id);
    show(repo);
    ctx.progress(`Fetching ${repo.name}`);
    try {
      const s = await git.fetchRepo(repo.id);
      toasts.success('Fetched', s.behind > 0 ? `${s.behind} new commit${s.behind === 1 ? '' : 's'} on ${s.upstream ?? 'the upstream'}` : repo.name);
      return statusResult(repo, s);
    } catch (e) {
      throw asUiError(e);
    }
  },

  async git_pull(args: { repo_id?: string; mode?: PullMode }, ctx: UiCommandCtx) {
    await ready();
    if (args.mode !== undefined && !PULL_MODES.includes(args.mode)) throw new UiCommandError('invalid_args', `Unknown mode “${args.mode}”.`);
    const repo = repoFor(args.repo_id);
    show(repo, 'graph');
    const s = await freshStatus(repo);
    if (!s.upstream) throw new UiCommandError('invalid_args', `${s.branch} has no upstream to pull from.`);
    const ok = await ctx.confirmWrite({
      what: `Pull ${s.upstream} into ${s.branch}${args.mode ? ` (${args.mode.replace('_', '-')})` : ''}${s.behind ? ` — ${s.behind} incoming commit${s.behind === 1 ? '' : 's'}` : ''}.`,
      where: `${repo.name} (${s.branch})`,
      verb: 'Pull',
      connId: `git:${repo.id}`,
    });
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined the pull.');
    try {
      const r = await git.pull(repo.id, false, args.mode);
      reportPull(r, (next) => git.setStatus(repo.id, next));
      git.refsRev[repo.id] = (git.refsRev[repo.id] ?? 0) + 1;
      return { ...statusResult(repo, r.status), note: r.note ?? null };
    } catch (e) {
      if (isDirtyGitRefusal(e)) {
        throw new UiCommandError('failed', 'Uncommitted changes are in the way of the pull — commit or stash them first.');
      }
      throw asUiError(e);
    }
  },

  async git_push(args: { repo_id?: string }, ctx: UiCommandCtx) {
    await ready();
    const repo = repoFor(args.repo_id);
    show(repo, 'graph');
    const s = await freshStatus(repo);
    if (s.upstream && s.ahead === 0) {
      return { pushed: false, note: `${s.branch} has nothing to push to ${s.upstream}.`, ...statusResult(repo, s) };
    }
    const n = s.ahead;
    const ok = await ctx.confirmWrite({
      what: `${s.upstream
        ? `Push ${n} commit${n === 1 ? '' : 's'} from ${s.branch} to ${s.upstream}.`
        : `Publish the new branch ${s.branch} to the remote${repo.remote_url ? ` (${repo.remote_url})` : ''}.`
      }\n\nWho sees it: everyone with access to the remote repository.`,
      where: `${repo.name} (${s.branch})`,
      verb: 'Push',
      outward: true,
    });
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined the push.');
    ctx.progress(`Pushing ${s.branch}`);
    try {
      const next = await git.push(repo.id);
      toasts.success(s.upstream ? 'Pushed' : 'Branch published', next.upstream ?? s.branch);
      return { pushed: true, ...statusResult(repo, next) };
    } catch (e) {
      throw asUiError(e);
    }
  },
});
