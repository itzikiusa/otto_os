// Auth / boot state: GET /meta → onboarding | login | ready.

import {
  api,
  setToken,
  getToken,
  getImpersonationToken,
  setImpersonationToken,
  ApiError,
  UNAUTHORIZED_EVENT,
  setAltLoopbackBase,
} from '../api/client';
import { lsRemove, ssGet, ssSet, ssRemove } from '../storage';
import type { CapabilitiesResp, LoginResp, MeResp, MetaResp, User } from '../api/types';
import type { Capability, Feature } from '../api/types';

export type BootPhase = 'loading' | 'onboarding' | 'login' | 'ready' | 'offline';

/** Older builds kept the admin's own bearer here (sessionStorage, and before
 *  that localStorage) while impersonating, and swapped the impersonation
 *  token into the SHARED `otto_token` (S13-303). The impersonation bearer now
 *  lives per tab (`setImpersonationToken`) and the admin token never leaves
 *  `otto_token`, so there is nothing to save — purge any leftover copy. */
const ADMIN_TOKEN_KEY = 'otto_admin_token';
lsRemove(ADMIN_TOKEN_KEY);
ssRemove(ADMIN_TOKEN_KEY);

/** When this tab's impersonation began (ms), beside the impersonation token in
 *  THIS tab's sessionStorage (S13-07). It used to live in shared localStorage,
 *  where a second, non-impersonating admin tab cleared it and the banner
 *  froze at "30:00" while the real 30-minute server TTL ran out. */
const IMP_START_KEY = 'otto_imp_start_ms';
lsRemove(IMP_START_KEY); // older builds' shared copy

function beginImpersonation(impToken: string): void {
  setImpersonationToken(impToken);
  ssSet(IMP_START_KEY, String(Date.now()));
}

/** End THIS tab's impersonation (if any): the tab falls back to the shared
 *  sign-in token. Returns whether one was active. */
function endImpersonation(): boolean {
  const active = getImpersonationToken() !== null;
  if (active) setImpersonationToken(null);
  ssRemove(IMP_START_KEY);
  return active;
}

/** Start of this tab's impersonation (ms since epoch), or null if unknown. */
export function impersonationStartedMs(): number | null {
  const v = parseInt(ssGet(IMP_START_KEY) ?? '', 10);
  return Number.isFinite(v) && v > 0 ? v : null;
}

// Capability ladder: index = strength (higher = more permissive).
const CAP_ORDER: Capability[] = ['none', 'view', 'edit', 'admin'];

function capIndex(c: string): number {
  const i = CAP_ORDER.indexOf(c as Capability);
  return i < 0 ? 0 : i;
}


/** Boot's `/meta` deadline (S13-05). A daemon that accepts the connection but
 *  stalls used to hold boot on "Connecting to the Otto daemon…" forever: the
 *  2 s offline retry only runs in 'offline', and `booting` blocked re-entry.
 *  Past this, boot goes 'offline' and the retry loop takes over. */
export const META_BOOT_TIMEOUT_MS = 5_000;
/** The same deadline for an in-place re-boot of a RUNNING app (S13-304):
 *  `/meta`'s cold tool probe alone can take ~4 s, and a miss here no longer
 *  costs a spinner — the shell stays up — so give it more room. */
export const META_REBOOT_TIMEOUT_MS = 10_000;

/** `GET /meta`, aborted (and rejected) after `ms`. */
function metaWithin(ms: number): Promise<MetaResp> {
  const ctl = new AbortController();
  let timer: ReturnType<typeof setTimeout> | undefined;
  const expired = new Promise<never>((_, reject) => {
    timer = setTimeout(() => {
      ctl.abort();
      reject(new Error('timeout'));
    }, ms);
  });
  return Promise.race([api.get<MetaResp>('/meta', ctl.signal), expired]).finally(() => clearTimeout(timer));
}

/** Tab-scoped (sessionStorage) wizard step, saved once the root account
 *  exists: a reload mid-setup then RESUMES the wizard instead of landing in
 *  the app with the workspace / usage / tool steps silently skipped (S20-11).
 *  Cleared by Finish. */
export const ONBOARDING_RESUME_KEY = 'otto_onboarding_step';

class AuthStore {
  phase: BootPhase = $state('loading');
  /** Set when a reload resumed the first-run wizard past root creation. */
  onboardingResumeStep: number | null = $state(null);
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

  /**
   * Custom-plugin variant of {@link can}: returns true when the current user has
   * at least `required` on the plugin `key` (the plugin slug). Root always passes.
   * Plugin capabilities are returned by `/auth/capabilities` keyed by slug
   * (string-keyed RBAC axis, parallel to the closed `Feature` enum).
   */
  canPlugin(key: string, required: Capability): boolean {
    if (!this.me) return false;
    if (this.me.is_root) return true;
    const granted = this.capabilities[key] ?? 'none';
    return capIndex(granted) >= capIndex(required);
  }

