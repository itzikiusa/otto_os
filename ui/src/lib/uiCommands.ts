// Agent UI control — the runtime (docs/features/agent-ui-control.md).
//
// An agent session calls an `otto.ui_*` tool; the daemon picks ONE document of
// the user's window (ws.md §2: the `hello` / `presence` frames this module
// builds) and sends it a `ui_command` frame on that document's `/ws/events`
// socket. Here the frame finds the module's handler, runs it with a
// {@link UiCommandCtx} (attribution, AbortSignal, progress, highlight, an
// attributed write confirm) and POSTs the outcome to
// `/ui/commands/{id}/result` with this socket's `X-Otto-Ui-Conn`.
//
// Handlers register per module at load (`ui/src/lib/uiCommands/<module>.ts`,
// all imported from `uiCommands/index.ts`); the set of registered names is the
// `capabilities` list the daemon routes by. The catalog every name must match
// is `docs/contracts/ui-commands.json` (a unit test pins the two together).
//
//   registerUiCommands('connections', {
//     db_run_query: async (args, ctx) => {
//       const el = await whenMounted('[data-testid="query-editor"]', ctx.signal);
//       ctx.highlight(el);
//       …
//       return { rows };
//     },
//   });

import { baseUrl, getToken } from './api/client';
import type { Id, UiAgentRef, UiCommandErrorCode, UiCommandFrame, UiHelloFrame, UiPresenceFrame } from './api/types';
import { confirmer } from './confirm.svelte';
import { isEmbedded } from './desktop';
import { hostWindowId } from './embedGuest';
import { paneKey, routeOf } from './sidePane';
import { clientId } from './stores/ui.svelte';
import { agentName, uiControl } from './stores/uiControl.svelte';
import { windowId } from './win';
import {
  HUMAN_WAIT_MS,
  SEEN_IDS_MAX,
  absoluteDeadline,
  encodeResult,
  nearestDelta,
  readServerFrame,
} from './uiCommands/frames';

// ─── Handler API ────────────────────────────────────────────────────────────

export interface UiCommandCtx {
  /** The command id (echoed in progress / result). */
  id: string;
  /** The agent session this runs for — attribute anything it creates to it. */
  agent: { session_id: string; title: string; provider: string };
  /** Aborted on `ui_command_cancel` (deadline, the user's Stop, a revoked grant). */
  signal: AbortSignal;
  /** Tell the daemon (and the driving bar) what's happening. `awaitingHuman`
   *  extends the daemon's deadline to ≤ 120 s while a person decides. */
  progress(note: string, awaitingHuman?: boolean): void;
  /** Outline an element (`data-agent-target`) and scroll it into view —
   *  instantly under reduced motion. A selector is looked up in this document;
   *  `null` clears this command's highlight. */
  highlight(el: Element | string | null): void;
  /**
   * Attributed confirm for a write the agent wants to make ("Claude · ‹session›
   * wants to …"). With `connId` (any stable scope key: a connection id,
   * `k8s:<cluster>`, `vault:<id>` …) it offers "Allow writes on ‹where› for
   * this session" and honours that choice afterwards without asking.
   * `guarded` (prod / guarded targets) never offers or honours the memory;
   * `outward: true` does the same (outward actions always confirm).
   * Resolves false on Cancel — handlers then throw
   * `new UiCommandError('cancelled_by_user', …)`.
   */
  confirmWrite(opts: {
    what: string;
    where: string;
    connId?: string;
    /** The confirm button's verb ("Run", "Commit", "Send"). Default "Run". */
    verb?: string;
    guarded?: boolean;
    outward?: boolean;
  }): Promise<boolean>;
}

export type UiHandler = (args: any, ctx: UiCommandCtx) => Promise<unknown>;

/** A handler's refusal, reported to the agent as `{code, message}`. */
export class UiCommandError extends Error {
  code: UiCommandErrorCode | string;
  constructor(code: UiCommandErrorCode | string, message: string) {
    super(message);
    this.name = 'UiCommandError';
    this.code = code;
  }
}

interface Registered {
  module: string;
  handler: UiHandler;
}

