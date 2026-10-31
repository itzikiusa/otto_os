///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { plural } from '../../lib/plural';
import { toastError } from '../../lib/toastError';
import { LIST_PANE, loadPaneWidth } from '../../lib/paneResizer';
import PaneDivider from '../../lib/components/PaneDivider.svelte';
import { untrack } from 'svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { api, ApiError } from '../../lib/api/client';
import { toasts } from '../../lib/toast.svelte';
import { confirmProd } from '../../lib/confirmProd';
import LoadState from '../../lib/components/LoadState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { loadErrorText } from '../../lib/loadError';
import { TableWindow } from '../../lib/tableWindow.svelte';
import type { BrokerCluster, GroupDetail, GroupOffset, GroupSummary } from '../../lib/api/types';
import type { DryRunResp } from './types';
import ResetWarning, { resetWarning } from './ResetWarning.svelte';
;

  interface Props {
    cluster: BrokerCluster;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { cluster }: Props = $props();

  const guarded = $derived(cluster.read_only || cluster.environment === 'prod');

  // Raw: a cluster can hold thousands of groups; the list is replaced
  // wholesale per load, never mutated in place.
  let groups = $state.raw<GroupSummary[]>([]);
  let loading = $state(true);
  /** Failed loads — inline with Retry, never "No consumer groups." / a blank detail. */
  let loadError = $state<string | null>(null);
  let detailError = $state<string | null>(null);
  // True when the broker's ACLs deny consumer-group access (probed once, cached
  // server-side). We show a clear banner instead of erroring, and skip re-probes.
  let accessDenied = $state(false);
  let accessMsg = $state('');
  let selected = $state<string | null>(null);
  let detail = $state.raw<GroupDetail | null>(null);
  let detailLoading = $state(false);
  let detailRequest = 0;
  let listRequest = 0;
  // Sort offsets table by lag descending; toggle to sort by topic+partition.
  let sortByLag = $state(true);
  // Offset reset state.
  let resetting = $state(false);
  let resetMode = $state<'earliest' | 'latest' | 'offset' | 'timestamp'>('latest');
  let resetOffset = $state(0);
  let resetTs = $state('');
  let resetTopic = $state('');
  // Dry-run preview state.
  let dryRunLoading = $state(false);
  let dryRunResult = $state<DryRunResp | null>(null);
  const blockedBy = $derived(resetWarning(dryRunResult, selected)); // the preview's refusal warning

  // Resizable group list (PaneDivider): the width survives reloads. On phones /
  // a narrow container the list stacks full-width and the divider is hidden.
  let listW = $state(loadPaneWidth('brokers.groupsListW', LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));

  $effect(() => {
    void cluster.id;
    detailRequest++;
    selected = null;
    detail = null;
    detailLoading = false;
    dryRunResult = null;
    detailError = null;
    // Another cluster's groups are not "stale data" for this one.
    groups = [];
    untrack(() => groupsTw.reset(groupsEl));
    loadError = null;
    loadGroups();
  });

  function loadGroups(): void {
    const request = ++listRequest;
    const clusterId = cluster.id;
    const current = () => request === listRequest && cluster.id === clusterId;
    loading = true;
    accessDenied = false;
    api
      .get<GroupSummary[]>(`/brokers/clusters/${cluster.id}/groups`)
      .then((g) => {
        if (!current()) return;
        groups = g;
        accessDenied = false;
        loadError = null;
        // List/detail opens on an item, never on an empty "pick one" pane.
        if (!selected && g.length > 0) open(g[0].group_id);
      })
      .catch((e) => {
        if (!current()) return;
        if (e instanceof ApiError && e.status === 403 && /consumer-group access/i.test(e.message)) {
          // Broker ACLs deny group access — show the banner, don't toast/retry.
          accessDenied = true;
          accessMsg = e.message;
          groups = [];
        } else {
          loadError = loadErrorText(e);
        }
      })
      .finally(() => { if (current()) loading = false; });
  }

  function open(id: string) {
    const request = ++detailRequest;
    const clusterId = cluster.id;
    const current = () => request === detailRequest && cluster.id === clusterId && selected === id;
    dryRunResult = null;
    dryRunLoading = false;
    // Refreshing the open group keeps its rows on screen; switching blanks them.
    if (selected !== id) detail = null;
    selected = id;
    detailError = null;
    detailLoading = true;
    resetTopic = '';
    api
      .get<GroupDetail>(`/brokers/clusters/${cluster.id}/groups/${encodeURIComponent(id)}`)
      .then((d) => { if (current()) detail = d; })
      .catch((e) => { if (current()) detailError = loadErrorText(e); })
      .finally(() => { if (current()) detailLoading = false; });
  }

  function stateClass(s: string): string {
    const v = s.toLowerCase();
    if (v.includes('stable')) return 'ok';
    if (v.includes('empty') || v.includes('dead')) return 'dim';
    return 'warn';
  }

  // Per-topic subtotals: sum lag across partitions.
  const topicSubtotals = $derived.by(() => {
    if (!detail) return new Map<string, number>();
    const m = new Map<string, number>();
    for (const o of detail.offsets) {
      m.set(o.topic, (m.get(o.topic) ?? 0) + o.lag);
    }
    return m;
  });

  // Sorted offsets view.
  const sortedOffsets = $derived.by(() => {
    if (!detail) return [] as GroupOffset[];
    const offsets = [...detail.offsets];
    if (sortByLag) {
      offsets.sort((a, b) => b.lag - a.lag);
    }
    // else keep the default (topic+partition order from the server)
    return offsets;
  });

  // Max lag across all partitions — used to scale the per-row bars. (A loop,
  // not `Math.max(...spread)`: a 12k-partition group would blow the arg limit.)
  const maxLag = $derived(sortedOffsets.reduce((m, o) => (o.lag > m ? o.lag : m), 1));

  // Window the group list and the offsets table (SC-22): 1k groups / 12k
  // offset rows mount only the visible slice between two spacers.
  const groupsTw = new TableWindow();
  const groupsWin = $derived(groupsTw.range(groups.length));
  let groupsEl = $state<HTMLDivElement>();
  $effect(() => {
    void groupsWin;
    // The rows mount one flush AFTER `groups` lands (`loading` clears in the
    // load's `.finally`); without this the row height stays the 30 px guess,
    // the spacers are ~40 % short and scrolling to the end lands mid-list.
    void loading;
    groupsTw.measure(groupsEl, '.grow-row');
  });
  const offsetsTw = new TableWindow();
  const offsetsWin = $derived(offsetsTw.range(sortedOffsets.length));
  let offsetsEl = $state<HTMLDivElement>();
  $effect(() => {
    void offsetsWin;
    offsetsTw.measure(offsetsEl);
  });
  // ⌘F over every group / offset row, not just the mounted slice.
  $effect(() =>
    groupsTw.findRows(
      () => groupsEl,
      () => groups,
      (g) => `${g.group_id}\n${g.state}\n${plural(g.members, 'member')}`,
      '.grow-row',
    ),
  );
  $effect(() =>
    offsetsTw.findRows(
      () => offsetsEl,
      () => sortedOffsets,
      (o) =>
        [o.topic, o.partition, o.current_offset.toLocaleString(), o.high_watermark.toLocaleString(), o.lag.toLocaleString()].join('\n'),
    ),
  );
  // A new group (or a re-sort) starts the offsets table at the top.
  $effect(() => {
    void detail;
    void sortByLag;
    offsetsTw.reset(offsetsEl);
  });

  // Unique topics in the current group (for the topic filter dropdown).
  const groupTopics = $derived(
    [...new Set(detail?.offsets.map((o) => o.topic) ?? [])].sort(),
  );

  const resetError = $derived(
    resetMode === 'timestamp' && (!resetTs || !Number.isFinite(new Date(resetTs).getTime()))
      ? 'Choose a valid target time'
      : resetMode === 'offset' && (!Number.isSafeInteger(resetOffset) || resetOffset < 0)
        ? 'Offset must be a non-negative whole number' : null,
  );

  function buildResetBody(confirm: boolean): Record<string, unknown> {
    let body: Record<string, unknown>;
    if (resetMode === 'offset') {
      body = { mode: 'offset', offset: Number(resetOffset), confirm };
    } else if (resetMode === 'timestamp') {
      body = {
        mode: 'timestamp',
        timestamp_ms: new Date(resetTs).getTime(),
        confirm,
      };
    } else {
      body = { mode: resetMode, confirm };
    }
    if (resetTopic) body['topic'] = resetTopic;
    return body;
  }

  async function previewReset() {
    if (!selected || resetError) return;
    const groupId = selected;
    const clusterId = cluster.id;
    const request = detailRequest;
    const body = buildResetBody(false);
    const current = () => request === detailRequest && groupId === selected && clusterId === cluster.id && JSON.stringify(body) === JSON.stringify(buildResetBody(false));
    dryRunLoading = true;
    dryRunResult = null;
    try {
      const preview = await api.post<DryRunResp>(
        `/brokers/clusters/${clusterId}/groups/${encodeURIComponent(groupId)}/reset?dry_run=true`,
        body,
      );
      if (current()) dryRunResult = preview;
    } catch (e) {
      if (current()) toastError("Couldn’t preview the reset", e);
    } finally {
      if (request === detailRequest) dryRunLoading = false;
    }
  }

  async function applyReset() {
    if (!selected || resetError) return;
    // The preview STAYS visible while the user types the confirm — its
    // partition count and lag delta are what the confirm summarises.
    await resetOffsets();
  }

  /** "to earliest" / "to offset 42" / "to 2026-10-05 12:00" — the reset target. */
  function resetTargetText(): string {
    if (resetMode === 'offset') return `to offset ${Number(resetOffset)}`;
    if (resetMode === 'timestamp') return `to ${new Date(resetTs).toLocaleString()}`;
    return `to ${resetMode}`;
  }

  async function resetOffsets() {
    if (!selected || resetError) return;
    const groupId = selected;
    const clusterId = cluster.id;
    const request = detailRequest;
    const body = buildResetBody(guarded);
    const current = () => cluster.id === clusterId && selected === groupId && detailRequest === request;
    // Where (cluster · env · group · topic scope) and what (mode/target + the
    // dry-run's partition count and lag change, when previewed), then the
    // typed group name — consumers resume from the new offsets.
    const preview = dryRunResult && dryRunResult.group === groupId ? dryRunResult : null;
    const ok = await confirmProd({
      env: cluster.environment,
      verb: 'Reset offsets',
      title: `Reset offsets for "${groupId}"`,
      where: `Consumer group ${groupId} · ${resetTopic ? `topic ${resetTopic}` : 'all topics'} · cluster ${cluster.name}`,
      what: [
        `Reset ${resetTargetText()}`,
        preview
          ? `${plural(preview.partitions.length, 'partition')} · lag ${preview.total_lag_before.toLocaleString()} → ${preview.total_lag_after.toLocaleString()}`
          : 'Not previewed — use Preview to see the per-partition change first.',
      ].join(' · '),
      typed: groupId,
      danger: true,
    });
    if (!ok || !current()) return;

    resetting = true;
    try {
      const updated = await api.post<GroupDetail>(
        `/brokers/clusters/${clusterId}/groups/${encodeURIComponent(groupId)}/reset`,
        body,
      );
      if (current()) detail = updated;
      toasts.success(`Offsets reset for "${groupId}"`);
    } catch (e) {
      toastError("Couldn’t reset offsets", e);
    } finally {
      resetting = false;
    }
  }
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  $effect(() => { if (resetError) announce(resetError, { assertive: true }); });
;
async () => {

 { svelteHTML.createElement("div", { "class":`groups-container`,});
 { svelteHTML.createElement("div", { "class":`groups`,});
   { const $$_div2 = svelteHTML.createElement("div", {          "class":`list`,"style":`--groups-list-w:${listW}px`,"onscroll":groupsTw.onscroll,});groupsTw.active(groups.length);groupsEl = $$_div2;groupsTw.viewH= $$_div2.clientHeight;
    if(accessDenied){
       { svelteHTML.createElement("div", { "class":`acl-denied pad`,});
         { svelteHTML.createElement("p", { "class":`acl-title`,});    }
         { svelteHTML.createElement("p", { "class":`muted`,});accessMsg; }
         { svelteHTML.createElement("p", { "class":`muted`,});
                { svelteHTML.createElement("code", {});  }    
                        
                      
         }
       }
    }else{
      
       { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL3 = new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {          children:() => { return __sveltets_2_any(0); },"what":`consumer groups`,"variant":`compact`,loading,"error":loadError,"empty":groups.length === 0,"onretry":loadGroups,emptyView:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("p", { "class":`muted pad`,});      }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL3.$$prop_def;/*Ωignore_endΩ*/
        
      if(groupsWin.top){ { svelteHTML.createElement("div", {     "class":`tw-spacer`,"aria-hidden":`true`,"style":`height:${groupsWin.top}px`,}); }}
         for(let g of __sveltets_2_ensureArray((groups.slice(groupsWin.start, groupsWin.end)))){g.group_id;
         { svelteHTML.createElement("button", {    "class":`grow-row`,"onclick":() => open(g.group_id),});selected === g.group_id;
           { svelteHTML.createElement("span", {   "class":`gid`,"title":g.group_id,});g.group_id; }
           { svelteHTML.createElement("span", { "class":`badges`,});
             { svelteHTML.createElement("span", { "class":`state ${stateClass(g.state)}`,});g.state; }
             { svelteHTML.createElement("span", { "class":`muted`,});plural(g.members, 'member'); }
           }
         }
      }
      if(groupsWin.bottom){ { svelteHTML.createElement("div", {     "class":`tw-spacer`,"aria-hidden":`true`,"style":`height:${groupsWin.bottom}px`,}); }}
       LoadState}
    }
   }

  
   { svelteHTML.createElement("div", { "class":`divider-slot`,}); { const $$_rediviDenaP3C = __sveltets_2_ensureComponent(PaneDivider); const $$_rediviDenaP3 = new $$_rediviDenaP3C({ target: __sveltets_2_any(), props: {       width:listW,"storageKey":`brokers.groupsListW`,"label":`Resize the group list`,}});/*Ωignore_startΩ*/() => listW = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rediviDenaP3.$$bindings = 'width';} }

   { svelteHTML.createElement("div", { "class":`detail`,});
    if((detailLoading || detailError) && !detail && selected){
      const gid = selected;
       { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {        "what":`this group`,"loading":detailLoading,"error":detailError,"empty":true,"onretry":() => open(gid),}});}
    } else if (detail){
      if(detailError && selected){
        const gid = selected;
         { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {        "what":`this group`,"error":detailError,"empty":false,"onretry":() => open(gid),}});}
      }
       { svelteHTML.createElement("header", {});
         { svelteHTML.createElement("span", { "class":`gid big`,});detail.group_id; }
         { svelteHTML.createElement("span", { "class":`state ${stateClass(detail.state)}`,});detail.state; }
         { svelteHTML.createElement("span", {  "class":`lag-total`,});detail.total_lag > 0;
            detail.total_lag.toLocaleString();
         }
        if(selected){
          const gid = selected;
           { svelteHTML.createElement("button", {           "class":`icon-btn`,"onclick":() => open(gid),"disabled":detailLoading,"aria-label":`Refresh lag`,"title":`Refresh lag`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});}
           }
        }
       }

      if(detail.members.length > 0){
         { svelteHTML.createElement("h5", {});  }
         { svelteHTML.createElement("table", {});
           { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } } }
           { svelteHTML.createElement("tbody", {});
               for(let m of __sveltets_2_ensureArray(detail.members)){m.member_id;
               { svelteHTML.createElement("tr", {});
                 { svelteHTML.createElement("td", { "class":`mono small`,});m.member_id; }
                 { svelteHTML.createElement("td", {});m.client_id; }
                 { svelteHTML.createElement("td", { "class":`muted`,});m.host; }
                 { svelteHTML.createElement("td", { "class":`muted small`,});m.assignments.length;  }
               }
            }
           }
         }
      }

       { svelteHTML.createElement("div", { "class":`offsets-header`,});
         { svelteHTML.createElement("h5", {});    }
         { svelteHTML.createElement("label", { "class":`sort-toggle`,});
           { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":sortByLag,});/*Ωignore_startΩ*/() => sortByLag = __sveltets_2_any(null);/*Ωignore_endΩ*/}
            
         }
       }
       { const $$_div3 = svelteHTML.createElement("div", {        "class":`offsets-wrap`,"onscroll":offsetsTw.onscroll,});offsetsTw.active(sortedOffsets.length);offsetsEl = $$_div3;offsetsTw.viewH= $$_div3.clientHeight;
       { svelteHTML.createElement("table", {});
         { svelteHTML.createElement("thead", {});
           { svelteHTML.createElement("tr", {});
             { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  }
             { svelteHTML.createElement("th", { "class":`lag-col`,});  } { svelteHTML.createElement("th", { "class":`bar-col`,}); }
           }
         }
         { svelteHTML.createElement("tbody", {});
          if(offsetsWin.top){ { svelteHTML.createElement("tr", {   "class":`tw-spacer`,"aria-hidden":`true`,}); { svelteHTML.createElement("td", {   "colspan":6,"style":`height:${offsetsWin.top}px`,}); } }}
             for(let o of __sveltets_2_ensureArray((sortedOffsets.slice(offsetsWin.start, offsetsWin.end)))){o.topic + '-' + o.partition;
             { svelteHTML.createElement("tr", {});
               { svelteHTML.createElement("td", { "class":`mono`,});o.topic; }
               { svelteHTML.createElement("td", {});o.partition; }
               { svelteHTML.createElement("td", { "class":`muted`,});o.current_offset.toLocaleString(); }
               { svelteHTML.createElement("td", { "class":`muted`,});o.high_watermark.toLocaleString(); }
               { svelteHTML.createElement("td", { });o.lag > 0;o.lag.toLocaleString(); }
               { svelteHTML.createElement("td", { "class":`bar-col`,});
                if(o.lag > 0){
                   { svelteHTML.createElement("div", { "class":`lag-bar`,});
                     { svelteHTML.createElement("div", {   "class":`lag-fill`,"style":`width: ${Math.round((o.lag / maxLag) * 100)}%`,}); }
                   }
                }
               }
             }
          }
          if(offsetsWin.bottom){ { svelteHTML.createElement("tr", {   "class":`tw-spacer`,"aria-hidden":`true`,}); { svelteHTML.createElement("td", {   "colspan":6,"style":`height:${offsetsWin.bottom}px`,}); } }}
         }
       }
       }
      if(topicSubtotals.size > 1){
         { svelteHTML.createElement("h5", {});  }
         { svelteHTML.createElement("table", {});
           { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } } }
           { svelteHTML.createElement("tbody", {});
               for(let [t, lag] of __sveltets_2_ensureArray(([...topicSubtotals.entries()].sort((a, b) => b[1] - a[1])))){t;
               { svelteHTML.createElement("tr", {});
                 { svelteHTML.createElement("td", { "class":`mono`,});t; }
                 { svelteHTML.createElement("td", { });lag > 0;lag.toLocaleString(); }
               }
            }
           }
         }
      }
      if(detail.offsets.length === 0){
         { svelteHTML.createElement("p", { "class":`muted pad`,});   }
      }

      
       { svelteHTML.createElement("h5", {});  }
       { svelteHTML.createElement("div", { "class":`reset-bar`,});
         { svelteHTML.createElement("select", {       "bind:value":resetMode,"aria-label":`Reset to`,"title":`Reset to`,"onchange":() => (dryRunResult = null),});/*Ωignore_startΩ*/() => resetMode = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", { "value":`earliest`,});  }
           { svelteHTML.createElement("option", { "value":`latest`,});  }
           { svelteHTML.createElement("option", { "value":`offset`,});  }
           { svelteHTML.createElement("option", { "value":`timestamp`,});  }
         }
        if(resetMode === 'offset'){
           { svelteHTML.createElement("input", {            "type":`number`,"class":`sm-input`,"bind:value":resetOffset,"placeholder":`offset`,"aria-label":`Target offset`,"oninput":() => (dryRunResult = null),});/*Ωignore_startΩ*/() => resetOffset = __sveltets_2_any(null);/*Ωignore_endΩ*/}
        }
        if(resetMode === 'timestamp'){
           { svelteHTML.createElement("input", {          "type":`datetime-local`,"class":`sm-input wide`,"bind:value":resetTs,"aria-label":`Target time`,"oninput":() => (dryRunResult = null),});/*Ωignore_startΩ*/() => resetTs = __sveltets_2_any(null);/*Ωignore_endΩ*/}
        }
        if(groupTopics.length > 1){
           { svelteHTML.createElement("select", {       "bind:value":resetTopic,"title":`Scope to one topic (blank = all)`,"aria-label":`Topic scope`,"onchange":() => (dryRunResult = null),});/*Ωignore_startΩ*/() => resetTopic = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", {"value":"",});  }
               for(let t of __sveltets_2_ensureArray(groupTopics)){t; { svelteHTML.createElement("option", { "value":t,});t; }}
           }
        }
         { svelteHTML.createElement("button", {         "class":`btn small`,"onclick":previewReset,"disabled":dryRunLoading || resetting || !!resetError,"title":`Preview what this reset would do without committing`,});
          dryRunLoading ? 'Previewing…' : 'Preview';
         }
         { svelteHTML.createElement("button", {        "class":`btn small danger`,"onclick":applyReset,"disabled":resetting || dryRunLoading || !!resetError || !!blockedBy,"title":blockedBy ? `Can’t reset yet: ${blockedBy}` : guarded ? 'Cluster is guarded — requires confirmation' : 'Reset committed offsets',});
          resetting ? 'Resetting…' : 'Reset';
         }
       }

      if(resetError){ { svelteHTML.createElement("p", { "class":`muted small`,});resetError; }}

      if(dryRunResult){
         { const $$_gninraWteseR3C = __sveltets_2_ensureComponent(ResetWarning); new $$_gninraWteseR3C({ target: __sveltets_2_any(), props: {    "preview":dryRunResult,"group":selected,}});}
         { svelteHTML.createElement("div", { "class":`dryrun-preview`,});
           { svelteHTML.createElement("div", { "class":`dryrun-summary`,});
             { svelteHTML.createElement("span", {});   { svelteHTML.createElement("strong", {});dryRunResult.total_lag_before.toLocaleString(); }
              { svelteHTML.createElement("strong", {  });dryRunResult.total_lag_after < dryRunResult.total_lag_before;dryRunResult.total_lag_after > dryRunResult.total_lag_before;
              dryRunResult.total_lag_after.toLocaleString(); }
            plural(dryRunResult.partitions.length, 'partition'); 
             }
             { svelteHTML.createElement("button", {         "class":`icon-btn close-dry`,"onclick":() => (dryRunResult = null),"aria-label":`Close preview`,"title":`Close preview`,});
               { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
             }
           }
           { svelteHTML.createElement("table", { "class":`dryrun-table`,});
             { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } } }
             { svelteHTML.createElement("tbody", {});
                 for(let p of __sveltets_2_ensureArray(dryRunResult.partitions)){p.topic + '-' + p.partition;
                 { svelteHTML.createElement("tr", {});
                   { svelteHTML.createElement("td", { "class":`mono`,});p.topic; }
                   { svelteHTML.createElement("td", {});p.partition; }
                   { svelteHTML.createElement("td", { "class":`muted`,});p.current_offset.toLocaleString(); }
                   { svelteHTML.createElement("td", { "class":`muted`,});p.target_offset.toLocaleString(); }
                   { svelteHTML.createElement("td", {  });p.lag_delta > 0;p.lag_delta < 0;
                    p.lag_delta >= 0 ? '-' : '+';Math.abs(p.lag_delta).toLocaleString();
                   }
                 }
              }
             }
           }
          if(!blockedBy){ { svelteHTML.createElement("p", { "class":`muted small`,});  { svelteHTML.createElement("strong", {});  }      }}
         }
      }
    }else{
       { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {      "icon":`people`,"title":`Pick a consumer group`,"body":`${groups.length} ${groups.length === 1 ? 'group' : 'groups'} in this cluster. Select one to see its members and lag.`,}});}
    }
   }
 }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const GroupsTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type GroupsTab__SvelteComponent_ = ReturnType<typeof GroupsTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default GroupsTab__SvelteComponent_;