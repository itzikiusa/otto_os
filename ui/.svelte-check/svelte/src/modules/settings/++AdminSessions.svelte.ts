///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
import PageHeader from '../../lib/components/PageHeader.svelte';
import { toastError } from '../../lib/toastError';
import { sectionLabel } from './sections';
import PageBody from '../../lib/components/PageBody.svelte';
import SectionIntro from './SectionIntro.svelte';
// Admin active-sessions overview: list every session daemon-wide; terminate
// (kill the PTY, keep the row) or remove (delete the row + history), one at a
// time or in bulk. "Remove all exited" prunes the background/ephemeral
// sessions (insights, analysis, …) that otherwise accumulate without bound.
import { api } from '../../lib/api/client';
import type { AdminSessionRow, AdminSessionsResp } from '../../lib/api/types';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { loadErrorText } from '../../lib/loadError';
import { sentenceCase, type Tone } from '../../lib/status';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let sessions: AdminSessionRow[] = $state([]);
  let loading = $state(true);
  let loadError = $state('');
  let busy: Set<string> = $state(new Set());
  let selected: Set<string> = $state(new Set());
  let bulkBusy = $state(false);
  // backend-performance-6-05: the daemon pages the overview (unarchived by
  // default, newest first); archived history is opt-in.
  let includeArchived = $state(false);
  let nextCursor = $state<string | null>(null);
  let loadingMore = $state(false);

  const exitedCount = $derived(sessions.filter((s) => !s.live).length);
  const liveCount = $derived(sessions.length - exitedCount);
  // Background runs (insights, analysis…) pile up as ended rows; a Live /
  // Ended filter finds the running ones without scrolling past them.
  let show = $state<'all' | 'live' | 'ended'>('all');
  // Only filter while both kinds exist (the control hides otherwise), so a
  // refresh that empties one side can't strand the table on nothing.
  const effShow = $derived(exitedCount > 0 && liveCount > 0 ? show : 'all');
  const visible = $derived(
    effShow === 'all' ? sessions : sessions.filter((s) => (effShow === 'live' ? s.live : !s.live)),
  );
  const allSelected = $derived(visible.length > 0 && visible.every((s) => selected.has(s.id)));
  function setShow(next: 'all' | 'live' | 'ended'): void {
    show = next;
    // Never act on rows the filter hides.
    const ids = new Set(visible.map((s) => s.id));
    selected = new Set([...selected].filter((id) => ids.has(id)));
  }

  $effect(() => {
    void load();
  });

  function overviewUrl(cursor?: string): string {
    const q = new URLSearchParams();
    if (includeArchived) q.set('include_archived', 'true');
    if (cursor) q.set('cursor', cursor);
    const qs = q.toString();
    return qs ? `/admin/sessions?${qs}` : '/admin/sessions';
  }

  async function load(): Promise<void> {
    loading = true;
    try {
      const resp = await api.get<AdminSessionsResp>(overviewUrl());
      sessions = resp.sessions;
      nextCursor = resp.next_cursor ?? null;
      // Drop selections for rows that no longer exist.
      const ids = new Set(sessions.map((s) => s.id));
      selected = new Set([...selected].filter((id) => ids.has(id)));
      loadError = '';
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  async function loadMore(): Promise<void> {
    if (!nextCursor || loadingMore) return;
    loadingMore = true;
    try {
      const resp = await api.get<AdminSessionsResp>(overviewUrl(nextCursor));
      const have = new Set(sessions.map((s) => s.id));
      sessions = [...sessions, ...resp.sessions.filter((s) => !have.has(s.id))];
      nextCursor = resp.next_cursor ?? null;
    } catch (e) {
      toastError('Couldn’t load more sessions', e);
    } finally {
      loadingMore = false;
    }
  }

  function setIncludeArchived(on: boolean): void {
    includeArchived = on;
    void load();
  }

  function toggle(id: string): void {
    const next = new Set(selected);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    selected = next;
  }
  function toggleAll(): void {
    selected = allSelected ? new Set() : new Set(visible.map((s) => s.id));
  }

  async function act(id: string, kind: 'terminate' | 'remove'): Promise<void> {
    busy = new Set([...busy, id]);
    try {
      await api.post(`/admin/sessions/${id}/${kind}`, {});
    } finally {
      busy = new Set([...busy].filter((x) => x !== id));
    }
  }

  async function terminate(id: string, title: string): Promise<void> {
    const ok = await confirmer.ask(
      `Terminate “${title || 'Untitled session'}”? Its process is killed and every viewer is disconnected. The session stays in the list (and its history is kept).`,
      { title: 'Terminate session', confirmLabel: 'Terminate', danger: true },
    );
    if (!ok) return;
    try {
      await act(id, 'terminate');
      toasts.success('Session terminated', title);
      await load();
    } catch (e) {
      toastError(`Couldn’t terminate “${title}”`, e);
    }
  }

  async function remove(id: string, title: string): Promise<void> {
    const ok = await confirmer.ask(
      `Delete “${title || 'Untitled session'}”? The session and its history are permanently deleted.`,
      { title: 'Delete session', confirmLabel: 'Delete', danger: true },
    );
    if (!ok) return;
    try {
      await act(id, 'remove');
      toasts.success('Session deleted', title);
      await load();
    } catch (e) {
      toastError(`Couldn’t delete “${title}”`, e);
    }
  }

  // Run `kind` over `ids` with bounded concurrency, after one confirm.
  async function runBulk(ids: string[], kind: 'terminate' | 'remove', label: string): Promise<void> {
    if (ids.length === 0) return;
    const ok = await confirmer.ask(
      `${label} ${plural(ids.length, 'session')}?${
        kind === 'remove' ? ' They and their history are permanently deleted.' : ' Their processes are killed; the rows and history are kept.'
      }`,
      { title: `${label} sessions`, confirmLabel: label, danger: true },
    );
    if (!ok) return;
    bulkBusy = true;
    let done = 0;
    let failed = 0;
    const queue = [...ids];
    const worker = async (): Promise<void> => {
      while (queue.length) {
        const id = queue.shift();
        if (id === undefined) break;
        try {
          await act(id, kind);
          done++;
        } catch {
          failed++;
        }
      }
    };
    await Promise.all(Array.from({ length: 4 }, worker));
    bulkBusy = false;
    selected = new Set();
    const past = kind === 'remove' ? 'Deleted' : 'Terminated';
    if (failed === 0) toasts.success(`${past} ${plural(done, 'session')}`);
    else toasts.error(`Couldn’t ${label.toLowerCase()} ${plural(failed, 'session')}`, `${done} succeeded, ${failed} failed. Refresh and try again.`);
    await load();
  }

  const bulkTerminate = (): Promise<void> => runBulk([...selected], 'terminate', 'Terminate');
  const bulkRemove = (): Promise<void> => runBulk([...selected], 'remove', 'Delete');
  const removeExited = (): Promise<void> =>
    runBulk(
      sessions.filter((s) => !s.live).map((s) => s.id),
      'remove',
      'Delete',
    );

  // One word + one tone per state (matches StatusDot's vocabulary): working
  // is the only green; idle is neutral; an exited PTY reads "Ended".
  function statusInfo(s: AdminSessionRow): { tone: Tone; label: string } {
    const st = s.status.toLowerCase();
    if (s.live && st === 'working') return { tone: 'success', label: 'Working' };
    if (s.live && st === 'running') return { tone: 'info', label: 'Running' };
    if (s.live && st === 'idle') return { tone: 'neutral', label: 'Idle' };
    if (st === 'reconnectable') return { tone: 'neutral', label: 'Suspended' };
    if (st === 'exited' || !s.live) return { tone: 'neutral', label: 'Ended' };
    return { tone: 'neutral', label: sentenceCase(st) };
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "title":sectionLabel('sessions'),"subtitle":`All sessions across all users`,actions:() => { async ()/*Ωignore_positionΩ*/ => {
      if(exitedCount > 0){
         { svelteHTML.createElement("button", {         "class":`btn small`,...__sveltets_2_empty({"data-overflow":`-1`}),...__sveltets_2_empty({"data-icon":`trash`}),"onclick":removeExited,"disabled":bulkBusy || loading,});
             exitedCount;
         }
      }
       { svelteHTML.createElement("button", {       "class":`btn small`,...__sveltets_2_empty({"data-icon":`refresh`}),"onclick":load,"disabled":loading || bulkBusy,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});} loading && sessions.length ? 'Refreshing…' : 'Refresh';
       }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
   PageHeader}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
   { const $$_ortnInoitceS2C = __sveltets_2_ensureComponent(SectionIntro); new $$_ortnInoitceS2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
     { svelteHTML.createElement("strong", {});  }             { svelteHTML.createElement("strong", {});  }
           
   SectionIntro}

  if(sessions.length > 0 && exitedCount > 0 && liveCount > 0){
     { svelteHTML.createElement("div", {     "class":`segmented show`,"role":`group`,"aria-label":`Show sessions`,});
       { svelteHTML.createElement("button", {     "aria-pressed":show === 'all',"onclick":() => setShow('all'),});show === 'all';  { svelteHTML.createElement("span", { "class":`n`,});sessions.length; } }
       { svelteHTML.createElement("button", {     "aria-pressed":show === 'live',"onclick":() => setShow('live'),});show === 'live';  { svelteHTML.createElement("span", { "class":`n`,});liveCount; } }
       { svelteHTML.createElement("button", {     "aria-pressed":show === 'ended',"onclick":() => setShow('ended'),});show === 'ended';  { svelteHTML.createElement("span", { "class":`n`,});exitedCount; } }
     }
  }

   { svelteHTML.createElement("label", { "class":`archived-toggle`,});
     { svelteHTML.createElement("input", {        "type":`checkbox`,"checked":includeArchived,"onchange":(e) => setIncludeArchived(e.currentTarget.checked),"disabled":loading,});}
      
   }

  if(selected.size > 0){
     { svelteHTML.createElement("div", {     "class":`bulk-bar`,"role":`toolbar`,"aria-label":`Selected sessions`,});
       { svelteHTML.createElement("span", { "class":`bulk-count`,});selected.size;  }
       { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":() => (selected = new Set()),"disabled":bulkBusy,});  }
       { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":bulkTerminate,"disabled":bulkBusy,});  }
       { svelteHTML.createElement("button", {     "class":`btn small danger`,"onclick":bulkRemove,"disabled":bulkBusy,});  }
     }
  }

   { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          children:() => { return __sveltets_2_any(0); },"what":`sessions`,loading,"error":loadError,"empty":sessions.length === 0,"rows":4,"onretry":() => void load(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {        "variant":`page`,"icon":`terminal`,"title":`No sessions`,"body":`No user has an agent or connection session right now. New sessions appear here for every user.`,}});}
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
    
     { svelteHTML.createElement("div", { "class":`card session-table`,});
       { svelteHTML.createElement("div", { "class":`table-inner`,});
         { svelteHTML.createElement("div", { "class":`session-head`,});
           { svelteHTML.createElement("span", { "class":`col-sel`,});
             { svelteHTML.createElement("input", {        "type":`checkbox`,"checked":allSelected,"onchange":toggleAll,"aria-label":`Select all sessions`,});}
           }
           { svelteHTML.createElement("span", { "class":`col-owner`,});  }
           { svelteHTML.createElement("span", { "class":`col-kind`,});   }
           { svelteHTML.createElement("span", { "class":`col-title`,});  }
           { svelteHTML.createElement("span", { "class":`col-status`,});  }
           { svelteHTML.createElement("span", { "class":`col-viewers`,});  }
           { svelteHTML.createElement("span", { "class":`col-action`,}); { svelteHTML.createElement("span", { "class":`sr-only`,});  } }
         }
           for(let s of __sveltets_2_ensureArray(visible)){s.id;
          const info = statusInfo(s);
           { svelteHTML.createElement("div", {  "class":`session-row`,});selected.has(s.id);
             { svelteHTML.createElement("span", { "class":`col-sel`,});
               { svelteHTML.createElement("input", {         "type":`checkbox`,"checked":selected.has(s.id),"onchange":() => toggle(s.id),"aria-label":`Select ${s.title || 'untitled session'}`,});}
             }
             { svelteHTML.createElement("span", {   "class":`col-owner ellip`,"title":s.owner_username,});
               { svelteHTML.createElement("span", { "class":`owner-name`,});s.owner_username; }
             }
             { svelteHTML.createElement("span", {   "class":`col-kind ellip`,"title":`${s.kind} · ${s.provider}`,});
               { svelteHTML.createElement("span", { "class":`chip kind-chip`,});sentenceCase(s.kind); }
               { svelteHTML.createElement("span", { "class":`dim provider-name`,});s.provider; }
             }
             { svelteHTML.createElement("span", {   "class":`col-title ellip`,"title":s.title,});s.title || '—'; }
             { svelteHTML.createElement("span", { "class":`col-status`,});
               { const $$_egdaBsutatS7C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS7C({ target: __sveltets_2_any(), props: {        "variant":`text`,"tone":info.tone,"label":info.label,"title":`Stored status: ${sentenceCase(s.status)}`,}});}
             }
             { svelteHTML.createElement("span", { "class":`col-viewers dim`,});s.viewers > 0 ? s.viewers : '—'; }
             { svelteHTML.createElement("span", { "class":`col-action`,});
              if(s.live){
                 { svelteHTML.createElement("button", {       "class":`btn small ghost`,"disabled":busy.has(s.id) || bulkBusy,"onclick":() => terminate(s.id, s.title),});
                  busy.has(s.id) ? 'Working…' : 'Terminate…';
                 }
              }
               { svelteHTML.createElement("button", {           "class":`icon-btn danger-icon`,"disabled":busy.has(s.id) || bulkBusy,"aria-label":`Delete ${s.title || 'untitled session'}`,"title":`Delete ${s.title || 'untitled session'}`,"onclick":() => remove(s.id, s.title),});
                 { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});}
               }
             }
           }
        }
       }
     }
    if(nextCursor){
       { svelteHTML.createElement("div", { "class":`more`,});
         { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => void loadMore(),"disabled":loadingMore,"aria-busy":loadingMore,});
          loadingMore ? 'Loading more sessions…' : 'Load more';
         }
       }
    }
   LoadState}
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AdminSessions__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AdminSessions__SvelteComponent_ = ReturnType<typeof AdminSessions__SvelteComponent_>;
/*Ωignore_endΩ*/export default AdminSessions__SvelteComponent_;