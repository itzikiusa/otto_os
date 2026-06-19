// Auth / boot state: GET /meta → onboarding | login | ready.

import { api, setToken, getToken, ApiError } from '../api/client';
import type { CapabilitiesResp, LoginResp, MeResp, MetaResp, User } from '../api/types';
import type { Capability, Feature } from '../api/types';

export type BootPhase = 'loading' | 'onboarding' | 'login' | 'ready' | 'offline';

/** localStorage key used to persist the admin's own token across page reloads
 *  while an impersonation session is active. Cleared on `stopImpersonating`. */
const ADMIN_TOKEN_KEY = 'otto_admin_token';

// Capability ladder: index = strength (higher = more permissive).
const CAP_ORDER: Capability[] = ['none', 'view', 'edit', 'admin'];

function capIndex(c: string): number {
  const i = CAP_ORDER.indexOf(c as Capability);
  return i < 0 ? 0 : i;
}

class AuthStore {
  phase: BootPhase = $state('loading');
  meta: MetaResp | null = $state(null);
  /** Effective (acted-as) user — the identity the UI renders as. */
  me: User | null = $state(null);
  /** Real token owner. Equals `me` for a normal session. */
  realUser: User | null = $state(null);
  /** Effective capabilities map — feature → capability string. Populated after boot. */
  capabilities: Record<string, string> = $state({});

  /** True when the active bearer is an impersonation token. */
  get isImpersonating(): boolean {
    return !!(this.realUser && this.me && this.realUser.id !== this.me.id);
  }

  get isRoot(): boolean {
    return this.me?.is_root ?? false;
  }

  /**
   * Returns true when the current user has at least `required` on `feature`.
   * Root always returns true. Non-authenticated users always return false.
   */
  can(feature: Feature, required: Capability): boolean {
    if (!this.me) return false;
    if (this.me.is_root) return true;
    const granted = this.capabilities[feature] ?? 'none';
    return capIndex(granted) >= capIndex(required);
  }

  /** Re-fetch /meta without touching the boot phase (e.g. after provider changes). */
  async refreshMeta(): Promise<void> {
    try {
      this.meta = await api.get<MetaResp>('/meta');
    } catch {
      // non-fatal: keep the stale meta
    }
  }

  /** Fetch the caller's effective capabilities from /auth/capabilities. */
  private async loadCapabilities(): Promise<void> {
    try {
      const resp = await api.get<CapabilitiesResp>('/auth/capabilities');
      this.capabilities = resp.capabilities;
    } catch {
      // non-fatal: capabilities stay empty (all-deny for non-root)
    }
  }

  /** Load identity + capabilities from /auth/me. */
  private async loadMe(): Promise<void> {
    const resp = await api.get<MeResp>('/auth/me');
    this.me = resp.user;
    this.realUser = resp.real_user;
    // Restore the isImpersonating state persisted in localStorage on reload
    // (the admin token survives because we persisted it before swapping).
  }

  async boot(): Promise<void> {
    this.phase = 'loading';
    try {
      this.meta = await api.get<MetaResp>('/meta');
    } catch {
      this.phase = 'offline';
      return;
    }
    if (this.meta.needs_onboarding) {
      this.phase = 'onboarding';
      return;
    }
    if (!getToken()) {
      this.phase = 'login';
      return;
    }
    try {
      await this.loadMe();
      // Fetch capabilities alongside identity; errors are non-fatal.
      await this.loadCapabilities();
      this.phase = 'ready';
    } catch (e) {
      if (e instanceof ApiError && e.status === 401) setToken(null);
      this.phase = 'login';
    }
  }

  async login(username: string, password: string): Promise<void> {
    const resp = await api.post<LoginResp>('/auth/login', { username, password });
    setToken(resp.token);
    this.me = resp.user;
    this.realUser = resp.user;
    await this.loadCapabilities();
    this.phase = 'ready';
  }

  /** Used by onboarding which returns a LoginResp directly. */
  async acceptLogin(resp: LoginResp): Promise<void> {
    setToken(resp.token);
    this.me = resp.user;
    this.realUser = resp.user;
    await this.loadCapabilities();
    this.phase = 'ready';
  }

  /**
   * Begin impersonating `userId`.
   *
   * Saves the current (admin) token to localStorage under `otto_admin_token`
   * so it can be recovered after a page reload, swaps the active bearer to the
   * short-lived impersonation token, then re-boots so the whole app (identity,
   * capabilities, banner) reflects the target user.
   */
  async impersonate(userId: string): Promise<void> {
    const adminToken = getToken();
    if (!adminToken) throw new Error('not authenticated');

    const { token: impToken } = await api.post<{ token: string }>(
      `/admin/impersonate/${userId}`,
      {},
    );

    // Persist the admin token so Exit works even after a page reload.
    localStorage.setItem(ADMIN_TOKEN_KEY, adminToken);
    setToken(impToken);

    // Re-load identity + capabilities as the impersonated user.
    await this.loadMe();
    await this.loadCapabilities();
  }

  /**
   * End the active impersonation session.
   *
   * Calls `/admin/impersonate/stop` to revoke the impersonation token on the
   * server, restores the saved admin token (from in-memory or localStorage),
   * clears the persisted key, then re-boots so the app reverts to the admin.
   */
  async stopImpersonating(): Promise<void> {
    try {
      await api.post('/admin/impersonate/stop', {});
    } catch {
      // Revoke is best-effort; proceed regardless (token may already be expired).
    }

    const savedAdmin =
      localStorage.getItem(ADMIN_TOKEN_KEY);
    localStorage.removeItem(ADMIN_TOKEN_KEY);

    if (savedAdmin) {
      setToken(savedAdmin);
    } else {
      // Fallback: no saved token — go to login.
      setToken(null);
      this.me = null;
      this.realUser = null;
      this.capabilities = {};
      this.phase = 'login';
      return;
    }

    // Re-load as the real (admin) user.
    await this.loadMe();
    await this.loadCapabilities();
  }

  async logout(): Promise<void> {
    try {
      await api.post('/auth/logout');
    } catch {
      /* token may already be invalid */
    }
    localStorage.removeItem(ADMIN_TOKEN_KEY);
    setToken(null);
    this.me = null;
    this.realUser = null;
    this.capabilities = {};
    this.phase = 'login';
  }
}

export const auth = new AuthStore();
