// The local native side-pane transport. Native commands derive the owner from
// the invoking webview; no caller can choose a target window or child label.
import { isTauri, isNativePane } from './desktop';
import type { GuestMsg, HostMsg } from './sidePane';
import { mayRestorePaneFocus } from './nativePanePolicy';
import type { PaneBounds } from './nativePanePolicy';
export type { PaneBounds } from './nativePanePolicy';
export type PaneMode = 'attached' | 'side' | 'primary';
export interface PaneState {
  host: string;
  child: string;
  mode: PaneMode;
  alwaysOnTop: boolean;
  fullscreen: boolean;
  visible: boolean;
}
export interface PaneMonitor { id: number; name: string; current: boolean }
export type PaneWindowAction = 'toggle-top' | 'toggle-fullscreen' | 'maximize' | 'move-display';
export const nativePaneAvailable = isTauri;

let snapshot: PaneState | null = null;
const observers = new Set<(state: PaneState | null) => void>();
export function nativePaneSnapshot(): PaneState | null { return snapshot; }
function publish(state: PaneState | null): void {
  const visibilityChanged = snapshot?.visible !== state?.visible;
  snapshot = state;
  for (const observer of observers) observer(state);
  // WebKit reports visible even for a hidden native child. Reuse the existing
  // presence trigger, with uiCommands consulting this authoritative state.
  if (isNativePane && visibilityChanged) document.dispatchEvent(new Event('visibilitychange'));
}
async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const api = await import('@tauri-apps/api/core');
  return api.invoke<T>(command, args);
}
// Serialise destructive/lifecycle work, including component teardown followed
// by a quick reopen. Relay messages must bypass this queue (ready during open).
let tail: Promise<unknown> = Promise.resolve();
function enqueue<T>(task: () => Promise<T>): Promise<T> {
  const result = tail.then(task);
  tail = result.catch(() => {});
  return result;
}
function mutate<T>(command: string, args?: Record<string, unknown>): Promise<T> { return enqueue(() => invoke<T>(command, args)); }
function stateCommand(command: string, args?: Record<string, unknown>): Promise<PaneState> {
  return enqueue(async () => {
    const state = await invoke<PaneState>(command, args);
    publish(state);
    return state;
  });
}
let listening: Promise<void> | null = null;
export function startNativePaneState(): Promise<void> {
  if (!isTauri) return Promise.resolve();
  if (!listening) listening = (async () => {
    await listenNative<PaneState | null>('otto://pane-state', publish);
    publish(await invoke<PaneState | null>('pane_state'));
  })().catch((error) => { listening = null; throw error; });
  return listening;
}
export function observeNativePane(observer: (state: PaneState | null) => void): () => void {
  observers.add(observer);
  observer(snapshot);
  return () => { observers.delete(observer); };
}
async function listenNative<T>(event: string, receive: (message: T) => void): Promise<() => void> {
  const { getCurrentWebview } = await import('@tauri-apps/api/webview');
  return getCurrentWebview().listen<T>(event, (e) => receive(e.payload));
}
export const nativePane = {
  open: (route: string, bounds: PaneBounds) => stateCommand('pane_open', { route, bounds }),
  layout: (bounds: PaneBounds, visible: boolean) => mutate<void>('pane_layout', { bounds, visible }),
  close: () => enqueue(async () => { await invoke<void>('pane_close'); publish(null); }),
  detach: (pane: 'side' | 'primary') => stateCommand('pane_detach', { pane }),
  return: () => stateCommand('pane_return'),
  focus: (pane: 'side' | 'primary') => mutate<void>('pane_focus', { pane }),
  restoreAttachedFocus: (allowed: () => boolean) => enqueue(async () => {
    if (mayRestorePaneFocus(snapshot?.mode, snapshot?.visible === true, !allowed())) {
      await invoke<void>('pane_focus', { pane: 'side' });
    }
  }),
  action: (action: PaneWindowAction, monitor?: number) => stateCommand('pane_window_action', { action, ...(monitor === undefined ? {} : { monitor }) }),
  monitors: () => invoke<PaneMonitor[]>('pane_monitors'),
  toHost: (message: GuestMsg) => invoke<void>('pane_to_host', { message }),
  toGuest: (message: HostMsg) => invoke<void>('pane_to_guest', { message }),
  onHost: (receive: (message: unknown) => void) => listenNative('otto://pane-to-host', receive),
  onGuest: (receive: (message: unknown) => void) => listenNative('otto://pane-to-guest', receive),
};
