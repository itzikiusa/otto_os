// The run-configuration editor's footer on a phone (review I4-11): one row — the
// primary Save plus one ⋯ holding the rest. Five buttons wrapped into three rows
// and took ~140 px of a 740 px screen. Cancel is gone there: the sheet's ✕ closes
// (and asks about unsaved edits the same way).

import type { MenuItem } from '../../../lib/contextmenu.svelte';

export interface RunConfigFooterMenuOpts {
  /** A save is in flight: every row is disabled. */
  busy: boolean;
  /** The draft is a saved configuration (not new, not auto-detected): Delete applies. */
  deletable: boolean;
  /** The draft has a name to copy. */
  canDuplicate: boolean;
  onSave: (after: 'run' | 'debug') => void;
  onDuplicate: () => void;
  onDelete: () => void;
}

/** Pure: the ⋯ rows beside the phone footer's Save. */
export function runConfigFooterMenu(o: RunConfigFooterMenuOpts): MenuItem[] {
  const rows: MenuItem[] = [
    { label: 'Save & run', icon: 'play', disabled: o.busy, action: () => o.onSave('run') },
    { label: 'Save & debug', icon: 'bug', disabled: o.busy, action: () => o.onSave('debug') },
  ];
  if (o.canDuplicate) rows.push({ label: 'Duplicate', icon: 'copy', disabled: o.busy, action: o.onDuplicate });
  if (o.deletable) {
    rows.push({ separator: true });
    rows.push({ label: 'Delete…', icon: 'trash', danger: true, disabled: o.busy, action: o.onDelete });
  }
  return rows;
}
