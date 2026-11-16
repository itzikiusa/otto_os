// Choosing what Run launches on a phone (review I4-12). The stacked header shows
// only Run; the workbench's launch-target selector (`IdeRunToolbar` →
// `IdeRunConfigPopover`) is not on screen there. The phone ⋯ gets one
// "Run target: <name>" row whose submenu lists the same targets in the same
// order — Current file, saved, auto-detected (the active file's ecosystem first)
// — and a pick sets the same store field and launch memory the popover does.

import type { MenuItem } from '../../../lib/contextmenu.svelte';
import { ide } from '../ideStore.svelte';
import {
  CURRENT_FILE_CONFIG_ID,
  currentFileRunnable,
  launchSelectorLabel,
  orderDetectedForLanguage,
  saveLaunchMemory,
} from '../store/ideLaunchTarget';

export interface RunTargetConfig {
  id: string;
  name: string;
  auto_detected?: boolean | null;
}

export interface RunTargetRowsInput {
  configs: readonly RunTargetConfig[];
  /** `ide.selectedConfigId`; no match = Current file (as the toolbar reads it). */
  selectedId: string | null | undefined;
  /** The active tab's name, or null with no file open. */
  fileName: string | null;
  /** Whether the active file can run, and why not. */
  currentFile: { ok: boolean; reason: string | null };
}

export interface RunTargetRow {
  id: string;
  label: string;
  checked: boolean;
  disabled: boolean;
  title?: string;
  /** The first row of a section (a separator goes above it). */
  sectionStart: boolean;
}

/** Pure: the run-target rows, in the popover's order. `configs` must already be
 *  ordered (auto-detected by the active file's language). */
export function runTargetRows(input: RunTargetRowsInput): RunTargetRow[] {
  const selected = input.configs.some((c) => c.id === input.selectedId) ? input.selectedId : null;
  const rows: RunTargetRow[] = [{
    id: CURRENT_FILE_CONFIG_ID,
    label: input.fileName ? `Current file — ${input.fileName}` : 'Current file',
    checked: !selected,
    disabled: !input.currentFile.ok,
    title: input.currentFile.ok ? undefined : (input.currentFile.reason ?? undefined),
    sectionStart: false,
  }];
  const saved = input.configs.filter((c) => !c.auto_detected);
  const detected = input.configs.filter((c) => Boolean(c.auto_detected));
  for (const group of [saved, detected]) {
    group.forEach((c, i) => rows.push({ id: c.id, label: c.name, checked: c.id === selected, disabled: false, sectionStart: i === 0 }));
  }
  return rows;
}

/** The ⋯ row's text: "Run target: <what Run launches>". */
export function runTargetRowLabel(): string {
  const selected = ide.runConfigs.find((c) => c.id === ide.selectedConfigId) ?? null;
  return `Run target: ${launchSelectorLabel(selected, ide.activeTab)}`;
}

/** Same as the popover's pick: the store field plus the per-project memory. */
export function selectRunTarget(id: string): void {
  ide.selectedConfigId = id;
  saveLaunchMemory(ide.target, { selected: id });
}

/** The submenu under "Run target", from the live store. */
export function runTargetMenuItems(): MenuItem[] {
  const saved = ide.runConfigs.filter((c) => !c.auto_detected);
  const detected = orderDetectedForLanguage(ide.runConfigs.filter((c) => Boolean(c.auto_detected)), ide.activeTab?.language);
  const rows = runTargetRows({
    configs: [...saved, ...detected],
    selectedId: ide.selectedConfigId,
    fileName: ide.activeTab?.name ?? null,
    currentFile: currentFileRunnable(ide.activeTab),
  });
  const out: MenuItem[] = [];
  for (const r of rows) {
    if (r.sectionStart) out.push({ separator: true });
    out.push({ label: r.label, checked: r.checked, disabled: r.disabled, title: r.title, action: () => selectRunTarget(r.id) });
  }
  out.push({ separator: true }, { label: 'Edit run configurations…', icon: 'gear', action: () => ide.openEnvironmentsModal('configs') });
  return out;
}