  /** Re-fetch /meta without touching the boot phase (e.g. after provider changes). */
  async refreshMeta(): Promise<boolean> {
    try {
      this.meta = await api.get<MetaResp>('/meta');
      setAltLoopbackBase(this.meta.alt_loopback_base);
      return true;
    } catch {
      // non-fatal: keep the stale meta
      return false;
    }
  }

  /** Fetch the caller's effective capabilities from /auth/capabilities. */
  private async loadCapabilities(): Promise<void> {
    // Never show the previous identity's grants while refreshing or on failure.
    const token = getToken();
    this.capabilities = {};
    try {
      const resp = await api.get<CapabilitiesResp>('/auth/capabilities');
      if (token === getToken()) this.capabilities = resp.capabilities;
    } catch {
      // non-fatal: capabilities stay empty (all-deny for non-root)
    }
  }

  /** Load identity + capabilities from /auth/me. */
  private async loadMe(): Promise<void> {
    const token = getToken();
    const response = await api.get<MeResp>('/auth/me');
    if (token !== getToken()) throw new Error('Identity changed while refreshing');
    this.applyMe(response);
  }

  /** A changed bearer cannot keep the previous identity's actionable shell.
   * On failure the existing offline retry verifies the current token again. */
  private async refreshChangedIdentity(): Promise<void> {
    const token = getToken();
    this.me = null;
    this.realUser = null;
    this.capabilities = {};
    this.phase = 'loading';
    try {
      await this.loadMe();
      await this.loadCapabilities();
      if (token === getToken()) this.phase = 'ready';
    } catch (error) {
      if (token === getToken()) this.phase = 'offline';
      throw error;
    }
  }

  private applyMe(resp: MeResp): void {
    this.me = resp.user;
    this.realUser = resp.real_user;
    // Restore the isImpersonating state persisted in sessionStorage on reload
    // (the admin token survives because we persisted it before swapping).
  }

  private booting = false;

  async boot(retry = false): Promise<void> {
    if (this.booting) return;
    this.booting = true;
    try {
      await this.performBoot(retry);
    } finally {
      this.booting = false;
    }
  }

  private async performBoot(retry: boolean): Promise<void> {
    // Quiet retries keep the offline explanation and focused Retry available,
    // and a re-boot of an already-running app refreshes in place: dropping
    // back to 'loading' unmounts the whole shell (a visible full reload).
    if (!retry && this.phase !== 'ready') this.phase = 'loading';
    // With a token, identity + grants go out WITH /meta instead of after it
    // (perf F3: three serial round-trips → one; it matters on a remote
    // daemon). Onboarding / login / offline below just drop the results.
    const token = getToken();
    const early = token
      ? { me: api.get<MeResp>('/auth/me'), caps: api.get<CapabilitiesResp>('/auth/capabilities') }
      : null;
    // Observed now, so a branch that never awaits them can't leave an
    // unhandled rejection behind.
    early?.me.catch(() => {});
    early?.caps.catch(() => {});
    const running = this.phase === 'ready';
    try {
      this.meta = await metaWithin(running ? META_REBOOT_TIMEOUT_MS : META_BOOT_TIMEOUT_MS);
      // Background/slow calls move to the daemon's second loopback host
      // (a separate socket pool) when it advertises one.
      setAltLoopbackBase(this.meta.alt_loopback_base);
    } catch {
      // Like the /auth/me catch below: an in-place re-boot of a running app
      // keeps the shell (editors, drafts, terminals) up — only a cold boot
      // falls to the offline screen and its retry loop (S13-304).
      if (this.phase !== 'ready') this.phase = 'offline';
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
      if (early && token === getToken()) {
        this.applyMe(await early.me);
        this.capabilities = {};
        try {
          const resp = await early.caps;
          if (token === getToken()) this.capabilities = resp.capabilities;
        } catch {
          // non-fatal: capabilities stay empty (all-deny for non-root)
        }
      } else {
        await this.loadMe();
        // Errors are non-fatal.
        await this.loadCapabilities();
      }
      const resume = Number(ssGet(ONBOARDING_RESUME_KEY) ?? '');
      if (Number.isInteger(resume) && resume >= 2 && resume <= 4) {
        this.onboardingResumeStep = resume;
        this.phase = 'onboarding';
        return;
      }
      this.phase = 'ready';
    } catch (e) {
      // Only a rejected token means "sign in again". A timeout / 5xx / dropped
      // connection on /auth/me (daemon restarting, sleep/wake) used to land on
      // the login screen with a perfectly valid token: stay offline instead —
      // App polls boot(true) every 2 s — or, for an in-place re-boot of a
      // running app, keep the shell up.
      if (e instanceof ApiError && e.status === 401) {
        // An expired impersonation token (S13-302: the 4401 re-boot, a reload
        // past the 30-minute TTL) falls back to the admin's own session in
        // this tab instead of signing them out of it.
        if (token !== null && token === getImpersonationToken()) {
          endImpersonation();
          if (getToken()) return this.performBoot(true);
        }
        setToken(null);
        this.phase = 'login';
      } else if (this.phase !== 'ready') {
        this.phase = 'offline';
      }
    }
  }

