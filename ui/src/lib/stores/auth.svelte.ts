// Auth / boot state: GET /meta → onboarding | login | ready.

import { api, setToken, getToken, ApiError } from '../api/client';
import type { CapabilitiesResp, LoginResp, MetaResp, User } from '../api/types';
import type { Capability, Feature } from '../api/types';

export type BootPhase = 'loading' | 'onboarding' | 'login' | 'ready' | 'offline';

// Capability ladder: index = strength (higher = more permissive).
const CAP_ORDER: Capability[] = ['none', 'view', 'edit', 'admin'];

function capIndex(c: string): number {
  const i = CAP_ORDER.indexOf(c as Capability);
  return i < 0 ? 0 : i;
}

class AuthStore {
  phase: BootPhase = $state('loading');
  meta: MetaResp | null = $state(null);
  me: User | null = $state(null);
  /** Effective capabilities map — feature → capability string. Populated after boot. */
  capabilities: Record<string, string> = $state({});

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
      this.me = await api.get<User>('/auth/me');
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
    await this.loadCapabilities();
    this.phase = 'ready';
  }

  /** Used by onboarding which returns a LoginResp directly. */
  async acceptLogin(resp: LoginResp): Promise<void> {
    setToken(resp.token);
    this.me = resp.user;
    await this.loadCapabilities();
    this.phase = 'ready';
  }

  async logout(): Promise<void> {
    try {
      await api.post('/auth/logout');
    } catch {
      /* token may already be invalid */
    }
    setToken(null);
    this.me = null;
    this.capabilities = {};
    this.phase = 'login';
  }
}

export const auth = new AuthStore();
