// Auth / boot state: GET /meta → onboarding | login | ready.

import { api, setToken, getToken, ApiError } from '../api/client';
import type { LoginResp, MetaResp, User } from '../api/types';

export type BootPhase = 'loading' | 'onboarding' | 'login' | 'ready' | 'offline';

class AuthStore {
  phase: BootPhase = $state('loading');
  meta: MetaResp | null = $state(null);
  me: User | null = $state(null);

  get isRoot(): boolean {
    return this.me?.is_root ?? false;
  }

  /** Re-fetch /meta without touching the boot phase (e.g. after provider changes). */
  async refreshMeta(): Promise<void> {
    try {
      this.meta = await api.get<MetaResp>('/meta');
    } catch {
      // non-fatal: keep the stale meta
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
    this.phase = 'ready';
  }

  /** Used by onboarding which returns a LoginResp directly. */
  acceptLogin(resp: LoginResp): void {
    setToken(resp.token);
    this.me = resp.user;
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
    this.phase = 'login';
  }
}

export const auth = new AuthStore();
