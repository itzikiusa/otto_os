// Minimal fetch wrapper for /api/v1 + WS URL helper.

import type {
  Problem,
  ImportReq,
  ImportResult,
  NlToSqlReq,
  NlToSqlOutcome,
  DbAssistReq,
  DbAssistResp,
  DbAssistSummaryResp,
  DbCloseResp,
  ImportSource,
  SourceStatus,
  ImportScanResult,
  ImportCreateReq,
  ImportCreateResult,
  Finding,
  FindingDetail,
  FindingActionResp,
  RepoRule,
  ReviewProofPack,
  ReviewProofPackExport,
} from './types';
import { apiComponent, clientSpanName, startMeasurement } from '../telemetry';
import { serviceHealth } from '../stores/serviceHealth.svelte';
import { inheritedLane, type Lane } from './lane';

export class ApiError extends Error {
  code: string;
  status: number;

  constructor(status: number, problem: Problem) {
    super(problem.message);
    this.code = problem.code;
    this.status = status;
  }
}

/** Endpoints whose 5xx actually means "a remote git PROVIDER (GitHub /
 *  Bitbucket / GitLab) failed" — the only ones allowed to raise the global
 *  provider-outage banner. Everything else (local git ops like pull/merge/
 *  checkout, Kafka/DB/SSH infra, LLM-agent endpoints) reports its failure in
 *  its own error toast; a 502 there is NOT a provider outage, and the old
 *  any-path rule made "commit your changes before you merge" announce a
 *  GitHub outage. */
function isProviderPath(path: string): boolean {
  return /\/repos\/[^/]+\/(prs|collaborators)([/?]|$)/.test(path);
}

/** Guarded storage (see lib/storage.ts — inlined: this is the base module).
 *  A throwing accessor here failed EVERY request, i.e. the whole app. */
