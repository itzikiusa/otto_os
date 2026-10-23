// The collapsed-row ref popover's controller (arch-1-09: split out of
// GraphView.svelte): which commit's refs are listed and where the popover is
// anchored, its viewport clamp, and the row gestures — click selects the
// commit, double-click checks the ref out (or opens its worktree), right-click
// opens the full branch/tag menu. RefPopover.svelte renders it.

import type { CommitInfo, RefBranch, RefTag, RefsResp, WorktreeInfo } from '../../../lib/api/types';
import { splitChips, type RefChip } from '../ref-menus';

export interface RefMenu {
  commit: CommitInfo;
  branches: RefChip[];
  tags: RefChip[];
  /** Anchor in viewport coords (position:fixed clears the scroll container). */
  x: number;
  y: number;
}

export interface RefPopoverHost {
  readonly refs: RefsResp | null;
  readonly selectedSha: string | null;
  readonly worktreeByBranch: Map<string, WorktreeInfo>;
  selectCommit(c: CommitInfo, opts: { deferDiff?: boolean }): void;
  holdDiff(): void;
  checkout(name: string, create: boolean): Promise<void> | void;
  checkoutRemoteRef(label: string): Promise<void> | void;
  openWorktree(w: WorktreeInfo): Promise<void> | void;
  branchMenu(e: MouseEvent, b: RefBranch): void;
  tagMenu(e: MouseEvent, t: RefTag): void;
}

export type RefPopover = ReturnType<typeof createRefPopover>;

/** Gap kept between the popover and the window edge. */
const PAD = 8;

/** Height cap measured against the window (never a fixed 60vh): the room left
 *  under the anchor, but at least ~a third of the window so a popover near the
 *  bottom edge flips up into the clamp instead of shrinking to nothing. */
export function popoverMaxHeight(innerH: number, anchorY: number): number {
  return Math.max(Math.round(innerH / 3), Math.min(innerH - 16, innerH - anchorY - 8 + 160), 160);
}

/** Clamp a measured popover into the viewport (mirrors ContextMenu): an anchor
 *  near the bottom/right edge would otherwise push part of the (internally
 *  scrollable) popover off-screen and out of reach. Never a flip without a
 *  floor — the top-left corner stays at least PAD inside the window. */
export function clampPopover(
  anchor: { x: number; y: number },
  size: { w: number; h: number },
  view: { w: number; h: number },
): { x: number; y: number } {
  return {
    x: Math.max(PAD, Math.min(anchor.x, view.w - size.w - PAD)),
    y: Math.max(PAD, Math.min(anchor.y, view.h - size.h - PAD)),
  };
}

export function createRefPopover(h: RefPopoverHost) {
  let menu = $state<RefMenu | null>(null);

  /** The "▾ +N" expander on a collapsed commit row: list that row's refs. */
  function open(e: MouseEvent, commit: CommitInfo, chips: RefChip[]): void {
    e.stopPropagation(); // don't also select the commit (the row is a button)
    e.preventDefault();
    const { branches, tags } = splitChips(chips);
    const trigger = e.currentTarget as HTMLElement;
    trigger.focus(); // WebKit pointer clicks do not focus buttons automatically.
    const r = trigger.getBoundingClientRect();
    menu = { commit, branches, tags, x: r.left, y: r.bottom + 4 };
  }

  function close(): void {
    menu = null;
  }

  // Single-click a row → select the commit (this lights up the branch's spine).
  // Idempotent and harmless as the lead click of a double-click, which is what
  // actually checks out.
  function rowSelect(commit: CommitInfo): void {
    if (h.selectedSha !== commit.sha) h.selectCommit(commit, { deferDiff: true });
  }

  // Double-click a row → check it out (or open its worktree). Remote chips
  // check out a local tracking branch; tags check out detached; worktree
  // branches open the linked tree as a git tab.
  function rowCheckout(chip: RefChip): void {
    // Keep the explicit local/remote identity; their display names may match.
    close();
    h.holdDiff();
    if (chip.kind === 'remote') {
      void h.checkoutRemoteRef(chip.label);
      return;
    }
    if (chip.worktree) {
      const wt = h.worktreeByBranch.get(chip.label);
      if (wt) {
        void h.openWorktree(wt);
        return;
      }
    }
    // local/head branch, or a tag (detached) — both check out by their label.
    void h.checkout(chip.label, false);
  }

  // Right-click a row → the full branch/tag context menu (rename, merge, PR,
  // delete…), resolved from the refs response. Silent no-op if the ref can't
  // be matched (e.g. refs changed underneath the open popover).
  function rowMenu(e: MouseEvent, chip: RefChip): void {
    e.preventDefault();
    e.stopPropagation();
    const r: RefsResp | null = h.refs;
    if (!r) return;
    if (chip.kind === 'tag') {
      const t = r.tags.find((x) => x.name === chip.label);
      if (t) h.tagMenu(e, t);
      return;
    }
    const b = (chip.kind === 'remote' ? r.remote : r.local).find((x) => x.name === chip.label);
    if (b) h.branchMenu(e, b);
  }

  return {
    get menu() { return menu; },
    open,
    close,
    rowSelect,
    rowCheckout,
    rowMenu,
  };
}
