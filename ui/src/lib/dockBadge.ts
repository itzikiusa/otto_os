// macOS dock badge = sessions waiting on you (review A1). The desktop shell's
// `set_badge_count` Tauri command (apps/desktop/src-tauri/src/main.rs) only
// honours the MAIN window — the badge is app-global — so pop-outs and embedded
// panes may call this freely; it is a no-op outside Tauri. Kept free of
// Svelte imports so node:test can cover the debounce (unit/dockBadge.test.ts).

export type BadgeSink = (count: number) => void | Promise<void>;

/** Tauri sink: lazily imports the invoke bridge; failures are swallowed (an
 *  older shell without the command must never break the page). */
export const tauriBadgeSink: BadgeSink = async (count) => {
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('set_badge_count', { count });
  } catch {
    /* not the desktop shell, or an older one */
  }
};

/**
 * A debounced badge writer: bursts of count changes (a sweep flagging several
 * sessions at once) collapse into one write `delayMs` later, and a write only
 * happens when the count differs from the last one SENT. `flush(n)` writes
 * immediately (sign-out / teardown → 0).
 */
export function badgeWriter(sink: BadgeSink, delayMs = 250) {
  let sent: number | null = null;
  let pending: number | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  const write = (n: number) => {
    if (n === sent) return;
    sent = n;
    void sink(n);
  };
  return {
    set(count: number): void {
      pending = Math.max(0, Math.floor(count));
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => {
        timer = null;
        if (pending !== null) write(pending);
        pending = null;
      }, delayMs);
    },
    flush(count: number): void {
      if (timer) clearTimeout(timer);
      timer = null;
      pending = null;
      write(Math.max(0, Math.floor(count)));
    },
  };
}
