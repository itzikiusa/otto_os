// The recorder behind lib/composerTrace.ts, loaded only while the trace is on
// for this device. It keeps the last TRACE_CAP events in a ring buffer, with
// performance.now() times: touches (first move of a gesture and whether
// anything cancelled it), the visual viewport and window scroll, the keyboard
// store, focus, and the composer's typing (input type, lengths and caret only
// — NEVER the typed text) — plus what lib/viewportFit.ts, lib/holdTouchScroll.ts
// and the caret-restore action (lib/stores/runtimeDraftCursor.ts) report
// through `trace()`. A small bar at the top of the app sends it as a scratch
// file (PUT /scratches/file → <scratches home>/<user id>/otto-traces/…),
// copies it, clears it, or turns the trace off. The bar floats (fixed) so it
// never changes the layout it is measuring.

import { setTraceSink, toggleComposerTrace } from './composerTrace';
import { onKeyboard } from './viewportFit';
import { api } from './api/client';
import { copyWithToast } from './clipboard';
import { toasts } from './toast.svelte';
import { toastError } from './toastError';

export const TRACE_CAP = 1000;

export interface TraceEntry {
  /** performance.now(), ms. */
  t: number;
  /** What happened (see the report's field list). */
  k: string;
  [field: string]: unknown;
}

/** The last `cap` entries, oldest first. */
export class TraceRing {
  private buf: TraceEntry[] = [];
  private head = 0;
  dropped = 0;
  constructor(readonly cap = TRACE_CAP) {}

  push(e: TraceEntry): void {
    if (this.buf.length < this.cap) {
      this.buf.push(e);
      return;
    }
    this.buf[this.head] = e;
    this.head = (this.head + 1) % this.cap;
    this.dropped++;
  }

  entries(): TraceEntry[] {
    return [...this.buf.slice(this.head), ...this.buf.slice(0, this.head)];
  }

  clear(): void {
    this.buf = [];
    this.head = 0;
    this.dropped = 0;
  }

  get size(): number {
    return this.buf.length;
  }
}

/** `tag.class.class.class [composer,textarea,conv]`: what an element is and where it sits. */
export function describe(el: Element | null | undefined): string | null {
  if (!el?.tagName) return null;
  const cls = String(el.getAttribute?.('class') ?? '')
    .split(/\s+/)
    .filter((c) => c && !c.startsWith('svelte-'))
    .slice(0, 3);
  const where = [
    el.closest?.('.rt-composer, .nt-composer') ? 'composer' : '',
    el.tagName === 'TEXTAREA' ? 'textarea' : '',
    el.closest?.('.rt-scroll') ? 'conv' : '',
  ].filter(Boolean);
  return [el.tagName.toLowerCase(), ...cls].join('.') + (where.length ? ` [${where.join(',')}]` : '');
}

const isElement = (v: unknown): v is Element => typeof v === 'object' && v !== null && typeof (v as Element).tagName === 'string';

/** One entry: numbers to 0.1, elements described, strings capped (callers pass tags, never text). */
export function entry(k: string, data?: Record<string, unknown>, t = performance.now()): TraceEntry {
  const out: TraceEntry = { t: Math.round(t * 10) / 10, k };
  for (const [f, v] of Object.entries(data ?? {})) {
    out[f] = typeof v === 'number' ? Math.round(v * 10) / 10 : isElement(v) ? describe(v) : typeof v === 'string' ? v.slice(0, 80) : v;
  }
  return out;
}

interface TextEventLike {
  inputType?: string;
  isComposing?: boolean;
  data?: string | null;
}
interface TextFieldLike {
  selectionStart: number | null;
  selectionEnd: number | null;
  value: string;
}

/** beforeinput / input / composition* on a text field: the input type and LENGTHS
 *  only, with the caret and the value's length (before the edit for
 *  beforeinput, after it for input). */
export function textFacts(e: TextEventLike, field: TextFieldLike): Record<string, unknown> {
  return {
    type: e.inputType ?? '',
    composing: e.isComposing,
    len: e.data?.length ?? 0,
    s: field.selectionStart,
    e: field.selectionEnd,
    vlen: field.value.length,
  };
}

