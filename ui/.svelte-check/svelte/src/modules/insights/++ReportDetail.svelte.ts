///<reference types="svelte" />
;
  import type { MetricKey, Delta } from './insightsParse';

  export type ViewMode = 'preview' | 'markdown' | 'html';

  /** One KPI tile, pre-computed by the page (value + delta + trend). */
  export interface Kpi {
    key: MetricKey;
    label: string;
    value: number;
    /** Where the value came from, for the tooltip. */
    source: 'summary' | 'index';
    delta: Delta | null;
    /** Oldest → newest, ending at this report; null = no reading that period. */
    trend: (number | null)[];
    trendLabels: string[];
  }
;;

import { announce } from '../../lib/announce.svelte';
import { plural } from '../../lib/plural';
import Skeleton from '../../lib/components/Skeleton.svelte';
// The right-hand pane of Insights → Reports: one report, rendered.
//
//   header   — period chip · date · generated-at, the one-line headline, and
//              the Preview / Markdown / HTML switch + window/export tools
//   Preview  — key findings parsed out of the summary markdown (KPI tiles
//              with deltas + sparklines, the Action Plan as a checklist with
//              effort + status chips and ledger detail), then the rest of
//              the summary rendered through the sanitized GFM renderer
//   Markdown — the summary source, verbatim
//   HTML     — the agent-generated report in a SANDBOXED iframe: srcdoc with
//              `allow-scripts` but never `allow-same-origin`, so its charts
//              run while the app's storage (bearer token) stays out of reach
//
// Parsing is best-effort (insightsParse.ts): when the summary doesn't have
// the expected shape, the key-findings block simply doesn't render and the
// markdown shows as-is.
import type { InsightReport } from '../../lib/api/types';
import { insightsApi } from '../../lib/api/insights';
import { openExternal } from '../../lib/external';
import { reportLinkBridge } from './reportFrame';
import { renderMarkdownGfm } from '../../lib/md';
import { rel } from '../../lib/stores/now.svelte';
import Icon from '../../lib/components/Icon.svelte';
import Badge from '../../lib/components/Badge.svelte';
import { badgeTone } from '../../lib/status';
import Sparkline from '../../lib/components/Sparkline.svelte';
import {
    itemStatus,
    statusLabel,
    statusTone,
    type ActionItem,
    type LedgerEntry,
    type ParsedSummary,
  } from './insightsParse';
