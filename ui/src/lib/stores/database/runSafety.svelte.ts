// Per-device safety preference for the query editor: confirm DELETE / UPDATE
// without WHERE, DROP, TRUNCATE and ALTER TABLE … DROP before they run on a
// connection that is NOT marked Prod / read-only (guarded connections keep
// their typed confirmation). On by default; stored like the other Database
// Explorer prefs (`dbPrefs`), so it survives reloads on this device only.
import { loadFlag, saveFlag } from '../dbPrefs.svelte';

export const CONFIRM_DESTRUCTIVE_KEY = 'otto_db_confirm_destructive';

class RunSafety {
  /** Ask before destructive statements on every (unguarded) connection. */
  confirmDestructive: boolean = $state(loadFlag(CONFIRM_DESTRUCTIVE_KEY, true));

  setConfirmDestructive(on: boolean): void {
    this.confirmDestructive = on;
    saveFlag(CONFIRM_DESTRUCTIVE_KEY, on);
  }
}

export const runSafety = new RunSafety();
