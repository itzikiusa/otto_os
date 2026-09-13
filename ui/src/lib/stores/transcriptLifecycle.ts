/** Window-local ownership and bounded recovery reads. Closing a view revokes
 * its generation even if the transport ignores AbortSignal. */
export class TranscriptLifecycle {
  /** Conservative incoming-payload charge with bounded work and no serialized copy.
   * Saturation makes an oversized reader eligible for eviction after release. */
  static payloadCharge(value: unknown): number {
    const cap = 32 * 1024 * 1024 + 1;
    let bytes = 0, nodes = 0;
    function visit(value: unknown, depth: number): void {
      if (bytes >= cap) return;
      if (++nodes > 16_384 || depth > 32) { bytes = cap; return; }
      if (typeof value === 'string') { bytes += 32 + value.length * 2; return; }
      if (value === null || typeof value !== 'object') { bytes += 16; return; }
      bytes += 64;
      if (Array.isArray(value)) {
        bytes += value.length * 8;
        for (const item of value) { if (bytes >= cap) break; visit(item, depth + 1); }
      } else {
        for (const key in value) {
          if (bytes >= cap) break;
          if (Object.hasOwn(value, key)) { bytes += 32 + key.length * 2; visit((value as Record<string, unknown>)[key], depth + 1); }
        }
      }
    }
    visit(value, 0);
    return Math.min(cap, bytes * 2);
  }

  private entries = new Map<string, {refs: number; run: (signal: AbortSignal) => Promise<void>; pending: boolean; controller: AbortController | null}>();
  private running = 0;
  private visible = true;

  held(key: string): boolean { return (this.entries.get(key)?.refs ?? 0) > 0; }
  active(key: string): boolean { return this.visible && this.held(key); }

  acquire(key: string, run: (signal: AbortSignal) => Promise<void>): () => void {
    let entry = this.entries.get(key);
    if (entry) entry.refs++;
    else {
      entry = {refs: 1, run, pending: true, controller: null};
      this.entries.set(key, entry);
      this.drain();
    }
    const owned = entry;
    let released = false;
    return () => {
      if (released) return;
      released = true;
      if (--owned.refs === 0) {
        owned.pending = false;
        owned.controller?.abort();
        if (this.entries.get(key) === owned) this.entries.delete(key);
      }
    };
  }

  request(key?: string): void {
    for (const [id, entry] of this.entries) if (key === undefined || key === id) entry.pending = true;
    this.drain();
  }

  setVisible(visible: boolean): void {
    if (this.visible === visible) return;
    this.visible = visible;
    for (const entry of this.entries.values()) {
      entry.pending = visible;
      if (!visible) entry.controller?.abort();
    }
    this.drain();
  }

  clear(): void {
    for (const entry of this.entries.values()) entry.controller?.abort();
    this.entries.clear();
  }

  private drain(): void {
    if (!this.visible) return;
    for (const [key, entry] of this.entries) {
      if (this.running >= 2) break;
      if (!entry.pending || entry.controller) continue;
      entry.pending = false;
      const ac = new AbortController();
      entry.controller = ac;
      this.running++;
      void entry.run(ac.signal).catch(() => {/* The conversation owns its error. */}).finally(() => {
        this.running--;
        entry.controller = null;
        if (this.entries.get(key) !== entry) entry.pending = false;
        this.drain();
      });
    }
  }
}
