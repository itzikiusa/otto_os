// The git graph's context menus (arch-1-09: split out of GraphView.svelte):
// commit, branch (local + remote), tag, stash, worktree, submodule and the
// commit-diff file-tools menus, plus the GitFlow and "Merge into" item lists.
// Pure item builders over the shared `ctxMenu`; every item runs a handler
// from graphActions.svelte.ts or a GraphView callback passed in the host.

import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
import type { CommitInfo, RefBranch, RefTag, RefsResp, StashInfo, SubmoduleInfo, WorktreeInfo } from '../../../lib/api/types';
import { ui } from '../../../lib/stores/ui.svelte';
import { splitRemoteRef } from '../refTracking';
import { gitBridge } from '../gitBridge.svelte';
import type { RefChip } from '../ref-menus';
import type { GraphActions, RefKnowledge } from './graphActions.svelte';

/** GraphView's side of the menus: live state (getters) and its callbacks. */
export interface GraphMenusHost {
  readonly repoId: string;
  readonly onmergerequest: ((source: string, target: string) => void) | undefined;
  readonly refs: RefsResp | null;
  readonly selectedCommit: CommitInfo | null;
  readonly worktreeByBranch: Map<string, WorktreeInfo>;
  readonly worktreeBranches: Set<string>;
  readonly refKnowledge: RefKnowledge;
  readonly baseBranch: string | null;
  openCreatePr(source: string): void;
  chipsFor(c: CommitInfo): RefChip[];
  setCleanupBase(name: string | null): Promise<void>;
}

