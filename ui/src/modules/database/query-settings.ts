// The query editor's "Query settings" gear: row limit, statement timeout and
// masking for the active tab, as ONE menu (they used to be three always-visible
// controls on the toolbar). QueryEditor shows a dot on the gear while any value
// is not the default, and keeps a "Masked" chip — a safety state is never only
// a dot.
import { confirmer } from '../../lib/confirm.svelte';
import type { MenuItem } from '../../lib/contextmenu.svelte';
import { unpersistEditorState } from '../../lib/editor-history';
import { database, DEFAULT_ROW_LIMIT, ROW_LIMIT_ALL, TAB_HISTORY_PREFIX } from '../../lib/stores/database.svelte';
import { runSafety } from '../../lib/stores/database/runSafety.svelte';

// The editor's run path imports its pre-run confirms from here, next to the
// setting that turns the destructive-statement one off.
export { confirmRun } from './destructive-gate';

/** Flip the destructive-statement confirm. Turning a safety net OFF asks once
 *  (a stray click in a menu must not silently disarm it); turning it on doesn't. */
export async function toggleConfirmDestructive(): Promise<void> {
  if (!runSafety.confirmDestructive) return runSafety.setConfirmDestructive(true);
  const ok = await confirmer.ask(
    'DELETE or UPDATE without WHERE, DROP, TRUNCATE and ALTER TABLE … DROP will run on the first ⌘↵ on every connection not marked Prod or read-only (Prod keeps its typed confirmation; Read-only refuses writes). This applies on this device only.',
    { title: 'Stop confirming destructive statements?', confirmLabel: 'Turn off', cancelLabel: 'Keep confirming', danger: true },
  );
  if (ok) runSafety.setConfirmDestructive(false);
}

/** Default row-cap options (applied when a statement has no explicit LIMIT). */
export const ROW_LIMIT_OPTS: { label: string; value: number }[] = [
  { label: '100', value: 100 },
  { label: '500', value: 500 },
  { label: '1,000', value: 1000 },
  { label: '5,000', value: 5000 },
  { label: '10,000', value: 10000 },
  { label: '50,000', value: 50000 },
  // Unbounded rows, still bounded by the daemon's 32 MB response budget.
  { label: 'All (≤ 32 MB)', value: ROW_LIMIT_ALL },
];

export function setMask(on: boolean): void {
  database.tab.mask = on;
  // A masked tab's undo history (every pasted string) must not stay on disk.
  if (on) unpersistEditorState(TAB_HISTORY_PREFIX + database.tab.uid);
}

export function fmtTimeout(ms: number): string {
  return ms % 1000 === 0 ? `${ms / 1000}s` : `${ms} ms`;
}

export async function editTimeout(): Promise<void> {
  const t = database.tab;
  const v = await confirmer.promptText('Stop a statement that runs longer than this (milliseconds). Leave blank for no limit.', {
    title: 'Statement timeout',
    confirmLabel: 'Set timeout',
    initial: t.timeout_ms ? String(t.timeout_ms) : '',
    placeholder: 'e.g. 30000',
    validate: (raw) => (raw.trim() === '' || /^\d+$/.test(raw.trim()) ? null : 'Enter a whole number of milliseconds'),
  });
  if (v === null) return;
  const n = Number(v.trim());
  t.timeout_ms = n > 0 ? n : null;
}

/** "limit 500 · timeout 30s · masked" — empty when every setting is default. */
export function settingsSummary(rowLimit: number, tab: { timeout_ms?: number | null; mask?: boolean }): string {
  const limit = ROW_LIMIT_OPTS.find((o) => o.value === rowLimit)?.label ?? String(rowLimit);
  return [
    rowLimit !== DEFAULT_ROW_LIMIT ? `limit ${limit}` : '',
    tab.timeout_ms ? `timeout ${fmtTimeout(tab.timeout_ms)}` : '',
    tab.mask ? 'masked' : '',
  ]
    .filter(Boolean)
    .join(' · ');
}

export function settingsMenuItems(): MenuItem[] {
  const tab = database.tab;
  return [
    { label: 'Row limit — an explicit LIMIT in the query overrides it', disabled: true },
    ...ROW_LIMIT_OPTS.map((o) => ({
      label: o.value === DEFAULT_ROW_LIMIT ? `${o.label} rows (default)` : `${o.label}${o.value === ROW_LIMIT_ALL ? '' : ' rows'}`,
      checked: database.rowLimit === o.value,
      action: () => database.setRowLimit(o.value),
    })),
    { separator: true },
    { label: 'Statement timeout…', icon: 'clock', hint: tab.timeout_ms ? fmtTimeout(tab.timeout_ms) : 'none', action: () => void editTimeout() },
    {
      label: 'Mask sensitive values',
      checked: !!tab.mask,
      title: 'The server redacts emails, tokens and keys before results leave the daemon',
      action: () => setMask(!tab.mask),
    },
    {
      label: 'Confirm destructive statements (DELETE/UPDATE without WHERE, DROP, TRUNCATE)',
      checked: runSafety.confirmDestructive,
      title: 'Safety: ask before DELETE / UPDATE without WHERE, DROP, TRUNCATE or ALTER TABLE … DROP on any connection (this device). Prod connections always take the typed confirm; Read-only ones refuse writes.',
      action: () => void toggleConfirmDestructive(),
    },
  ];
}
