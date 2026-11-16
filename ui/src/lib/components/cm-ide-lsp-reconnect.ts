// Automatic reattach of the pooled language-server sockets (`/ws/lsp`) after they drop:
// a daemon restart (deploy, launchd), the daemon evicting an idle server from its LSP
// pool, or a network blip. Without it every open editor kept a dead client and silently
// lost diagnostics, completion and hover until each tab was reopened.
//
// Pure scheduling (no CodeMirror, no marimo import) so node unit tests run it directly.
// The reattach itself is the existing `otto:ide-reconnect-lsp` window event, which the
// IDE editor area answers by reconfiguring the mounted editors' language extension.

/** Backoff between attempts; after the last one the editor shows "language features
 *  unavailable" with a Retry instead of trying forever. */
export const LSP_RECONNECT_DELAYS_MS: readonly number[] = [1000, 2000, 5000, 10_000, 20_000];

export const IDE_RECONNECT_LSP_EVENT = 'otto:ide-reconnect-lsp';
export const IDE_LSP_LINK_EVENT = 'otto:ide-lsp-link';

/** `waiting`: the daemon said it is shutting down; reattach once the app is connected again.
 *  `starting` / `ready` / `unavailable` describe a server's own life (with `lang`): a socket
 *  dialled, its handshake done, or it closed before any session (no binary, refused). */
export type IdeLspLinkState = 'reconnecting' | 'waiting' | 'connected' | 'lost' | 'starting' | 'ready' | 'unavailable';
export interface IdeLspLinkDetail { rootPath: string; state: IdeLspLinkState; attempt: number; lang?: string; reason?: string }

interface ReconnectSeq { root: string; attempt: number; timer: ReturnType<typeof setTimeout> | null }

const seqs = new Map<string, ReconnectSeq>();
/** Roots waiting for the daemon to come back (after `otto/lspEvicted` reason `shutdown`). */
const waits = new Map<string, () => void>();

function normRoot(p: string): string {
  return p.replace(/\\/g, '/').replace(/\/+$/, '') || '/';
}

/** The sequence covering `root`: the same root, or a module root inside / around it (the
 *  daemon reports a canonical module root that can differ from the one first asked for). */
function findSeq(root: string): ReconnectSeq | null {
  const direct = seqs.get(root);
  if (direct) return direct;
  for (const s of seqs.values()) {
    if (root.startsWith(`${s.root}/`) || s.root.startsWith(`${root}/`)) return s;
  }
  return null;
}

function emit(type: string, detail: unknown): void {
  if (typeof window === 'undefined' || typeof CustomEvent === 'undefined') return;
  try { window.dispatchEvent(new CustomEvent(type, { detail })); } catch { /* ignore */ }
}

/** The last link events, so a status item mounted after a server started still knows it. */
const recent: IdeLspLinkDetail[] = [];
const RECENT_MAX = 40;

function emitDetail(detail: IdeLspLinkDetail): void {
  recent.push(detail);
  if (recent.length > RECENT_MAX) recent.splice(0, recent.length - RECENT_MAX);
  emit(IDE_LSP_LINK_EVENT, detail);
}

function emitLink(root: string, state: IdeLspLinkState, attempt: number): void {
  emitDetail({ rootPath: root, state, attempt });
}

/** Link events seen so far (oldest first), for a listener that mounts late. */
export function recentLspLinks(): readonly IdeLspLinkDetail[] {
  return recent.slice();
}

/** A pooled server for `lang` at `rootPath` is dialling, finished its handshake, or closed
 *  before any session (`unavailable`, with a short `reason`). */
export function noteLspStatus(rootPath: string, state: 'starting' | 'ready' | 'unavailable', lang: string, reason?: string): void {
  emitDetail({ rootPath: normRoot(rootPath), state, attempt: 0, lang, ...(reason ? { reason } : {}) });
}

/** True while a reattach sequence runs for `rootPath`: a failed attempt then must not put
 *  the language on its 30 s "unavailable" cooldown, or the next attempt would not even dial. */
export function isLspReconnecting(rootPath: string): boolean {
  return findSeq(normRoot(rootPath)) !== null;
}

/**
 * A pooled socket for `rootPath` closed (plain close, close with a reason, a failed
 * handshake, or the daemon's eviction notice: all the same here).
 * - `hadSession`: it was a working session with editors attached, so start a sequence.
 * - Otherwise it only continues a sequence already running (a reattach attempt that
 *   failed because the daemon is still down).
 */
export function noteLspSocketClosed(rootPath: string, hadSession: boolean): void {
  const root = normRoot(rootPath);
  let seq = findSeq(root);
  if (!seq) {
    if (!hadSession) return;
    seq = { root, attempt: 0, timer: null };
    seqs.set(root, seq);
  }
  if (seq.timer !== null) return;
  if (seq.attempt >= LSP_RECONNECT_DELAYS_MS.length) {
    seqs.delete(seq.root);
    emitLink(seq.root, 'lost', seq.attempt);
    return;
  }
  const delay = LSP_RECONNECT_DELAYS_MS[seq.attempt];
  seq.attempt += 1;
  emitLink(seq.root, 'reconnecting', seq.attempt);
  const current = seq;
  current.timer = setTimeout(() => {
    current.timer = null;
    if (seqs.get(current.root) !== current) return;
    emit(IDE_RECONNECT_LSP_EVENT, { rootPath: current.root });
  }, delay);
}

/** A session for `rootPath` finished its handshake: any running sequence is done. */
export function noteLspSessionReady(rootPath: string): void {
  const seq = findSeq(normRoot(rootPath));
  if (!seq) return;
  if (seq.timer !== null) clearTimeout(seq.timer);
  seqs.delete(seq.root);
  emitLink(seq.root, 'connected', seq.attempt);
}

/**
 * The daemon announced `otto/lspEvicted` with reason `shutdown`: retrying now would only
 * hit a daemon that is going down, so wait for the app's own connection to come back
 * (`waitForDaemon` calls `resume` then, and returns its unsubscribe) and reattach once.
 */
export function noteLspDaemonShutdown(
  rootPath: string,
  hadSession: boolean,
  waitForDaemon: (resume: () => void) => () => void,
): void {
  if (!hadSession) return;
  const root = normRoot(rootPath);
  cancelLspReconnects(root);
  emitLink(root, 'waiting', 0);
  let done = false;
  let off: (() => void) | null = null;
  off = waitForDaemon(() => {
    if (done) return;
    done = true;
    off?.();
    waits.delete(root);
    retryLspNow(root);
  });
  if (done) off();
  else waits.set(root, () => { done = true; off?.(); });
}

/** Stop reattaching (project switch, every editor closed, a manual Retry restarting it). */
export function cancelLspReconnects(rootPath?: string): void {
  for (const [r, stop] of [...waits.entries()]) {
    if (rootPath && r !== normRoot(rootPath)) continue;
    stop();
    waits.delete(r);
  }
  for (const s of [...seqs.values()]) {
    if (rootPath && findSeq(normRoot(rootPath)) !== s) continue;
    if (s.timer !== null) clearTimeout(s.timer);
    seqs.delete(s.root);
  }
}

/** Manual Retry from the editor after the attempts ran out. */
export function retryLspNow(rootPath: string): void {
  const root = normRoot(rootPath);
  cancelLspReconnects(root);
  seqs.set(root, { root, attempt: 0, timer: null });
  emitLink(root, 'reconnecting', 0);
  emit(IDE_RECONNECT_LSP_EVENT, { rootPath: root });
}
