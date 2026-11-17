// How the comparison / Local History modal lays out (review I4-09). On the
// workbench it is the desktop split: a resizable snapshot list beside the diff.
// In the IDE's stacked (phone) layout a 290 px list left the diff an 80 px
// sliver, so there it is list → detail: the diff fills the sheet, and the
// snapshot list is a step of its own reached by "‹ Snapshots".

export type IdeCompareStep = 'list' | 'diff';

export interface IdeCompareLayout {
  /** The snapshot / revision / branch list is on screen. */
  showList: boolean;
  /** The diff pane is on screen. */
  showDiff: boolean;
  /** The list and the diff sit side by side with a drag divider between. */
  split: boolean;
  /** The diff shows a "‹ Snapshots" (or Revisions / Branches) way back. */
  showBack: boolean;
  /** The diff is forced to the unified view (side by side cannot fit). */
  unifiedOnly: boolean;
  /** The modal footer is dropped: ✕ closes, the one action sits in the diff's bar. */
  footer: boolean;
}

/** Pure: the layout for the IDE's stacked flag, whether this mode has a list,
 *  and which step a stacked sheet is on. */
export function ideCompareLayout(stack: boolean, hasList: boolean, step: IdeCompareStep): IdeCompareLayout {
  if (!stack) {
    return { showList: hasList, showDiff: true, split: hasList, showBack: false, unifiedOnly: false, footer: true };
  }
  const listStep = hasList && step === 'list';
  return {
    showList: listStep,
    showDiff: !listStep,
    split: false,
    showBack: hasList && !listStep,
    unifiedOnly: true,
    footer: false,
  };
}

/** Pure: the stacked step after a pick in the list — the diff of what was picked. */
export function ideCompareStepAfterPick(stack: boolean, step: IdeCompareStep): IdeCompareStep {
  return stack ? 'diff' : step;
}

/** The back control's words: what the list holds in this mode. */
export function ideCompareListName(mode: string): string {
  if (mode === 'git-branch') return 'Branches';
  if (mode === 'git-revision') return 'Revisions';
  return 'Snapshots';
}

/** The modal title: a phone sheet names the file only (the path wrapped to two
 *  lines at 375 px), the workbench keeps the project-relative path. A caller's
 *  left label ("On disk") replaces the clipboard wording. */
export function ideCompareTitle(
  mode: string,
  fileName: string,
  displayPath: string,
  leftLabel: string | null,
  stack: boolean,
): string {
  if (mode === 'local-history') return `Local History — ${stack ? fileName : displayPath}`;
  if (mode === 'clipboard') return leftLabel && leftLabel !== 'Clipboard' ? `${leftLabel} vs current — ${fileName}` : `Compare with Clipboard — ${fileName}`;
  if (mode === 'git-branch') return `Compare with Branch — ${fileName}`;
  if (mode === 'file') return `Compare Files — ${fileName}`;
  return `Compare Revisions — ${fileName}`;
}
