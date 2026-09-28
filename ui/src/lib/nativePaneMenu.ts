// Shared native window actions for either pane's toolbar and the host palette.
import { ctxMenu, type MenuItem } from './contextmenu.svelte';
import { nativePane, nativePaneSnapshot, type PaneWindowAction } from './nativePane';
import { toasts } from './toast.svelte';

let pending = false;
export function runPaneWindowAction(action: () => Promise<unknown>): void {
  if (pending) return;
  pending = true;
  void action().catch(() => {
    toasts.error('Couldn’t change the pane window', 'Try again. Your work remains in its current pane.');
  }).finally(() => { pending = false; });
}
export function paneWindowAction(action: PaneWindowAction, monitor?: number): void {
  runPaneWindowAction(() => nativePane.action(action, monitor));
}
export async function showPaneDisplays(anchor?: Element | null): Promise<void> {
  try {
    const monitors = await nativePane.monitors();
    ctxMenu.showAt(anchor, monitors.length ? monitors.map((monitor) => ({
      label: monitor.name, checked: monitor.current,
      action: () => paneWindowAction('move-display', monitor.id),
    })) : [{ label: 'No displays available', disabled: true }], { filter: true, filterPlaceholder: 'Find display…', maxVisible: 12 });
  } catch {
    toasts.error('Couldn’t list displays', 'Try opening the display menu again.');
  }
}
export function paneWindowMenuItems(): MenuItem[] {
  const state = nativePaneSnapshot();
  if (!state) return [];
  if (state.mode === 'attached') return [
    { label: 'Detach main pane', icon: 'external', disabled: pending, action: () => runPaneWindowAction(() => nativePane.detach('primary')) },
    { label: 'Detach side pane', icon: 'external', disabled: pending, action: () => runPaneWindowAction(() => nativePane.detach('side')) },
  ];
  return [
    { label: 'Return to split', icon: 'columns', disabled: pending, action: () => runPaneWindowAction(() => nativePane.return()) },
    { separator: true },
    { label: 'Keep on top', checked: state.alwaysOnTop, disabled: pending, action: () => paneWindowAction('toggle-top') },
    { label: 'Fill display', icon: 'maximize', disabled: pending, action: () => paneWindowAction('maximize') },
    { label: state.fullscreen ? 'Exit fullscreen' : 'Enter fullscreen', icon: 'maximize', disabled: pending, action: () => paneWindowAction('toggle-fullscreen') },
    { label: 'Move to display…', icon: 'external', disabled: pending, action: () => { void showPaneDisplays(); } },
  ];
}