function storedItem(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

export function baseUrl(): string {
  // Native app + remote browser both talk to the daemon. When the SPA is
  // served BY the daemon, same-origin works; the localStorage override is for
  // dev mode (vite on :5173, daemon on :7700).
  return storedItem('otto_base') ?? defaultBase();
}

function defaultBase(): string {
  if (location.port === '5173' || location.protocol === 'tauri:' || location.protocol === 'file:') {
    return 'http://127.0.0.1:7700';
  }
  return location.origin;
}

/** sessionStorage key of THIS tab's impersonation bearer (S13-303). It used
 *  to overwrite the shared `localStorage.otto_token`, so every other window
 *  and tab silently started acting as the impersonated user while still
 *  rendering the admin. Per tab, like the admin token it overlays. */
const IMP_TOKEN_KEY = 'otto_imp_token';
let impTokenMem: string | null = null;

function sessionItem(key: string): string | null {
  try {
    return sessionStorage.getItem(key);
  } catch {
    return null;
  }
}

/** This tab's impersonation bearer, or null when not impersonating. */
export function getImpersonationToken(): string | null {
  return impTokenMem ?? sessionItem(IMP_TOKEN_KEY);
}

/** Start (token) or end (null) THIS tab's impersonation. Never touches the
 *  shared `otto_token`, so other windows keep their own identity. */
export function setImpersonationToken(token: string | null): void {
  impTokenMem = token;
  try {
    if (token === null) sessionStorage.removeItem(IMP_TOKEN_KEY);
    else sessionStorage.setItem(IMP_TOKEN_KEY, token);
  } catch {
    /* blocked storage: the in-memory copy still drives this document */
  }
  if (typeof window !== 'undefined') window.dispatchEvent(new Event('otto:auth-changed'));
}

/** The bearer every request sends: this tab's impersonation token when one is
 *  active, else the shared sign-in token. */
export function getToken(): string | null {
  return getImpersonationToken() ?? storedItem('otto_token');
}

export function setToken(token: string | null): void {
  try {
    if (token === null) localStorage.removeItem('otto_token');
    else localStorage.setItem('otto_token', token);
  } catch {
    /* blocked storage: the token cannot persist; callers still proceed */
  }
  if (typeof window !== 'undefined') window.dispatchEvent(new Event('otto:auth-changed'));
}

/** Window event fired when a request made WITH the stored token is answered
 *  401. The auth store listens (it imports this module, so it cannot be
 *  called from here) and confirms against /auth/me before treating the
 *  session as expired — without this, an expired/revoked token left every
 *  page failing with scattered error toasts instead of returning to login. */
export const UNAUTHORIZED_EVENT = 'otto:unauthorized';

// ---------------------------------------------------------------------------
// Request lanes (TRANSPORT_PLAN §3d/§4). The webview reaches the daemon over
// HTTP/1.1 and the engine keeps ~6 sockets per HOST, shared by EVERY Otto
// window (one network process). A poll or a 20 s git fetch holding those
// sockets made a click wait for seconds ("barely usable while agents work").
//
//  - `int`  — what the user just did; always on the interactive base.
//  - `bg`   — poll ticks, live-query refetches and reconnect / lag resyncs:
//             `api.bg.*`, or ANY call made inside a poll tick (ambient, see
//             ./lane.ts). ≤ BG_MAX per document and ≤ BG_GLOBAL app-wide.
//  - `long` — calls known to hold a socket for seconds (remote git, provider
//             PRs, CLI auth checks, kubectl/Kafka/AWS, DB queries, Jira,
//             usage, the API client's Send, SSH dials, `/wait`):
//             `api.long.*`, any path in LONG_PATHS, and the raw streams
//             (`laneFetch('long', …)`: k8s follow, DB export/import, S3).
//
// When the daemon advertises `alt_loopback_base` (`/meta`; it binds BOTH
// loopback addresses) `bg`/`long` go to that second host — a physically
// separate socket pool, across all windows — and `127.0.0.1` stays free for
// `int`. Without it (remote mode, IPv6 off, older daemon) every lane uses the
// one base, exactly as before.
// ---------------------------------------------------------------------------

export type { Lane } from './lane';

/** Paths that hold a socket for seconds. Matched against the `/api/v1`-relative
 *  path (query included); a caller can still force a lane explicitly. */
export const LONG_PATHS: readonly RegExp[] = [
  /^\/repos\/[^/]+\/(fetch|pull|push)([/?]|$)/,
  /^\/repos\/[^/]+\/(prs|collaborators)([/?]|$)/,
  // Local git that walks history / the tree or runs hooks: `log --all`
  // (10k commits), `/refs` (2k branches with ahead/behind), status, commit
  // (pre-commit hooks), merge, checkout, stash, blame, rebase. NOT `/diff`:
  // the diff of a commit the user just clicked is interactive, and the
  // alt host is a separate origin, so each distinct `/diff?…` URL paid its
  // own CORS preflight there (the huge-branch file list missed 300 ms).
  /^\/repos\/[^/]+\/(log|refs|status|commit|merge|checkout|stash|blame|rebase-preview|rebase|bisect|reflog|submodules)([/?]|$)/,
  /^\/auth\/provider-accounts\/[^/]+\/status([/?]|$)/,
  /^\/sessions\/[^/]+\/wait([/?]|$)/,
  /(^|\/)k8s\//,
  /(^|\/)aws\//,
  /^\/brokers\/clusters\/[^/]+\/[^?]/, // every sub-route dials Kafka
  /^\/connections\/[^/]+\/db\/(query|nl-to-sql|assist|explain|test|search-objects|schema|schema\/children|object|schema-graph|completion|query-status)([/?]|$)/,
  // SSH / DB dial on open or test.
  /^\/connections\/[^/]+\/(open|test)([/?]|$)/,
  // A DB dashboard widget runs its query.
  /^\/db\/widgets\/[^/]+\/run([/?]|$)/,
  // The user's remote HTTP call (API client "Send"), up to its own timeout.
  /^\/workspaces\/[^/]+\/api-client\/execute([/?]|$)/,
  // Jira / Confluence (remote Atlassian APIs).
  /^\/issue\//,
  // Usage reads spawn `clickhouse local`.
  /^\/usage\//,
  // Workspace code search and vault full-text search.
  /^\/workspaces\/[^/]+\/search([/?]|$)/,
  /^\/workspaces\/[^/]+\/vault\/vaults\/[^/]+\/search([/?]|$)/,
];

export function isLongPath(path: string): boolean {
  return LONG_PATHS.some((re) => re.test(path));
}

// ---- The alias host ---------------------------------------------------------
// `altBase` is what the daemon advertised (validated). A network-level failure
// on it SUSPENDS it (every lane falls back to the interactive base) and arms a
// cheap re-probe (`GET {alt}/api/v1/health`) with backoff (5 s → 60 s); a pass
// re-enables it. It is never dropped for good on one error: the old
// `altBase = null` left every `bg`/`long` call of every later hour on the six
// interactive sockets after a single daemon restart. The events client also
// suspends it while its socket is down and re-arms it from `/meta` when the
// daemon's `boot_id` changed (a restarted daemon may no longer hold the alias).

const ALT_RETRY_MIN_MS = 5_000;
const ALT_RETRY_MAX_MS = 60_000;
let altBase: string | null = null;
let altSuspended = false;
let altRetryMs = ALT_RETRY_MIN_MS;
let altProbeTimer: ReturnType<typeof setTimeout> | null = null;

function clearAltProbe(): void {
  if (altProbeTimer !== null) clearTimeout(altProbeTimer);
  altProbeTimer = null;
}

/** Adopt (or drop) the daemon's advertised second loopback base. Accepted only
 *  when the interactive base is the loopback IP, the alias is a loopback host
 *  on the SAME port and a DIFFERENT host (else it is not a separate pool).
 *  Re-arming clears any suspension and its backoff. */
export function setAltLoopbackBase(alt: string | null | undefined): void {
  clearAltProbe();
  altBase = null;
  altSuspended = false;
  altRetryMs = ALT_RETRY_MIN_MS;
  if (!alt) return;
  try {
    const b = new URL(baseUrl());
    const a = new URL(alt);
    const loopback = ['localhost', '127.0.0.1', '[::1]'];
    if (
      a.protocol === b.protocol &&
      b.hostname === '127.0.0.1' &&
      loopback.includes(a.hostname) &&
      a.hostname !== b.hostname &&
      a.port === b.port
    ) {
      altBase = `${a.protocol}//${a.host}`;
    }
  } catch {
    /* malformed — single-host transport */
  }
}

/** Stop using the alias until a probe (`probeAfterMs`) or a re-arm restores
 *  it. The events client calls this when its socket drops: the daemon may be
 *  restarting, and the next one may not hold the alias. */
export function suspendAltLoopback(probeAfterMs?: number): void {
  if (!altBase) return;
  altSuspended = true;
  clearAltProbe();
  if (probeAfterMs !== undefined) scheduleAltProbe(probeAfterMs);
}

/** Same daemon is back (socket reconnected, unchanged boot id): probe the
 *  alias now instead of waiting out its backoff. */
export function resumeAltLoopback(): void {
  if (altBase && altSuspended) scheduleAltProbe(0);
}

/** `active` | `suspended` | `none` (diagnostics / tests). */
export function altLoopbackState(): 'active' | 'suspended' | 'none' {
  return altBase ? (altSuspended ? 'suspended' : 'active') : 'none';
}

function scheduleAltProbe(ms: number): void {
  clearAltProbe();
  altProbeTimer = setTimeout(() => void probeAlt(), ms);
}

async function probeAlt(): Promise<void> {
  altProbeTimer = null;
  const alt = altBase;
  if (!alt || !altSuspended) return;
  let ok = false;
  try {
    const resp = await fetch(`${alt}/api/v1/health`, { cache: 'no-store' });
    ok = resp.ok && ((await resp.json()) as { ok?: unknown } | null)?.ok === true;
  } catch {
    ok = false;
  }
  // Re-armed / dropped meanwhile: that decision wins.
  if (alt !== altBase || !altSuspended) return;
  if (ok) {
    altSuspended = false;
  } else {
    scheduleAltProbe(altRetryMs);
    altRetryMs = Math.min(altRetryMs * 2, ALT_RETRY_MAX_MS);
  }
}

function altNetworkFailure(alt: string): void {
  if (alt !== altBase || altSuspended) return;
  altSuspended = true;
  scheduleAltProbe(altRetryMs);
  // A flapping alias backs off further each time; a request that succeeds on
  // it resets this (see laneFetch).
  altRetryMs = Math.min(altRetryMs * 2, ALT_RETRY_MAX_MS);
}

/** The base a lane's requests go to (exported for tests / diagnostics). */
export function laneBase(lane: Lane): string {
  return lane !== 'int' && altBase && !altSuspended ? altBase : baseUrl();
}

/** `fetch(<lane base>/api/v1<path>)` with the alias's fail-soft rule: a
 *  NETWORK-level failure on the alias (not an HTTP error, not an abort)
 *  suspends it and — for a GET/HEAD only, since a write may have reached the
 *  daemon before the connection broke — retries on the interactive base.
 *  Streams (k8s follow, DB export/import, S3 downloads) use it directly. */
export async function laneFetch(lane: Lane, path: string, init: RequestInit = {}): Promise<Response> {
  const base = laneBase(lane);
  const measured = !path.startsWith('/telemetry') ? startMeasurement('http.client', apiComponent(path), 'client') : null;
  if (measured) {
    const headers = new Headers(init.headers);
    headers.set('traceparent', measured.traceparent);
    init = { ...init, headers };
  }
  let status = 0;
  let route: string | null = null;
  try {
    const resp = await fetch(`${base}/api/v1${path}`, init);
    if (base !== baseUrl()) altRetryMs = ALT_RETRY_MIN_MS;
    status = resp.status;
    route = resp.headers.get('x-otto-route');
    return resp;
  } catch (e) {
    if (base === baseUrl() || isAbortError(e)) throw e;
    altNetworkFailure(base);
    const method = (init.method ?? 'GET').toUpperCase();
    if (method !== 'GET' && method !== 'HEAD') throw e;
    const resp = await fetch(`${baseUrl()}/api/v1${path}`, init);
    status = resp.status;
    route = resp.headers.get('x-otto-route');
    return resp;
  } finally {
    const method = (init.method ?? 'GET').toUpperCase();
    measured?.finish(status > 0 && status < 400 ? 'ok' : 'error', { 'http.request.method': method, ...(status ? { 'http.response.status_code': status } : {}), 'otto.lane': lane }, clientSpanName(method, route));
  }
}

/** Default deadline for an `int` / `bg` READ (S13-05). Without one, a daemon
 *  that accepted the connection but stalled (runtime lag, sleep/wake, a
 *  wedged handler) left the promise pending forever: pollers with an
 *  in-flight guard never ticked again and spinners spun until a reload.
 *  Writes get none (a timed-out write may still have been applied — a retry
 *  could duplicate it), nor does the `long` lane (slow by design) or a
 *  stream (`laneFetch` direct callers). */
export const READ_DEADLINE_MS = 20_000;

/** `caller` combined with a `ms` deadline. `timedOut()` tells the deadline
 *  apart from the caller's own abort; `done()` clears the timer. */
export function withDeadline(
  ms: number,
  caller?: AbortSignal,
): { signal: AbortSignal; timedOut: () => boolean; done: () => void } {
  const ctl = new AbortController();
  let fired = false;
  const timer = setTimeout(() => {
    fired = true;
    ctl.abort(new DOMException('The request timed out.', 'TimeoutError'));
  }, ms);
  const onAbort = (): void => ctl.abort(caller?.reason);
  if (caller?.aborted) ctl.abort(caller.reason);
  else caller?.addEventListener('abort', onAbort, { once: true });
  return {
    signal: ctl.signal,
    timedOut: () => fired && !caller?.aborted,
    done: () => {
      clearTimeout(timer);
      caller?.removeEventListener('abort', onAbort);
    },
  };
}

/** The ApiError a deadline expiry surfaces as (status 0, code `timeout`). */
function timeoutError(): ApiError {
  return new ApiError(0, { code: 'timeout', message: 'The Otto daemon didn’t answer in time.' } as Problem);
}

function resolveLane(path: string, signal: AbortSignal | undefined, lane: Lane | undefined): Lane {
  return lane ?? inheritedLane(signal) ?? (isLongPath(path) ? 'long' : 'int');
}

async function request<T>(
  method: string,
  path: string,
  body: unknown,
  signal: AbortSignal | undefined,
  lane: Lane,
  cond?: ConditionalOpts,
): Promise<T> {
  const isRead = method === 'GET' || method === 'HEAD';
  if (!isRead || lane === 'long') return requestOnce<T>(method, path, body, signal, lane, cond);
  const deadline = withDeadline(READ_DEADLINE_MS, signal);
  try {
    return await requestOnce<T>(method, path, body, deadline.signal, lane, cond);
  } catch (e) {
    if (deadline.timedOut()) throw timeoutError();
    throw e;
  } finally {
    deadline.done();
  }
}

/** Shared auth + 401 + provider-health handling for every daemon call that
 *  goes through `fetch` (S13-10): the JSON `request` and the raw helpers
 *  (text / blob / ndjson). Returns the Response when it is OK; throws the
 *  parsed Problem as an `ApiError` otherwise. */
async function rawRequest(
  lane: Lane,
  path: string,
  init: { method?: string; body?: BodyInit | null; contentType?: string; accept?: string; signal?: AbortSignal; headers?: Record<string, string> } = {},
): Promise<Response> {
  const headers: Record<string, string> = { ...(init.headers ?? {}) };
  const token = getToken();
  if (token) headers['Authorization'] = `Bearer ${token}`;
  if (init.contentType) headers['Content-Type'] = init.contentType;
  if (init.accept) headers['Accept'] = init.accept;
  const resp = await laneFetch(lane, path, { method: init.method ?? 'GET', headers, body: init.body, signal: init.signal });
  if (isProviderPath(path)) serviceHealth.report(resp.status);
  if (resp.status === 401 && token && typeof window !== 'undefined') {
    window.dispatchEvent(new CustomEvent(UNAUTHORIZED_EVENT, { detail: { token } }));
  }
  if (!resp.ok) throw await problemOf(resp);
  return resp;
}

/** The `ApiError` for a non-OK response (JSON Problem body, else statusText). */
async function problemOf(resp: Response): Promise<ApiError> {
  let problem: Problem = { code: 'internal', message: resp.statusText };
  try {
    problem = await resp.json();
  } catch {
    // non-JSON error body — keep statusText
  }
  return new ApiError(resp.status, problem);
}

async function requestOnce<T>(
  method: string,
  path: string,
  body: unknown,
  signal: AbortSignal | undefined,
  lane: Lane,
  cond?: ConditionalOpts,
): Promise<T> {
  const headers: Record<string, string> = {};
  const token = getToken();
  if (token) headers['Authorization'] = `Bearer ${token}`;
  if (body !== undefined) headers['Content-Type'] = 'application/json';
  if (cond?.ifNoneMatch) headers['If-None-Match'] = cond.ifNoneMatch;

  const resp = await laneFetch(lane, path, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
    signal,
  });

  // Surface git-provider outages (the daemon maps provider failures to a 502)
  // — but ONLY from provider-backed endpoints. A 5xx anywhere else (local git,
  // Kafka/DB infra, agent endpoints) is that call's own failure, reported by
  // its caller's error toast, never a GitHub/Bitbucket outage.
  if (isProviderPath(path)) serviceHealth.report(resp.status);

  if (resp.status === 401 && token && typeof window !== 'undefined') {
    window.dispatchEvent(new CustomEvent(UNAUTHORIZED_EVENT, { detail: { token } }));
  }

  if (cond) {
    cond.status = resp.status;
    cond.etag = resp.headers.get('ETag');
    // 304 to a conditional GET: no body to read — the caller keeps its copy.
    if (resp.status === 304) return undefined as T;
  }

  if (!resp.ok) throw await problemOf(resp);

  // Accepted responses may carry a job/review object. Only an actually empty
  // successful body is void; an empty error must still reject above.
  if (resp.status === 204) return undefined as T;
  const decode = !path.startsWith('/telemetry') ? startMeasurement('ui.response.decode', apiComponent(path)) : null;
  try {
    const text = await resp.text();
    const result = text.trim() === '' ? undefined as T : JSON.parse(text) as T;
    decode?.finish();
    return result;
  } catch (error) {
    decode?.finish('error');
    throw error;
  }
}

