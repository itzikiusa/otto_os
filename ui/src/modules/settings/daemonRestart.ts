// "Restart daemon now" for Settings ▸ Daemon (and its save toast). It reuses
// the desktop supervisor's restart — the same Tauri `daemon_restart` command
// the offline screen (App.svelte) and the menu-bar popover (TrayPage) run via
// modules/desktop/daemonState (`launchctl kickstart -k`; invoked directly
// here, since one module may not reach into another's files) — so a
// listener change can take effect without quitting Otto. Only the desktop
// app can do it: in a browser (phone, the network listener) there is no
// supervisor, and the page says to restart Otto on the Mac instead.
import { confirmer } from '../../lib/confirm.svelte';
import { isTauri } from '../../lib/desktop';
import { auth } from '../../lib/stores/auth.svelte';
import { toasts } from '../../lib/toast.svelte';

/** This window can restart the daemon (desktop app only). */
export const canRestartDaemon = isTauri;

/** The confirm text: what a restart interrupts, and that it is recoverable. */
export const RESTART_CONFIRM_TEXT =
  'Otto restarts its background service (ottod) on this Mac. Running agent sessions are interrupted and can be ' +
  'resumed: an agent picks up its conversation when you reopen it, and sessions you started keep running when ' +
  '“Keep sessions running when the daemon restarts” is on. Workflow steps, reviews and channel replies in ' +
  'progress stop. This window reconnects by itself.';

let inFlight = false;

/** Ask, then restart the daemon and wait for it to answer again. Resolves
 *  true once the new daemon is up; false when cancelled or it failed (the
 *  failure is shown as a toast). */
export async function restartDaemonNow(): Promise<boolean> {
  if (!canRestartDaemon || inFlight) return false;
  const ok = await confirmer.ask(RESTART_CONFIRM_TEXT, {
    title: 'Restart the daemon?',
    confirmLabel: 'Restart daemon',
  });
  if (!ok) return false;
  inFlight = true;
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke<string>('daemon_restart');
    // The boot id changes, so the events socket re-reads /meta and the
    // status bar re-reads what the listener is bound to.
    await auth.boot(true);
    return true;
  } catch (e) {
    const raw = (e instanceof Error ? e.message : String(e ?? '')).trim();
    toasts.error('Couldn’t restart the daemon', raw || 'The daemon did not respond.');
    return false;
  } finally {
    inFlight = false;
  }
}
