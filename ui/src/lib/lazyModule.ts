// A module the shell routes work to without importing it up front (perf F2).
// `use(fn)` runs `fn` with the module — synchronously once it's loaded, else
// after the dynamic import resolves. Calls made while the import is in flight
// queue on the same promise, so they still run in arrival order. A failed
// import (offline chunk fetch) drops those calls and retries on the next one.

export interface LazyModule<T> {
  /** Run `fn` with the module, importing it first when needed. */
  use(fn: (m: T) => void): void;
  /** The module if already loaded through this handle, else `null`. */
  peek(): T | null;
}

export function lazyModule<T>(load: () => Promise<T>): LazyModule<T> {
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
    peek: () => mod,
  };
}
