///<reference types="svelte" />
;
import { rowMenu } from '../../lib/rowMenu';
// A dashboard tile: runs /db/widgets/{id}/run on mount (and on a refresh
// interval set per-dashboard) and renders the result via Chart per its viz.
import Icon from '../../lib/components/Icon.svelte';
import Chart from './Chart.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { database } from '../../lib/stores/database.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { api, isAbortError } from '../../lib/api/client';
import { pollWhileVisible } from '../../lib/poll';
import { widgetGate } from './widgetGate';
import type { DbWidget, QueryResult } from '../../lib/api/types';
;

  interface Props {
    widget: DbWidget;
    /** Refresh interval in seconds (0/undefined = manual only). */
    refreshSecs?: number | null;
    /** Open the edit dialog for this widget (owned by the dashboard view). */
    onedit?: (w: DbWidget) => void;
    /** False while the card is off screen (an inactive Home space, review 06
     *  F3): no runs and no auto-refresh; the last result stays. A manual-only
     *  card (refreshSecs 0) runs ONCE per widget — never again on its own,
     *  however often it is shown again. */
    active?: boolean;
  };;
  /** `'aborted'` is neither a success nor a failure: it must not stamp
   *  freshness (the card would then never re-run on return — ui-correctness
   *  2-01) and must not leave the spinner up. */
  type RunOutcome = 'ok' | 'failed' | 'aborted';;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { widget, refreshSecs = null, onedit, active = true }: Props = $props();
  /** The widget id + query whose result is on screen (manual-only cards run
   *  once per that key), and when it last succeeded (auto-refresh resumes
   *  without an immediate re-run while that result is within its cadence). */
  let ranKey = '';
  let okAt = 0;

  let result = $state<QueryResult | null>(null);
  let loading = $state(false);
  // Failure message rendered IN the card — a banner over the last good chart
  // when one exists (the chart is never wiped by a failed refresh).
  let error = $state<string | null>(null);
  /** Bumped by every run: an aborted run clears `loading` only while it is
   *  still the newest one (a refresh click may already own the spinner). */
  let runToken = 0;

  async function run(manual = false, signal?: AbortSignal): Promise<RunOutcome> {
    const token = ++runToken;
    loading = true;
    try {
      // The store's runWidget toasts on failure — right for an explicit click,
      // wrong for a background tick (20 tiles on a broken connection would storm
      // the toaster every refresh), so auto-refresh posts directly and surfaces
      // the failure inline instead. Auto runs share one dashboard-wide gate
      // (widgetGate: ≤ 2 queries at once across every tile) and ride the
      // background lane; unmount aborts the in-flight one.
      const r = manual
        ? await database.runWidget(widget.id)
        : await widgetGate(() => api.bg.post<QueryResult>(`/db/widgets/${widget.id}/run`, {}, signal), signal);
      if (r) {
        result = r; // only a SUCCESS replaces the data
        error = null;
        // The edit sheet maps axes from THIS widget's columns.
        database.dash.noteWidgetColumns(widget.id, r.columns.map((c) => c.name));
        return 'ok';
      }
      // runWidget already toasted the cause; the card keeps a short inline one.
      error = 'The query failed.';
      return 'failed';
    } catch (e) {
      if (isAbortError(e) || signal?.aborted) return 'aborted';
      error = loadErrorText(e);
      return 'failed';
    } finally {
      if (token === runToken) loading = false;
    }
  }

  // Initial run + auto-refresh, recreated whenever the widget id or cadence
  // changes and stopped on unmount. `pollWhileVisible` supplies the rules the
  // old hand-rolled setTimeout chain lacked (r3-04-03): no ticks while the
  // window is hidden (one catch-up run on return), the in-flight query is
  // ABORTED on unmount, ±15 % jitter so 20 tiles don't fire together, and a
  // ×2-per-failure backoff (capped ×16) so a dead connection isn't hammered.
  $effect(() => {
    const key = `${widget.id}\u0000${widget.connection_id}\u0000${widget.statement}`; // a swapped/edited widget re-runs
    const secs = refreshSecs ?? 0;
    if (!active) return;
    const fresh = key === ranKey && okAt > 0;
    if (secs <= 0) {
      if (fresh) return; // manual only: shown again ≠ asked again
      const ctl = new AbortController();
      void run(false, ctl.signal).then((outcome) => {
        if (outcome === 'ok') {
          ranKey = key;
          okAt = Date.now();
        }
      });
      return () => ctl.abort();
    }
    const poller = pollWhileVisible(
      async (signal) => {
        const outcome = await run(false, signal);
        if (outcome === 'ok') {
          ranKey = key;
          okAt = Date.now();
        }
        // An abort (the card went off screen) is no failure to back off from,
        // and no success to stamp: only 'failed' counts against the cadence.
        return outcome !== 'failed';
      },
      {
        ms: secs * 1000,
        floorMs: 5000,
        jitter: 0.15,
        maxBackoff: 16,
        lane: 'bg',
        immediate: !(fresh && Date.now() - okAt < secs * 1000),
      },
    );
    return () => poller.stop();
  });

  const canEdit = $derived(ws.myRole !== 'viewer');

  // The connection this widget's query runs on — widgets bind to whichever
  // connection was focused at creation, so surface it on the card.
  const connName = $derived(
    database.connections.find((c) => c.id === widget.connection_id)?.name ?? widget.connection_id,
  );

  async function confirmDelete(): Promise<void> {
    if (await confirmer.ask(`Delete widget “${widget.title}”?`, { title: 'Delete widget', confirmLabel: 'Delete', danger: true })) {
      await database.deleteWidget(widget.id);
    }
  }

  function menu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Refresh', icon: 'refresh', action: () => void run(true) },
      ...(canEdit && onedit ? [{ label: 'Edit…', icon: 'edit', action: () => onedit?.(widget) }] : []),
      ...(canEdit
        ? [
            { separator: true },
            { label: 'Delete widget…', icon: 'trash', danger: true as const, action: () => void confirmDelete() },
          ]
        : []),
    ]);
  }
