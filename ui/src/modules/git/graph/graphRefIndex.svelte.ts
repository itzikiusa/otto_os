// What the git graph knows about refs (arch-1-09: split out of GraphView.svelte):
// the ref-chip classification (GitKraken-style local / remote / tag / HEAD),
// worktree-held branches, lane names, HEAD, the "merged into" marks, stash
// plumbing detection and the first-parent indexes behind the branch-line
// highlight. Derived only — it reads GraphView's data through a host and
// owns no loading.

import type { CommitInfo, RefBranch, RefsResp, RepoStatusResp, StashInfo, WorktreeInfo } from '../../../lib/api/types';
import { haveTypedRefs as typedRefs, headShaOf, isTagRef, refLabel, stashShortMsg, type ChipKind, type RefChip } from '../ref-menus';
import type { RefKnowledge } from './graphActions.svelte';

export interface GraphRefIndexHost {
  readonly refs: RefsResp | null;
  readonly commits: CommitInfo[];
  readonly stashes: StashInfo[];
  readonly worktrees: WorktreeInfo[];
  readonly status: RepoStatusResp;
  /** A worktree other than the open repo tab (main or linked). */
  isForeignWorktree(w: WorktreeInfo): boolean;
}

export type GraphRefIndex = ReturnType<typeof createGraphRefIndex>;

// Chips are right-aligned and CLIP ON THE LEFT (they butt against the graph
// node). So the RIGHTMOST chip is the one guaranteed visible — give that slot
// to the branch you actually care about (the checked-out/local branch) and
// let long tags clip first. Hence head/local sort LAST (rightmost), tag first.
const CHIP_ORDER: Record<ChipKind, number> = {
  tag: 0, stash: 0, remote: 1, local: 2, detached: 3, head: 3,
};