const handlers = new Map<string, Registered>();
const capabilityListeners = new Set<() => void>();

function capabilitiesChanged(): void {
  for (const fn of capabilityListeners) fn();
}

/**
 * Register a module's handlers (at module load). Names are catalog names
 * (`db_run_query`, not the tool name). A duplicate name replaces the earlier
 * handler (hot reload) with a console warning. Returns the unregister.
 */
export function registerUiCommands(module: string, map: Record<string, UiHandler>): () => void {
  const names = Object.keys(map);
  for (const name of names) {
    const prev = handlers.get(name);
    if (prev && prev.module !== module) {
      console.warn(`[uiCommands] "${name}" re-registered by ${module} (was ${prev.module})`);
    }
    handlers.set(name, { module, handler: map[name] });
  }
  capabilitiesChanged();
  return () => {
    for (const name of names) {
      if (handlers.get(name)?.handler === map[name]) handlers.delete(name);
    }
    capabilitiesChanged();
  };
}

/** The registered command names (what `hello.capabilities` reports). */
export function uiCapabilities(): string[] {
  return [...handlers.keys()].sort();
}

/** The module a registered command belongs to. */
export function uiCommandModule(name: string): string | null {
  return handlers.get(name)?.module ?? null;
}

/** Called when the registered set changes (the events client re-sends `hello`). */
export function onUiCapabilitiesChanged(fn: () => void): () => void {
  capabilityListeners.add(fn);
  return () => capabilityListeners.delete(fn);
}

// ─── Module view state (for `ui_state`) ─────────────────────────────────────

const stateProviders = new Map<string, () => unknown>();

/**
 * Contribute a module's view state to `otto.ui_state` ("which connection and
 * tab are open, what the grid shows"). Keep it small and free of secrets or
 * row data. Returns the unregister.
 */
export function registerUiState(module: string, provider: () => unknown): () => void {
  stateProviders.set(module, provider);
  return () => {
    if (stateProviders.get(module) === provider) stateProviders.delete(module);
  };
}

/** This document's view state for its current module (`null` when the module
 *  registered none, or it threw). */
export function localModuleState(): unknown {
  const p = stateProviders.get(describeDocument().module);
  if (!p) return null;
  try {
    return p() ?? null;
  } catch {
    return null;
  }
}

// ─── Document identity (hello / presence) ───────────────────────────────────

function currentRoute(): string {
  return typeof window === 'undefined' ? '' : routeOf(window.location.hash);
}

/** The main document is "focused" only while focus is in IT — a focused side
 *  pane iframe makes the parent's `hasFocus()` true as well. */
function documentFocused(): boolean {
  if (typeof document === 'undefined') return false;
  if (!document.hasFocus()) return false;
  return !(document.activeElement instanceof HTMLIFrameElement);
}

export interface UiDocumentInfo {
  pane: 'main' | 'side';
  window_id: string;
  host_window_id?: string;
  route: string;
  module: string;
  focused: boolean;
  visible: boolean;
}

export function describeDocument(): UiDocumentInfo {
  const route = currentRoute();
  const host = isEmbedded ? hostWindowId() : null;
  return {
    pane: isEmbedded ? 'side' : 'main',
    // The side pane has no window of its own: it reports its host's id.
    window_id: host ?? windowId,
    ...(host ? { host_window_id: host } : {}),
    route,
    module: paneKey(route),
    focused: documentFocused(),
    visible: typeof document === 'undefined' ? false : document.visibilityState === 'visible',
  };
}

export function helloFrame(): UiHelloFrame {
  const d = describeDocument();
  return {
    type: 'hello',
    client_id: clientId(),
    window_id: d.window_id,
    pane: d.pane,
    ...(d.host_window_id ? { host_window_id: d.host_window_id } : {}),
    route: d.route,
    module: d.module,
    focused: d.focused,
    visible: d.visible,
    capabilities: uiCapabilities(),
  };
}

export function presenceFrame(): UiPresenceFrame {
  const d = describeDocument();
  return { type: 'presence', route: d.route, module: d.module, focused: d.focused, visible: d.visible };
}