// Background-request lane. Poll ticks, live-query refetches and reconnect /
// lag resyncs land here (ambiently — see ./lane.ts — or via `api.bg.*`). Two
// caps, so background work can never take a whole six-socket pool:
//  - BG_MAX per document (JS queue below), and
//  - BG_GLOBAL across EVERY document (main window, side pane, popouts, tray),
//    via Web Locks (one lock name per slot). The old comment claimed "three
//    windows × BG_MAX still < 6" — per-document budgets add up (main + pane +
//    2 popouts + tray = 10 > 6). Now at most 3 bg sockets are open app-wide:
//    on the alias host that leaves 3 for `long` calls; without an alias, 3
//    for interactive ones. No Web Locks (or a lock wait over
//    BG_GLOBAL_WAIT_MS — a frozen document holding a slot) → the per-document
//    cap alone, as before.
// Interactive calls (`api.get` & co.) never wait on this lane.
const BG_MAX = 2;
const BG_GLOBAL = 3;
const BG_GLOBAL_WAIT_MS = 5_000;
let bgActive = 0;
const bgWaiters: (() => void)[] = [];

function abortError(): DOMException {
  return new DOMException('The operation was aborted.', 'AbortError');
}

type LockManagerLike = {
  request(name: string, opts: { ifAvailable?: boolean; signal?: AbortSignal }, cb: (lock: unknown) => unknown): Promise<unknown>;
};