export function createGraphRefIndex(h: GraphRefIndexHost) {
  // Branches checked out in a DIFFERENT worktree than the open repo tab (linked
  // or main). Chips + ref rows mark them so a branch that "lives elsewhere" is
  // self-explaining; click/open routes to that worktree instead of checkout.
  const worktreeByBranch = $derived.by(() => {
    const m = new Map<string, WorktreeInfo>();
    for (const w of h.worktrees) {
      if (w.branch && h.isForeignWorktree(w)) m.set(w.branch, w);
    }
    return m;
  });
  const worktreeBranches = $derived(new Set(worktreeByBranch.keys()));
  const localBranchByName = $derived.by(() =>
    new Map<string, RefBranch>((h.refs?.local ?? []).map((b) => [b.name, b])),
  );

  // Snapshot the refs response into plain lookup sets, used to classify a
  // decoration token that isn't obviously a tag/HEAD. Reading `refs` through a
  // local const sidesteps a runes/null-narrowing quirk in svelte-check.
  const refKnowledge = $derived.by((): RefKnowledge => {
    const r: RefsResp | null = h.refs;
    const local = r?.local ?? [];
    const remote = r?.remote ?? [];
    return {
      remoteNames: new Set<string>(remote.map((b) => b.name)),
      localNames: new Set<string>(local.map((b) => b.name)),
      currentBranch: local.find((b) => b.is_current)?.name ?? h.status.branch ?? null,
    };
  });

  // Decorations from `git log %D` can contain the same text for a local
  // origin/foo and the remote origin/foo. The refs endpoint already gives us
  // unambiguous type and tip SHA; index it once instead of guessing per row.
  const haveTypedRefs = $derived(typedRefs(h.refs));
  const branchChipsBySha = $derived.by(() => {
    const bySha = new Map<string, RefChip[]>();
    for (const b of [...(h.refs?.local ?? []), ...(h.refs?.remote ?? [])]) {
      if (!b.sha) continue;
      const chip: RefChip = {
        kind: b.remote ? 'remote' : b.is_current ? 'head' : 'local',
        label: b.name,
        current: b.is_current,
        worktree: !b.remote && worktreeBranches.has(b.name),
      };
      const list = bySha.get(b.sha) ?? [];
      list.push(chip);
      bySha.set(b.sha, list);
    }
    return bySha;
  });

  // Decoration strings from `git log %D` arrive as e.g. "HEAD -> main", "HEAD"
  // (detached), "origin/main", "main", "tag: v1.0". Each is classified into a
  // chip kind so the row instantly shows what's local vs remote vs the
  // checked-out branch. `RefsResp` (local/remote names) disambiguates a bare
  // name when the decoration alone is ambiguous.
  function classifyRef(ref: string): RefChip {
    // Stash ref (`refs/stash` for the top stash, or a `stash@{N}` selector) —
    // its own kind, never bucketed as a remote branch.
    if (ref === 'refs/stash' || /^stash@\{/.test(ref)) {
      return { kind: 'stash', label: 'stash', current: false };
    }
    if (isTagRef(ref)) return { kind: 'tag', label: refLabel(ref), current: false };
    // "HEAD -> branch": the checked-out branch — the most prominent chip.
    const arrow = ref.match(/^HEAD\s*->\s*(.+)$/);
    if (arrow) {
      const label = arrow[1].trim();
      return { kind: 'head', label, current: true, worktree: worktreeBranches.has(label) };
    }
    // A bare "HEAD" decoration = detached HEAD (no branch).
    if (ref === 'HEAD') return { kind: 'detached', label: 'HEAD', current: false };
    // Remote-tracking ref (origin/…): match the refs response, else fall back to
    // the "<remote>/<name>" shape (but never a purely-local name).
    const { remoteNames, localNames, currentBranch } = refKnowledge;
    if (remoteNames.has(ref) || (/^[^/]+\/.+/.test(ref) && !localNames.has(ref))) {
      return { kind: 'remote', label: ref, current: false };
    }
    // Otherwise a local branch; it's "current" if it's the checked-out branch.
    return { kind: 'local', label: ref, current: ref === currentBranch, worktree: worktreeBranches.has(ref) };
  }

  // Chips for one commit row, ordered head → local → remote → tag → stash for a
  // tidy column. `origin/HEAD` (and any `<remote>/HEAD` symbolic pointer) is
  // dropped — it's noise that never helps the reader.
  function chipsFor(c: CommitInfo): RefChip[] {
    const decorations = c.refs.filter((r) => !/\/HEAD$/.test(r)).map(classifyRef);
    const all = haveTypedRefs
      ? [...(branchChipsBySha.get(c.sha) ?? []), ...decorations.filter((chip) =>
          chip.kind === 'tag' || chip.kind === 'stash' || chip.kind === 'detached')]
      : decorations;
    // Collapse a local/head branch and its same-named remote twin (origin/<name>)
    // into ONE chip flagged `onRemote`: keeps the branch NAME, adds a small remote
    // glyph to show it's pushed, and frees a whole chip in the narrow column.
    const branchLabels = new Set(all.filter((x) => x.kind === 'local' || x.kind === 'head').map((x) => x.label));
    const merged: RefChip[] = [];
    for (const chip of all) {
      if (chip.kind === 'remote' && branchLabels.has(chip.label.replace(/^[^/]+\//, ''))) {
        continue; // its local twin carries the remote flag instead
      }
      if (chip.kind === 'local' || chip.kind === 'head') {
        const twin = all.some((o) => o.kind === 'remote' && o.label.replace(/^[^/]+\//, '') === chip.label);
        merged.push({ ...chip, onRemote: twin });
      } else {
        merged.push(chip);
      }
    }
    return merged.sort((a, b) => CHIP_ORDER[a.kind] - CHIP_ORDER[b.kind]);
  }

  // Branch name per commit sha, for lane labels (tooltips). Split out of the
  // layout so the layout depends on `commits` + THIS map only — refs,
  // worktrees or status changing without renaming any lane (the usual case:
  // an auto-fetch round, a checkout) no longer re-runs the whole O(N·L) pass.
  // Same precedence as chipsFor: the checked-out branch, then a local branch,
  // then a remote-tracking one.
  let prevLaneNames = new Map<string, string>();
  const laneNames = $derived.by((): Map<string, string> => {
    const m = new Map<string, string>();
    const r: RefsResp | null = h.refs;
    if (r && haveTypedRefs) {
      for (const b of r.local) if (b.sha && b.is_current) m.set(b.sha, b.name);
      for (const b of r.local) if (b.sha && !m.has(b.sha)) m.set(b.sha, b.name);
      for (const b of r.remote) if (b.sha && !m.has(b.sha)) m.set(b.sha, b.name);
    } else {
      // Daemon without typed refs: classify the log decorations.
      for (const c of h.commits) {
        if (c.refs.length === 0) continue;
        const chips = chipsFor(c);
        const name = (chips.find((chip) => chip.kind === 'head')
          ?? chips.find((chip) => chip.kind === 'local')
          ?? chips.find((chip) => chip.kind === 'remote'))?.label;
        if (name) m.set(c.sha, name);
      }
    }
    // Same content ⇒ same object, so the layout doesn't re-run.
    const prev = prevLaneNames;
    if (prev.size === m.size && [...m].every(([k, v]) => prev.get(k) === v)) return prev;
    prevLaneNames = m;
    return m;
  });

  const headSha = $derived(headShaOf(h.refs));
  /** A commit is the HEAD ("you are here"): the typed current branch's tip,
   *  else any HEAD-ish decoration. */
  function isHeadCommit(c: CommitInfo): boolean {
    const sha = headSha;
    if (sha) return c.sha === sha;
    return c.refs.some((r) => r === 'HEAD' || /^HEAD\s*->/.test(r));
  }

  const baseBranch = $derived.by((): string | null => {
    const r: RefsResp | null = h.refs;
    return r?.base_branch ?? null;
  });
  /** The integration branches (develop / main / the base…) already holding this
   *  branch's tip — what the row's "landed" mark names. A daemon predating
   *  `merged_into` only knows the base, so fall back to that single flag. */
  function landedIn(b: RefBranch): string[] {
    if (b.merged_into) return b.merged_into;
    if (!b.merged_into_base || !baseBranch) return [];
    if (b.remote) return [baseBranch]; // backend already excludes the base's remote twin
    return !b.is_current && b.name !== baseBranch ? [baseBranch] : [];
  }

  // ── Stash detection (in-graph dashed rendering) ───────────────────────────
  // From the stash list we know each stash's commit sha and its internal helper
  // commits (the index/untracked parents = parents[1..]; parents[0] is the base
  // commit = real history, left alone). Commits in the log matching either get
  // a dashed, de-emphasised "stash plumbing" treatment instead of a normal node.
  const stashShas = $derived(new Set(h.stashes.map((s) => s.sha)));
  const stashHelperShas = $derived.by(() => {
    const set = new Set<string>();
    for (const s of h.stashes) for (const p of s.parents.slice(1)) set.add(p);
    return set;
  });
  const stashMsgBySha = $derived.by(() => {
    const m = new Map<string, string>();
    for (const s of h.stashes) m.set(s.sha, stashShortMsg(s));
    return m;
  });
  // Fallback: the stash (WIP) commit also carries a `refs/stash` decoration in
  // the log, so the in-graph dashed treatment survives even if the /stashes
  // fetch failed (the two sources can't then visibly disagree on the entry node).
  const stashRefShas = $derived.by(() => {
    const set = new Set<string>();
    for (const c of h.commits) if (c.refs.includes('refs/stash')) set.add(c.sha);
    return set;
  });
  function isStashNode(sha: string): boolean {
    return stashShas.has(sha) || stashHelperShas.has(sha) || stashRefShas.has(sha);
  }
  /** The label a stash chip shows on a commit row or the detail header. */
  function stashLabel(sha: string): string {
    return stashMsgBySha.get(sha) ?? 'stash';
  }

  // ── First-parent indexes (the branch-line highlight walks them) ───────────
  const bySha = $derived.by(() => {
    const m = new Map<string, CommitInfo>();
    for (const c of h.commits) m.set(c.sha, c);
    return m;
  });
  // parentSha → child sha where the child's FIRST parent is parentSha. Commits
  // arrive newest-first, so the first match is the most-recent (tip-ward) child.
  const firstParentChild = $derived.by(() => {
    const m = new Map<string, string>();
    for (const c of h.commits) {
      const fp = c.parents[0];
      if (fp && !m.has(fp)) m.set(fp, c.sha);
    }
    return m;
  });

  /** The tip commit of a branch ref, if it's present in the loaded log — a local
   *  branch decorates its tip with `<name>` (or `HEAD -> <name>`); a remote ref
   *  decorates with its full `origin/<name>`. Used for single-click highlight. */
  function branchTipCommit(b: RefBranch): CommitInfo | null {
    for (const c of h.commits) {
      for (const r of c.refs) {
        if (r === b.name) return c;
        if (!b.remote) {
          const arrow = r.match(/^HEAD\s*->\s*(.+)$/);
          if (arrow && arrow[1].trim() === b.name) return c;
        }
      }
    }
    return null;
  }

  return {
    get worktreeByBranch() { return worktreeByBranch; },
    get worktreeBranches() { return worktreeBranches; },
    get localBranchByName() { return localBranchByName; },
    get refKnowledge() { return refKnowledge; },
    get laneNames() { return laneNames; },
    get headSha() { return headSha; },
    get baseBranch() { return baseBranch; },
    get bySha() { return bySha; },
    get firstParentChild() { return firstParentChild; },
    classifyRef,
    chipsFor,
    isHeadCommit,
    landedIn,
    isStashNode,
    stashLabel,
    branchTipCommit,
  };
}