// ─── Runtime: frames in, results out ────────────────────────────────────────

let connId: string | null = null;

/** The socket's `conn_id` (from `hello_ack`); null while disconnected. */
export function uiConnId(): string | null {
  return connId;
}

interface Running {
  id: string;
  sessionId: Id;
  controller: AbortController;
  guard: ReturnType<typeof setTimeout> | null;
  /** Result must go back on the connection the command came in on. */
  conn: string;
}

const running = new Map<string, Running>();
const seen: string[] = [];

async function postToDaemon(path: string, conn: string, body: string): Promise<void> {
  const token = getToken();
  const headers: Record<string, string> = { 'Content-Type': 'application/json', 'X-Otto-Ui-Conn': conn };
  if (token) headers['Authorization'] = `Bearer ${token}`;
  const resp = await fetch(`${baseUrl()}/api/v1${path}`, { method: 'POST', headers, body });
  if (!resp.ok && resp.status !== 404 && resp.status !== 409) {
    // 404/409: the daemon already gave up on it (cancelled / timed out).
    console.warn(`[uiCommands] ${path} → ${resp.status}`);
  }
}

function postResult(r: Running, body: string): void {
  void postToDaemon(`/ui/commands/${encodeURIComponent(r.id)}/result`, r.conn, body).catch(() => {
    /* daemon unreachable — it times the command out on its side */
  });
}

function postError(r: Running, code: string, message: string): void {
  postResult(r, JSON.stringify({ ok: false, error: { code, message: message.slice(0, 2000) } }));
}

function armGuard(r: Running, until: number): void {
  if (r.guard) clearTimeout(r.guard);
  // A little after the daemon's own deadline: it sends the cancel; this only
  // covers a socket that died without one.
  r.guard = setTimeout(() => abortOne(r.id, 'Timed out'), Math.max(0, until - Date.now()) + 2000);
}

function abortOne(id: string, reason: string): void {
  const r = running.get(id);
  if (!r) return;
  running.delete(id);
  if (r.guard) clearTimeout(r.guard);
  r.controller.abort(new UiCommandError('cancelled', reason || 'Cancelled'));
}

/** Abort every running command (socket closed). */
export function cancelAllUiCommands(reason: string): void {
  for (const id of [...running.keys()]) abortOne(id, reason);
}

function abortSession(sessionId: Id, reason: string): void {
  for (const r of [...running.values()]) if (r.sessionId === sessionId) abortOne(r.id, reason);
}
uiControl.installAbort(abortSession);

// Highlights: one attribute, cleared after a beat. Many can coexist (one per
// command); a new highlight from the same command replaces its previous one.
const HIGHLIGHT_MS = 2400;
const highlightTimers = new WeakMap<Element, ReturnType<typeof setTimeout>>();

function reducedMotion(): boolean {
  try {
    return window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  } catch {
    return false;
  }
}

function scrollsOn(style: CSSStyleDeclaration, axis: 'x' | 'y'): boolean {
  const v = axis === 'y' ? style.overflowY : style.overflowX;
  return v === 'auto' || v === 'scroll' || v === 'overlay';
}

/**
 * Bring `el` into view by scrolling only the containers a PERSON can scroll
 * (`overflow: auto|scroll`), nearest edge, and only when it's out of view.
 * `scrollIntoView` also scrolls `overflow: hidden` / `clip` ancestors, which
 * shoved whole page regions (a Database tab strip) out of reach for good.
 */
export function revealElement(el: Element): void {
  const behavior: ScrollBehavior = reducedMotion() ? 'auto' : 'smooth';
  // Past the first scroller, the thing to reveal is that scroller itself
  // (its inner scroll may still be animating).
  let target: Element = el;
  let node = el.parentElement;
  while (node && node !== document.body && node !== document.documentElement) {
    const style = getComputedStyle(node);
    const y = scrollsOn(style, 'y') && node.scrollHeight > node.clientHeight;
    const x = scrollsOn(style, 'x') && node.scrollWidth > node.clientWidth;
    if (x || y) {
      const box = node.getBoundingClientRect();
      const r = target.getBoundingClientRect();
      let top = 0;
      let left = 0;
      if (y) top = nearestDelta(r.top, r.bottom, box.top, box.bottom);
      if (x) left = nearestDelta(r.left, r.right, box.left, box.right);
      if (top !== 0 || left !== 0) node.scrollBy({ top, left, behavior });
      target = node;
    }
    node = node.parentElement;
  }
}