function webLocks(): LockManagerLike | null {
  const n = typeof navigator !== 'undefined' ? (navigator as unknown as { locks?: LockManagerLike }) : null;
  return n?.locks && typeof n.locks.request === 'function' ? n.locks : null;
}

/** Hold lock `name`; resolves with its release, or `null` when `ifAvailable`
 *  found it taken. */
function holdLock(
  locks: LockManagerLike,
  name: string,
  opts: { ifAvailable?: boolean; signal?: AbortSignal },
): Promise<(() => void) | null> {
  return new Promise((resolve, reject) => {
    let release!: () => void;
    const held = new Promise<void>((r) => (release = r));
    locks
      .request(name, opts, (lock) => {
        if (!lock) {
          resolve(null);
          return undefined;
        }
        resolve(release);
        return held;
      })
      .catch(reject);
  });
}

const noop = (): void => {};

/** One of the BG_GLOBAL app-wide slots; its release. */
async function globalBgSlot(signal?: AbortSignal): Promise<() => void> {
  const locks = webLocks();
  if (!locks) return noop;
  const first = Math.floor(Math.random() * BG_GLOBAL);
  try {
    for (let k = 0; k < BG_GLOBAL; k++) {
      const rel = await holdLock(locks, `otto-bg-${(first + k) % BG_GLOBAL}`, { ifAvailable: true });
      if (rel) return rel;
    }
    // All taken: wait for one (bounded — see BG_GLOBAL_WAIT_MS).
    const ctl = new AbortController();
    const onAbort = (): void => ctl.abort();
    signal?.addEventListener('abort', onAbort, { once: true });
    const timer = setTimeout(() => ctl.abort(), BG_GLOBAL_WAIT_MS);
    try {
      return (await holdLock(locks, `otto-bg-${first}`, { signal: ctl.signal })) ?? noop;
    } catch (e) {
      if (signal?.aborted) throw abortError();
      if (isAbortError(e)) return noop; // waited long enough: per-document cap only
      throw e;
    } finally {
      clearTimeout(timer);
      signal?.removeEventListener('abort', onAbort);
    }
  } catch (e) {
    if (signal?.aborted) throw abortError();
    return noop; // Web Locks misbehaving: never fail a request over it
  }
}

