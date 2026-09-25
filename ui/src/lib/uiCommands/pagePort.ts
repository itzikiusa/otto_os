// Shared plumbing for the Agent-5 module handlers (k8s, aws, brokers, vault,
// workflows, scheduled tasks, home, swarm, loops): waiting on store state the
// page fills in, and a "page port" for the modules whose view state lives in a
// component rather than a store.
//
// Why a port: handlers are registered at module load (so the catalog↔handler
// parity test sees them), but e.g. the Workflows page keeps "which workflow is
// open" as component `$state`. The page binds a tiny imperative API from an
// `$effect` (`port.bind({...})` returns the unbind) and a handler awaits it
// with `port.get(signal)` — which also covers "the page is still mounting
// after the navigation that brought it up". Plain module state, no runes, so
// the handler files stay `.ts`.

import { UiCommandError, whenMounted } from '../uiCommands';
import { confirmer } from '../confirm.svelte';

/** Poll `pred` (every animation-ish tick) until it returns a non-null,
 *  non-false value; rejects with `failed` after `timeoutMs` and with
 *  `cancelled_by_user` when `signal` aborts. */
export function waitFor<T>(
  pred: () => T | null | undefined | false,
  signal: AbortSignal,
  timeoutMs = 10_000,
  what = 'the view',
): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const started = Date.now();
    let timer: ReturnType<typeof setTimeout> | null = null;
    const onAbort = (): void => {
      if (timer) clearTimeout(timer);
      reject(new UiCommandError('cancelled_by_user', 'Cancelled'));
    };
    const tick = (): void => {
      if (signal.aborted) return onAbort();
      let v: T | null | undefined | false;
      try {
        v = pred();
      } catch (e) {
        signal.removeEventListener('abort', onAbort);
        reject(e);
        return;
      }
      if (v !== null && v !== undefined && v !== false) {
        signal.removeEventListener('abort', onAbort);
        resolve(v);
        return;
      }
      if (Date.now() - started > timeoutMs) {
        signal.removeEventListener('abort', onAbort);
        reject(new UiCommandError('failed', `Timed out waiting for ${what}`));
        return;
      }
      timer = setTimeout(tick, 50);
    };
    signal.addEventListener('abort', onAbort, { once: true });
    tick();
  });
}

/** A page-bound imperative API (see the file header). */
export interface PagePort<T> {
  /** Called by the page (in an `$effect`); returns the unbind for the cleanup. */
  bind(impl: T): () => void;
  /** The bound API, waiting up to `timeoutMs` for the page to mount. */
  get(signal: AbortSignal, timeoutMs?: number): Promise<T>;
  /** The bound API right now, or null. */
  peek(): T | null;
}

export function createPagePort<T>(what: string): PagePort<T> {
  let impl: T | null = null;
  return {
    bind(next: T): () => void {
      impl = next;
      return () => {
        if (impl === next) impl = null;
      };
    },
    get(signal: AbortSignal, timeoutMs = 10_000): Promise<T> {
      return waitFor(() => impl, signal, timeoutMs, what);
    },
    peek: () => impl,
  };
}

/** Resolve an entity by exact id, then case-insensitive name. Throws
 *  `not_found` naming the candidates so the agent can retry with a real one. */
export function resolveByIdOrName<T>(
  list: readonly T[],
  key: string,
  id: (t: T) => string,
  name: (t: T) => string,
  kind: string,
): T {
  const k = String(key ?? '').trim();
  const byId = list.find((t) => id(t) === k);
  if (byId) return byId;
  const lower = k.toLowerCase();
  const byName = list.filter((t) => name(t).toLowerCase() === lower);
  if (byName.length === 1) return byName[0];
  if (byName.length > 1) {
    throw new UiCommandError('invalid_args', `More than one ${kind} is named “${k}” — pass its id.`);
  }
  const known = list.slice(0, 20).map((t) => `${name(t)} (${id(t)})`).join(', ');
  throw new UiCommandError('not_found', `No ${kind} “${k}”.${known ? ` Known: ${known}` : ''}`);
}

/** Cap a list for the agent-facing result, saying so when it was cut. */
export function capList<T>(items: readonly T[], max = 200): { items: T[]; total: number; truncated: boolean } {
  return { items: items.slice(0, max), total: items.length, truncated: items.length > max };
}

/** Keep the LAST `max` characters of a long text (logs), marking the cut. */
export function tailText(text: string, max = 60_000): { text: string; truncated: boolean } {
  if (text.length <= max) return { text, truncated: false };
  return { text: text.slice(text.length - max), truncated: true };
}

/** Error text from anything thrown. */
export function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Wrap a store/API failure as a `failed` UiCommandError (UiCommandErrors pass through). */
export function asUiError(e: unknown): Error {
  if (e instanceof UiCommandError) return e;
  return new UiCommandError('failed', errText(e));
}

/** The attributed title for a confirm raised on the agent's behalf. */
export function agentLabel(agent: { title: string; provider: string }): string {
  const who = agent.provider ? agent.provider.charAt(0).toUpperCase() + agent.provider.slice(1) : 'Agent';
  return agent.title ? `${who} · ${agent.title}` : who;
}

/** Highlight `selector` once it renders (≤ `timeoutMs`); never fails the
 *  command — the highlight is a courtesy, the result is what matters. */
export async function highlightWhenReady(
  ctx: { highlight(el: Element | string | null): void; signal: AbortSignal },
  selector: string,
  timeoutMs = 3000,
): Promise<void> {
  if (typeof document === 'undefined') return;
  try {
    ctx.highlight(await whenMounted(selector, ctx.signal, timeoutMs));
  } catch {
    /* not rendered (phone layout, collapsed pane…) — skip the outline */
  }
}

/** Await a dialog (confirmer / confirmOutward / typed confirm) that the
 *  command's Stop or deadline must close: on abort the open dialog is
 *  dismissed, so it resolves as a cancel instead of lingering. */
export async function dismissOnAbort<T>(signal: AbortSignal, dialog: Promise<T>): Promise<T> {
  let settled = false;
  const onAbort = (): void => {
    if (!settled) confirmer.dismiss();
  };
  signal.addEventListener('abort', onAbort, { once: true });
  try {
    return await dialog;
  } finally {
    settled = true;
    signal.removeEventListener('abort', onAbort);
  }
}