;
async () => {

 {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {        "class":`widget-card`,"role":`group`,"aria-label":widget.title,"oncontextmenu":menu,});
   { svelteHTML.createElement("div", { "class":`wc-head`,});
     { svelteHTML.createElement("span", {   "class":`wc-title ellipsis`,"title":widget.title,});widget.title; }
     { svelteHTML.createElement("span", {   "class":`wc-conn ellipsis`,"title":`Connection: ${connName}`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`db`,"size":12,}});}connName; }
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => void run(true),"title":`Refresh`,"aria-label":`Refresh widget`,});
      if(loading){ { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 12px`,"aria-hidden":`true`,}); }}else{ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}}
     }
    if(canEdit){
      if(onedit){
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => onedit?.(widget),"title":`Edit`,"aria-label":`Edit widget`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});}
         }
      }
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => void confirmDelete(),"title":`Delete`,"aria-label":`Delete widget`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});}
       }
    }
   }
   { svelteHTML.createElement("div", { "class":`wc-body`,});
    
     { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          children:() => { return __sveltets_2_any(0); },"what":widget.title,"variant":`compact`,"loading":loading && !result,error,"empty":!result,"onretry":() => void run(true),}});
       { const $$_trahC3C = __sveltets_2_ensureComponent(Chart); new $$_trahC3C({ target: __sveltets_2_any(), props: {     result,"viz":widget.viz,"mapping":widget.mapping,}});}
     LoadState}
   }
 }}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const WidgetCard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type WidgetCard__SvelteComponent_ = ReturnType<typeof WidgetCard__SvelteComponent_>;
/*Ωignore_endΩ*/export default WidgetCard__SvelteComponent_;