async function withBgSlot<T>(run: () => Promise<T>, signal?: AbortSignal): Promise<T> {
  if (signal?.aborted) throw abortError();
  if (bgActive < BG_MAX) {
    bgActive += 1;
  } else {
    // Wait for a finishing request to hand its slot over (no re-count, so a
    // newcomer can never slip in between the release and this resume).
    await new Promise<void>((resolve, reject) => {
      const go = (): void => {
        signal?.removeEventListener('abort', onAbort);
        resolve();
      };
      const onAbort = (): void => {
        const i = bgWaiters.indexOf(go);
        if (i >= 0) bgWaiters.splice(i, 1);
        reject(abortError());
      };
      signal?.addEventListener('abort', onAbort, { once: true });
      bgWaiters.push(go);
    });
  }
  let releaseGlobal = noop;
  try {
    releaseGlobal = await globalBgSlot(signal);
    return await run();
  } finally {
    releaseGlobal();
    const next = bgWaiters.shift();
    if (next) next();
    else bgActive -= 1;
  }
}

/** Issue a request on its effective lane (resolved NOW, synchronously, so an
 *  ambient `inLane` scope applies); `bg` goes through the slot caps. */
function send<T>(method: string, path: string, body: unknown, signal: AbortSignal | undefined, lane?: Lane): Promise<T> {
  const eff = resolveLane(path, signal, lane);
  if (eff === 'bg') {
    const queued = !path.startsWith('/telemetry') ? startMeasurement('ui.request.queue', apiComponent(path)) : null;
    return withBgSlot(() => { queued?.finish(); return request<T>(method, path, body, signal, 'bg'); }, signal).catch((error: unknown) => { queued?.finish('error'); throw error; });
  }
  return request<T>(method, path, body, signal, eff);
}

// --- perf K8s: conditional GET (If-None-Match → 304) -------------------------
/** In/out bag for a conditional `request` (internal): the validator to send,
 *  and the status + `ETag` the response carried. */
interface ConditionalOpts {
  ifNoneMatch?: string | null;
  status?: number;
  etag?: string | null;
}

/** A conditional GET's outcome ({@link api.getConditional}): `notModified`
 *  on a 304 (no body was read — keep what you have), else the parsed body.
 *  `etag` is the response's `ETag` when the daemon exposes it (CORS), or null. */
export type Conditional<T> =
  | { notModified: true; etag: string | null }
  | { notModified: false; etag: string | null; data: T };

/** `send` for a conditional GET: the same lane resolution + bg slot caps, so
 *  it rides exactly the socket pool a plain `api.get` of `path` would. */
function sendConditional<T>(path: string, ifNoneMatch: string | null | undefined, signal: AbortSignal | undefined, lane?: Lane): Promise<Conditional<T>> {
  const eff = resolveLane(path, signal, lane);
  const cond: ConditionalOpts = { ifNoneMatch };
  const run = (): Promise<T> => request<T>('GET', path, undefined, signal, eff, cond);
  return (eff === 'bg' ? withBgSlot(run, signal) : run()).then((data) =>
    cond.status === 304
      ? { notModified: true as const, etag: cond.etag ?? ifNoneMatch ?? null }
      : { notModified: false as const, etag: cond.etag ?? null, data },
  );
}

