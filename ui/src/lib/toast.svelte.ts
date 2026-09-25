// Toast store — bottom-right notifications, auto-dismiss.
//
// • A repeat of a toast that is still on screen (same level, title and body)
//   doesn't stack a copy: it bumps a ×N count and restarts the timer.
// • The timer pauses while the pointer or keyboard focus is on the toast
//   (pause/resume from Toasts.svelte), so a long git message can be read.
// • An optional `action` adds one button ("Undo", "Open"): running it
//   dismisses the toast. patterns.md §7 prefers an undo over a confirm for
//   reversible actions.
// • At most MAX_VISIBLE stay up; a burst drops the oldest non-error first.

export interface ToastAction {
  label: string;
  run: () => void | Promise<void>;
}

export interface Toast {
  id: number;
  level: 'info' | 'warn' | 'error' | 'success';
  title: string;
  body?: string;
  /** How many times this exact toast fired while it was up (≥ 1). */
  count: number;
  action?: ToastAction;
}

const MAX_VISIBLE = 5;

let nextId = 1;

interface Timer {
  handle: ReturnType<typeof setTimeout> | null;
  /** Time left when paused (ms). */
  left: number;
  startedAt: number;
}

class ToastStore {
  toasts: Toast[] = $state([]);
  private timers = new Map<number, Timer>();

  push(
    level: Toast['level'],
    title: string,
    body?: string,
    ttlMs = 4500,
    opts: { action?: ToastAction } = {},
  ): number {
    const dup = this.toasts.find(
      (t) => t.level === level && t.title === title && (t.body ?? '') === (body ?? '') && !t.action && !opts.action,
    );
    if (dup) {
      this.toasts = this.toasts.map((t) => (t.id === dup.id ? { ...t, count: t.count + 1 } : t));
      this.arm(dup.id, ttlMs);
      return dup.id;
    }
    const id = nextId++;
    let next = [...this.toasts, { id, level, title, body, count: 1, action: opts.action }];
    while (next.length > MAX_VISIBLE) {
      const drop = next.find((t) => t.level !== 'error') ?? next[0];
      this.clearTimer(drop.id);
      next = next.filter((t) => t.id !== drop.id);
    }
    this.toasts = next;
    this.arm(id, ttlMs);
    return id;
  }

  info(title: string, body?: string): number {
    return this.push('info', title, body);
  }
  success(title: string, body?: string): number {
    return this.push('success', title, body);
  }
  warn(title: string, body?: string): number {
    return this.push('warn', title, body);
  }
  error(title: string, body?: string): number {
    // Errors linger longer than the 4.5s default: git failures are often
    // multi-line instructions ("your changes remain stashed — …") that 7s
    // wasn't enough to actually read.
    return this.push('error', title, body, 12000);
  }

  /** Run a toast's action, then dismiss it. */
  async runAction(id: number): Promise<void> {
    const t = this.toasts.find((x) => x.id === id);
    if (!t?.action) return;
    this.dismiss(id);
    try {
      await t.action.run();
    } catch (e) {
      this.error(`“${t.action.label}” failed`, e instanceof Error ? e.message : String(e));
    }
  }

  /** Hold the timer (pointer / focus on the toast). */
  pause(id: number): void {
    const tm = this.timers.get(id);
    if (!tm || tm.handle === null) return;
    clearTimeout(tm.handle);
    tm.handle = null;
    tm.left = Math.max(0, tm.left - (Date.now() - tm.startedAt));
  }

  /** Let it run out again, with at least a short beat to move away. */
  resume(id: number): void {
    const tm = this.timers.get(id);
    if (!tm || tm.handle !== null) return;
    this.arm(id, Math.max(tm.left, 1500));
  }

  dismiss(id: number): void {
    this.clearTimer(id);
    this.toasts = this.toasts.filter((t) => t.id !== id);
  }

  private arm(id: number, ms: number): void {
    this.clearTimer(id);
    this.timers.set(id, { handle: setTimeout(() => this.dismiss(id), ms), left: ms, startedAt: Date.now() });
  }

  private clearTimer(id: number): void {
    const tm = this.timers.get(id);
    if (tm?.handle) clearTimeout(tm.handle);
    this.timers.delete(id);
  }
}

export const toasts = new ToastStore();
