// The desktop supervisor's daemon diagnosis (error-handling-5-01). The Tauri
// shell installs/starts ottod under launchd and publishes what it found as
// `otto://daemon-state` (apps/desktop/src-tauri/src/supervisor.rs
// `DaemonReport`): "loaded but not answering", "install failed: launchctl
// bootstrap …", and so on. The offline screen and the menu-bar popover show
// that detail and offer the recovery the supervisor can do — Restart daemon
// (`launchctl kickstart -k`, or a reinstall when the job isn't loaded) and
// Reinstall after a failed install.
//
// Self-contained (no relative imports) so `node --test` can load it. Every
// call is a no-op outside the Tauri shell.

/** Mirrors `supervisor::DaemonReport`. */
export interface DaemonReport {
  healthy: boolean;
  installed: boolean;
  daemon_version: string | null;
  app_version: string;
  detail: string;
}

const inTauri = (): boolean =>
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** The supervisor's own diagnosis worth showing while the daemon is down
 *  (`null` when healthy or when it said nothing). */
export function offlineDetail(report: DaemonReport | null): string | null {
  if (!report || report.healthy) return null;
  const detail = report.detail.trim();
  return detail || null;
}

/** The last install/bootstrap attempt failed: offer Reinstall. */
export function installFailed(report: DaemonReport | null): boolean {
  if (!report || report.healthy) return false;
  return /^install (failed|panicked)\b/.test(report.detail.trim());
}

/** A failed restart/install as one readable line. */
export function supervisorErrorText(e: unknown): string {
  const raw = (e instanceof Error ? e.message : String(e ?? '')).trim();
  return raw || 'the daemon did not respond';
}

async function invoke<T>(cmd: string): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<T>(cmd);
}

export const daemonSupervisor = {
  /** Subscribe to every published report (app-wide event). */
  async onState(cb: (report: DaemonReport) => void): Promise<() => void> {
    if (!inTauri()) return () => {};
    const { listen } = await import('@tauri-apps/api/event');
    return listen<DaemonReport>('otto://daemon-state', (e) => cb(e.payload));
  },
  /** Health now, plus the last diagnosis while the daemon is down. */
  status: (): Promise<DaemonReport | null> =>
    inTauri() ? invoke<DaemonReport>('daemon_status') : Promise.resolve(null),
  /** Install + start (single-flight in the shell); reports are published. */
  start: (): Promise<DaemonReport | null> =>
    inTauri() ? invoke<DaemonReport>('daemon_start') : Promise.resolve(null),
  /** Replace a wedged daemon (`kickstart -k`; reinstalls an unloaded job). */
  restart: (): Promise<string | null> =>
    inTauri() ? invoke<string>('daemon_restart') : Promise.resolve(null),
  /** Rewrite the launchd plist and bootstrap again. */
  reinstall: (): Promise<string | null> =>
    inTauri() ? invoke<string>('daemon_install') : Promise.resolve(null),
};
