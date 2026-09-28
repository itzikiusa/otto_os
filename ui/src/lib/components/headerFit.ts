// PageHeader's overflow fit, the parts that decide whether (and how much)
// layout work a pass needs (perf SF-12). Pure over tiny structural types so
// the node unit tests drive it without a DOM (unit/headerFit.test.ts).
//
// The fit used to run on every mutation inside the actions / title line:
// un-hide every action (write), read every width (forced layout), write the
// reservations, then read the wrap again (a second forced layout). Now:
// - a pass whose inputs are unchanged since the last one is skipped
//   (`FitGate`: the RO-reported sizes plus a text/structure signature),
// - an action whose own signature is unchanged keeps its cached width, so a
//   collapsed one is not un-hidden just to be measured again (`WidthCache`),
// - a style write that would store the same value is dropped
//   (`StyleWriter`), so a steady-state pass invalidates no layout at all.

/** The subset of a ResizeObserverEntry the gate reads. */
export interface SizeEntry {
  target: object;
  borderBoxSize?: ReadonlyArray<{ inlineSize: number }>;
  contentRect: { width: number };
}

/** The subset of an action element a signature reads (no layout). */
export interface SigNode {
  className: string | { baseVal?: string };
  textContent: string | null;
  childElementCount: number;
  getAttribute(name: string): string | null;
}

/** One action's layout-relevant identity: class, inline style / `hidden`,
 *  text, shape, label. Anything that can change its width without changing
 *  one of these (a font-scale switch) re-fits on the next size change. */
export function nodeSig(el: SigNode): string {
  const cls = typeof el.className === 'string' ? el.className : (el.className.baseVal ?? '');
  return [
    cls,
    el.getAttribute('style') ?? '',
    el.getAttribute('hidden') ?? '-',
    el.textContent ?? '',
    el.childElementCount,
    el.getAttribute('aria-label') ?? '',
  ].join('\u0001');
}

/** Skips a fit pass whose inputs match the last one. */
export class FitGate {
  private sizes = new Map<object, number>();
  private last: string | null = null;
  /** Passes actually run (perf probes read it). */
  runs = 0;
  /** Passes skipped as unchanged. */
  skips = 0;

  /** Record RO sizes from the entries (borderBoxSize — no layout read). */
  observe(entries: readonly SizeEntry[]): void {
    for (const e of entries) {
      const w = e.borderBoxSize?.[0]?.inlineSize ?? e.contentRect.width;
      this.sizes.set(e.target, Math.round(w * 2) / 2);
    }
  }

  /** The signature of a pass: the observed sizes, then the content key. */
  signature(targets: readonly (object | undefined)[], content: string): string {
    return `${targets.map((t) => (t ? (this.sizes.get(t) ?? '?') : '-')).join('|')}\u0002${content}`;
  }

  /** True when the pass must run; records it as the latest. */
  shouldRun(sig: string): boolean {
    if (sig === this.last) {
      this.skips++;
      return false;
    }
    this.last = sig;
    this.runs++;
    return true;
  }

  /** Forget the last pass (the next one always runs). */
  invalidate(): void {
    this.last = null;
  }
}

/** Per-element width memo keyed by the element's own signature. */
export class WidthCache<E extends object> {
  private m = new WeakMap<E, { sig: string; w: number }>();
  get(el: E, sig: string): number | undefined {
    const hit = this.m.get(el);
    return hit && hit.sig === sig ? hit.w : undefined;
  }
  set(el: E, sig: string, w: number): void {
    this.m.set(el, { sig, w });
  }
}

/** The subset of CSSStyleDeclaration a writer touches. */
export interface StyleLike {
  setProperty(name: string, value: string): void;
}

/** Drops writes that would store the value already written. */
export class StyleWriter {
  private last = new WeakMap<object, Map<string, string>>();
  /** Writes that actually reached the DOM. */
  writes = 0;
  set(owner: object, style: StyleLike, name: string, value: string): boolean {
    let m = this.last.get(owner);
    if (!m) this.last.set(owner, (m = new Map()));
    if (m.get(name) === value) return false;
    m.set(name, value);
    style.setProperty(name, value);
    this.writes++;
    return true;
  }
}
