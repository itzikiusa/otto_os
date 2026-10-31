///<reference types="svelte" />
;// Everything sent from this workspace, newest first: method + path, host,
// status and when. Opening a row loads its request into a tab (it is NOT
// re-sent; masked credentials are never sent back — see loadHistoryIntoDraft).

import Icon from '../../lib/components/Icon.svelte';
import { toastError } from '../../lib/toastError';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import AgentChip from '../../lib/components/AgentChip.svelte';
import VirtualList from '../../lib/components/VirtualList.svelte';
import MethodTag, { methodWord } from './MethodTag.svelte';
import StatusChip from './StatusChip.svelte';
import RetentionDialog from './RetentionDialog.svelte';
import { apiClient } from '../../lib/stores/apiClient.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { rel } from '../../lib/stores/now.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { splitUrl } from '../../lib/api/apiVars';
import type { ApiHistorySummary } from '../../lib/api/types';
import { RETENTION_PRESETS, formatBytes, suggestRetention, type RetentionPreset } from './storageGauge';
import { toasts } from '../../lib/toast.svelte';
;

  interface Props { onopen?: () => void };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { onopen }: Props = $props();

  const canEdit = $derived(ws.myRole !== 'viewer');
  let retentionOpen = $state(false);

  // Search: every whitespace-separated token must match method/url/status —
  // the url covers domain, path and query string, so "api.foo 404 get" works.
  let search = $state('');
  const tokens = $derived(search.trim().toLowerCase().split(/\s+/).filter(Boolean));
  function entryMatches(h: ApiHistorySummary): boolean {
    const hay = `${h.method} ${h.url} ${h.status ?? ''}`.toLowerCase();
    return tokens.every((t) => hay.includes(t));
  }
  // The list holds only the newest HISTORY_PAGE summaries. Once it is full, a
  // search also asks the daemon (debounced, aborted on the next keystroke) so
  // older entries are findable; results merge with the local rows and go
  // through the same all-tokens filter below.
  const HISTORY_PAGE = 100;
  let serverRows = $state.raw<ApiHistorySummary[] | null>(null);
  $effect(() => {
    const t = tokens;
    const full = apiClient.history.length >= HISTORY_PAGE;
    serverRows = null;
    if (!t.length || !full) return;
    // The daemon matches ONE literal against method/URL (or an exact status):
    // send the most selective token; the rest narrow client-side.
    const text = t.filter((x) => !/^\d{3}$/.test(x)).sort((a, b) => b.length - a.length)[0];
    const params = text ? { q: text } : { status: Number(t[0]) };
    const ctl = new AbortController();
    const timer = setTimeout(() => {
      apiClient.searchHistory(params, ctl.signal).then(
        (rows) => { if (!ctl.signal.aborted) serverRows = rows; },
        () => { /* aborted / offline: the local rows still filter */ },
      );
    }, 200);
    return () => { clearTimeout(timer); ctl.abort(); };
  });
  const searchPool = $derived.by(() => {
    if (!serverRows) return apiClient.history;
    const seen = new Set(apiClient.history.map((h) => h.id));
    const extra = serverRows.filter((h) => !seen.has(h.id));
    if (!extra.length) return apiClient.history;
    return [...apiClient.history, ...extra].sort((a, b) =>
      a.executed_at === b.executed_at ? (a.id < b.id ? 1 : -1) : a.executed_at < b.executed_at ? 1 : -1,
    );
  });
  const filtered = $derived(
    searchPool.filter(
      (h) =>
        (!apiClient.historyAgentOnly || apiClient.historySource(h)?.kind === 'agent') &&
        (!tokens.length || entryMatches(h)),
    ),
  );

  function open(h: ApiHistorySummary): void {
    void apiClient.selectHistory(h.id);
    onopen?.();
  }

  async function clear(): Promise<void> {
    if (!(await confirmer.ask(
      'Delete all request history in this workspace, including the stored responses? Saved requests are not affected.',
      { title: 'Clear history', confirmLabel: 'Clear history' },
    ))) return;
    await apiClient.clearHistory();
  }

  const retention = $derived(apiClient.historyRetention);
  const retentionText = $derived(
    retention.rows || retention.days
      ? `Keeping ${retention.rows ? `the newest ${retention.rows}` : 'all'}${retention.days ? ` for up to ${retention.days} days` : ''}`
      : 'Keeping every request',
  );

  // Storage gauge (perf2 N2): how big history + runs are, and — past a size
  // with no limit set — an inline offer of retention presets. Opt-in only:
  // nothing is deleted until someone picks a preset and confirms.
  $effect(() => {
    if (ws.currentId) void apiClient.loadStorage();
  });
  const gauge = $derived(apiClient.storage);
  const dismissKey = $derived(`otto_api_storage_banner_dismissed:${ws.currentId ?? ''}`);
  let dismissed = $state(false);
  $effect(() => {
    try { dismissed = localStorage.getItem(dismissKey) === '1'; } catch { dismissed = false; }
  });
  const showBanner = $derived(
    !dismissed && suggestRetention(gauge, { rows: retention.rows, days: retention.days, runsKeep: apiClient.runsKeep }),
  );
  function dismissBanner(): void {
    dismissed = true;
    try { localStorage.setItem(dismissKey, '1'); } catch { /* this session only */ }
  }
  let applying = $state(false);
  async function applyPreset(p: RetentionPreset): Promise<void> {
    if (!canEdit || applying) return;
    const what = [
      p.rows ? `keeps the newest ${p.rows.toLocaleString()} requests` : '',
      p.days ? `deletes requests older than ${p.days} days` : '',
      p.runsKeep ? `keeps the newest ${p.runsKeep} runs of each automation` : '',
    ].filter(Boolean).join(', ');
    if (!(await confirmer.ask(
      `This workspace’s history ${what}. Older entries, with their stored responses, are deleted now and after every request — for everyone in the workspace. Saved requests are not affected.`,
      { title: `Retention: ${p.label}`, confirmLabel: 'Apply retention' },
    ))) return;
    applying = true;
    try {
      await apiClient.setHistoryRetention(p.rows, p.days);
      if (p.runsKeep !== null && !apiClient.runsKeep) await apiClient.setRunsKeep(p.runsKeep);
      if (await apiClient.applyRetention()) toasts.success('Retention applied', p.label);
    } catch (e) {
      toastError('Couldn’t save retention', e);
    } finally {
      applying = false;
    }
  }

  function menu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Retention…', icon: 'clock', action: () => (retentionOpen = true), disabled: !canEdit },
      { separator: true },
      { label: 'Clear history…', icon: 'trash', danger: true, disabled: !canEdit || apiClient.history.length === 0, action: () => void clear() },
    ]);
  }

  /** ⌘F text of a history row — its cells in DOM order (see the VirtualList). */
  function histFindText(h: ApiHistorySummary): string {
    const u = splitUrl(h.url);
    const agent = apiClient.historySource(h)?.kind === 'agent' ? 'Agent' : '';
    return [methodWord(h.method), u.path, u.host || '—', agent, h.status ?? 'No response', rel(h.executed_at)].join('\n');
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`hist-wrap`,});
   { svelteHTML.createElement("div", { "class":`tools`,});
     { svelteHTML.createElement("label", { "class":`search`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}
       { svelteHTML.createElement("input", {        "dir":`auto`,"placeholder":`Filter by URL, method or status…`,"bind:value":search,"aria-label":`Search request history`,});/*Ωignore_startΩ*/() => search = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      if(search){
         { svelteHTML.createElement("button", {       "class":`icon-btn clear`,"onclick":() => (search = ''),"aria-label":`Clear search`,"title":`Clear search`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
      }
     }
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":menu,"aria-label":`History options`,"title":`History options`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
   }
   { svelteHTML.createElement("div", { "class":`filters`,});
     { svelteHTML.createElement("button", {            "class":`pill-toggle small`,"aria-pressed":apiClient.historyAgentOnly,"aria-label":`Show only agent runs`,"title":`Show only requests that agents sent through Otto’s tools`,"onclick":() => (apiClient.historyAgentOnly = !apiClient.historyAgentOnly),});apiClient.historyAgentOnly;   }
     { svelteHTML.createElement("span", {   "class":`ret`,"title":`History retention (workspace setting)`,});retentionText;if(gauge){ formatBytes(gauge.history_bytes);} }
   }
  if(showBanner && gauge){
     { svelteHTML.createElement("div", { "class":`size-banner`,});
       { svelteHTML.createElement("div", { "class":`sb-text`,});
            { svelteHTML.createElement("strong", {});formatBytes(gauge.history_bytes + gauge.run_bytes); }
        gauge.history_rows.toLocaleString();  gauge.run_rows.toLocaleString();          
       }
       { svelteHTML.createElement("div", { "class":`sb-actions`,});
        if(canEdit){
             for(let p of __sveltets_2_ensureArray(RETENTION_PRESETS)){p.label;
             { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":applying,"onclick":() => void applyPreset(p),});p.label; }
          }
           { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => (retentionOpen = true),});  }
        }
         { svelteHTML.createElement("button", {       "class":`icon-btn sb-dismiss`,"onclick":dismissBanner,"aria-label":`Dismiss storage notice`,"title":`Dismiss`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
       }
     }
  }

  if(apiClient.historyLoadingId){ { svelteHTML.createElement("div", { "class":`state`,}); { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 11px`,"aria-hidden":`true`,}); }   }}

  if(apiClient.history.length === 0 && (apiClient.historyLoadError || !apiClient.historyLoaded)){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {            "what":`history`,"variant":`compact`,"loading":!apiClient.historyLoaded && !apiClient.historyLoadError,"error":apiClient.historyLoadError,"empty":true,"onretry":() => apiClient.retryHistory(),}});}
  } else if (apiClient.history.length === 0){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {      "icon":`clock`,"title":`Nothing sent yet`,"body":`Every request you send appears here, so you can open it again later.`,}});}
  } else if (filtered.length === 0){
     { svelteHTML.createElement("div", { "class":`state`,});
      apiClient.historyAgentOnly && !tokens.length ? 'No agent runs yet.' : `No history matches “${search.trim()}”.`;
     }
  }else{
    
     { const $$_tsiLlautriV1C = __sveltets_2_ensureComponent(VirtualList); const $$_tsiLlautriV1 = new $$_tsiLlautriV1C({ target: __sveltets_2_any(), props: {       "items":filtered,"estimateHeight":44,"class":`hist-vlist`,"findText":histFindText,row:(h: ApiHistorySummary) => { async ()/*Ωignore_positionΩ*/ => {
        const src = apiClient.historySource(h);
        const u = splitUrl(h.url);
         { svelteHTML.createElement("button", {       "class":`hist-row`,...__sveltets_2_empty({"data-url":h.url}),"onclick":() => open(h),"title":`${h.method} ${h.url}&#10;${new Date(h.executed_at).toLocaleString()}`,});
           { svelteHTML.createElement("span", { "class":`l1`,});
              { const $$_gaTdohteM2C = __sveltets_2_ensureComponent(MethodTag); new $$_gaTdohteM2C({ target: __sveltets_2_any(), props: {  "method":h.method,"fixed":true,}});}
             { svelteHTML.createElement("span", {   "class":`path mono`,"dir":`ltr`,});u.path; }
           }
           { svelteHTML.createElement("span", { "class":`l2`,});
             { svelteHTML.createElement("span", {   "class":`host`,"dir":`ltr`,});u.host || '—'; }
            if(src?.kind === 'agent'){ { const $$_pihCtnegA2C = __sveltets_2_ensureComponent(AgentChip); new $$_pihCtnegA2C({ target: __sveltets_2_any(), props: {  "title":`Sent by an agent session`,}});}}
              { const $$_pihCsutatS2C = __sveltets_2_ensureComponent(StatusChip); new $$_pihCsutatS2C({ target: __sveltets_2_any(), props: {  "status":h.status,"small":true,}});}
             { svelteHTML.createElement("span", { "class":`when`,});rel(h.executed_at); }
           }
         }
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {row} = $$_tsiLlautriV1.$$prop_def;/*Ωignore_endΩ*/
      
     VirtualList}
  }
 }

if(retentionOpen){ { const $$_golaiDnoitneteR0C = __sveltets_2_ensureComponent(RetentionDialog); new $$_golaiDnoitneteR0C({ target: __sveltets_2_any(), props: {  "onclose":() => (retentionOpen = false),}});}}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const HistoryList__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type HistoryList__SvelteComponent_ = ReturnType<typeof HistoryList__SvelteComponent_>;
/*Ωignore_endΩ*/export default HistoryList__SvelteComponent_;