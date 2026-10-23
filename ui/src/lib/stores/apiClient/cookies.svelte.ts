// The API client's daemon-global cookie jar (list / clear) and the one-pass
// "secure all secrets" sweep. Split out of apiClient.svelte.ts (arch-6-03);
// the store forwards `cookies`, `loadCookies`, `clearCookies` and `secureAll`.

import { api } from '../../api/client';
import { toasts } from '../../toast.svelte';
import { loadErrorText } from '../../loadError';
import type { ApiClientStore } from '../apiClient.svelte';
import type { ApiCookie } from './model';

/** The store members this part reads and calls (its whole contract with the façade). */
export type CookiesHost = Pick<ApiClientStore, 'base' | 'loadRequests' | 'loadEnvironments'>;

export class ApiCookies {
  cookies: ApiCookie[] = $state([]);
  cookiesLoading = $state(false);
  cookiesError: string | null = $state(null);

  constructor(private readonly host: CookiesHost) {}

  async loadCookies(): Promise<void> {
    const base = this.host.base();
    if (!base) return;
    this.cookiesLoading = true;
    try {
      this.cookies = await api.get<ApiCookie[]>(`${base}/cookies`);
      this.cookiesError = null;
    } catch (e) {
      this.cookiesError = loadErrorText(e);
    } finally {
      this.cookiesLoading = false;
    }
  }

  /** The list empties only once the daemon cleared the jar; a failure keeps
   *  the rows (they are still there) and toasts why. */
  async clearCookies(): Promise<void> {
    const base = this.host.base();
    if (!base) return;
    try {
      await api.del(`${base}/cookies`);
      this.cookies = [];
      toasts.success('Cookies cleared');
    } catch (e) {
      toasts.error('Couldn’t clear cookies', loadErrorText(e));
    }
  }

  /** One-pass sweep moving every plaintext secret (request auth members +
   * secret-shaped environment variables) into the macOS Keychain. */
  async secureAll(): Promise<void> {
    const base = this.host.base();
    if (!base) return;
    try {
      const res = await api.post<{ requests_secured: number; env_keys_secured: number }>(
        `${base}/secure-all`, {},
      );
      toasts.success(
        'Secrets secured',
        `${res.requests_secured} request(s) · ${res.env_keys_secured} env value(s) moved to the Keychain`,
      );
      // Rows were rewritten (markers / stripped variables) — refresh both.
      void this.host.loadRequests();
      void this.host.loadEnvironments();
    } catch (e) {
      toasts.error('Couldn’t secure the secrets', loadErrorText(e));
    }
  }
}