export const api = {
  /** Conditional GET (perf K8s): sends `If-None-Match: <ifNoneMatch>` when
   *  given and resolves `{ notModified: true }` on a 304 instead of throwing. */
  getConditional: <T>(path: string, ifNoneMatch: string | null | undefined, signal?: AbortSignal) =>
    sendConditional<T>(path, ifNoneMatch, signal),
  /** Background (poll) lane — see {@link withBgSlot}. */
  bg: {
    get: <T>(path: string, signal?: AbortSignal) => send<T>('GET', path, undefined, signal, 'bg'),
    /** A background WRITE that tolerates delay (a keep-alive ping). */
    post: <T>(path: string, body?: unknown, signal?: AbortSignal) => send<T>('POST', path, body, signal, 'bg'),
  },
  /** Known-slow lane (remote calls, dials, infra sweeps): the alias host
   *  when advertised, never queued in JS. Paths in LONG_PATHS get it anyway;
   *  call sites use this where the PATH alone can't tell (the API client's
   *  "Send", an SSH open). */
  long: {
    get: <T>(path: string, signal?: AbortSignal) => send<T>('GET', path, undefined, signal, 'long'),
    post: <T>(path: string, body?: unknown, signal?: AbortSignal) => send<T>('POST', path, body, signal, 'long'),
  },
  get: <T>(path: string, signal?: AbortSignal) => send<T>('GET', path, undefined, signal),
  post: <T>(path: string, body?: unknown, signal?: AbortSignal) => send<T>('POST', path, body, signal),
  patch: <T>(path: string, body?: unknown) => send<T>('PATCH', path, body, undefined),
  put: <T>(path: string, body?: unknown) => send<T>('PUT', path, body, undefined),
  del: <T>(path: string) => send<T>('DELETE', path, undefined, undefined),
};

/** True for the daemon's 409 "the working tree is in the way" git refusals —
 *  the ones a stash can fix (dirty tree blocking a pull/checkout/merge), as
 *  opposed to auth/network failures. Shared by the git views that offer a
 *  "stash & retry" follow-up on exactly these. */
export function isDirtyGitRefusal(e: unknown): boolean {
  return (
    e instanceof ApiError &&
    e.status === 409 &&
    /overwritten|commit your changes|stash|unstaged changes/i.test(e.message)
  );
}

/** The API client's `409 needs_confirm=new_host` refusal: a stored secret
 *  would be sent to a host it isn't bound to. Returns that host (or `''`
 *  when the message doesn't name one), `null` for any other error. */
export function newHostConfirmHost(e: unknown): string | null {
  if (!(e instanceof ApiError) || e.status !== 409 || !e.message.includes('needs_confirm=new_host')) {
    return null;
  }
  return /host '([^']*)'/.exec(e.message)?.[1] ?? '';
}

/** True when an error is a fetch abort (caller cancelled via AbortSignal). */
export function isAbortError(e: unknown): boolean {
  return e instanceof DOMException && e.name === 'AbortError';
}

/**
 * Fetch a text resource (e.g. a `text/markdown` report) from /api/v1<path> with
 * the stored Bearer token. Throws `ApiError` on a non-2xx response; a 401
 * sends the user back to login like any `api.*` call (via `rawRequest`).
 */
export async function authedText(path: string): Promise<string> {
  return (await rawRequest('int', path)).text();
}

/**
 * Fetch a binary resource from /api/v1<path> with the stored Bearer token,
 * then return a revocable object URL. The caller is responsible for calling
 * URL.revokeObjectURL() when done (e.g. on component unmount).
 */
export async function authedBlobUrl(path: string): Promise<string> {
  return URL.createObjectURL(await (await rawRequest('int', path)).blob());
}

/**
 * POST a raw binary body (e.g. an `image/png` Blob) to /api/v1<path> and parse
 * the JSON reply. Skips JSON/base64 framing entirely — a 15 MB PNG goes over
 * the wire as 15 MB, not ~20 MB of base64 inside a JSON string.
 */
export async function postBlob<T>(path: string, body: Blob, contentType: string): Promise<T> {
  const resp = await rawRequest('int', path, { method: 'POST', body, contentType });
  return (await resp.json()) as T;
}

/**
 * POST a JSON body to /api/v1<path> with the bearer token and return the RAW
 * response body as text. For download/export endpoints that reply with a
 * non-JSON body (e.g. `text/csv`) — which the JSON-parsing `request()` helper
 * cannot read (it would `resp.json()` the CSV and throw a SyntaxError).
 */
export async function postForText(
  path: string,
  body: unknown,
  signal?: AbortSignal,
): Promise<string> {
  const resp = await rawRequest('int', path, {
    method: 'POST',
    body: JSON.stringify(body),
    contentType: 'application/json',
    signal,
  });
  return resp.text();
}

/**
 * POST a JSON body to /api/v1<path> and read a streamed NDJSON response,
 * invoking `onLine` for each parsed JSON line as it arrives. For long-running
 * endpoints that emit incremental progress (e.g. the streaming DB export) so the
 * caller can drive a progress bar and the connection never idles out.
 */
