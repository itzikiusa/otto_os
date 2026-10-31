///<reference types="svelte" />
;
import { latestOnly } from '../../lib/latest';
import PageHeader from '../../lib/components/PageHeader.svelte';
import { sectionLabel } from './sections';
import PageBody from '../../lib/components/PageBody.svelte';
// Trust & Safety Center (root only): a derived security-posture summary plus
// the filterable, paged security audit log. Reads `GET /security-posture` and
// `GET /audit-log`; writes nothing. user_id -> username is resolved from the
// users list so entries read sensibly.
import { api } from '../../lib/api/client';
import type { AuditEntry, AuditLogResp, SecurityPostureResp, User } from '../../lib/api/types';
import Icon from '../../lib/components/Icon.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import { toasts } from '../../lib/toast.svelte';
import { copyAsJson } from '../../lib/components/exporters';
import { COPY_FAILED_BODY } from '../../lib/clipboard';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { router } from '../../lib/router.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { loadErrorText } from '../../lib/loadError';
import { auth } from '../../lib/stores/auth.svelte';
import SecretsStore from './SecretsStore.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const PAGE_SIZE = 100;

  // The actions we audit today; drives the filter dropdown. Free-form on the
  // wire, so an unknown action still renders — this is just the curated list.
  const KNOWN_ACTIONS: { value: string; label: string }[] = [
    { value: '', label: 'All actions' },
    { value: 'login.success', label: 'Login success' },
    { value: 'login.failure', label: 'Login failure' },
    { value: 'login.lockout', label: 'Login lockout' },
    { value: 'token.mint', label: 'API token minted' },
    { value: 'token.revoke', label: 'API token revoked' },
    { value: 'settings.change', label: 'Settings changed' },
    { value: 'network_listener.toggle', label: 'Network listener toggled' },
    { value: 'db.write_confirmed', label: 'DB write confirmed' },
  ];

  let posture: SecurityPostureResp | null = $state(null);
  let postureLoading = $state(true);
  let postureError = $state('');

  let entries: AuditEntry[] = $state([]);
  let total = $state(0);
  let logLoading = $state(true);
  let logError = $state('');

  // Filters.
  let action = $state('');
  let fromDate = $state(''); // yyyy-mm-dd
  let toDate = $state(''); // yyyy-mm-dd
  let offset = $state(0);

  // user_id -> username, populated from /users (best-effort; root sees all).
  let usernames: Record<string, string> = $state({});

  $effect(() => {
    void loadPosture();
    void loadUsers();
  });

  // Re-fetch the log whenever a filter or the page changes.
  $effect(() => {
    // touch the reactive deps so the effect re-runs on change
    void action;
    void fromDate;
    void toDate;
    void offset;
    void loadLog();
  });

  async function loadPosture(): Promise<void> {
    postureLoading = true;
    try {
      posture = await api.get<SecurityPostureResp>('/security-posture');
      postureError = '';
    } catch (e) {
      // Inline, with Retry — a toast vanished and left an empty strip.
      postureError = loadErrorText(e);
    } finally {
      postureLoading = false;
    }
  }

  async function loadUsers(): Promise<void> {
    try {
      const users = await api.get<User[]>('/users');
      const map: Record<string, string> = {};
      for (const u of users) map[u.id] = u.username;
      usernames = map;
    } catch {
      // Non-fatal: entries still render with the raw id.
    }
  }

  function startOfDayIso(d: string): string | undefined {
    if (!d) return undefined;
    return new Date(`${d}T00:00:00`).toISOString();
  }
  function endOfDayIso(d: string): string | undefined {
    if (!d) return undefined;
    return new Date(`${d}T23:59:59.999`).toISOString();
  }

  // Filters change faster than the log answers: only the latest filter's
  // response may land (an older one showed entries that didn't match it).
  const logSeq = latestOnly();
  async function loadLog(): Promise<void> {
    const t = logSeq.begin();
    logLoading = true;
    logError = '';
    try {
      const params = new URLSearchParams();
      params.set('limit', String(PAGE_SIZE));
      params.set('offset', String(offset));
      if (action) params.set('action', action);
      const from = startOfDayIso(fromDate);
      const to = endOfDayIso(toDate);
      if (from) params.set('from', from);
      if (to) params.set('to', to);
      const resp = await api.get<AuditLogResp>(`/audit-log?${params.toString()}`, t.signal);
      if (!t.current) return;
      entries = resp.entries;
      total = resp.total;
    } catch (e) {
      if (!t.current) return;
      logError = loadErrorText(e);
    } finally {
      if (t.current) logLoading = false;
    }
  }

  function resetFilters(): void {
    action = '';
    fromDate = '';
    toDate = '';
    offset = 0;
  }

  // Quick-preset helpers: set from/to to the last N days from now.
  function toYmd(d: Date): string {
    // yyyy-mm-dd in local time — matches the date input value format.
    const y = d.getFullYear();
    const m = String(d.getMonth() + 1).padStart(2, '0');
    const day = String(d.getDate()).padStart(2, '0');
    return `${y}-${m}-${day}`;
  }

  // Presets are whole calendar days (the date inputs have day granularity):
  // "Last 7 days" is today plus the six days before it — a "Last 24h" label
  // over a two-calendar-day range claimed a precision the filter doesn't have.
  const PRESETS: { days: number; label: string }[] = [
    { days: 1, label: 'Today' },
    { days: 7, label: 'Last 7 days' },
    { days: 30, label: 'Last 30 days' },
  ];
  function presetRange(days: number): { from: string; to: string } {
    const now = new Date();
    const from = new Date(now);
    from.setDate(from.getDate() - (days - 1));
    return { from: toYmd(from), to: toYmd(now) };
  }
  function applyPreset(days: number): void {
    const r = presetRange(days);
    fromDate = r.from;
    toDate = r.to;
    offset = 0;
  }
  // The preset the current range matches (so the segment shows as selected).
  const activePreset = $derived(
    PRESETS.find((p) => {
      const r = presetRange(p.days);
      return r.from === fromDate && r.to === toDate;
    })?.days ?? null,
  );

  // Per-entry copy: tracks which entry is being copied (for brief feedback).
  let copyingId: string | null = $state(null);

  async function copyEntry(e: AuditEntry): Promise<void> {
    try {
      await copyAsJson(e);
      copyingId = e.id;
      // Brief check mark so the user sees it landed.
      setTimeout(() => {
        if (copyingId === e.id) copyingId = null;
      }, 1200);
    } catch {
      toasts.error('Couldn’t copy the entry', COPY_FAILED_BODY);
    }
  }

  const filtered = $derived(!!action || !!fromDate || !!toDate);

  function actorLabel(e: AuditEntry): string {
    if (!e.user_id) return 'Anonymous';
    return usernames[e.user_id] ? `@${usernames[e.user_id]}` : e.user_id;
  }

  function actionLabel(a: string): string {
    return KNOWN_ACTIONS.find((k) => k.value === a)?.label ?? a;
  }

  function fmtTime(ts: string): string {
    const d = new Date(ts);
    return Number.isNaN(d.getTime()) ? ts : d.toLocaleString();
  }

  function detailText(e: AuditEntry): string {
    if (e.detail == null) return '';
    try {
      return typeof e.detail === 'string' ? e.detail : JSON.stringify(e.detail);
    } catch {
      return '';
    }
  }

  const pageStart = $derived(total === 0 ? 0 : offset + 1);
  const pageEnd = $derived(Math.min(offset + entries.length, total));
  const canPrev = $derived(offset > 0);
  const canNext = $derived(offset + PAGE_SIZE < total);
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section trust-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "title":sectionLabel('trust-safety'),"subtitle":`Security posture and audit log`,actions:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {         "class":`btn small`,...__sveltets_2_empty({"data-icon":`refresh`}),"disabled":logLoading || postureLoading,"onclick":() => {
          void loadPosture();
          void loadLog();
        },});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}
        logLoading || postureLoading ? 'Refreshing…' : 'Refresh';
       }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
   PageHeader}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: {  children:() => { return __sveltets_2_any(0); },"padded":false,"fill":true,}});

  
  if(auth.isRoot){ { const $$_erotSsterceS2C = __sveltets_2_ensureComponent(SecretsStore); new $$_erotSsterceS2C({ target: __sveltets_2_any(), props: {}});}}

  
   { svelteHTML.createElement("section", {   "class":`posture`,"aria-label":`Security posture`,});
    if(postureLoading && !posture){
       { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":1,"height":64,}});}
    } else if (postureError && !posture){
       { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {        "what":`the security posture`,"error":postureError,"empty":true,"variant":`compact`,"onretry":() => void loadPosture(),}});}
    } else if (posture){
       { svelteHTML.createElement("div", { "class":`cards`,});
         { svelteHTML.createElement("div", { "class":`card`,});
           { svelteHTML.createElement("div", { "class":`card-label`,});  }
           { svelteHTML.createElement("div", { "class":`card-value`,});
            if(posture.network_listener){
               { const $$_egdaBsutatS6C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS6C({ target: __sveltets_2_any(), props: {    "tone":`warning`,"label":posture.network_listener_port ? `On · port ${posture.network_listener_port}` : 'On',}});}
            }else{
               { const $$_egdaBsutatS6C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS6C({ target: __sveltets_2_any(), props: {    "tone":`success`,"label":`Off`,}});}
            }
           }
           { svelteHTML.createElement("div", { "class":`card-note`,});
            posture.network_listener ? 'Reachable from your network' : 'Only this Mac can connect'; 
             { svelteHTML.createElement("button", {   "class":`link-btn`,"onclick":() => router.go('settings/daemon'),});  }
            if(posture.network_listener_restart_required){
               { svelteHTML.createElement("div", {});               }
            }
           }
         }
         { svelteHTML.createElement("div", { "class":`card`,});
           { svelteHTML.createElement("div", { "class":`card-label`,});  }
           { svelteHTML.createElement("div", { "class":`card-value mono-val`,});posture.loopback_only ? '127.0.0.1' : '0.0.0.0'; }
           { svelteHTML.createElement("div", { "class":`card-note`,});posture.loopback_only ? 'Loopback only' : 'All network interfaces'; }
         }
         { svelteHTML.createElement("div", { "class":`card`,});
           { svelteHTML.createElement("div", { "class":`card-label`,});   }
           { svelteHTML.createElement("div", { "class":`card-value num`,});posture.active_api_tokens; }
           { svelteHTML.createElement("div", { "class":`card-note`,});
                    
             { svelteHTML.createElement("button", {   "class":`link-btn`,"onclick":() => router.go('settings/tokens'),});  }
           }
         }
       }
    }
   }

  
   { svelteHTML.createElement("div", {     "class":`toolbar`,"role":`group`,"aria-label":`Filter the audit log`,});
     { svelteHTML.createElement("label", { "class":`tfield`,});
       { svelteHTML.createElement("span", {});  }
       { svelteHTML.createElement("select", {     "class":`input`,"bind:value":action,"onchange":() => (offset = 0),});/*Ωignore_startΩ*/() => action = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let a of __sveltets_2_ensureArray(KNOWN_ACTIONS)){a.value;
           { svelteHTML.createElement("option", { "value":a.value,});a.label; }
        }
       }
     }
     { svelteHTML.createElement("label", { "class":`tfield`,});
       { svelteHTML.createElement("span", {});  }
       { svelteHTML.createElement("input", {        "class":`input`,"type":`date`,"bind:value":fromDate,"onchange":() => (offset = 0),});/*Ωignore_startΩ*/() => fromDate = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
     { svelteHTML.createElement("label", { "class":`tfield`,});
       { svelteHTML.createElement("span", {});  }
       { svelteHTML.createElement("input", {        "class":`input`,"type":`date`,"bind:value":toDate,"onchange":() => (offset = 0),});/*Ωignore_startΩ*/() => toDate = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
    
     { svelteHTML.createElement("div", {     "class":`segmented presets`,"role":`group`,"aria-label":`Quick range`,});
         for(let p of __sveltets_2_ensureArray(PRESETS)){p.days;
         { svelteHTML.createElement("button", {     "aria-pressed":activePreset === p.days,"onclick":() => applyPreset(p.days),});activePreset === p.days;
          p.label;
         }
      }
     }
    if(filtered){
       { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":resetFilters,});  }
    }
   }

  
   { svelteHTML.createElement("span", {   "class":`sr-only`,"role":`status`,});total > 0 ? `${pageStart}–${pageEnd} of ${total} entr${total === 1 ? 'y' : 'ies'}` : ''; }
  
  if(total > 0){
   { svelteHTML.createElement("div", { "class":`log-meta`,});
     { svelteHTML.createElement("span", {});
      pageStart; pageEnd;  total; total === 1 ? 'y' : 'ies';
     }
    if(total > PAGE_SIZE){
       { svelteHTML.createElement("div", { "class":`pager`,});
         { svelteHTML.createElement("button", {     "class":`btn small ghost`,"disabled":!canPrev,"onclick":() => (offset = Math.max(0, offset - PAGE_SIZE)),});
           { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":12,}});} 
         }
         { svelteHTML.createElement("button", {     "class":`btn small ghost`,"disabled":!canNext,"onclick":() => (offset = offset + PAGE_SIZE),});
            { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":12,}});}
         }
       }
    }
   }
  }

   { svelteHTML.createElement("div", { "class":`log-body`,});
    if(logLoading && entries.length === 0){
       { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":8,"height":34,}});}
    } else if (logError){
       { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {      "what":`the audit log`,"error":logError,"empty":true,"onretry":() => void loadLog(),}});}
    } else if (entries.length === 0){
      if(filtered){
         { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {             "icon":`filter`,"title":`No entries match these filters`,"body":`Widen the date range or pick another action.`,"actionLabel":`Clear filters`,"actionKind":`secondary`,"onaction":resetFilters,}});}
      }else{
         { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {       "icon":`shield`,"title":`No audit entries yet`,"body":`Sign-ins, API token changes and security settings changes are recorded here, and can’t be edited or deleted.`,}});}
      }
    }else{
       { svelteHTML.createElement("table", { "class":`audit-table`,});
         { svelteHTML.createElement("thead", {});
           { svelteHTML.createElement("tr", {});
             { svelteHTML.createElement("th", {   "class":`col-time`,"scope":`col`,});  }
             { svelteHTML.createElement("th", {   "class":`col-action`,"scope":`col`,});  }
             { svelteHTML.createElement("th", {   "class":`col-actor`,"scope":`col`,});  }
             { svelteHTML.createElement("th", {   "class":`col-target`,"scope":`col`,});  }
             { svelteHTML.createElement("th", {   "class":`col-ip`,"scope":`col`,});  }
             { svelteHTML.createElement("th", {   "class":`col-detail`,"scope":`col`,});  }
             { svelteHTML.createElement("th", {   "class":`col-copy`,"scope":`col`,}); { svelteHTML.createElement("span", { "class":`sr-only`,});  } }
           }
         }
         { svelteHTML.createElement("tbody", {});
             for(let e of __sveltets_2_ensureArray(entries)){e.id;
             { svelteHTML.createElement("tr", {});
               { svelteHTML.createElement("td", { "class":`col-time mono`,});fmtTime(e.ts); }
               { svelteHTML.createElement("td", { "class":`col-action`,});
                 { svelteHTML.createElement("span", {  "class":`chip`,});e.action === 'login.failure' || e.action === 'login.lockout';
                  actionLabel(e.action);
                 }
               }
               { svelteHTML.createElement("td", {   "class":`col-actor mono`,"title":e.user_id ?? undefined,});actorLabel(e); }
               { svelteHTML.createElement("td", {   "class":`col-target mono`,"title":e.target ?? '',});e.target ?? '—'; }
               { svelteHTML.createElement("td", { "class":`col-ip mono`,});e.ip ?? '—'; }
               { svelteHTML.createElement("td", {   "class":`col-detail mono`,"title":detailText(e),});detailText(e) || '—'; }
               { svelteHTML.createElement("td", { "class":`col-copy`,});
                 { svelteHTML.createElement("button", {         "class":`icon-btn copy-btn`,"title":copyingId === e.id ? 'Copied' : 'Copy entry as JSON',"aria-label":copyingId === e.id ? 'Copied' : 'Copy entry as JSON',"onclick":() => void copyEntry(e),});
                   { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":copyingId === e.id ? 'check' : 'copy',"size":14,}});}
                 }
               }
             }
          }
         }
       }
    }
   }
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const TrustSafety__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TrustSafety__SvelteComponent_ = ReturnType<typeof TrustSafety__SvelteComponent_>;
/*Ωignore_endΩ*/export default TrustSafety__SvelteComponent_;