/** `fn@file:line` for the first app frames of a stack (who called a patched method). */
export function callers(stack: string | undefined, skip = /composerTraceRecorder/): string {
  return (stack ?? '')
    .split('\n')
    .slice(1)
    .filter((l) => !skip.test(l))
    .slice(0, 3)
    .map((l) => {
      // V8: "at fn (url:line:col)" or "at url:line:col"; WebKit: "fn@url:line:col".
      const s = l.trim().replace(/^at\s+/, '');
      const v8 = /^(.*?)\s*\((.*)\)$/.exec(s);
      const wk = /^([^@/]*)@(.*)$/.exec(s);
      const [fn, url] = v8 ? [v8[1], v8[2]] : wk ? [wk[1], wk[2]] : ['', s];
      const m = /([^/?:]+)(?:\?[^:]*)?:(\d+)(?::\d+)?$/.exec(url);
      return `${fn || '?'}@${m ? `${m[1]}:${m[2]}` : '?'}`;
    })
    .join(' < ');
}

/** The JSON the bar sends and copies: one entry per line. */
export function traceJson(ring: TraceRing, meta: Record<string, unknown>): string {
  const lines = ring.entries().map((e) => JSON.stringify(e));
  return `{"meta":${JSON.stringify({ ...meta, dropped: ring.dropped })},"entries":[\n${lines.join(',\n')}\n]}\n`;
}

let started = false;

export function startComposerTrace(): void {
  if (started) return;
  started = true;
  const ring = new TraceRing();
  const note = (k: string, data?: Record<string, unknown>): void => ring.push(entry(k, data));
  setTraceSink(note);
  const vv = window.visualViewport;
  const meta = (): Record<string, unknown> => ({
    v: 1,
    at: new Date().toISOString(),
    ua: navigator.userAgent,
    screen: `${screen.width}x${screen.height}@${devicePixelRatio}`,
    inner: `${innerWidth}x${innerHeight}`,
    standalone: matchMedia('(display-mode: standalone)').matches || (navigator as { standalone?: boolean }).standalone === true,
    route: location.hash.split('/').slice(0, 2).join('/'),
  });
  note('start', meta());

  const composerTop = (): number | null => document.querySelector('.rt-composer, .nt-composer')?.getBoundingClientRect().top ?? null;
  const viewport = (k: string): void =>
    note(k, {
      h: vv?.height,
      top: vv?.offsetTop,
      pageTop: vv?.pageTop,
      scale: vv?.scale,
      sy: scrollY,
      se: document.scrollingElement?.scrollTop ?? null,
      appTop: document.documentElement.style.getPropertyValue('--app-top') || null,
      composerTop: composerTop(),
    });
  vv?.addEventListener('resize', () => viewport('vv-resize'));
  vv?.addEventListener('scroll', () => viewport('vv-scroll'));
  addEventListener('scroll', () => viewport('win-scroll'), { passive: true });

  let moved = false;
  let y0 = 0;
  const opt = { capture: true, passive: true };
  addEventListener(
    'touchstart',
    (e) => {
      moved = false;
      y0 = e.touches[0]?.clientY ?? 0;
      note('touchstart', { n: e.touches.length, y: y0, at: e.target as Element });
    },
    opt,
  );
  // Bubble phase on the window: after every handler, so `prevented` is the outcome.
  addEventListener(
    'touchmove',
    (e) => {
      if (moved) return;
      moved = true;
      note('move1', { n: e.touches.length, dy: (e.touches[0]?.clientY ?? y0) - y0, cancelable: e.cancelable, prevented: e.defaultPrevented });
    },
    { passive: true },
  );
  for (const type of ['touchend', 'touchcancel'] as const) {
    addEventListener(type, (e) => note(type, { n: e.touches.length, y: e.changedTouches[0]?.clientY, at: e.target as Element, top: vv?.offsetTop }), opt);
  }

  for (const type of ['focusin', 'focusout'] as const) {
    addEventListener(type, (e) => note(type, { at: e.target as Element, other: e.relatedTarget as Element | null }), true);
  }
  addEventListener(
    'keydown',
    (e) => {
      // The key's NAME only for non-characters (Enter, Backspace, Unidentified…); a typed character is just "char".
      if (e.target instanceof HTMLTextAreaElement) note('keydown', { key: e.key.length === 1 ? 'char' : e.key, composing: e.isComposing, vlen: e.target.value.length });
    },
    true,
  );
  patchTextareas(note);
  for (const type of ['beforeinput', 'input', 'compositionstart', 'compositionupdate', 'compositionend'] as const) {
    addEventListener(
      type,
      (e) => {
        if (e.target instanceof HTMLTextAreaElement) note(type, textFacts(e as InputEvent, e.target));
      },
      true,
    );
  }
  document.addEventListener('selectionchange', () => {
    const a = document.activeElement;
    if (a instanceof HTMLTextAreaElement) note('selection', { s: a.selectionStart, e: a.selectionEnd, vlen: a.value.length });
  });

  let kbOpen: boolean | null = null;
  onKeyboard((s) => {
    if (s.open === kbOpen) return;
    kbOpen = s.open;
    note('keyboard', { open: s.open, h: s.visibleHeight, inset: s.inset });
  });

  mountBar(ring, meta);
}