/** Outline + scroll into view (see {@link UiCommandCtx.highlight}). */
export function highlightTarget(el: Element): void {
  const prev = highlightTimers.get(el);
  if (prev) clearTimeout(prev);
  el.setAttribute('data-agent-target', '');
  try {
    revealElement(el);
  } catch {
    /* detached */
  }
  highlightTimers.set(
    el,
    setTimeout(() => {
      el.removeAttribute('data-agent-target');
      highlightTimers.delete(el);
    }, HIGHLIGHT_MS),
  );
}

function clearHighlight(el: Element): void {
  const t = highlightTimers.get(el);
  if (t) clearTimeout(t);
  highlightTimers.delete(el);
  el.removeAttribute('data-agent-target');
}

/**
 * Resolve once an element matching `selector` is in this document (a handler
 * that navigated waits for the page to mount). Rejects with
 * `UiCommandError('failed')` after `timeoutMs`, or the signal's reason when aborted.
 */
export function whenMounted(selector: string, signal?: AbortSignal, timeoutMs = 10_000): Promise<Element> {
  return new Promise((resolve, reject) => {
    const hit = document.querySelector(selector);
    if (hit) return resolve(hit);
    if (signal?.aborted) return reject(signal.reason);
    let done = false;
    const finish = (fn: () => void): void => {
      if (done) return;
      done = true;
      mo.disconnect();
      clearTimeout(timer);
      signal?.removeEventListener('abort', onAbort);
      fn();
    };
    const mo = new MutationObserver(() => {
      const el = document.querySelector(selector);
      if (el) finish(() => resolve(el));
    });
    mo.observe(document.documentElement, { childList: true, subtree: true, attributes: true });
    const timer = setTimeout(
      () => finish(() => reject(new UiCommandError('failed', `The page didn't show ${selector} in time`))),
      timeoutMs,
    );
    const onAbort = (): void => finish(() => reject(signal?.reason));
    signal?.addEventListener('abort', onAbort, { once: true });
  });
}

/** Race a promise against the signal. */
function untilAborted<T>(p: Promise<T>, signal: AbortSignal): Promise<T> {
  if (signal.aborted) return Promise.reject(signal.reason);
  return new Promise<T>((resolve, reject) => {
    const onAbort = (): void => reject(signal.reason);
    signal.addEventListener('abort', onAbort, { once: true });
    p.then(
      (v) => {
        signal.removeEventListener('abort', onAbort);
        resolve(v);
      },
      (e) => {
        signal.removeEventListener('abort', onAbort);
        reject(e);
      },
    );
  });
}

