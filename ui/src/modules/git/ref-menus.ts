// Pure helpers behind GraphView's ref sidebar rows and the commit-row ref
// menu (the "▾ +N" multi-ref popover): branch folder grouping, ref-chip
// types and ordering, and the middle-ellipsis split of long branch names.
// No component state — GraphView passes refs in and renders what comes back.
import type { RefBranch, RefsResp, StashInfo } from '../../lib/api/types';

// Group a flat branch list into top-level "folders" by first path segment, so
// feature/PROJ-1, feature/PROJ-2 nest under a single collapsible "feature" group.
// Remote refs are grouped by the path AFTER the remote name (origin/feature/X →
// folder "feature", leaf "X"); names with no slash stay loose at the top. The
// leaf `label` drops the folder prefix; `b.name` stays the full ref for actions.
export interface BranchLeaf {
  b: RefBranch;
  label: string;
}
export interface BranchFolder {
  name: string;
  leaves: BranchLeaf[];
}
/** `refs/remotes/origin/HEAD` is a symref, not a branch — but `for-each-ref
 *  %(refname:short)` shortens it to the bare remote name ("origin"), which
 *  slips past the daemon's `/HEAD` suffix filter and rendered as a stray
 *  "origin" leaf under REMOTE. Every real remote-tracking ref is
 *  `<remote>/<branch>`, so a slash-less remote name is always that symref. */
export function withoutRemoteHeads(r: RefsResp): RefsResp {
  return r.remote.some((b) => !b.name.includes('/'))
    ? { ...r, remote: r.remote.filter((b) => b.name.includes('/')) }
    : r;
}

export function groupBranches(list: RefBranch[]): { loose: BranchLeaf[]; folders: BranchFolder[] } {
  const loose: BranchLeaf[] = [];
  const folderMap = new Map<string, BranchLeaf[]>();
  for (const b of list) {
    // Display path: strip the leading remote name for remote refs.
    const display = b.remote ? b.name.replace(/^[^/]+\//, '') : b.name;
    const slash = display.indexOf('/');
    if (slash < 0) {
      loose.push({ b, label: display });
    } else {
      const folder = display.slice(0, slash);
      const leaf = display.slice(slash + 1);
      (folderMap.get(folder) ?? folderMap.set(folder, []).get(folder)!).push({ b, label: leaf });
    }
  }
  const folders = [...folderMap.entries()]
    .map(([name, leaves]) => ({ name, leaves }))
    .sort((a, b) => a.name.localeCompare(b.name));
  loose.sort((a, b) => a.label.localeCompare(b.label));
  return { loose, folders };
}

export function isTagRef(ref: string): boolean {
  return ref.startsWith('tag: ') || ref.startsWith('tag:');
}

export function refLabel(ref: string): string {
  return ref.replace(/^tag:\s*/, '');
}

export type ChipKind = 'head' | 'local' | 'remote' | 'tag' | 'detached' | 'stash';
export interface RefChip {
  kind: ChipKind;
  label: string; // text shown on the chip
  current: boolean; // the checked-out branch (most prominent)
  onRemote?: boolean; // local/head chip whose same-named origin twin lives on this commit
  worktree?: boolean; // branch checked out in a LINKED worktree (not this checkout)
}

// ── Multi-ref dropdown (collapse a commit row that carries 2+ refs) ──────────
// When a single commit holds several branches/tags the narrow column can't show
// them all, so we keep the most important one visible and tuck the rest behind a
// "▾ +N" expander that opens a grouped popover. Gated OFF stash/detached rows —
// those decorations are plumbing/state markers, not a list to collapse.
export function shouldCollapseRow(chips: RefChip[]): boolean {
  return chips.length >= 2 && chips.every((c) => c.kind !== 'stash' && c.kind !== 'detached');
}

// The one chip kept visible when a row collapses: checked-out branch first, then
// any local/head branch, then a remote, then a tag.
export function primaryChip(chips: RefChip[]): RefChip {
  return (
    chips.find((c) => c.current) ??
    chips.find((c) => c.kind === 'head' || c.kind === 'local') ??
    chips.find((c) => c.kind === 'remote') ??
    chips.find((c) => c.kind === 'tag') ??
    chips[0]
  );
}

// Split a row's chips into the popover's two groups — branches (current, then
// local/head, then remote) ahead of tags ("branch > tag").
export function splitChips(chips: RefChip[]): { branches: RefChip[]; tags: RefChip[] } {
  const rank = (c: RefChip): number =>
    c.current ? 0 : c.kind === 'head' || c.kind === 'local' ? 1 : 2;
  const branches = chips
    .filter((c) => c.kind === 'head' || c.kind === 'local' || c.kind === 'remote')
    .sort((a, b) => rank(a) - rank(b));
  const tags = chips.filter((c) => c.kind === 'tag');
  return { branches, tags };
}

// Middle-ellipsize a long branch leaf so the DISTINGUISHING SUFFIX stays visible
// (e.g. "PROJ-1234-really-long…-retry-fix"). Returns null for short names (render
// plain). The head span shrinks + ellipsizes; the tail span is pinned (see CSS).
//
// The tail starts at a WORD boundary (after a `-` `_` `.` `/`) so the pinned
// suffix reads as whole words — a fixed 14-char cut chopped mid-word
// ("…maintaining…nd-fixes-todos"). Falls back to the fixed cut when the name
// has no separator in range.
export function midParts(label: string): { head: string; tail: string } | null {
  if (label.length <= 18) return null;
  const maxTail = Math.min(16, Math.floor(label.length / 2));
  let cut = label.length - Math.min(14, maxTail);
  for (let i = label.length - maxTail; i <= label.length - 5; i++) {
    if ('-_./'.includes(label[i - 1] ?? '')) {
      cut = i;
      break;
    }
  }
  return { head: label.slice(0, cut), tail: label.slice(cut) };
}

/** Every branch carries its tip sha (a daemon predating `RefBranch.sha` does
 *  not): only then are the typed refs trusted over the log decorations. */
export function haveTypedRefs(r: RefsResp | null): boolean {
  return r !== null && [...r.local, ...r.remote].every((b) => !!b.sha);
}

/** HEAD's sha from the typed refs (the checked-out branch's tip), so a
 *  checkout moves the "you are here" marker without re-reading the log.
 *  `null` = unknown or detached — the log decorations decide then. */
export function headShaOf(r: RefsResp | null): string | null {
  if (!r || !haveTypedRefs(r)) return null;
  return r.local.find((b) => b.is_current)?.sha ?? null;
}

/** Strip the "WIP on <branch>: " / "On <branch>: " prefix for a compact label. */
export function stashShortMsg(s: StashInfo): string {
  const m = s.message.replace(/^(WIP on|On)\s+[^:]+:\s*/, '').trim();
  return m || s.message;
}

/** Strip trailing slashes so worktree paths match registered repo paths. */
export function normPath(p: string): string {
  return p.replace(/\/+$/, '');
}