export async function postNdjsonStream(
  path: string,
  body: unknown,
  onLine: (obj: unknown) => void,
  signal?: AbortSignal,
): Promise<void> {
  // DB export-to-path / import run for minutes: the long lane (alias host),
  // never one of the six interactive sockets.
  const resp = await rawRequest('long', path, {
    method: 'POST',
    body: JSON.stringify(body),
    contentType: 'application/json',
    signal,
  });
  if (!resp.body) throw new ApiError(resp.status, { code: 'internal', message: 'empty response stream' } as Problem);
  const reader = resp.body.getReader();
  const decoder = new TextDecoder();
  let buf = '';
  const drain = (chunk: string): void => {
    buf += chunk;
    let nl: number;
    while ((nl = buf.indexOf('\n')) >= 0) {
      const line = buf.slice(0, nl).trim();
      buf = buf.slice(nl + 1);
      if (line) onLine(JSON.parse(line) as unknown);
    }
  };
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    drain(decoder.decode(value, { stream: true }));
  }
  const tail = buf.trim();
  if (tail) onLine(JSON.parse(tail) as unknown);
}

/**
 * Import a local file into a SQL table (`POST …/db/import`). Reads the same
 * streamed NDJSON the export uses; `onLine` fires for each line and the promise
 * resolves with the final `{done…}` / `{error}` line so the caller can drive a
 * progress UI and handle the guarded-write retry. The server emits one final
 * line in v1 (run-to-completion), but the streaming reader is future-proof.
 */
export async function dbImport(
  connId: string,
  body: ImportReq,
  onLine?: (line: ImportResult) => void,
): Promise<ImportResult> {
  let last: ImportResult = {};
  await postNdjsonStream(`/connections/${connId}/db/import`, body, (msg) => {
    const line = msg as ImportResult;
    last = line;
    onLine?.(line);
  });
  return last;
}

/**
 * Draft a verified read query from natural language (`POST …/db/nl-to-sql`).
 * Plain JSON in/out; the server returns only an `EXPLAIN`-validated read. A 400
 * Problem surfaces as an {@link ApiError} the caller inspects (`.message` starts
 * with "NL-to-SQL is not configured" when no drafter is wired, or "could not
 * produce a valid read query" when the loop was exhausted).
 */
export function dbNlToSql(connId: string, body: NlToSqlReq): Promise<NlToSqlOutcome> {
  return api.post<NlToSqlOutcome>(`/connections/${connId}/db/nl-to-sql`, body);
}

// --- DB Assistant (file-backed, embedded agent panel) -----------------------
//
// The embedded DB Assistant runs an agent as a managed Otto session in an
// ephemeral, trusted working dir seeded with the full schema + a read-only `q`
// tool. Each turn POSTs a question; the live session shows up via the
// `db_assist_session_started` WS event (and is mirrored in the response), and the
// agent's proposed SQL streams in via `db_assist_updated`. See db_assist.rs.

/** Run ONE DB Assistant turn (start a new assist, or resume one via `assist_id`). */
export function dbAssistStart(
  connId: string,
  body: DbAssistReq,
  signal?: AbortSignal,
): Promise<DbAssistResp> {
  return api.post<DbAssistResp>(`/connections/${connId}/db/assist`, body, signal);
}

/** Ask the assist agent to write SUMMARY.md and return its rendered markdown
 *  (the panel triggers a browser download of it). */
export function dbAssistSummary(connId: string, assistId: string): Promise<DbAssistSummaryResp> {
  return api.post<DbAssistSummaryResp>(`/connections/${connId}/db/assist/${assistId}/summary`, {});
}

/** Close an assist: kill its session + discard its working dir (Close = discard). */
export function dbAssistClose(connId: string, assistId: string): Promise<void> {
  return api.del<void>(`/connections/${connId}/db/assist/${assistId}`);
}

/** Tear down a connection's live server-side resources (`POST …/db/close`):
 *  drops the SSH tunnel, closes pooled engine connections, and cancels any
 *  in-flight queries. Fire-and-forget from the workbench — closing the tab must
 *  never block on (or surface) a failure here. */
export function dbCloseConnection(connId: string): Promise<DbCloseResp> {
  return api.post<DbCloseResp>(`/connections/${connId}/db/close`, {});
}

// --- Import connections from other DB tools ---------------------------------
//
// The daemon runs locally and reads each tool's config from its default macOS
// location — the user picks a tool, never a file. Editor-gated; created
// connections always use `secret:null` (passwords are unrecoverable from the
// source tools — the user adds them later via edit). `wsId` authorizes the
// caller; created connections are global.

/** Detect which DB tools have importable configs (GET …/import/sources). */
export function importSources(wsId: string): Promise<SourceStatus[]> {
  return api.get<SourceStatus[]>(`/workspaces/${wsId}/connections/import/sources`);
}

/** Read + parse one tool's default config into ParsedConnections (POST …/import/scan). */
export function importScan(wsId: string, source: ImportSource): Promise<ImportScanResult> {
  return api.post<ImportScanResult>(`/workspaces/${wsId}/connections/import/scan`, { source });
}

/** Best-effort batch-create the chosen connections (POST …/import/create). */
export function importCreate(wsId: string, body: ImportCreateReq): Promise<ImportCreateResult> {
  return api.post<ImportCreateResult>(`/workspaces/${wsId}/connections/import/create`, body);
}

// --- Review findings workflow ----------------------------------------------
//
// The multi-agent code review persists each finding as a tracked workflow
// record (6-state `status` + immutable event trail). These mirror the
// `/findings/*`, `/repo-rules/*`, and `/reviews/{id}/proof-pack*` endpoints
// (see crates/otto-server/src/routes/{findings,repo_rules,proof_pack}.rs).

/** List all persistent workflow findings for a completed review. */
export function listFindings(reviewId: string): Promise<Finding[]> {
  return api.get<Finding[]>(`/reviews/${reviewId}/findings`);
}