function makeCtx(frame: UiCommandFrame, r: Running): UiCommandCtx {
  const agent: UiAgentRef = frame.agent;
  const signal = r.controller.signal;
  const lit = new Set<Element>();
  signal.addEventListener('abort', () => {
    for (const el of lit) clearHighlight(el);
  }, { once: true });

  const progress = (note: string, awaitingHuman = false): void => {
    if (signal.aborted) return;
    uiControl.note(r.id, note, awaitingHuman);
    if (awaitingHuman) armGuard(r, Date.now() + HUMAN_WAIT_MS);
    void postToDaemon(
      `/ui/commands/${encodeURIComponent(r.id)}/progress`,
      r.conn,
      JSON.stringify({ note: note.slice(0, 500), awaiting_human: awaitingHuman }),
    ).catch(() => {});
  };

  return {
    id: frame.id,
    agent,
    signal,
    progress,
    highlight(target) {
      for (const el of lit) clearHighlight(el);
      lit.clear();
      if (target === null || signal.aborted) return;
      const el = typeof target === 'string' ? document.querySelector(target) : target;
      if (!el) return;
      lit.add(el);
      highlightTarget(el);
    },
    async confirmWrite(opts) {
      const scope = opts.connId?.trim() || '';
      const rememberable = !!scope && !opts.guarded && !opts.outward;
      if (rememberable && uiControl.writesAllowed(agent.session_id, scope)) return true;
      if (signal.aborted) return false;
      const who = agentName(agent);
      const verb = opts.verb?.trim() || 'Run';
      progress('Waiting for you to confirm', true);
      let settled = false;
      const asked = confirmer
        .choose(`${who} wants to ${verb.toLowerCase()} this on ${opts.where}:\n\n${opts.what}`, {
          title: `Allow ${verb.toLowerCase()} on ${opts.where}?`,
          options: [{ label: verb, value: 'run', kind: opts.guarded ? 'danger' : 'primary' }],
          checkboxLabel: rememberable ? `Allow writes on ${opts.where} for this session` : undefined,
        })
        .finally(() => (settled = true));
      // Stop / deadline while the dialog is up: close it (if it's still ours).
      const onAbort = (): void => {
        if (!settled) confirmer.dismiss();
      };
      signal.addEventListener('abort', onAbort, { once: true });
      try {
        const { value, remember } = await asked;
        if (value !== 'run' || signal.aborted) return false;
        if (remember && rememberable) uiControl.rememberWrites(agent.session_id, scope);
        progress(`${verb} confirmed`);
        return true;
      } finally {
        signal.removeEventListener('abort', onAbort);
      }
    },
  };
}

async function run(frame: UiCommandFrame): Promise<void> {
  const conn = connId;
  if (!conn) return; // no hello_ack yet — the daemon won't have routed here
  if (seen.includes(frame.id) || running.has(frame.id)) return;
  seen.push(frame.id);
  if (seen.length > SEEN_IDS_MAX) seen.splice(0, seen.length - SEEN_IDS_MAX);

  const r: Running = { id: frame.id, sessionId: frame.session_id, controller: new AbortController(), guard: null, conn };
  running.set(frame.id, r);
  armGuard(r, absoluteDeadline(frame.deadline_ms, Date.now()));

  const reg = handlers.get(frame.command);
  if (!reg) {
    running.delete(frame.id);
    if (r.guard) clearTimeout(r.guard);
    postError(r, 'not_found', `This Otto window doesn't implement ${frame.command} (update the app?)`);
    return;
  }

  uiControl.begin(frame.id, frame.agent, frame.command, reg.module);
  const ctx = makeCtx(frame, r);
  try {
    const result = await untilAborted(reg.handler(frame.args, ctx), r.controller.signal);
    if (r.controller.signal.aborted) return;
    const body = encodeResult(result);
    if (body === null) postError(r, 'failed', 'The result was too large or not serialisable');
    else postResult(r, body);
    uiControl.end(frame.id, true);
  } catch (e) {
    const aborted = r.controller.signal.aborted;
    const err = e instanceof UiCommandError ? e : null;
    const message = e instanceof Error ? e.message : String(e);
    uiControl.end(frame.id, false, aborted ? 'Stopped' : message);
    // A cancelled command's daemon side is already gone — nothing to report.
    if (!aborted) postError(r, err ? err.code : 'failed', message || 'The command failed');
  } finally {
    running.delete(frame.id);
    if (r.guard) clearTimeout(r.guard);
  }
}

/**
 * The events client hands every parsed server frame here first. True = it was
 * a UI-protocol frame (handled); false = an ordinary `OttoEvent`.
 */
export function handleUiFrame(data: unknown): boolean {
  const frame = readServerFrame(data);
  if (!frame) return false;
  switch (frame.type) {
    case 'hello_ack':
      connId = frame.conn_id;
      break;
    case 'ui_command':
      void run(frame);
      break;
    case 'ui_command_cancel':
      abortOne(frame.id, frame.reason);
      break;
  }
  return true;
}

/** The socket closed: its conn id is dead and nothing can report back. */
export function uiSocketClosed(): void {
  connId = null;
  cancelAllUiCommands('Disconnected');
}
