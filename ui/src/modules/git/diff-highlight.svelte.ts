// Deferred syntax highlighting for the windowed diff. A row paints escaped
// text immediately; the line is queued and tokenized in rAF slices of at most
// SLICE_MS, then `version` bumps once per slice so the (few, mounted) rows
// re-read their now-cached HTML. Only rows that actually render ever reach
// the queue, so highlighting cost is O(viewport), never O(diff) — the old
// renderer ran hljs synchronously for every rendered line (1.2–1.4 s per 100k).
import { canHighlight, escapeHtml, highlightLine, peekHighlight } from '../../lib/hl';

/** Lines longer than this stay plain: minified/generated lines cost the most
 *  to tokenize and are unreadable highlighted anyway. */
export const HL_MAX_LINE = 500;
/** Main-thread budget per frame for tokenizing. */
const SLICE_MS = 6;
/** Rows scrolled past quickly leave work behind; drop the oldest beyond this. */
const QUEUE_MAX = 3_000;

export class DeferredHighlighter {
  /** Bumped after each tokenizing slice; readers depend on it. */
  version = $state(0);
  #queue = new Map<string, [string, string]>();
  #frame = 0;

  /** Trusted HTML for one line: highlighted when cached, escaped (and queued)
   *  otherwise. Reads `version`, so a caller in a template re-runs once the
   *  queued line is ready. */
  html(content: string, lang: string | null): string {
    void this.version;
    if (!canHighlight(lang) || content.length > HL_MAX_LINE) return escapeHtml(content);
    const hit = peekHighlight(content, lang);
    if (hit !== undefined) return hit;
    this.#enqueue(content, lang);
    return escapeHtml(content);
  }

  /** Drop queued work (the diff it belonged to is gone). */
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
      highlightLine(content, lang); // fills the shared LRU
      did++;
      if ((did & 15) === 0 && performance.now() - t0 > SLICE_MS) break;
    }
    if (did > 0) this.version++;
    if (this.#queue.size > 0) this.#frame = requestAnimationFrame(() => this.#drain());
  }
}
