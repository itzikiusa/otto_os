///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
// Settings → Backup & restore → "Database storage" (root only).
// Shows otto.db's size + reclaimable free space and runs the ONE-TIME
// compaction (POST /admin/db/compact): auto_vacuum=INCREMENTAL + VACUUM.
// "Compact now" rewrites the whole file and blocks every database write
// meanwhile, so it only runs after an explicit confirm. "Compact at next
// restart" (at: next_restart) copies + swaps the file offline before the
// daemon opens it — no write stall, the start takes a few seconds longer.
// Afterwards the daemon's hourly maintenance gives free pages back on its own.
// Also hosts the opt-in "Run history" window (`data_retention.
// run_history_days`, default 0 = keep forever): turning it on deletes old
// finished runs, so it asks first and says exactly what goes.
import { onMount } from 'svelte';
import { api } from '../../lib/api/client';
import type { DbStatsResp, DbCompactReport, DbCompactScheduled } from '../../lib/api/types';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { formatBytes } from '../../lib/metric-format';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { RUN_HISTORY_CHOICES, runHistoryDays, runHistoryLabel, withRunHistoryDays } from './runHistory';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let stats = $state<DbStatsResp | null>(null);
  let loading = $state(true);
  let loadError = $state('');
  let compacting = $state(false);
  let last = $state<DbCompactReport | null>(null);
  const msg = (e: unknown): string => (e instanceof Error ? e.message : String(e));

  // Run-history retention (opt-in). `settings` is the stored map, so a change
  // writes the whole `data_retention` object back with one field changed.
  let settings = $state<Record<string, unknown> | null>(null);
  let historyError = $state('');
  let savingHistory = $state(false);
  const historyDays = $derived(runHistoryDays(settings));
  // A stored custom window (e.g. 45) stays selectable.
  const historyChoices = $derived(
    RUN_HISTORY_CHOICES.includes(historyDays) ? RUN_HISTORY_CHOICES : [...RUN_HISTORY_CHOICES, historyDays].sort((a, b) => a - b),
  );

  async function loadSettings() {
    historyError = '';
    try {
      settings = await api.get<Record<string, unknown>>('/settings');
    } catch (e) {
      historyError = loadErrorText(e);
    }
  }

  async function load() {
    loading = true;
    loadError = '';
    void loadSettings();
    try {
      stats = await api.get<DbStatsResp>('/admin/db/stats');
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }
  onMount(load);

  async function setHistory(ev: Event) {
    const select = ev.currentTarget as HTMLSelectElement;
    const days = Number(select.value);
    if (days === historyDays) return;
    if (days > 0) {
      const ok = await confirmer.ask(
        `Delete finished runs older than ${days} days, every hour from now on? This removes Run with Otto runs and their ` +
          'events (unless a Proof Pack is attached), finished swarm runs and swarm messages, and all but the last iteration ' +
          'of finished goal loops. Running work is never touched, and swarm budgets keep counting what was removed. ' +
          'Deleted history can’t be recovered.',
        { title: 'Prune old run history', confirmLabel: `Keep ${days} days`, danger: true },
      );
      if (!ok) {
        select.value = String(historyDays);
        return;
      }
    }
    savingHistory = true;
    try {
      settings = await api.put<Record<string, unknown>>('/settings', {
        data_retention: withRunHistoryDays(settings, days),
      });
      toasts.success(
        days > 0 ? 'Run history window set' : 'Run history kept forever',
        days > 0 ? `Finished runs older than ${days} days are pruned hourly.` : 'Nothing is pruned from run history.',
      );
    } catch (e) {
      select.value = String(historyDays);
      toasts.error('Couldn’t change run history retention', msg(e));
    } finally {
      savingHistory = false;
    }
  }

  const compacted = $derived(stats?.auto_vacuum === 2);
  let scheduling = $state(false);
  const secs = (ms: number): string => (ms < 10_000 ? (ms / 1000).toFixed(1) : Math.round(ms / 1000).toString());

  async function schedule(at: 'next_restart' | 'cancel') {
    if (!stats) return;
    if (at === 'next_restart') {
      const ok = await confirmer.ask(
        `Compact otto.db (${formatBytes(stats.size_bytes)}) the next time Otto’s daemon starts? ` +
          `It makes a compacted, verified copy before anything opens the database, so no write waits; that start takes ` +
          `about ${secs(stats.estimated_offline_ms)} s longer. The original file is kept until the new one opens cleanly. ` +
          'Nothing is deleted.',
        { title: 'Compact at next restart', confirmLabel: 'Schedule' },
      );
      if (!ok) return;
    }
    scheduling = true;
    try {
      const r = await api.post<DbCompactScheduled>('/admin/db/compact', { confirm: true, at });
      if (at === 'next_restart') {
        toasts.success('Compaction scheduled', `Runs at the next daemon start (≈${secs(r.estimated_offline_ms)} s).`);
      } else {
        toasts.success('Scheduled compaction canceled');
      }
      await load();
    } catch (e) {
      toasts.error('Couldn’t schedule the compaction', msg(e));
    } finally {
      scheduling = false;
    }
  }

  async function compact() {
    if (!stats) return;
    const ok = await confirmer.ask(
      `Rewrite otto.db (${formatBytes(stats.size_bytes)}) to give back about ${formatBytes(stats.free_bytes)} of free space? ` +
        'While it runs — usually under a minute, longer for a large database — every database write waits, so sessions, ' +
        'workflows and the UI may stall or show errors. Nothing is deleted. Run it when no agents are busy.',
      { title: 'Compact database', confirmLabel: 'Compact now', danger: true },
    );
    if (!ok) return;
    compacting = true;
    try {
      last = await api.post<DbCompactReport>('/admin/db/compact', { confirm: true });
      toasts.success('Database compacted', `Freed ${formatBytes(last.freed_bytes)} in ${(last.duration_ms / 1000).toFixed(1)} s.`);
      await load();
    } catch (e) {
      toasts.error('Couldn’t compact the database', msg(e));
    } finally {
      compacting = false;
    }
  }

  // a11y-6-01: announce at the state edge — a status element inserted
  // together with its text is not read by VoiceOver/WebKit.
  $effect(() => {
    if (last) announce(`Compacted: ${formatBytes(last.before_bytes)} → ${formatBytes(last.after_bytes)} in ${(last.duration_ms / 1000).toFixed(1)} s.`);
  });
;
async () => {

 { svelteHTML.createElement("section", {   "class":`db-card`,"aria-label":`Database storage`,});
   { svelteHTML.createElement("h2", { "class":`card-title`,});  }
   { svelteHTML.createElement("p", {});                 
                        
           }
   { svelteHTML.createElement("div", { "class":`history`,});
     { svelteHTML.createElement("label", { "for":`run-history-days`,});  }
    if(settings){
       { svelteHTML.createElement("select", {             "id":`run-history-days`,"class":`input`,"value":String(historyDays),"disabled":savingHistory,"aria-describedby":`run-history-hint`,"onchange":setHistory,});
           for(let d of __sveltets_2_ensureArray(historyChoices)){d;
           { svelteHTML.createElement("option", { "value":String(d),});runHistoryLabel(d); }
        }
       }
    } else if (historyError){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          "what":`the retention setting`,"variant":`compact`,"error":historyError,"empty":true,"onretry":loadSettings,}});}
    }else{
       { svelteHTML.createElement("span", { "class":`dim`,});  }
    }
   }
   { svelteHTML.createElement("p", {   "id":`run-history-hint`,"class":`dim`,});
                       
   }
  if(!stats){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {         "what":`the database size`,loading,"error":loadError,"empty":true,"onretry":load,}});}
  }else{
     { svelteHTML.createElement("dl", { "class":`facts`,});
       { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});formatBytes(stats.size_bytes); } }
       { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});formatBytes(stats.free_bytes); } }
       { svelteHTML.createElement("div", {});
         { svelteHTML.createElement("dt", {});  }
         { svelteHTML.createElement("dd", {});
          if(stats.compaction_scheduled){     secs(stats.estimated_offline_ms); }else{
            compacted ? 'Done — free space is returned hourly' : 'Not yet';
          }
         }
       }
     }
     { svelteHTML.createElement("div", { "class":`controls`,});
      if(stats.compaction_scheduled){
         { svelteHTML.createElement("button", {     "class":`btn`,"disabled":scheduling,"onclick":() => schedule('cancel'),});   }
      }else{
         { svelteHTML.createElement("button", {     "class":`btn`,"disabled":scheduling || compacting || stats.compacting,"onclick":() => schedule('next_restart'),});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`db`,"size":13,}});}
             
         }
      }
       { svelteHTML.createElement("button", {     "class":`btn`,"disabled":compacting || stats.compacting,"onclick":compact,});
        compacting || stats.compacting ? 'Compacting…' : compacted ? 'Compact again now…' : 'Compact now…';
       }
     }
    if(last){
       { svelteHTML.createElement("p", { "class":`notice`,});formatBytes(last.before_bytes);  formatBytes(last.after_bytes);  (last.duration_ms / 1000).toFixed(1);  }
    }
  }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const DatabaseCompact__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type DatabaseCompact__SvelteComponent_ = ReturnType<typeof DatabaseCompact__SvelteComponent_>;
/*Ωignore_endΩ*/export default DatabaseCompact__SvelteComponent_;