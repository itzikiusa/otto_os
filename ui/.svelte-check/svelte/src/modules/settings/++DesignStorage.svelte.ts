///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
// Settings → Backup & restore → "Design Hall storage" (root only).
// Every Design Hall content save keeps a full copy, so the blob store only
// grows. Auto-tidy squashes OLD autosave versions (older than a week, one
// kept per 10-minute editing window; never the head, approved, pinned,
// published or signal-cited versions) — it deletes user history, so it is
// OPT-IN: off by default, and this card shows what it would reclaim right
// now so the person decides with the number in front of them
// (GET /design/admin/storage, PUT /design/admin/auto-tidy).
import { onMount } from 'svelte';
import { api } from '../../lib/api/client';
import type { DesignStorageReport, DesignAutoTidyReq } from '../../lib/api/types';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { formatBytes } from '../../lib/metric-format';
import SettingToggle from './SettingToggle.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { toastError } from '../../lib/toastError';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let report = $state<DesignStorageReport | null>(null);
  let loading = $state(true);
  let loadError = $state('');
  let saving = $state(false);

  async function load() {
    loading = true;
    loadError = '';
    try {
      report = await api.get<DesignStorageReport>('/design/admin/storage');
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }
  onMount(load);

  const days = $derived(report ? Math.round(report.min_age_secs / 86_400) : 7);
  const forcedOff = $derived(report?.auto_tidy_forced === false);
  const forcedOn = $derived(report?.auto_tidy_forced === true);

  async function toggle(on: boolean): Promise<void> {
    if (!report) return;
    if (on) {
      const r = report.reclaimable;
      const ok = await confirmer.ask(
        `Once a day, Otto will delete Design Hall autosave versions older than ${days} days, keeping the last one ` +
          'of each 10-minute editing window. Named, agent, approved, pinned and published versions and the current ' +
          `version are never touched. Right now this would remove ${plural(r.versions, 'version')} ` +
          `and free about ${formatBytes(r.bytes)}. Deleted autosaves can’t be restored.`,
        { title: 'Turn on auto-tidy', confirmLabel: 'Turn on', danger: true },
      );
      if (!ok) return;
    }
    saving = true;
    try {
      report = await api.put<DesignStorageReport>('/design/admin/auto-tidy', { enabled: on } satisfies DesignAutoTidyReq);
      toasts.success(on ? 'Design auto-tidy is on' : 'Design auto-tidy is off');
    } catch (e) {
      toastError('Couldn’t change auto-tidy', e);
    } finally {
      saving = false;
    }
  }
;
async () => {

 { svelteHTML.createElement("section", {     "class":`ds-card`,"aria-label":`Design Hall storage`,...__sveltets_2_empty({"data-testid":`design-storage`}),});
   { svelteHTML.createElement("h2", { "class":`card-title`,});   }
   { svelteHTML.createElement("p", {});                     
            }
  if(!report){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {         "what":`Design Hall storage`,loading,"error":loadError,"empty":true,"onretry":load,}});}
  }else{
     { svelteHTML.createElement("dl", { "class":`facts`,});
       { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { ...__sveltets_2_empty({"data-testid":`design-storage-bytes`}),});formatBytes(report.blob_bytes); } }
       { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});report.version_count.toLocaleString(); } }
       { svelteHTML.createElement("div", {});
         { svelteHTML.createElement("dt", {});   }
         { svelteHTML.createElement("dd", { ...__sveltets_2_empty({"data-testid":`design-storage-reclaim`}),});
          formatBytes(report.reclaimable.bytes);
           { svelteHTML.createElement("span", { "class":`dim`,}); report.reclaimable.versions.toLocaleString(); report.reclaimable.versions === 1 ? 'old autosave' : 'old autosaves';  }
         }
       }
     }
     { const $$_elggoTgnitteS1C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS1C({ target: __sveltets_2_any(), props: {               "label":`Auto-tidy old autosaves`,"checked":report.auto_prune,"disabled":saving || report.auto_tidy_forced !== null,"title":forcedOff
        ? 'Turned off by OTTO_DESIGN_AUTO_PRUNE=0 on the daemon'
        : forcedOn
          ? 'Forced on by OTTO_DESIGN_AUTO_PRUNE=1 on the daemon'
          : undefined,"hint":forcedOff
        ? 'Disabled on this daemon (OTTO_DESIGN_AUTO_PRUNE=0).'
        : forcedOn
          ? 'Forced on by the daemon (OTTO_DESIGN_AUTO_PRUNE=1).'
          : `Once a day, delete autosaves older than ${days} days, keeping one per 10-minute editing window.`,"onchange":toggle,"testid":`design-auto-tidy`,}});}
    if(report.last_prune){
       { svelteHTML.createElement("p", { "class":`dim`,});
          new Date(report.last_prune.at).toLocaleString();  plural(report.last_prune.versions_removed, 'version');
       }
    }
  }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const DesignStorage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type DesignStorage__SvelteComponent_ = ReturnType<typeof DesignStorage__SvelteComponent_>;
/*Ωignore_endΩ*/export default DesignStorage__SvelteComponent_;