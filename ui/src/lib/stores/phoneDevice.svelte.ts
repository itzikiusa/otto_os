// Is THIS window a paired phone (signed in with a device token)? Asked once
// per signed-in identity (GET /phone/device/me). A paired phone hides the
// owner-only Phone-access controls (it can't pair, approve, or change
// serving — the daemon refuses those too) and the "Continue on phone" action.
import { phoneApi } from '../api/phone';
import type { PhoneDevice } from '../api/types';

class PhoneDeviceStore {
  /** The device this window is signed in as, or null on a desktop credential. */
  device: PhoneDevice | null = $state(null);
  /** True once the answer arrived (or the read failed: treated as desktop). */
  known = $state(false);
  private inflight: Promise<void> | null = null;

  get isPairedPhone(): boolean {
    return this.device !== null;
  }

  ensure(): Promise<void> {
    if (this.known) return Promise.resolve();
    this.inflight ??= phoneApi
      .me()
      .then((r) => {
        this.device = r.device ?? null;
      })
      .catch(() => {
        this.device = null;
      })
      .finally(() => {
        this.known = true;
        this.inflight = null;
      });
    return this.inflight;
  }

  reset(): void {
    this.device = null;
    this.known = false;
  }
}

export const phoneDevice = new PhoneDeviceStore();
