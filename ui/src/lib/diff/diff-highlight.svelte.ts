// Deferred syntax highlighting for windowed diff and blame views. A row paints
// escaped text immediately; the line is queued and tokenized in rAF slices of
// at most SLICE_MS, then `version` bumps once per slice so mounted rows
// re-read their cached HTML.
import { canHighlight, escapeHtml, highlightLine, peekHighlight } from '../hl';

export const HL_MAX_LINE = 500;
const SLICE_MS = 6;
const QUEUE_MAX = 3_000;

export class DeferredHighlighter {
  version = $state(0);
  #queue = new Map<string, [string, string]>();
  #frame = 0;

  html(content: string, lang: string | null): string {
    void this.version;
    if (!canHighlight(lang) || content.length > HL_MAX_LINE) return escapeHtml(content);
    const hit = peekHighlight(content, lang);
    if (hit !== undefined) return hit;
    this.#enqueue(content, lang);
    return escapeHtml(content);
  }

  clear(): void {
    this.#queue.clear();
    if (this.#frame) cancelAnimationFrame(this.#frame);
    this.#frame = 0;
  }

  #enqueue(content: string, lang: string): void {
    const key = `${lang} ${content}`;
    if (this.#queue.has(key)) return;
    if (this.#queue.size >= QUEUE_MAX) {
      const oldest = this.#queue.keys().next().value;
      if (oldest !== undefined) this.#queue.delete(oldest);
    }
    this.#queue.set(key, [content, lang]);
    if (!this.#frame) this.#frame = requestAnimationFrame(() => this.#drain());
  }

  #drain(): void {
    this.#frame = 0;
    const t0 = performance.now();
    let did = 0;
    for (const [key, [content, lang]] of this.#queue) {
      this.#queue.delete(key);
      highlightLine(content, lang);
      did++;
      if ((did & 15) === 0 && performance.now() - t0 > SLICE_MS) break;
    }
    if (did > 0) this.version++;
    if (this.#queue.size > 0) this.#frame = requestAnimationFrame(() => this.#drain());
  }
}
