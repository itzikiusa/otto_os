// Pure native-pane geometry and transport policies. No DOM or Svelte imports:
// unit tests exercise delayed/rejected IPC without starting the desktop app.
export interface PaneBounds { x: number; y: number; width: number; height: number }
export interface NativePaneContext { host: string }

export function readNativePaneContext(tauri: boolean, value: unknown): NativePaneContext | null {
  if (!tauri || !value || typeof value !== 'object') return null;
  const host = (value as { host?: unknown }).host;
  return typeof host === 'string' && /^(main|w\d+)$/.test(host) ? { host } : null;
}

export function paneBounds(r: PaneBounds, width: number, height: number, zoom = 1): PaneBounds | null {
  if (![r.x, r.y, r.width, r.height, width, height].every(Number.isFinite)) return null;
  const x = Math.max(0, r.x), y = Math.max(0, r.y);
  const w = Math.min(width, r.x + r.width) - x;
  const h = Math.min(height, r.y + r.height) - y;
  const z = Number.isFinite(zoom) && zoom > 0 ? zoom : 1;
  return w >= 1 && h >= 1 ? { x: x * z, y: y * z, width: w * z, height: h * z } : null;
}

export function nativePanePresence(visible: boolean, focused: boolean, nativeVisible: boolean): { visible: boolean; focused: boolean } {
  return { visible: visible && nativeVisible, focused: visible && nativeVisible && focused };
}

/** One in-flight layout; obsolete intermediate resizes are dropped. Native
 * visibility changes use the same queue so a late resize cannot undo a hide. */
export class LatestPaneLayout<T> {
  private pending: { value: T } | null = null;
  private running: Promise<void> | null = null;
  private stopped = false;
  private send: (value: T) => Promise<unknown>;
  private failed: (error: unknown) => void;
  constructor(send: (value: T) => Promise<unknown>, failed: (error: unknown) => void = () => {}) {
    this.send = send;
    this.failed = failed;
  }
  push(value: T): void {
    if (this.stopped) return;
    this.pending = { value };
    if (!this.running) this.running = this.drain();
  }
  private async drain(): Promise<void> {
    while (this.pending && !this.stopped) {
      const { value } = this.pending;
      this.pending = null;
      try { await this.send(value); } catch (error) { this.failed(error); }
    }
    this.running = null;
  }
  async flush(): Promise<void> { await this.running; }
  stop(): void { this.stopped = true; this.pending = null; }
}

/** A detached pane stays usable independently of the host's split geometry. */
export function paneSurfaceVisible(showing: boolean, detached: boolean, nativeVisible?: boolean): boolean {
  return (showing || detached) && nativeVisible !== false;
}

export function mayRestorePaneFocus(mode: string | undefined, visible: boolean, blocked: boolean): boolean {
  return mode === 'attached' && visible && !blocked;
}

/** Unlike an iframe, a native child makes the host document lose focus. A
 * macOS window activation can restore host focus without a DOM focusin. */
export function nativeSideFocused(reported: boolean, hostDocumentFocused: boolean): boolean {
  return reported && !hostDocumentFocused;
}
