// Cached, root-only daemon facts the always-mounted chrome warns about: the
// secret store's mode (`GET /admin/secrets/status`) and what the network
// listener is actually bound to (`GET /security-posture`). Both handlers are
// root-only, so a non-root user never reads them (and sees no warning: they
// can't act on it).
//
// Never polled. The secrets read happens once per signed-in root identity
// and is refreshed by the pages that change it (Trust & safety ▸ Secret
// storage). The posture is re-read when the caller asks — the status bar asks
// whenever `/meta` is re-read, which the events socket does on every daemon
// restart (a new boot id), so "after restart" clears itself when the restart
// is actually observed.
import { api } from '../../lib/api/client';
import type { SecretsStatus, SecurityPostureResp } from '../../lib/api/types';
import { secretsUnencrypted } from './daemonIndicators';

class DaemonStatusStore {
  secrets: SecretsStatus | null = $state(null);
  posture: SecurityPostureResp | null = $state(null);
  /** User id the cached secrets read belongs to (plain field: not reactive). */
  private secretsFor: string | null = null;
  private postureTicket = 0;

  /** Root's secrets sit in the plaintext store: warn (status bar, nav dot). */
  get secretsUnencrypted(): boolean {
    return secretsUnencrypted(this.secrets);
  }

  /** Read the secret-store mode once per root identity; drop it for anyone
   *  else. Safe to call from several effects — the read is shared. */
  ensureSecrets(userId: string | null | undefined, isRoot: boolean): void {
    if (!userId || !isRoot) {
      this.secretsFor = null;
      // Write without reading: callers run this inside effects, and a read
      // would make the store's own state a dependency of theirs.
      this.secrets = null;
      return;
    }
    if (this.secretsFor === userId) return;
    this.secretsFor = userId;
    api
      .get<SecretsStatus>('/admin/secrets/status')
      .then((s) => {
        if (this.secretsFor === userId) this.secrets = s;
      })
      .catch(() => {
        // No warning on a failed read (Trust & safety shows the error inline);
        // the next identity change or page visit tries again.
        if (this.secretsFor === userId) this.secretsFor = null;
      });
  }

  /** The Secret storage card read (or changed) the status: share it. */
  setSecrets(s: SecretsStatus | null): void {
    this.secrets = s;
  }

  /** Re-read what the listener is bound to (root only; others get null). */
  refreshPosture(isRoot: boolean): void {
    const ticket = ++this.postureTicket;
    if (!isRoot) {
      this.posture = null;
      return;
    }
    api
      .get<SecurityPostureResp>('/security-posture')
      .then((p) => {
        if (ticket === this.postureTicket) this.posture = p;
      })
      .catch(() => {
        /* keep the last known posture; the next /meta re-read retries */
      });
  }
}

export const daemonStatus = new DaemonStatusStore();