;

  interface Props {
    report: InsightReport;
    parsed: ParsedSummary;
    /** The summary markdown (full file when it could be read, else the listed excerpt). */
    markdown: string;
    kpis: Kpi[];
    /** Label of the report the deltas compare against ("Tue 22 Sep"), if any. */
    compareLabel: string | null;
    ledger: Map<string, LedgerEntry>;
    mode: ViewMode;
    onmode: (m: ViewMode) => void;
    onopenwindow: () => void;
    onexportmd: () => void;
    ondownloadhtml: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { report, parsed, markdown, kpis, compareLabel, ledger, mode, onmode, onopenwindow, onexportmd, ondownloadhtml }: Props =
    $props();

  const hasHtml = $derived(!!report.html_path);
  const effectiveMode = $derived<ViewMode>(mode === 'html' && !hasHtml ? 'preview' : mode);

  // ---- HTML (lazy, per report) -------------------------------------------
  // The report runs sandboxed WITHOUT popups: a link click is posted to us
  // (`otto-insights-open`) and opened in the system browser, so the report can
  // never spawn an unsandboxed window of its own.
  const LINK_BRIDGE =
    "<script>document.addEventListener('click',function(e){var a=e.target&&e.target.closest&&e.target.closest('a[href]');" +
    "if(!a)return;var h=a.href;if(!/^https?:/i.test(h))return;e.preventDefault();" +
    "parent.postMessage({type:'otto-insights-open',url:h},'*');},true);<\/script>";
  function withLinkBridge(doc: string): string {
    const i = doc.toLowerCase().lastIndexOf('</body>');
    return i >= 0 ? doc.slice(0, i) + LINK_BRIDGE + doc.slice(i) : doc + LINK_BRIDGE;
  }
  let frameEl = $state<HTMLIFrameElement | null>(null);
  // The report's JS is agent-written (possibly prompt-injected): it may post
  // `otto-insights-open` itself, without a click. The bridge (reportFrame.ts)
  // opens only this frame's http(s) links, on a real user gesture, at most one
  // per OPEN_GAP_MS, so a script can't spray system-browser tabs.
  const onFrameMessage = reportLinkBridge({
    frameWindow: () => frameEl?.contentWindow,
    open: (url) => void openExternal(url),
  });

  let html = $state<string | null>(null);
  let htmlFor = $state<string | null>(null);
  let htmlError = $state<string | null>(null);
  let htmlLoading = $state(false);
  /** The path whose last load FAILED: the auto-load skips it (a missing or
   *  mid-write file used to re-fire the GET back to back forever, flickering
   *  skeleton ↔ error); Retry clears it. */
  let htmlFailedFor = $state<string | null>(null);

  async function loadHtml(path: string): Promise<void> {
    htmlLoading = true;
    htmlError = null;
    htmlFailedFor = null;
    try {
      const text = await insightsApi.readText(path);
      if (report.html_path === path) {
        html = text;
        htmlFor = path;
      }
    } catch (e) {
      if (report.html_path === path) {
        htmlError = e instanceof Error ? e.message : String(e);
        htmlFailedFor = path;
      }
    } finally {
      htmlLoading = false;
    }
  }
  $effect(() => {
    const p = report.html_path;
    if (effectiveMode === 'html' && p && htmlFor !== p && htmlFailedFor !== p && !htmlLoading) void loadHtml(p);
  });

  // ---- Display helpers ------------------------------------------------------
  function kindLabel(k: string): string {
    return k === 'adhoc' ? 'Ad-hoc' : k ? k[0].toUpperCase() + k.slice(1) : '';
  }
  function dateLabel(iso: string): string {
    const d = new Date(`${iso}T00:00:00`);
    return Number.isNaN(d.getTime())
      ? iso
      : d.toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short', year: 'numeric' });
  }
  const periodText = $derived(
    report.period_start === report.period_end
      ? dateLabel(report.period_start)
      : `${dateLabel(report.period_start)} – ${dateLabel(report.period_end)}`,
  );

  function fmt(key: Kpi['key'], v: number): string {
    if (key === 'spend') return v >= 1000 ? `$${Math.round(v).toLocaleString()}` : `$${v.toFixed(2)}`;
    if (key === 'achievement') return `${Math.round(v)}%`;
    return Math.round(v).toLocaleString();
  }
  function deltaText(k: Kpi): string {
    const d = k.delta;
    if (!d) return '';
    if (d.direction === 'flat') return 'No change';
    if (k.key === 'achievement') return `${Math.abs(Math.round(d.diff))} pts`;
    if (d.pct != null) return `${Math.abs(Math.round(d.pct * 100))}%`;
    return fmt(k.key, Math.abs(d.diff));
  }
  function deltaSr(k: Kpi): string {
    const d = k.delta;
    if (!d) return '';
    const vs = compareLabel ? ` vs ${compareLabel}` : ' vs the previous report';
    if (d.direction === 'flat') return `No change${vs}`;
    const verdict = d.tone === 'good' ? ', better' : d.tone === 'bad' ? ', worse' : '';
    return `${d.direction === 'up' ? 'Up' : 'Down'} ${deltaText(k)}${vs}${verdict}`;
  }

  const bodyHtml = $derived(renderMarkdownGfm(parsed.body));
  const regressed = $derived(parsed.actions.filter((a) => itemStatus(a, ledger) === 'regressed').length);

  // Action rows expand to show their ledger entries.
  let expanded = $state<Set<number>>(new Set());
  function toggle(i: number): void {
    const next = new Set(expanded);
    if (next.has(i)) next.delete(i);
    else next.add(i);
    expanded = next;
  }
  $effect(() => {
    void report.html_path;
    void report.period_end;
    expanded = new Set();
  });

  function ledgerFor(a: ActionItem): LedgerEntry[] {
    return a.ids.map((id) => ledger.get(id)).filter((x): x is LedgerEntry => !!x);
  }
  function markerIcon(s: string | null): 'check' | 'warning' | 'arrowUp' | 'dot' {
    if (s === 'closed') return 'check';
    if (s === 'regressed') return 'warning';
    if (s === 'improved') return 'arrowUp';
    return 'dot';
  }
  function periodKeyLabel(k: string): string {
    const m = /^(\w+):(\d{4})(\d{2})(\d{2})_/.exec(k);
    if (!m) return k;
    return `${kindLabel(m[1])} ${dateLabel(`${m[2]}-${m[3]}-${m[4]}`)}`;
  }
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  $effect(() => { if (htmlError) announce(`Couldn’t load the HTML report. ${htmlError}`, { assertive: true }); });
;
async () => {

 { svelteHTML.createElement("svelte:window", {  "onmessage":onFrameMessage,});}



 { svelteHTML.createElement("article", {   "class":`report`,...__sveltets_2_empty({"data-testid":`insight-report`}),});
   { svelteHTML.createElement("header", { "class":`r-head`,});
     { svelteHTML.createElement("div", { "class":`r-meta`,});
       { const $$_egdaB3C = __sveltets_2_ensureComponent(Badge); new $$_egdaB3C({ target: __sveltets_2_any(), props: {  "label":kindLabel(report.kind),}});}
       { svelteHTML.createElement("span", { "class":`r-period`,});periodText; }
      if(report.created_at){
         { svelteHTML.createElement("span", {   "class":`dim r-created`,"title":new Date(report.created_at).toLocaleString(),});  rel(report.created_at); }
      }
     }
    if(parsed.headline){
       { svelteHTML.createElement("h2", { "class":`r-headline`,});parsed.headline; }
    } else if (parsed.title){
       { svelteHTML.createElement("h2", { "class":`r-headline`,});parsed.title; }
    }
     { svelteHTML.createElement("div", { "class":`r-tools`,});
       { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Report view`,});
         { svelteHTML.createElement("button", {    "aria-pressed":effectiveMode === 'preview',"onclick":() => onmode('preview'),});effectiveMode === 'preview';  }
         { svelteHTML.createElement("button", {    "aria-pressed":effectiveMode === 'markdown',"onclick":() => onmode('markdown'),});effectiveMode === 'markdown';  }
         { svelteHTML.createElement("button", {         "aria-pressed":effectiveMode === 'html',"disabled":!hasHtml,"title":hasHtml ? 'The full HTML report' : 'This period has no HTML report yet',"onclick":() => onmode('html'),});effectiveMode === 'html';  }
       }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
       { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":onexportmd,"title":`Download the summary as a .md file`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});}   }
      if(hasHtml){
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":ondownloadhtml,"aria-label":`Download HTML report`,"title":`Download HTML report`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":14,}});} }
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":onopenwindow,"aria-label":`Open in new window`,"title":`Open in new window`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":14,}});} }
      }
     }
   }

  if(effectiveMode === 'html'){
     { svelteHTML.createElement("div", { "class":`r-html`,});
      if(htmlError){
         { svelteHTML.createElement("div", { "class":`r-error`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
           { svelteHTML.createElement("div", {});
             { svelteHTML.createElement("strong", {});     }
             { svelteHTML.createElement("span", { "class":`dim`,});           }
             { svelteHTML.createElement("span", { "class":`dim mono err-detail`,});htmlError; }
           }
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => report.html_path && loadHtml(report.html_path),});  }
         }
      } else if (html == null || htmlLoading){
         { svelteHTML.createElement("div", { "class":`r-loading`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {      "rows":6,"height":28,"label":`the HTML report`,}});} }
      }else{
        
         { const $$_iframe2 = svelteHTML.createElement("iframe", {          "class":`r-frame`,"sandbox":`allow-scripts`,"srcdoc":withLinkBridge(html),"title":`Insight report ${periodText}`,});frameEl = $$_iframe2; }
      }
     }
  } else if (effectiveMode === 'markdown'){
     { svelteHTML.createElement("pre", {     "class":`r-source`,"dir":`ltr`,...__sveltets_2_empty({"data-testid":`report-markdown`}),});markdown; }
  }else{
     { svelteHTML.createElement("div", { "class":`r-scroll`,});
      if(kpis.length > 0){
         { svelteHTML.createElement("section", {   "class":`r-section`,"aria-labelledby":`kf-title`,});
           { svelteHTML.createElement("div", { "class":`r-section-head`,});
             { svelteHTML.createElement("h3", {   "id":`kf-title`,"class":`section-title`,});  }
            if(compareLabel){ { svelteHTML.createElement("span", { "class":`dim r-vs`,});  compareLabel; }}
           }
           { svelteHTML.createElement("div", { "class":`kpis`,});
               for(let k of __sveltets_2_ensureArray(kpis)){k.key;
               { svelteHTML.createElement("div", {   "class":`kpi card`,...__sveltets_2_empty({"data-testid":`kpi-${k.key}`}),});
                 { svelteHTML.createElement("div", {   "class":`kpi-label`,"title":k.source === 'summary' ? 'From the report summary' : 'From the metrics index',});k.label; }
                 { svelteHTML.createElement("div", { "class":`kpi-row`,});
                   { svelteHTML.createElement("span", { "class":`kpi-value`,});fmt(k.key, k.value); }
                  if(k.delta){
                     { svelteHTML.createElement("span", {   "class":`kpi-delta ${k.delta.tone}`,"title":deltaSr(k),});
                      if(k.delta.direction === 'up'){ { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`arrowUp`,"size":12,}});}} else if (k.delta.direction === 'down'){ { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":12,}});}}
                       { svelteHTML.createElement("span", { "aria-hidden":`true`,});deltaText(k); }
                       { svelteHTML.createElement("span", { "class":`sr-only`,});deltaSr(k); }
                     }
                  }
                 }
                if(k.trend.filter((v) => v != null).length >= 2){
                   { const $$_enilkrapS5C = __sveltets_2_ensureComponent(Sparkline); new $$_enilkrapS5C({ target: __sveltets_2_any(), props: {            "values":k.trend,"labels":k.trendLabels,"name":k.label,"format":(v) => fmt(k.key, v),"width":120,"height":26,}});}
                }
               }
            }
           }
         }
      }

      if(parsed.actions.length > 0){
         { svelteHTML.createElement("section", {   "class":`r-section`,"aria-labelledby":`ap-title`,});
           { svelteHTML.createElement("div", { "class":`r-section-head`,});
             { svelteHTML.createElement("h3", {   "id":`ap-title`,"class":`section-title`,});  }
             { svelteHTML.createElement("span", { "class":`dim r-vs`,});
              plural(parsed.actions.length, 'item');if(regressed > 0){' · '; { svelteHTML.createElement("span", { "class":`warn-text`,});regressed;  }}
             }
           }
           { svelteHTML.createElement("ol", {   "class":`actions`,...__sveltets_2_empty({"data-testid":`action-plan`}),});
               for(let a of __sveltets_2_ensureArray(parsed.actions)){a.index;
              const st = itemStatus(a, ledger);
              const entries = ledgerFor(a);
               { svelteHTML.createElement("li", { "class":`action`,});
                 { svelteHTML.createElement("span", {   "class":`marker ${st ? statusTone(st) : 'neutral'}`,"title":st ? statusLabel(st) : 'No status',});
                   { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":markerIcon(st),"size":12,}});}
                 }
                 { svelteHTML.createElement("div", { "class":`a-main`,});
                   { svelteHTML.createElement("div", { "class":`a-top`,});
                     { svelteHTML.createElement("span", { "class":`a-title`,});a.title; }
                     { svelteHTML.createElement("span", { "class":`a-chips`,});
                         for(let s of __sveltets_2_ensureArray(a.statuses.length ? a.statuses : st ? [st] : [])){s;
                         { const $$_egdaB8C = __sveltets_2_ensureComponent(Badge); new $$_egdaB8C({ target: __sveltets_2_any(), props: {    "tone":badgeTone(statusTone(s)),"label":statusLabel(s),}});}
                      }
                      if(a.effort){
                         { const $$_egdaB8C = __sveltets_2_ensureComponent(Badge); new $$_egdaB8C({ target: __sveltets_2_any(), props: {    "title":`Effort: ${a.effort === 'S' ? 'small' : a.effort === 'M' ? 'medium' : 'large'}`,"label":`Effort ${a.effort}`,}});}
                      }
                     }
                   }
                  if(a.metric || a.current){
                     { svelteHTML.createElement("div", { "class":`a-metric`,});
                      if(a.metric){ { svelteHTML.createElement("span", { "class":`dim`,});a.metric; }}
                      if(a.current){
                         { svelteHTML.createElement("span", { "class":`a-values`,}); { svelteHTML.createElement("span", {});a.current; }if(a.target){ { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":12,}});} { svelteHTML.createElement("strong", {});a.target; }} }
                      }
                     }
                  }
                  if(a.ids.length > 0){
                     { svelteHTML.createElement("div", { "class":`a-ids`,});
                         for(let id of __sveltets_2_ensureArray(a.ids)){id; { svelteHTML.createElement("span", {   "class":`mono a-id`,"title":ledger.has(id) ? 'In the action ledger' : 'Not found in the ledger',});id; }}
                      if(entries.length > 0){
                         { svelteHTML.createElement("button", {     "class":`btn small ghost a-ledger-btn`,"aria-expanded":expanded.has(a.index),"onclick":() => toggle(a.index),});
                           { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":expanded.has(a.index) ? 'chevronDown' : 'chevronRight',"size":12,}});} 
                         }
                      }
                     }
                  }
                  if(expanded.has(a.index) && entries.length > 0){
                     { svelteHTML.createElement("div", {     "class":`ledger`,"role":`region`,"aria-label":`Ledger entries for ${a.title}`,});
                         for(let e of __sveltets_2_ensureArray(entries)){e.id;
                         { svelteHTML.createElement("dl", { "class":`ledger-row`,});
                           { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { "class":`mono`,});e.id; } }
                           { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {}); { const $$_egdaB10C = __sveltets_2_ensureComponent(Badge); new $$_egdaB10C({ target: __sveltets_2_any(), props: {    "tone":badgeTone(statusTone(e.status)),"label":statusLabel(e.status),}});} } }
                          if(e.openedPeriod){ { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});periodKeyLabel(e.openedPeriod);if(e.openedValue){' · ';e.openedValue;} } }}
                          if(e.latestValue){ { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});e.latestValue; } }}
                          if(e.targetValue){ { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});e.targetValue; } }}
                         }
                      }
                     }
                  }
                 }
               }
            }
           }
         }
      }

      if(parsed.body.trim()){
         { svelteHTML.createElement("section", {   "class":`r-section`,"aria-labelledby":`sum-title`,});
          if(kpis.length > 0 || parsed.actions.length > 0){
             { svelteHTML.createElement("div", { "class":`r-section-head`,}); { svelteHTML.createElement("h3", {   "id":`sum-title`,"class":`section-title`,});  } }
          }
          
           { svelteHTML.createElement("div", {   "class":`md-body report-md`,...__sveltets_2_empty({"data-testid":`report-rendered`}),}); bodyHtml; }
         }
      }
       { svelteHTML.createElement("p", { "class":`dim r-attrib`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":12,}});}     { svelteHTML.createElement("code", {});  }                }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ReportDetail__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ReportDetail__SvelteComponent_ = ReturnType<typeof ReportDetail__SvelteComponent_>;
/*Ωignore_endΩ*/export default ReportDetail__SvelteComponent_;