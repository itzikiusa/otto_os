// "Continue on phone" requests: any pane header / ⋯ menu asks to show the
// QR sheet for a route; one host (PhoneHandoffHost, mounted by the shell)
// renders it. Hidden on a paired phone — it is already the phone.
import type { MenuItem } from '../contextmenu.svelte';
import { phoneDevice } from './phoneDevice.svelte';

export interface PhoneHandoffRequest {
  /** `agents/<id>` or `threads/<id>` (validated again by the sheet). */
  route: string;
  /** "this session" / "this thread" — the sheet's copy. */
  what: string;
}

class PhoneHandoffStore {
  request: PhoneHandoffRequest | null = $state(null);

  open(route: string, what: string): void {
    this.request = { route, what };
  }

  close(): void {
    this.request = null;
  }

  /** The ⋯ menu row, or nothing on a paired phone. */
  menuItem(route: string, what: string): MenuItem[] {
    void phoneDevice.ensure();
    if (phoneDevice.isPairedPhone) return [];
    return [{ label: 'Continue on phone…', icon: 'phone', action: () => this.open(route, what) }];
  }
}

export const phoneHandoff = new PhoneHandoffStore();