/** Programmatic writes to any textarea — its value, its selection, focus() and
 *  blur() — with lengths and who called them (never the value itself). */
function patchTextareas(note: (k: string, data?: Record<string, unknown>) => void): void {
  const proto = HTMLTextAreaElement.prototype;
  const value = Object.getOwnPropertyDescriptor(proto, 'value')!;
  Object.defineProperty(proto, 'value', {
    ...value,
    set(this: HTMLTextAreaElement, v: string) {
      const from = (value.get!.call(this) as string).length;
      value.set!.call(this, v);
      if (from !== v.length || from > 0) note('set-value', { from, to: v.length, by: callers(new Error().stack) });
    },
  });
  const sel = proto.setSelectionRange;
  proto.setSelectionRange = function (this: HTMLTextAreaElement, s: number | null, e: number | null, d?: 'forward' | 'backward' | 'none') {
    note('set-selection', { from: `${this.selectionStart},${this.selectionEnd}`, to: `${s},${e}`, vlen: this.value.length, by: callers(new Error().stack) });
    return sel.call(this, s, e, d);
  };
  for (const name of ['focus', 'blur'] as const) {
    const orig = HTMLElement.prototype[name];
    proto[name] = function (this: HTMLTextAreaElement, ...a: unknown[]) {
      note(`${name}()`, { vlen: this.value.length, active: document.activeElement === this, by: callers(new Error().stack) });
      return (orig as (...x: unknown[]) => void).apply(this, a);
    };
  }
}

function mountBar(ring: TraceRing, meta: () => Record<string, unknown>): void {
  const bar = document.createElement('div');
  bar.dataset.testid = 'composer-trace-bar';
  bar.style.cssText =
    'position:fixed;z-index:2147483000;inset-block-start:calc(env(safe-area-inset-top, 0px) + 2px);inset-inline-end:4px;' +
    'display:flex;gap:4px;align-items:center;padding-block:2px;padding-inline:6px;border-radius:8px;' +
    'background:var(--surface-2);border:1px solid var(--border);color:var(--text);font-size:var(--fs-xs);opacity:0.92';
  const count = document.createElement('span');
  const show = (): void => void (count.textContent = `trace ${ring.size}`);
  const button = (label: string, run: () => unknown): void => {
    const b = document.createElement('button');
    b.type = 'button';
    b.textContent = label;
    b.title = `${label} the composer trace`;
    b.style.cssText = 'font-size:var(--fs-xs);padding-block:2px;padding-inline:6px;color:inherit;background:var(--surface);border:1px solid var(--border);border-radius:6px';
    b.onclick = () => void run();
    bar.append(b);
  };
  bar.append(count);
  button('Send', async () => {
    const path = `otto-traces/composer-trace-${new Date().toISOString().replace(/[:.]/g, '-')}.json`;
    try {
      await api.put('/scratches/file', { path, content: traceJson(ring, meta()) });
      toasts.success('Trace sent', `Scratches ▸ ${path}`);
    } catch (e) {
      toastError('Couldn’t send the trace — use Copy', e);
    }
  });
  button('Copy', () => copyWithToast(traceJson(ring, meta()), 'Trace'));
  button('Clear', () => {
    ring.clear();
    show();
  });
  button('Off', toggleComposerTrace);
  show();
  addEventListener('touchend', show, { passive: true });
  (document.getElementById('app') ?? document.body).append(bar);
}
