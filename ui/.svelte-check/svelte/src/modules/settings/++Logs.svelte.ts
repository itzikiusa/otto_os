///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
import { pollWhileVisible } from '../../lib/poll';
import PageHeader from '../../lib/components/PageHeader.svelte';
import { sectionLabel } from './sections';
import PageBody from '../../lib/components/PageBody.svelte';
import { tick } from 'svelte';
import { api } from '../../lib/api/client';
import type { DaemonLogs } from '../../lib/api/types';
import Icon from '../../lib/components/Icon.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import { toasts } from '../../lib/toast.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { escapeHtml } from '../../lib/html';
import { COPY_FAILED_BODY, copyText } from '../../lib/clipboard';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  type LogMode = 'all' | 'tail';

  const ALL_FILES = '__all__';
  /** Ring cap on what the page holds and renders. "All log files" was ~80k
   *  lines re-split, re-escaped and re-rendered through `{@html}` on every
   *  Live tick; the newest 5k lines are what anyone reads here (Copy copies
   *  what is shown — the files themselves are in the log folder). */
  const MAX_LINES = 5000;

  let loading = $state(true);
  let refreshing = $state(false);
  let autoRefresh = $state(true);
  let follow = $state(true);
  let mode: LogMode = $state('all');
  let tailLines = $state(2000);
  let selected = $state('');
  // The input binds to `filterInput`; the (whole-buffer) filter pass runs on
  // `filter`, debounced so typing doesn't re-scan per keystroke.
  let filterInput = $state('');
  let filter = $state('');
  let content = $state('');
  let nextOffset = $state(0);
  // The file Live follows with `mode=since`: the selected file, or for "All
  // log files" the newest one (the only one still being written).
  let liveFile = '';
  // Bumped by every full read, so an in-flight Live append that started
  // before a file/mode switch is discarded instead of landing on the new view.
  let readGen = 0;
  // True when older lines were dropped — by the ring cap or the daemon's byte cap.
  let clipped = $state(false);
  let payload: DaemonLogs | null = $state(null);
  let logEl: HTMLDivElement | null = $state(null);
  let loadError = $state('');
  let wrap = $state(true);

  const visibleContent = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return content;
    return content
      .split('\n')
      .filter((line) => line.toLowerCase().includes(q))
      .join('\n');
  });

  const lineCount = $derived(content ? content.split('\n').filter(Boolean).length : 0);
  const shownCount = $derived(visibleContent ? visibleContent.split('\n').filter(Boolean).length : 0);

  const status = $derived.by(() => {
    if (!payload) return '';
    const fileCount = payload.files.length;
    const nf = new Intl.NumberFormat();
    const lines = filter.trim()
      ? `${nf.format(shownCount)} of ${nf.format(lineCount)} lines match`
      : `${nf.format(lineCount)} lines`;
    const clip = clipped ? ` (newest ${nf.format(mode === 'tail' ? Math.min(tailLines, MAX_LINES) : MAX_LINES)} kept)` : '';
    return `${lines}${clip} · ${plural(fileCount, 'file')} in the log folder`;
  });

  $effect(() => {
    const q = filterInput;
    const id = setTimeout(() => (filter = q), 150);
    return () => clearTimeout(id);
  });

  /** Keep at most `max` lines (from the end); cheap — scans back from the end. */
  function capLines(s: string, max: number): { text: string; cut: boolean } {
    let idx = s.length;
    // A trailing newline terminates the last line; it doesn't start a new one.
    if (s.endsWith('\n')) idx -= 1;
    for (let n = 0; n < max; n++) {
      idx = s.lastIndexOf('\n', idx - 1);
      if (idx < 0) return { text: s, cut: false };
    }
    return { text: s.slice(idx + 1), cut: true };
  }

  function lineCap(): number {
    return mode === 'tail' && selected !== ALL_FILES ? Math.min(Math.max(1, tailLines), MAX_LINES) : MAX_LINES;
  }

  /** Compare-before-assign: an unchanged read must not re-derive/re-render. */
  function setContent(next: string, serverTruncated: boolean): void {
    const { text, cut } = capLines(next, lineCap());
    if (text !== content) content = text;
    const c = cut || serverTruncated;
    if (c !== clipped) clipped = c;
  }

  function sameFiles(a: DaemonLogs | null, b: DaemonLogs): boolean {
    if (!a || a.log_dir !== b.log_dir || a.files.length !== b.files.length) return false;
    return a.files.every((f, i) => f.name === b.files[i].name);
  }

  function liveFileOf(data: DaemonLogs): string {
    return data.selected === ALL_FILES ? (data.files.at(-1)?.name ?? '') : data.selected;
  }

  // Severity markers: the level word on each line gets a tone (ERROR/WARN),
  // never the whole line (patterns.md → Logs). Escaped first, so log text can
  // never inject markup.
  const esc = escapeHtml;
  const highlighted = $derived(
    esc(visibleContent).replace(
      /(^|\s)(ERROR|WARN|WARNING)(?=\s)/gm,
      (_m, pre: string, lvl: string) => `${pre}<span class="lvl ${lvl === 'ERROR' ? 'err' : 'warn'}">${lvl}</span>`,
    ),
  );

  // Follow tails the log until the user scrolls up; "Jump to latest" resumes.
  function onLogScroll(): void {
    if (!logEl) return;
    const atBottom = logEl.scrollHeight - logEl.scrollTop - logEl.clientHeight < 24;
    if (follow && !atBottom) follow = false;
    else if (!follow && atBottom) follow = true;
  }
  function jumpToLatest(): void {
    follow = true;
    if (logEl) logEl.scrollTop = logEl.scrollHeight;
  }
  async function copyLog(): Promise<void> {
    const ok = await copyText(visibleContent);
    if (ok) toasts.success(filter.trim() ? 'Matching lines copied' : 'Log copied');
    else toasts.error('Couldn’t copy the log', COPY_FAILED_BODY);
  }

  $effect(() => {
    void loadInitial();
  });

  // Shared poll chain: a slow read never overlaps the next, and nothing runs
  // while the window is hidden (one catch-up read on return).
  $effect(() => {
    if (!autoRefresh) return;
    const p = pollWhileVisible(() => refreshIncremental(), { ms: 1500, immediate: false });
    return () => p.stop();
  });

  async function loadInitial(): Promise<void> {
    loading = true;
    loadError = '';
    try {
      const data = await fetchLogs(mode);
      payload = data;
      selected = data.selected;
      liveFile = liveFileOf(data);
      setContent(data.content, data.truncated);
      nextOffset = data.next_offset;
      await maybeFollow();
    } catch (e) {
      // Inline with Retry (a toast left an empty toolbar with nothing under it).
      loadError = loadErrorText(e);
      autoRefresh = false;
    } finally {
      loading = false;
    }
  }

  async function refreshFull(): Promise<void> {
    refreshing = true;
    const gen = ++readGen;
    try {
      const data = await fetchLogs(mode);
      if (gen !== readGen) return;
      if (!sameFiles(payload, data) || payload?.selected !== data.selected || payload?.mode !== data.mode) payload = data;
      selected = data.selected;
      liveFile = liveFileOf(data);
      setContent(data.content, data.truncated);
      nextOffset = data.next_offset;
      await maybeFollow();
    } catch (e) {
      failed('Couldn’t refresh the log', e);
    } finally {
      refreshing = false;
    }
  }

  /** Live tick: fetch only the bytes written since the last read of the live
   *  file (every mode — "All log files" and Tail included) and append. Falls
   *  back to a full read on rollover (a new newest file) or truncation. */
  async function refreshIncremental(): Promise<void> {
    if (refreshing || loading || loadError) return;
    if (!liveFile) {
      await refreshFull();
      return;
    }
    refreshing = true;
    let full = false;
    const gen = readGen;
    const file = liveFile;
    const from = nextOffset;
    try {
      const data = await fetchLogs('since', from, file);
      // A full read (file/mode switch) landed meanwhile — drop this append.
      if (gen !== readGen || file !== liveFile || from !== nextOffset) return;
      if (
        (selected === ALL_FILES && data.files.at(-1)?.name !== liveFile) ||
        data.next_offset < nextOffset
      ) {
        full = true; // new day's file, or the file shrank under us
      } else {
        if (!sameFiles(payload, data)) payload = { ...data, selected, mode };
        if (data.truncated) setContent(data.content, true);
        else if (data.content) setContent(content + data.content, clipped);
        nextOffset = data.next_offset;
        if (data.content) await maybeFollow();
      }
    } catch (e) {
      failed('Couldn’t update the log', e);
    } finally {
      refreshing = false;
    }
    if (full) await refreshFull();
  }

  /** A failed read. While Live is on this ran every 1.5 s and stacked a toast
   *  per tick; pause Live instead and say so once. */
  function failed(title: string, e: unknown): void {
    const msg = e instanceof Error ? e.message : String(e);
    if (autoRefresh) {
      autoRefresh = false;
      toasts.error(`${title} — live updates paused`, `${msg} Turn Live back on to retry.`);
    } else {
      toasts.error(title, msg);
    }
  }

  async function fetchLogs(fetchMode: LogMode | 'since', offset?: number, file?: string): Promise<DaemonLogs> {
    const params = new URLSearchParams();
    const f = file ?? selected;
    if (f) params.set('file', f);
    params.set('mode', fetchMode);
    if (fetchMode === 'tail') params.set('lines', String(Math.max(1, tailLines)));
    if (fetchMode === 'since') params.set('offset', String(offset ?? 0));
    return api.get<DaemonLogs>(`/logs/daemon?${params.toString()}`);
  }

  async function onFileChange(): Promise<void> {
    if (selected === ALL_FILES) mode = 'all';
    await refreshFull();
  }

  async function onModeChange(): Promise<void> {
    await refreshFull();
  }

  async function maybeFollow(): Promise<void> {
    if (!follow) return;
    await tick();
    if (logEl) logEl.scrollTop = logEl.scrollHeight;
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section logs-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "title":sectionLabel('logs'),"subtitle":payload?.log_dir ?? '~/Library/Logs/Otto',actions:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {         "class":`btn small`,...__sveltets_2_empty({"data-icon":`copy`}),...__sveltets_2_empty({"data-overflow":`-1`}),"disabled":loading || !visibleContent,"onclick":() => void copyLog(),});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});} 
       }
       { svelteHTML.createElement("button", {       "class":`btn small`,...__sveltets_2_empty({"data-icon":`refresh`}),"disabled":refreshing || loading,"onclick":() => (loadError ? void loadInitial() : void refreshFull()),});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}
        refreshing ? 'Refreshing…' : 'Refresh';
       }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
   PageHeader}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: {  children:() => { return __sveltets_2_any(0); },"padded":false,"fill":true,}});

  if(loading){
     { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":8,"height":34,}});} }
  } else if (loadError && !payload){
     { svelteHTML.createElement("div", { "class":`pad`,});
       { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {      "what":`the daemon logs`,"error":loadError,"empty":true,"onretry":() => void loadInitial(),}});}
     }
  }else{
     { svelteHTML.createElement("div", { "class":`toolbar`,});
       { svelteHTML.createElement("label", { "class":`field compact`,});
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("select", {     "class":`input mono`,"bind:value":selected,"onchange":onFileChange,});/*Ωignore_startΩ*/() => selected = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", { "value":ALL_FILES,});   }
             for(let file of __sveltets_2_ensureArray(payload?.files ?? [])){file.name;
             { svelteHTML.createElement("option", { "value":file.name,});file.name; }
          }
         }
       }

       { svelteHTML.createElement("label", { "class":`field mode`,});
         { svelteHTML.createElement("span", {});  }
        
         { svelteHTML.createElement("select", {           "class":`input`,"bind:value":mode,"onchange":onModeChange,"disabled":selected === ALL_FILES,"title":selected === ALL_FILES ? 'Pick one file to tail it' : undefined,});/*Ωignore_startΩ*/() => mode = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", { "value":`all`,});  }
           { svelteHTML.createElement("option", { "value":`tail`,});  }
         }
       }

      if(mode === 'tail'){
         { svelteHTML.createElement("label", { "class":`field lines`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("input", {               "class":`input mono`,"type":`number`,"min":`1`,"max":`50000`,"bind:value":tailLines,"title":`Up to 50,000 lines — press Enter or Refresh to apply`,"onkeydown":(e) => e.key === 'Enter' && refreshFull(),});/*Ωignore_startΩ*/() => tailLines = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
      }

       { svelteHTML.createElement("label", { "class":`field search-field`,});
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`input`,"type":`search`,"placeholder":`slack, telegram, bridge…`,"bind:value":filterInput,});/*Ωignore_startΩ*/() => filterInput = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }

       { svelteHTML.createElement("div", { "class":`checks`,});
         { svelteHTML.createElement("label", {   "class":`checkbox-row check-control`,"title":`Poll for new log lines every 1.5 s`,});
           { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":autoRefresh,});/*Ωignore_startΩ*/() => autoRefresh = __sveltets_2_any(null);/*Ωignore_endΩ*/}
          
         }
         { svelteHTML.createElement("label", {   "class":`checkbox-row check-control`,"title":`Keep the view scrolled to the newest line`,});
           { svelteHTML.createElement("input", {      "type":`checkbox`,"bind:checked":follow,"onchange":() => follow && jumpToLatest(),});/*Ωignore_startΩ*/() => follow = __sveltets_2_any(null);/*Ωignore_endΩ*/}
          
         }
         { svelteHTML.createElement("label", {   "class":`checkbox-row check-control`,"title":`Wrap long lines`,});
           { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":wrap,});/*Ωignore_startΩ*/() => wrap = __sveltets_2_any(null);/*Ωignore_endΩ*/}
          
         }
       }
     }

     { svelteHTML.createElement("div", { "class":`log-meta`,});
       { svelteHTML.createElement("span", {});status; }
      if(autoRefresh){ { svelteHTML.createElement("span", { "class":`live`,}); { svelteHTML.createElement("span", {   "class":`live-dot`,"aria-hidden":`true`,}); }  }}else{ { svelteHTML.createElement("span", {});  }}
     }

     { svelteHTML.createElement("div", { "class":`log-wrap`,});
      if(filter.trim() && !visibleContent){
         { svelteHTML.createElement("div", { "class":`no-match`,});
             filter.trim();
           { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => (filterInput = filter = ''),});  }
         }
      } else if (!content.trim()){
        
         { svelteHTML.createElement("div", { "class":`no-match`,});
          selected === ALL_FILES ? 'The log folder is empty.' : 'This log file is empty so far.';
          autoRefresh ? 'New lines appear here as the daemon writes them.' : 'Turn on Live to watch for new lines.';
         }
      }
      
       { const $$_div3 = svelteHTML.createElement("div", {               "class":`log-view`,"onscroll":onLogScroll,"role":`textbox`,"aria-readonly":`true`,"aria-multiline":`true`,"tabindex":0,"aria-label":`Daemon log`,});!wrap;logEl = $$_div3; highlighted; }
      if(!follow && visibleContent){
         { svelteHTML.createElement("button", {   "class":`btn small jump`,"onclick":jumpToLatest,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":12,}});}    }
      }
     }
  }
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Logs__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Logs__SvelteComponent_ = ReturnType<typeof Logs__SvelteComponent_>;
/*Ωignore_endΩ*/export default Logs__SvelteComponent_;