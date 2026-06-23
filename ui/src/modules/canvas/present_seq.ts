// Sequence-diagram stepping for Present mode. Mermaid renders a sequence diagram
// as one SVG; to "play" it message-by-message we walk its message elements and
// toggle opacity. This is intentionally best-effort and selector-driven (mermaid
// markup is not a stable contract), so every helper degrades to "reveal all"
// when the expected nodes aren't found — Present mode keeps working, it just
// stops animating. Selectors target mermaid 11.x with the neutral theme.

/** Ordered message elements of a rendered sequence diagram, top-to-bottom.
 *  Each "message" is a label (`.messageText`) plus its line (`.messageLine0/1`);
 *  we group by document order so step N reveals the Nth message + its arrow. */
export interface SeqSteps {
  /** Groups of SVG elements, one per message, in playback order. */
  groups: SVGElement[][];
  /** Total message count (== groups.length). */
  count: number;
}

const MSG_SELECTOR = '.messageText, .messageLine0, .messageLine1';

/** Parse a rendered mermaid `<svg>` into ordered message groups. Pre-compute
 *  this ONCE on slide entry (it walks the DOM) and reuse it per step. */
export function parseSeq(svg: SVGSVGElement | null): SeqSteps {
  if (!svg) return { groups: [], count: 0 };
  // All message-related primitives in document (== vertical) order.
  const els = Array.from(svg.querySelectorAll<SVGElement>(MSG_SELECTOR));
  if (!els.length) return { groups: [], count: 0 };

  // Pair each text label with the line(s) that immediately follow/precede it.
  // mermaid emits, per message: a line element then its text (order varies by
  // version), so we bucket greedily: a new group starts at each `.messageText`.
  const groups: SVGElement[][] = [];
  let current: SVGElement[] = [];
  for (const el of els) {
    const isText = el.classList.contains('messageText');
    if (isText && current.length) {
      groups.push(current);
      current = [];
    }
    current.push(el);
  }
  if (current.length) groups.push(current);

  // Fallback: if we couldn't find any text labels (only lines), treat each line
  // as its own step so playback still has granularity.
  if (groups.length <= 1 && els.length > 1) {
    return { groups: els.map((e) => [e]), count: els.length };
  }
  return { groups, count: groups.length };
}

/** Reveal messages `0..n` (inclusive) and hide the rest, by toggling opacity.
 *  `n < 0` hides all; `n >= count` (or no steps found) reveals everything. */
export function revealUpTo(svg: SVGSVGElement | null, n: number, steps?: SeqSteps): void {
  if (!svg) return;
  const s = steps ?? parseSeq(svg);
  if (!s.count) return; // nothing to step — leave the SVG fully visible.
  s.groups.forEach((group, i) => {
    const visible = i <= n;
    for (const el of group) {
      el.style.transition = 'opacity 180ms ease';
      el.style.opacity = visible ? '1' : '0';
    }
  });
}

/** Reveal the whole diagram (clears any opacity we set). */
export function revealAll(svg: SVGSVGElement | null, steps?: SeqSteps): void {
  if (!svg) return;
  const s = steps ?? parseSeq(svg);
  for (const group of s.groups) {
    for (const el of group) el.style.opacity = '1';
  }
}