export function createGraphMenus(h: GraphMenusHost, a: GraphActions) {
  const { openCreatePr, chipsFor, setCleanupBase } = h;
  const {
    clip,
    startFlowBranch,
    cherryPick,
    revertCommit,
    createBranchAt,
    createTagAt,
    createBranchFrom,
    renameBranch,
    deleteLocalBranch,
    deleteRemoteBranch,
    pushTag,
    deleteTag,
    stashApply,
    stashDrop,
    isForeignWorktree,
    openWorktree,
    wtRemove,
    wtPrune,
    subUpdate,
    checkout,
    checkoutRemote,
    pullCurrent,
    rebaseOnto,
  } = a;

  /** Menu items for the "GitFlow" section, parameterised by start point
   *  (a commit sha or a branch tip name). Shared by commit + branch menus. */
  function gitFlowItems(startPoint: string): MenuItem[] {
    return [
      { label: 'Start feature branch…', icon: 'branch', action: () => void startFlowBranch('feature', startPoint) },
      { label: 'Start release branch…', icon: 'branch', action: () => void startFlowBranch('release', startPoint) },
      { label: 'Start hotfix branch…', icon: 'branch', action: () => void startFlowBranch('hotfix', startPoint) },
    ];
  }

  /** Branch names decorating a commit (`HEAD -> x`, `x`, `origin/x` → `x`),
   *  deduped, excluding tags/stash/detached-HEAD markers and branches held by a
   *  FOREIGN worktree (checking those out errors hard — the refs tree routes
   *  them to their worktree instead). */
  function commitBranchNames(c: CommitInfo): string[] {
    const names: string[] = [];
    for (const chip of chipsFor(c)) {
      if (!['head', 'local', 'remote'].includes(chip.kind)) continue;
      const name = chip.kind === 'remote' ? chip.label.replace(/^[^/]+\//, '') : chip.label;
      if (!name || names.includes(name) || h.worktreeBranches.has(name)) continue;
      names.push(name);
    }
    return names;
  }

  function commitMenu(e: MouseEvent, c: CommitInfo): void {
    const { currentBranch } = h.refKnowledge;
    const items: MenuItem[] = [];
    // Branches on this commit → a one-click plain SWITCH (a branch that's easy
    // to spot on the graph but hard to find in a crowded refs tree). Nothing
    // here pulls: the current branch instead gets an explicit `Pull`, and a
    // branch held by another worktree routes to that worktree (git refuses to
    // check it out twice — same rule as `branchMenu`).
    for (const name of commitBranchNames(c)) {
      const wt = h.worktreeByBranch.get(name);
      if (name === currentBranch) {
        items.push({ label: `Pull ${name}`, icon: 'arrowDown', action: () => void pullCurrent() });
      } else if (wt) {
        items.push({
          label: `Open worktree (${name})`,
          icon: 'worktree',
          action: () => void openWorktree(wt),
        });
      } else {
        items.push({
          label: `Check out ${name}`,
          icon: 'branch',
          // A remote-only decoration (`origin/x` → `x`) must take the
          // `-b x --track origin/x` path, not git's `checkout.guess` DWIM
          // (which fails outright with several remotes).
          action: () => void checkout(name, !h.refKnowledge.localNames.has(name)),
        });
      }
    }
    if (items.length > 0) items.push({ separator: true });
    items.push(
      {
        label: 'Check out commit (detached)',
        icon: 'branch',
        action: () => void checkout(c.sha, false),
      },
      { label: 'Cherry-pick commit', icon: 'note', action: () => void cherryPick(c) },
      { label: 'Bisect: known good revision…', icon: 'check', action: () => gitBridge.selectBisect(h.repoId, 'good', c.sha) },
      { label: 'Bisect: known bad revision…', icon: 'info', action: () => gitBridge.selectBisect(h.repoId, 'bad', c.sha) },
      { label: 'Plan interactive rebase onto this commit…', icon: 'merge', action: () => gitBridge.openRecovery(h.repoId, 'rebase', c.sha) },
      { separator: true },
      { label: 'Create branch here…', icon: 'branch', action: () => void createBranchAt(c.sha) },
      { label: 'Create tag here…', icon: 'tag', action: () => void createTagAt(c.sha, false) },
      {
        label: 'Create annotated tag here…',
        icon: 'tag',
        action: () => void createTagAt(c.sha, true),
      },
    );
    // Open a PR from the checked-out branch (the "from a commit while on a
    // branch" case) — only meaningful when a branch is checked out.
    if (currentBranch) {
      items.push({ separator: true });
      items.push({
        label: `Create PR from ${currentBranch}…`,
        icon: 'send',
        action: () => openCreatePr(currentBranch),
      });
    }
    // GitFlow section — start a flow branch off this exact commit.
    items.push({ separator: true });
    items.push({ label: 'GitFlow', disabled: true });
    items.push(...gitFlowItems(c.sha));
    items.push(
      { separator: true },
      { label: 'Revert commit…', icon: 'refresh', action: () => void revertCommit(c) },
      { separator: true },
      { label: 'Copy commit SHA', icon: 'note', action: () => void clip(c.sha, c.sha) },
      { label: 'Copy short SHA', icon: 'note', action: () => void clip(c.short_sha, c.short_sha) },
      {
        label: 'Copy commit message',
        icon: 'note',
        action: () => void clip(c.subject, c.subject),
      },
    );
    ctxMenu.show(e, items, { filter: items.length > 12 });
  }

  /** Local branches OTHER than `selfMerge` (the menu subject's merge name),
   *  as "Merge into <t>" items routing through the existing merge+conflict flow
   *  via `onmergerequest(<source merge name>, t)`. Empty if nothing to merge into. */
  function mergeIntoItems(selfMerge: string): MenuItem[] {
    const targets = (h.refs?.local ?? []).map((b) => b.name).filter((t) => t !== selfMerge);
    if (!h.onmergerequest || targets.length === 0) return [];
    return targets.map((t) => ({
      label: `Merge into ${t}`,
      icon: 'branch',
      action: () => h.onmergerequest?.(selfMerge, t),
    }));
  }

  function branchMenu(e: MouseEvent, b: RefBranch): void {
    const { remoteNames, localNames, currentBranch } = h.refKnowledge;
    const items: MenuItem[] = [];

    if (b.remote) {
      // Remote ref row: checkout as a local tracking branch; delete on ITS remote
      // (`upstream/x` must never delete `origin/x`).
      const { remote: remoteName, branch: localName } = splitRemoteRef(b.name);
      items.push({ label: 'Checkout', icon: 'branch', action: () => checkoutRemote(b) });
      // Rebasing onto a remote ref is the common "catch up with origin" move.
      if (currentBranch && b.name !== currentBranch) {
        items.push({
          label: `Rebase ${currentBranch} onto ${b.name}…`,
          icon: 'merge',
          action: () => void rebaseOnto(b.name),
        });
      }
      items.push({ separator: true });
      items.push({
        label: 'Create branch from here…',
        icon: 'branch',
        action: () => void createBranchFrom(b.name),
      });
      // Merge into … — remote refs merge by their short local name (drag logic).
      const mergeInto = mergeIntoItems(localName);
      if (mergeInto.length > 0) {
        items.push({ separator: true });
        items.push({ label: `Merge ${localName} into`, disabled: true });
        items.push(...mergeInto);
      }
      // GitFlow — start a flow branch off the remote tip.
      items.push({ separator: true });
      items.push({ label: 'GitFlow', disabled: true });
      items.push(...gitFlowItems(b.name));
      // Delete options reflect what actually EXISTS: the remote ref always (this
      // is a remote row); the local twin + "local + remote" only when a local
      // branch of the same name exists and isn't the one checked out.
      const localTwin = localNames.has(localName) && localName !== currentBranch;
      items.push({ separator: true });
      items.push({
        label: `Delete ${b.name}…`,
        icon: 'trash',
        danger: true,
        action: () => void deleteRemoteBranch(localName, remoteName),
      });
      if (localTwin) {
        items.push({
          label: `Delete ${localName}…`,
          icon: 'trash',
          danger: true,
          action: () => void deleteLocalBranch(localName, false),
        });
        items.push({
          label: `Delete ${localName} + ${b.name}…`,
          icon: 'trash',
          danger: true,
          // The remote half targets THIS row's remote (S15-303) — never origin
          // by default when the row says `upstream/x`.
          action: () => void deleteLocalBranch(localName, true, remoteName),
        });
      }
      items.push({ separator: true });
      items.push({ label: 'Copy branch name', icon: 'note', action: () => void clip(localName, localName) });
    } else {
      const isCurrent = b.name === currentBranch;
      // Branch checked out in another worktree → open that tree, never checkout
      // (git refuses "already checked out at …").
      const wtElsewhere = h.worktreeByBranch.get(b.name);
      if (wtElsewhere) {
        items.push({
          label: 'Open worktree',
          icon: 'worktree',
          action: () => void openWorktree(wtElsewhere),
        });
      } else {
        items.push({
          label: 'Checkout',
          icon: 'branch',
          disabled: isCurrent,
          action: () => !isCurrent && void checkout(b.name, false),
        });
      }
      // Updating the branch is its own entry — only the CHECKED-OUT branch can
      // be pulled; any other one is a rebase target.
      if (isCurrent) {
        items.push({ label: `Pull ${b.name}`, icon: 'arrowDown', action: () => void pullCurrent() });
      } else if (currentBranch) {
        items.push({
          label: `Rebase ${currentBranch} onto ${b.name}…`,
          icon: 'merge',
          action: () => void rebaseOnto(b.name),
        });
      }
      items.push({ separator: true });
      items.push({
        label: 'Create branch from here…',
        icon: 'branch',
        action: () => void createBranchFrom(b.name),
      });
      items.push({ label: 'Rename…', icon: 'edit', action: () => void renameBranch(b.name) });
      // Open a PR sourced from this branch.
      items.push({ separator: true });
      items.push({
        label: `Create pull request from ${b.name}…`,
        icon: 'send',
        action: () => openCreatePr(b.name),
      });
      // Merge into … — pick any OTHER local branch as the destination.
      const mergeInto = mergeIntoItems(b.name);
      if (mergeInto.length > 0) {
        items.push({ separator: true });
        items.push({ label: `Merge ${b.name} into`, disabled: true });
        items.push(...mergeInto);
      }
      // GitFlow — start a flow branch off this branch tip.
      items.push({ separator: true });
      items.push({ label: 'GitFlow', disabled: true });
      items.push(...gitFlowItems(b.name));
      // A matching remote branch → offer remote deletes too. The remote is the
      // branch's TRACKING remote when it tracks the same name (a fork's
      // `upstream/x`), else origin — "local + remote" must delete the twin the
      // user sees, not whatever origin holds under that name (S15-303).
      const up = b.upstream ? splitRemoteRef(b.upstream) : null;
      const trackRemote = up && up.branch === b.name ? up.remote : 'origin';
      const hasRemote = remoteNames.has(`${trackRemote}/${b.name}`);
      // Never offer Delete on the checked-out branch (git refuses).
      if (!isCurrent) {
        items.push({ separator: true });
        items.push({
          label: `Delete ${b.name}…`,
          icon: 'trash',
          danger: true,
          action: () => void deleteLocalBranch(b.name, false),
        });
        if (hasRemote) {
          items.push({
            label: `Delete ${trackRemote}/${b.name}…`,
            icon: 'trash',
            danger: true,
            action: () => void deleteRemoteBranch(b.name, trackRemote),
          });
          items.push({
            label: `Delete ${b.name} + ${trackRemote}/${b.name}…`,
            icon: 'trash',
            danger: true,
            action: () => void deleteLocalBranch(b.name, true, trackRemote),
          });
        }
      }
      items.push({ separator: true });
      items.push({ label: 'Copy branch name', icon: 'note', action: () => void clip(b.name, b.name) });
      if (b.upstream) {
        items.push({
          label: 'Copy upstream name',
          icon: 'note',
          action: () => void clip(b.upstream!, b.upstream!),
        });
      }
    }
    // Cleanup base branch (Feature 3): choose which base the "merged → safe to
    // delete" indicators compute against, or clear back to the detected default.
    // Indicator-only; never deletes a branch.
    const baseName = b.remote ? b.name.replace(/^[^/]+\//, '') : b.name;
    items.push({ separator: true });
    if (baseName === h.baseBranch) {
      items.push({
        label: 'Clear cleanup base (use default)',
        icon: 'gear',
        action: () => void setCleanupBase(null),
      });
    } else {
      items.push({
        label: `Set “${baseName}” as cleanup base`,
        icon: 'gear',
        action: () => void setCleanupBase(baseName),
      });
    }
    ctxMenu.show(e, items, { filter: items.length > 12 });
  }

  function tagMenu(e: MouseEvent | KeyboardEvent, t: RefTag): void {
    const items: MenuItem[] = [
      {
        label: 'Check out tag (detached)',
        icon: 'branch',
        action: () => void checkout(t.name, false),
      },
      { separator: true },
      { label: 'Push tag to origin', icon: 'send', action: () => void pushTag(t.name) },
      {
        label: 'Delete tag…',
        icon: 'trash',
        danger: true,
        action: () => void deleteTag(t.name, false),
      },
      {
        label: 'Delete tag on origin…',
        icon: 'trash',
        danger: true,
        action: () => void deleteTag(t.name, true),
      },
      { separator: true },
      { label: 'Copy tag name', icon: 'note', action: () => void clip(t.name, t.name) },
    ];
    ctxMenu.show(e, items);
  }

  function stashMenu(e: MouseEvent | KeyboardEvent, s: StashInfo): void {
    const items: MenuItem[] = [
      { label: 'Apply stash', icon: 'stash', action: () => void stashApply(s) },
      { separator: true },
      { label: 'Drop stash…', icon: 'trash', danger: true, action: () => void stashDrop(s) },
      { separator: true },
      { label: 'Copy message', icon: 'note', action: () => void clip(s.message, s.message) },
    ];
    ctxMenu.show(e, items);
  }

  function wtMenu(e: MouseEvent | KeyboardEvent, w: WorktreeInfo): void {
    const items: MenuItem[] = [];
    if (!w.prunable && isForeignWorktree(w)) {
      items.push({
        label: 'Open worktree',
        icon: 'worktree',
        action: () => void openWorktree(w),
      });
      items.push({ separator: true });
    }
    if (!w.is_main) {
      if (w.prunable) {
        // Directory is already gone — the only cleanup is dropping the entry.
        items.push({ label: 'Remove stale entry', icon: 'trash', danger: true, action: () => void wtPrune() }); // ui-guards: allow — no confirm: the folder is already gone, only its record is dropped
      } else {
        items.push({
          label: w.dirty ? 'Remove (discard changes)…' : 'Remove worktree…',
          icon: 'trash',
          danger: true,
          action: () => void wtRemove(w),
        });
      }
      items.push({ separator: true });
    }
    items.push({ label: 'Copy path', icon: 'copy', action: () => void clip(w.path, w.path) });
    if (w.branch) {
      const branch = w.branch;
      items.push({ label: 'Copy branch', icon: 'branch', action: () => void clip(branch, branch) });
    }
    ctxMenu.show(e, items);
  }

  function subMenu(e: MouseEvent | KeyboardEvent, sub: SubmoduleInfo): void {
    const items: MenuItem[] = [
      {
        label: sub.state === 'uninitialized' ? 'Initialize (clone)' : 'Update to recorded commit',
        icon: 'refresh',
        action: () => void subUpdate(sub.path),
      },
      { label: 'Update all submodules', icon: 'refresh', action: () => void subUpdate() },
      { separator: true },
      { label: 'Copy path', icon: 'copy', action: () => void clip(sub.path, sub.path) },
    ];
    if (sub.url) {
      const url = sub.url;
      items.push({ label: 'Copy URL', icon: 'copy', action: () => void clip(url, url) });
    }
    items.push({ label: 'Copy commit SHA', icon: 'copy', action: () => void clip(sub.sha, sub.sha.slice(0, 10)) });
    ctxMenu.show(e, items);
  }

  /** ⋯ on a file row of the commit diff: the file-scoped history/blame tools.
   *  Routed through `gitBridge` because the panels that render them live in
   *  RepoView, above this component. */
  function fileToolsMenu(e: MouseEvent, path: string): void {
    const rev = h.selectedCommit?.sha;
    ctxMenu.show(e, [
      {
        label: 'History',
        icon: 'note',
        action: () => gitBridge.openFileTool({ kind: 'history', repoId: h.repoId, path }),
      },
      {
        label: 'Blame',
        icon: 'note',
        action: () => gitBridge.openFileTool({ kind: 'blame', repoId: h.repoId, path, rev }),
      },
    ]);
  }

  return {
    gitFlowItems,
    commitBranchNames,
    commitMenu,
    mergeIntoItems,
    branchMenu,
    tagMenu,
    stashMenu,
    wtMenu,
    subMenu,
    fileToolsMenu,
  };
}