/** Fetch one finding with its full event timeline. */
export function getFinding(id: string): Promise<FindingDetail> {
  return api.get<FindingDetail>(`/findings/${id}`);
}

/** Accept an open finding (open → accepted). */
export function acceptFinding(id: string): Promise<Finding> {
  return api.post<Finding>(`/findings/${id}/accept`);
}

/** Waive a finding (→ waived). */
export function waiveFinding(id: string, reason?: string): Promise<Finding> {
  return api.post<Finding>(`/findings/${id}/waive`, { reason });
}

/** Mark a finding a false positive (→ false_positive). */
export function falsePositiveFinding(id: string, reason?: string): Promise<Finding> {
  return api.post<Finding>(`/findings/${id}/false-positive`, { reason });
}

/** Flag a finding as needing human sign-off (sets the approval gate). */
export function requireApprovalFinding(id: string): Promise<Finding> {
  return api.post<Finding>(`/findings/${id}/require-approval`);
}

/** Resolve the approval gate. `approve` clears it (open → accepted); `reject` → false_positive. */
export function approveFinding(
  id: string,
  decision: 'approve' | 'reject',
  note?: string,
): Promise<Finding> {
  return api.post<Finding>(`/findings/${id}/approve`, { decision, note });
}

/** Create a Jira issue from a finding. 400 `{code:'invalid'}` when no Jira account is configured. */
export function findingToJira(
  id: string,
  body: { project_key: string; issue_type?: string; account_id?: string },
): Promise<Finding> {
  return api.post<Finding>(`/findings/${id}/jira`, body);
}

/** Generalize a finding into a durable repo rule (fed into the Context Engine). */
export function findingToRepoRule(
  id: string,
  body?: { title?: string; body?: string; glob?: string },
): Promise<RepoRule> {
  return api.post<RepoRule>(`/findings/${id}/repo-rule`, body ?? {});
}

/** Spawn a fix agent for a finding (open|accepted → accepted; commit → fixed). */
export function fixFinding(id: string): Promise<FindingActionResp> {
  return api.post<FindingActionResp>(`/findings/${id}/fix`);
}

/** Verify a finding is resolved (accepted|fixed|verified → verified on pass). */
export function verifyFinding(id: string): Promise<FindingActionResp> {
  return api.post<FindingActionResp>(`/findings/${id}/verify`);
}

/** Spawn an agent to add a regression test (sets `linked_test`). */
export function regressionTestFinding(id: string): Promise<FindingActionResp> {
  return api.post<FindingActionResp>(`/findings/${id}/regression-test`);
}

/** List the workspace's repo rules. */
export function listRepoRules(wsId: string): Promise<RepoRule[]> {
  return api.get<RepoRule[]>(`/workspaces/${wsId}/repo-rules`);
}

/** Enable/disable a repo rule (re-materializes the workspace's rules block). */
export function toggleRepoRule(id: string, enabled: boolean): Promise<RepoRule> {
  return api.post<RepoRule>(`/repo-rules/${id}/toggle`, { enabled });
}

/** Delete a repo rule. */
export function deleteRepoRule(id: string): Promise<void> {
  return api.del<void>(`/repo-rules/${id}`);
}

/** Assemble the live Proof Pack for a review (summary + per-finding evidence). */
export function getProofPack(reviewId: string): Promise<ReviewProofPack> {
  return api.get<ReviewProofPack>(`/reviews/${reviewId}/proof-pack`);
}

/** Persist a Proof Pack snapshot (markdown) + ingest verified findings into memory. */
export function exportProofPack(
  reviewId: string,
  format?: string,
): Promise<ReviewProofPackExport> {
  return api.post<ReviewProofPackExport>(`/reviews/${reviewId}/proof-pack/export`, { format });
}

/** Build a daemon WS URL, e.g. wsUrl('/ws/term/SESSION_ID'). It never carries
 *  the token: the daemon accepts the bearer ONLY in the `otto-bearer`
 *  subprotocol (S11-312) — open it with {@link wsConnect}, or pass
 *  `[WS_BEARER_SUBPROTOCOL, token]` yourself for a non-stored token. */
export function wsUrl(path: string): string {
  const base = new URL(baseUrl());
  const proto = base.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${proto}//${base.host}${path}`;
}

/** Fixed first subprotocol paired with the bearer token on auth-by-subprotocol
 *  WebSockets (keeps the token out of the URL/query string). The server reads
 *  the token from `Sec-WebSocket-Protocol` and echoes this marker back. */
export const WS_BEARER_SUBPROTOCOL = 'otto-bearer';

/**
 * Open a WebSocket whose bearer token travels in the `Sec-WebSocket-Protocol`
 * header — the only place the daemon accepts it (a `?token=` query is refused),
 * so the token never lands in access logs. The browser offers `[WS_BEARER_SUBPROTOCOL, token]`;
 * the daemon validates the token and echoes `WS_BEARER_SUBPROTOCOL` back.
 *
 * Tokens are not valid `Sec-WebSocket-Protocol` values if they contain spaces or
 * other separators; Otto tokens are URL-safe so this is fine. When no token is
 * stored we open without a subprotocol (the server will 401).
 */
export function wsConnect(path: string): WebSocket {
  const base = new URL(baseUrl());
  const proto = base.protocol === 'https:' ? 'wss:' : 'ws:';
  const url = `${proto}//${base.host}${path}`;
  const token = getToken();
  return token
    ? new WebSocket(url, [WS_BEARER_SUBPROTOCOL, token])
    : new WebSocket(url);
}