  async login(username: string, password: string): Promise<void> {
    const resp = await api.post<LoginResp>('/auth/login', { username, password });
    endImpersonation();
    setToken(resp.token);
    this.me = resp.user;
    this.realUser = resp.user;
    // `/meta` was fetched signed-out (tools/providers withheld from
    // anonymous callers); reload it with the new bearer.
    await Promise.all([this.loadCapabilities(), this.refreshMeta()]);
    this.phase = 'ready';
  }

  /** Used by onboarding which returns a LoginResp directly. */
  async acceptLogin(resp: LoginResp): Promise<void> {
    endImpersonation();
    setToken(resp.token);
    this.me = resp.user;
    this.realUser = resp.user;
    // `/meta` was fetched signed-out (tools/providers withheld from
    // anonymous callers); reload it with the new bearer.
    await Promise.all([this.loadCapabilities(), this.refreshMeta()]);
    this.phase = 'ready';
  }

  /**
   * Begin impersonating `userId`.
   *
   * The short-lived impersonation token becomes THIS tab's bearer
   * (sessionStorage, S13-303) — the admin's own token stays untouched in the
   * shared `otto_token`, so other windows keep acting as the admin and Exit /
   * expiry simply drop the overlay. Then identity + capabilities reload as
   * the target user.
   */
  async impersonate(userId: string): Promise<void> {
    if (!getToken()) throw new Error('not authenticated');

    const { token: impToken } = await api.post<{ token: string }>(
      `/admin/impersonate/${userId}`,
      {},
    );

    beginImpersonation(impToken);

    // Re-load identity + capabilities as the impersonated user.
    await this.refreshChangedIdentity();
  }

  /**
   * End the active impersonation session.
   *
   * Calls `/admin/impersonate/stop` (with the impersonation bearer) to revoke
   * it on the server, drops this tab's overlay, then reloads as the admin
   * from the shared sign-in token.
   */
  async stopImpersonating(): Promise<void> {
    try {
      await api.post('/admin/impersonate/stop', {});
    } catch {
      // Revoke is best-effort; proceed regardless (token may already be expired).
    }

    endImpersonation();

    if (!getToken()) {
      // The admin signed out elsewhere meanwhile — go to login.
      this.me = null;
      this.realUser = null;
      this.capabilities = {};
      this.phase = 'login';
      return;
    }

    // Re-load as the real (admin) user.
    await this.refreshChangedIdentity();
  }

  private verifying401 = false;

  /**
   * Global 401 handler (see `UNAUTHORIZED_EVENT`). A route may use 401 for a
   * request-specific reason (a wrong current password), so the session is
   * only dropped when /auth/me ALSO answers 401 for the same token. An
   * expired impersonation token falls back to the saved admin token.
   */
  async handleUnauthorized(token: string): Promise<void> {
    if (this.phase !== 'ready' || this.verifying401 || token !== getToken()) return;
    this.verifying401 = true;
    try {
      await api.get<MeResp>('/auth/me');
    } catch (e) {
      if (!(e instanceof ApiError && e.status === 401) || token !== getToken()) return;
      // An expired impersonation token falls back to this tab's admin session.
      if (token === getImpersonationToken() && endImpersonation() && getToken()) {
        try {
          await this.loadMe();
          await this.loadCapabilities();
          return;
        } catch {
          /* the admin token is gone too — fall through to login */
        }
      }
      setToken(null);
      this.me = null;
      this.realUser = null;
      this.capabilities = {};
      this.phase = 'login';
    } finally {
      this.verifying401 = false;
    }
  }

  async logout(): Promise<void> {
    // Signing out while impersonating: revoke the overlay first, then sign
    // the ADMIN's own session out (the request below must carry that token).
    if (getImpersonationToken()) {
      try {
        await api.post('/admin/impersonate/stop', {});
      } catch {
        /* already expired / revoked */
      }
      endImpersonation();
    }
    try {
      await api.post('/auth/logout');
    } catch {
      /* token may already be invalid */
    }
    setToken(null);
    this.me = null;
    this.realUser = null;
    this.capabilities = {};
    this.phase = 'login';
  }
}

export const auth = new AuthStore();

if (typeof window !== 'undefined') {
  window.addEventListener(UNAUTHORIZED_EVENT, (e) => {
    const token = (e as CustomEvent<{ token: string }>).detail?.token;
    if (token) void auth.handleUnauthorized(token);
  });
}
