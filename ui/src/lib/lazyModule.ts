// A module the shell routes work to without importing it up front (perf F2).
// `use(fn)` runs `fn` with the module — synchronously once it's loaded, else
// after the dynamic import resolves. Calls made while the import is in flight
// queue on the same promise, so they still run in arrival order. A failed
// import (offline chunk fetch) drops those calls and retries on the next one.

//
// `peek()` is for events that only matter to a store someone already uses
// (perf G2): a reconnect resync, a daemon restart, a periodic metrics tick.
// `use()` there imported the store into every document — main window, pop-out,
// side pane — to run a handler that is a no-op without page state. A page
// usually imports its store STATICALLY, not through this handle, so a store
// routed by `peek()` announces itself when it evaluates (`announceModule`),
// and `peek()` finds it under the handle's key.

export interface LazyModule<T> {
  /** Run `fn` with the module, importing it first when needed. */
  use(fn: (m: T) => void): void;
  /** The module if it is already loaded — through this handle, or announced
   *  under the handle's key by the module itself — else `null`. Never
   *  imports. */
  peek(): T | null;
}

/** Modules that announced themselves at evaluation, by key. */
const announced = new Map<string, unknown>();

/** Called once at a store module's top level, so `peek()` on a handle with
 *  the same key sees it however it was first imported. */
export function announceModule(key: string, mod: unknown): void {
  announced.set(key, mod);
}

export function lazyModule<T>(load: () => Promise<T>, key?: string): LazyModule<T> {
  let mod: T | null = null;
  let pending: Promise<T> | null = null;
  return {
    use(fn) {
      if (mod) {
        fn(mod);
        return;
      }
      pending ??= load().then(
        (m) => (mod = m),
        (e: unknown) => {
          pending = null;
          throw e;
        },
      );
      pending.then(fn).catch(() => {
        /* chunk failed to load, or the handler threw — like a malformed frame */
      });
    },
    peek: () => mod ?? (key !== undefined ? ((announced.get(key) as T | undefined) ?? null) : null),
  };
}
