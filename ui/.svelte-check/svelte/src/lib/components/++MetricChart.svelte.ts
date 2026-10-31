///<reference types="svelte" />
;// Generic time-series chart — hand-rolled inline SVG, no charting dependency
// (sibling of modules/database/Chart.svelte, which stays QueryResult-bound).
// Multi-series line/area over a shared time axis: y ticks in human units
// (bytes / percent / seconds / rates), x ticks in local time, gaps (`null`)
// break the line, hover snaps to the nearest sample and shows time + every
// series' value. The keyboard gets the same inspection: the plot is a slider
// over the samples (←/→ step, Home/End jump, PageUp/PageDown by ten) that
// moves the crosshair + tooltip and reads the values out, and a "Show data"
// disclosure renders the whole series as a table. Width is fluid (`viewBox` + `width:100%`), so it fits a
// phone; colours come from the theme's CSS vars. Shapes + formatters live
// in `lib/metric-format.ts` so stat rows can share them.

import {
    formatMetric,
    formatTimeTick,
    type MetricChartPoint,
    type MetricChartSeries,
    type MetricChartUnit,
  } from '../metric-format';
import { chartColor } from './chartPalette';
;

  interface Props {
    series: MetricChartSeries[];
    unit?: MetricChartUnit;
    /** Pixel height of the plot (the width follows the container). */
    height?: number;
    showLegend?: boolean;
    /** Fill under each line. */
    area?: boolean;
    /** Shown when every series is empty / all-null. */
    emptyText?: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  let {
    series,
    unit = 'count',
    height = 160,
    showLegend = true,
    area = true,
    emptyText = 'No data',
  }: Props = $props();

  // Match SVG coordinates to CSS pixels so narrow dashboard cards do not
  // squeeze 11px axis labels into unreadable 4px-wide text.
  let plotWidth = $state(600);
  const W = $derived(Math.max(160, plotWidth));
  const PAD = { l: 44, r: 10, t: 10, b: 22 };

  // ── formatting ─────────────────────────────────────────────────────────────
  const fmt = (v: number) => formatMetric(v, unit);
  const fmtTick = formatTimeTick;
  function fmtFull(t: number): string {
    return new Date(t).toLocaleString();
  }

  // ── domain ─────────────────────────────────────────────────────────────────
  const colorOf = (i: number) => series[i]?.color ?? chartColor(i);

  const hasData = $derived(series.some((s) => s.points.some((p) => p.v != null)));

  const xDomain = $derived.by(() => {
    let lo = Infinity;
    let hi = -Infinity;
    for (const s of series)
      for (const p of s.points) {
        if (p.t < lo) lo = p.t;
        if (p.t > hi) hi = p.t;
      }
    if (!Number.isFinite(lo)) return { lo: 0, hi: 1 };
    if (hi === lo) hi = lo + 60_000;
    return { lo, hi };
  });

  /** "Nice" y ceiling so ticks land on round numbers (binary steps for bytes). */
  function niceCeil(v: number): number {
    if (v <= 0) return 1;
    if (unit === 'bytes' || unit === 'bytes_per_sec') {
      let base = 1;
      while (v / base >= 1024) base *= 1024;
      const m = v / base;
      const step = m <= 1 ? 1 : m <= 2 ? 2 : m <= 4 ? 4 : m <= 8 ? 8 : m <= 16 ? 16 : m <= 32 ? 32 : m <= 64 ? 64 : m <= 128 ? 128 : m <= 256 ? 256 : m <= 512 ? 512 : 1024;
      return step * base;
    }
    const exp = Math.floor(Math.log10(v));
    const base = 10 ** exp;
    const m = v / base;
    const step = m <= 1 ? 1 : m <= 2 ? 2 : m <= 2.5 ? 2.5 : m <= 5 ? 5 : 10;
    return step * base;
  }

  const yDomain = $derived.by(() => {
    let lo = 0;
    let hi = 0;
    for (const s of series)
      for (const p of s.points) {
        if (p.v == null || !Number.isFinite(p.v)) continue;
        if (p.v < lo) lo = p.v;
        if (p.v > hi) hi = p.v;
      }
    if (unit === 'percent' && hi <= 100 && hi > 50) hi = 100;
    else hi = niceCeil(hi);
    if (lo < 0) lo = -niceCeil(-lo);
    if (hi === lo) hi = lo + 1;
    return { lo, hi };
  });

  const H = $derived(Math.max(80, height));
  const plotW = $derived(W - PAD.l - PAD.r);
  const plotH = $derived(H - PAD.t - PAD.b);

  function x(t: number): number {
    const { lo, hi } = xDomain;
    return PAD.l + (plotW * (t - lo)) / (hi - lo);
  }
  function y(v: number): number {
    const { lo, hi } = yDomain;
    return PAD.t + plotH - (plotH * (v - lo)) / (hi - lo);
  }
  const y0 = $derived(y(Math.max(0, yDomain.lo)));

  /** Line path per series — a `null` breaks the segment (gap). */
  function linePath(points: MetricChartPoint[]): string {
    let d = '';
    let pen = false;
    for (const p of points) {
      if (p.v == null || !Number.isFinite(p.v)) {
        pen = false;
        continue;
      }
      d += `${pen ? 'L' : 'M'} ${x(p.t).toFixed(1)} ${y(p.v).toFixed(1)} `;
      pen = true;
    }
    return d;
  }
  /** Area path: one closed polygon per contiguous run. */
  function areaPath(points: MetricChartPoint[]): string {
    let d = '';
    let run: MetricChartPoint[] = [];
    const flush = () => {
      if (run.length === 0) return;
      const first = run[0];
      const last = run[run.length - 1];
      d += `M ${x(first.t).toFixed(1)} ${y0.toFixed(1)} `;
      for (const p of run) d += `L ${x(p.t).toFixed(1)} ${y(p.v as number).toFixed(1)} `;
      d += `L ${x(last.t).toFixed(1)} ${y0.toFixed(1)} Z `;
      run = [];
    };
    for (const p of points) {
      if (p.v == null || !Number.isFinite(p.v)) flush();
      else run.push(p);
    }
    flush();
    return d;
  }

  // Single isolated points would be invisible as a line — draw dots for them.
  function lonePoints(points: MetricChartPoint[]): MetricChartPoint[] {
    const out: MetricChartPoint[] = [];
    for (let i = 0; i < points.length; i++) {
      const p = points[i];
      if (p.v == null) continue;
      const prev = points[i - 1]?.v ?? null;
      const next = points[i + 1]?.v ?? null;
      if (prev == null && next == null) out.push(p);
    }
    return out;
  }

  const yTicks = $derived.by(() => {
    const { lo, hi } = yDomain;
    const n = 4;
    const out: number[] = [];
    for (let i = 0; i <= n; i++) out.push(lo + ((hi - lo) * i) / n);
    return out;
  });

  const xTicks = $derived.by(() => {
    const { lo, hi } = xDomain;
    const span = hi - lo;
    // Leave room for the longest date/time label, including the start/end
    // anchors. A fixed six labels overlap in narrow dashboard cards.
    const n = Math.max(1, Math.min(5, Math.floor(plotW / 90)));
    const out: { t: number; label: string }[] = [];
    for (let i = 0; i <= n; i++) {
      const t = lo + (span * i) / n;
      const label = fmtTick(t, span);
      if (out.at(-1)?.label !== label) out.push({ t, label });
    }
    return out;
  });

  // ── hover ──────────────────────────────────────────────────────────────────
  let svgEl = $state<SVGSVGElement | null>(null);
  let hover = $state<{ t: number; px: number; values: (number | null)[] } | null>(null);

  /** All distinct sample times (sorted) — hover snaps to these. */
  const times = $derived.by(() => {
    const set = new Set<number>();
    for (const s of series) for (const p of s.points) set.add(p.t);
    return [...set].sort((a, b) => a - b);
  });

  function onMove(e: PointerEvent): void {
    if (!svgEl || times.length === 0) return;
    const rect = svgEl.getBoundingClientRect();
    const fx = ((e.clientX - rect.left) / rect.width) * W;
    const { lo, hi } = xDomain;
    const t = lo + ((fx - PAD.l) / plotW) * (hi - lo);
    // Nearest sample by binary search.
    let a = 0;
    let b = times.length - 1;
    while (a < b) {
      const mid = (a + b) >> 1;
      if (times[mid] < t) a = mid + 1;
      else b = mid;
    }
    const cand = [times[a], times[a - 1]].filter((v): v is number => v != null);
    const nearest = cand.reduce((best, c) => (Math.abs(c - t) < Math.abs(best - t) ? c : best), cand[0]);
    hoverAt(nearest);
  }
  function hoverAt(t: number): void {
    hover = { t, px: x(t), values: valuesAt(t) };
  }
  function valuesAt(t: number): (number | null)[] {
    return series.map((s) => s.points.find((p) => p.t === t)?.v ?? null);
  }
  function onLeave(): void {
    // A keyboard crosshair outlives the pointer leaving.
    hover = kbIdx >= 0 && times[kbIdx] != null ? { t: times[kbIdx], px: x(times[kbIdx]), values: valuesAt(times[kbIdx]) } : null;
  }

  // ── keyboard ───────────────────────────────────────────────────────────────
  /** Sample index of the keyboard crosshair (-1 = none yet). */
  let kbIdx = $state(-1);
  /** Polite readout, written only on keyboard steps (pointer hover stays quiet). */
  let readout = $state('');
  function describe(t: number): string {
    const vals = valuesAt(t);
    return `${fmtFull(t)}: ${series.map((s, i) => `${s.label} ${vals[i] == null ? 'no data' : fmt(vals[i] as number)}`).join(', ')}`;
  }
  function onKey(e: KeyboardEvent): void {
    const n = times.length;
    if (n === 0) return;
    const cur = kbIdx < 0 ? (e.key === 'ArrowLeft' || e.key === 'End' ? n : -1) : kbIdx;
    const next =
      e.key === 'ArrowRight' || e.key === 'ArrowUp' ? cur + 1
      : e.key === 'ArrowLeft' || e.key === 'ArrowDown' ? cur - 1
      : e.key === 'PageUp' ? cur + 10
      : e.key === 'PageDown' ? cur - 10
      : e.key === 'Home' ? 0
      : e.key === 'End' ? n - 1
      : e.key === 'Escape' && kbIdx >= 0 ? -2
      : null;
    if (next == null) return;
    e.preventDefault();
    if (next === -2) {
      kbIdx = -1;
      hover = null;
      readout = '';
      return;
    }
    kbIdx = Math.max(0, Math.min(n - 1, next));
    hoverAt(times[kbIdx]);
    readout = describe(times[kbIdx]);
  }
  function onBlur(): void {
    kbIdx = -1;
    hover = null;
  }
  const chartLabel = $derived(`${series.map((s) => s.label).join(', ')} over time`);

  /** The data-table disclosure renders its rows only while open. */
  let dataOpen = $state(false);

  // Tooltip sits left of the cursor once past the midpoint so it never leaves
  // the chart's own box (which is what the parent's overflow clips to).
  const tipLeft = $derived(hover ? (hover.px / W) * 100 : 0);
  const tipFlip = $derived(hover ? hover.px > W * 0.55 : false);
;
async () => {

 { const $$_div0 = svelteHTML.createElement("div", {    "class":`mc`,"style":`--mc-h:${H}px`,});plotWidth= $$_div0.clientWidth;
  if(!hasData){
     { svelteHTML.createElement("div", {   "class":`mc-empty`,"style":`height:${H}px`,});emptyText; }
  }else{
    
     { svelteHTML.createElement("div", {                     "class":`mc-plot`,"role":`slider`,"tabindex":0,"aria-label":chartLabel,"aria-valuemin":0,"aria-valuemax":Math.max(0, times.length - 1),"aria-valuenow":Math.max(0, kbIdx),"aria-valuetext":kbIdx >= 0 && times[kbIdx] != null ? describe(times[kbIdx]) : `${times.length} samples — use the arrow keys to read values`,"onkeydown":onKey,"onblur":onBlur,});
       { const $$_svg2 = svelteHTML.createElement("svg", {                  "viewBox":`0 0 ${W} ${H}`,"preserveAspectRatio":`none`,"class":`mc-svg`,"style":`height:${H}px`,"aria-hidden":`true`,"onpointermove":onMove,"onpointerleave":onLeave,"onpointercancel":onLeave,});svgEl = $$_svg2;
        
            for(let v of __sveltets_2_ensureArray(yTicks)){let i = 1;i;
           { svelteHTML.createElement("line", {          "x1":PAD.l,"x2":W - PAD.r,"y1":y(v).toFixed(1),"y2":y(v).toFixed(1),"class":`grid`,});}
           { svelteHTML.createElement("text", {         "x":PAD.l - 6,"y":y(v).toFixed(1),"class":`tick y`,"text-anchor":`end`,"dominant-baseline":`middle`,});fmt(v); }
        }
        
            for(let tk of __sveltets_2_ensureArray(xTicks)){let i = 1;i;
           { svelteHTML.createElement("text", {         "x":x(tk.t).toFixed(1),"y":H - 6,"class":`tick x`,"text-anchor":i === 0 ? 'start' : i === xTicks.length - 1 ? 'end' : 'middle',});tk.label; }
        }
        
            for(let s of __sveltets_2_ensureArray(series)){let i = 1;i;
          if(area){
             { svelteHTML.createElement("path", {        "d":areaPath(s.points),"fill":colorOf(i),"fill-opacity":`0.12`,"stroke":`none`,});}
          }
           { svelteHTML.createElement("path", {            "d":linePath(s.points),"fill":`none`,"stroke":colorOf(i),"stroke-width":`1.6`,"stroke-linejoin":`round`,"vector-effect":`non-scaling-stroke`,});}
             for(let p of __sveltets_2_ensureArray(lonePoints(s.points))){p.t;
             { svelteHTML.createElement("circle", {          "cx":x(p.t).toFixed(1),"cy":y(p.v as number).toFixed(1),"r":`2`,"fill":colorOf(i),"vector-effect":`non-scaling-stroke`,});}
          }
        }
        
        if(hover){
           { svelteHTML.createElement("line", {          "x1":hover.px.toFixed(1),"x2":hover.px.toFixed(1),"y1":PAD.t,"y2":PAD.t + plotH,"class":`cursor`,});}
              for(let v of __sveltets_2_ensureArray(hover.values)){let i = 1;i;
            if(v != null){
               { svelteHTML.createElement("circle", {              "cx":hover.px.toFixed(1),"cy":y(v).toFixed(1),"r":`3`,"fill":colorOf(i),"stroke":`var(--surface)`,"stroke-width":`1.5`,"vector-effect":`non-scaling-stroke`,});}
            }
          }
        }
       }
      if(hover){
         { svelteHTML.createElement("div", {    "class":`mc-tip`,"style":`left:${tipLeft}%`,});tipFlip;
           { svelteHTML.createElement("div", { "class":`mc-tip-t`,});fmtFull(hover.t); }
              for(let s of __sveltets_2_ensureArray(series)){let i = 1;i;
             { svelteHTML.createElement("div", { "class":`mc-tip-row`,});
               { svelteHTML.createElement("span", {   "class":`sw`,"style":`background:${colorOf(i)}`,}); }
               { svelteHTML.createElement("span", { "class":`lbl`,});s.label; }
               { svelteHTML.createElement("span", { "class":`val mono`,});hover.values[i] == null ? '—' : fmt(hover.values[i] as number); }
             }
          }
         }
      }
     }
     { svelteHTML.createElement("p", {   "class":`sr-only`,"aria-live":`polite`,});readout; }
    if(showLegend && series.length > 1){
       { svelteHTML.createElement("ul", { "class":`mc-legend`,});
            for(let s of __sveltets_2_ensureArray(series)){let i = 1;i;
           { svelteHTML.createElement("li", {}); { svelteHTML.createElement("span", {   "class":`sw`,"style":`background:${colorOf(i)}`,}); }s.label; }
        }
       }
    }
     { svelteHTML.createElement("details", {   "class":`mc-data`,"bind:open":dataOpen,});/*Ωignore_startΩ*/() => dataOpen = __sveltets_2_any(null);/*Ωignore_endΩ*/
       { svelteHTML.createElement("summary", {});  }
      if(dataOpen){
         { svelteHTML.createElement("div", { "class":`mc-table-wrap`,});
           { svelteHTML.createElement("table", {});
             { svelteHTML.createElement("caption", { "class":`sr-only`,});chartLabel; }
             { svelteHTML.createElement("thead", {});
               { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", { "scope":`col`,});  }    for(let s of __sveltets_2_ensureArray(series)){let i = 1;i; { svelteHTML.createElement("th", { "scope":`col`,});s.label; }} }
             }
             { svelteHTML.createElement("tbody", {});
                 for(let t of __sveltets_2_ensureArray(times)){t;
                const vals = valuesAt(t);
                 { svelteHTML.createElement("tr", {});
                   { svelteHTML.createElement("th", { "scope":`row`,});fmtFull(t); }
                      for(let v of __sveltets_2_ensureArray(vals)){let i = 1;i; { svelteHTML.createElement("td", { "class":`mono`,});v == null ? '—' : fmt(v); }}
                 }
              }
             }
           }
         }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MetricChart__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MetricChart__SvelteComponent_ = ReturnType<typeof MetricChart__SvelteComponent_>;
/*Ωignore_endΩ*/export default MetricChart__SvelteComponent_;