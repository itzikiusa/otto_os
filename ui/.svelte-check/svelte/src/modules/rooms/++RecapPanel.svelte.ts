///<reference types="svelte" />
;
import {onMount} from 'svelte';
import {recapStateLabel, sentenceCase} from '../../lib/labels';
import {pollWhileVisible} from '../../lib/poll';
import type {RecapDetail, RecapDraft, RecapEventData, RecapRevision} from '../../lib/api/room-recap-types';
import {recapRequest, recapBlob} from './recap-client';
import {confirmer} from '../../lib/confirm.svelte';
import {copyTextOrThrow} from '../../lib/clipboard';
import {formatBytes} from '../../lib/metric-format';
import LoadState from '../../lib/components/LoadState.svelte';
import {loadErrorText} from '../../lib/loadError';
import RecapImage from './RecapImage.svelte';
import {announce} from '../../lib/announce.svelte';

;type $$ComponentProps =  {recapId: string};function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  let {recapId}:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  let detail = $state<RecapDetail | null>(null), error = $state(''), actionError = $state(''), loading = $state(true), busy = $state(false), copied = $state(false), cancelling = $state(false), exporting = $state(false);
  // a11y-6-01: status and error edges are announced; the visible lines are not live regions.
  const summaryRunning = $derived(['queued', 'running'].includes(detail?.metadata.summary_status ?? ''));
  const summaryError = $derived(detail?.metadata.summary_error ?? '');
  $effect(() => { if (summaryRunning) announce('Codex is preparing a draft…'); });
  $effect(() => { if (summaryError) announce(summaryError, {assertive: true}); });
  $effect(() => { if (actionError) announce(actionError, {assertive: true}); });
  // A failed refresh over a loaded recap is LoadState's stale bar ("Couldn’t
  // refresh this recap — …" + Retry), which announces itself (ux-design-6w1-01).
  let tab = $state<'transcript' | 'activity' | 'summary'>('transcript'), after = $state(0), previous = $state<number[]>([]);
  let live = true, request = 0, loadingBody = false;
  let revision: RecapRevision | null = null, loadedAfter: number | null = null;
  onMount(() => { const poller = pollWhileVisible(signal => busy ? undefined : poll(signal), {ms: 4000}); return () => { live = false; request++; poller.stop(); }; });
  function readRevision(signal?: AbortSignal) {
    return recapRequest<RecapRevision>(`/room-recaps/${encodeURIComponent(recapId)}/revision`, undefined, signal);
  }
  function sameBody(a: RecapRevision, b: RecapRevision) {
    return a.events_revision === b.events_revision && a.draft_revision === b.draft_revision;
  }
  async function poll(signal?: AbortSignal) {
    if (loadingBody) return;
    if (!detail || !revision || loadedAfter !== after) return load(!detail, signal);
    const seq = ++request;
    try {
      const next = await readRevision(signal);
      if (!live || seq !== request) return;
      // Older complete pages are immutable. Refresh the current tail when it
      // grows, including the exact full-page boundary that exposes Next events.
      if (next.draft_revision !== revision.draft_revision ||
          (next.events_revision !== revision.events_revision && detail.next_cursor === null)) {
        return load(false, signal, next);
      }
      detail = {...detail, metadata: next.metadata};
      revision = next;
      error = '';
    } catch (e) {
      if (live && seq === request) error = loadErrorText(e);
      return false;
    }
  }
  async function load(showLoading = true, signal?: AbortSignal, knownRevision?: RecapRevision) {
    if (showLoading) loading = true;
    loadingBody = true;
    const seq = ++request, requestedAfter = after;
    try {
      let before = knownRevision ?? await readRevision(signal);
      // A changed archive may race the body request. Retry once, then leave the
      // body unacknowledged so the next visible tick recovers without a loop.
      for (let attempt = 0; attempt < 2; attempt++) {
        if (!live || seq !== request) return false;
        const value = await recapRequest<RecapDetail>(`/room-recaps/${encodeURIComponent(recapId)}?after=${requestedAfter}&limit=100`, undefined, signal);
        if (!live || seq !== request) return false;
        const latest = await readRevision(signal);
        if (!live || seq !== request) return false;
        if (sameBody(before, latest)) {
          detail = {...value, metadata: latest.metadata};
          revision = latest;
          loadedAfter = requestedAfter;
          error = '';
          return true;
        }
        if (attempt === 1) {
          detail = value;
          revision = null;
          loadedAfter = requestedAfter;
          error = '';
        }
        before = latest;
      }
      return true;
    } catch (e) {
      if (live && seq === request) error = loadErrorText(e);
      return false;
    } finally {
      if (seq === request) { loading = false; loadingBody = false; }
    }
  }
  async function generate() { if (!await confirmer.ask('Send captured text and representative shared-screen images to Codex using your ChatGPT subscription to create a draft. This uses your subscription allowance. Not every saved image is included; the draft reports coverage. The local archive remains available if generation fails.', {title: 'Generate recap summary?', confirmLabel: 'Generate summary'})) return; busy = true; actionError = ''; try { await recapRequest(`/room-recaps/${recapId}/summary`, {}); tab = 'summary'; await load(false); } catch (e) { actionError = `Couldn’t start the summary. ${loadErrorText(e)}`; } finally { busy = false; } }
  async function cancel() { cancelling = true; try { await recapRequest(`/room-recaps/${recapId}/summary`, undefined, undefined, 'DELETE'); await load(false); } catch { actionError = 'Couldn’t cancel generation. Try again.'; } finally { cancelling = false; } }
  async function exportArchive() { exporting = true; try { const blob = await recapBlob(`/room-recaps/${recapId}/export`); const url = URL.createObjectURL(blob); const link = document.createElement('a'); link.href = url; link.download = `room-recap-${recapId}.json`; link.click(); setTimeout(() => URL.revokeObjectURL(url), 30000); } catch { actionError = 'Couldn’t export the recap. Try again.'; } finally { exporting = false; } }
  function draftText(draft: RecapDraft) { return ['Codex draft', draft.overview, 'Decisions', ...draft.decisions, 'Actions', ...draft.actions, 'Open questions', ...draft.open_questions, 'Coverage', ...draft.coverage].join('\n\n'); }
  async function copyDraft() { if (!detail?.draft) return; try { await copyTextOrThrow(draftText(detail.draft)); copied = true; } catch { actionError = 'Couldn’t copy the draft. Select the text and copy it manually.'; } }
  function terminalText(data: string): string { try { return new TextDecoder().decode(Uint8Array.from(atob(data), c => c.charCodeAt(0))).replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, '').replace(/\x1b\][^\x07]*(?:\x07|\x1b\\)/g, ''); } catch { return 'Terminal text couldn’t be decoded.'; } }
  function activity(payload: RecapEventData): string {
    switch (payload.type) {
      case 'capture': return `Capture ${recapStateLabel(payload.state).toLowerCase()}${payload.reason ? `: ${payload.reason}` : ''}`;
      case 'participants': return `Participants: ${payload.members.map(m => m.name).join(', ')}`;
      case 'presentation': return `${payload.operation}: ${payload.presentation.title}`;
      case 'annotation': return `${payload.annotation.tool} annotation on source ${payload.annotation.source_id}`;
      case 'annotations_cleared': return 'Shared-screen annotations cleared';
      case 'annotation_removed': return 'Shared-screen annotation removed';
      case 'audio_pending': return `Transcribing ${payload.member_name}’s audio…`;
      default: return '';
    }
  }
;
async () => {
 { svelteHTML.createElement("div", { "class":`recap-panel`,});
   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"what":`this recap`,"variant":`compact`,loading,error,"empty":!detail,"onretry":() => load(),}});
  if(actionError){ { svelteHTML.createElement("p", { "class":`error`,});actionError; }}
  if(detail){
     { svelteHTML.createElement("div", { "class":`recap-heading`,}); { svelteHTML.createElement("div", {}); { svelteHTML.createElement("strong", {});detail.metadata.session_title; } { svelteHTML.createElement("p", {});recapStateLabel(detail.metadata.status);  formatBytes(detail.metadata.bytes_used);  formatBytes(detail.metadata.quota_bytes); } } { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":loading,"onclick":() => load(),});  } { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":exporting,"onclick":exportArchive,});exporting ? 'Exporting…' : 'Export full archive'; } }
    if(!detail.metadata.speech_available){ { svelteHTML.createElement("p", { "class":`coverage`,});   detail.metadata.speech_error ?? 'The archive contains available room activity only.'; }}
     { svelteHTML.createElement("nav", { "aria-label":`Recap sections`,});  for(let value of __sveltets_2_ensureArray((['transcript', 'activity', 'summary']))){ { svelteHTML.createElement("button", {     "class":`btn`,"aria-pressed":tab === value,"onclick":() => tab = value as typeof tab,});value === 'transcript' ? 'Transcript' : value === 'activity' ? 'Activity & screens' : 'Summary'; }} }
    if(tab === 'summary'){
       { svelteHTML.createElement("div", { "class":`summary-actions`,});if(['queued', 'running'].includes(detail.metadata.summary_status)){ { svelteHTML.createElement("p", {});     } { svelteHTML.createElement("button", {     "class":`btn`,"disabled":cancelling,"onclick":cancel,});cancelling ? 'Canceling…' : 'Cancel generation'; }}else{ { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy,"onclick":generate,});detail.draft ? 'Regenerate summary…' : 'Generate summary…'; }} }
      if(detail.metadata.summary_error){ { svelteHTML.createElement("p", { "class":`error`,});detail.metadata.summary_error; }}
      if(detail.draft){ { svelteHTML.createElement("article", { "class":`draft`,}); { svelteHTML.createElement("header", {}); { svelteHTML.createElement("strong", {});  } { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":copyDraft,});copied ? 'Copied' : 'Copy draft'; } } { svelteHTML.createElement("p", {});detail.draft.overview; }
          for(let group of __sveltets_2_ensureArray(([{title: 'Decisions', items: detail.draft.decisions}, {title: 'Actions', items: detail.draft.actions}, {title: 'Open questions', items: detail.draft.open_questions}, {title: 'Coverage', items: detail.draft.coverage}]))){ { svelteHTML.createElement("h3", {});group.title; }if(group.items.length){ { svelteHTML.createElement("ul", {});  for(let item of __sveltets_2_ensureArray(group.items)){ { svelteHTML.createElement("li", {});item; }} }}else{ { svelteHTML.createElement("p", {});  }}}
         { svelteHTML.createElement("p", { "class":`hint`,});  detail.draft.source_event_ids.join(', ') || 'Not supplied'; }
        if(detail.metadata.summary_through_seq !== null && detail.metadata.last_seq > detail.metadata.summary_through_seq){ { svelteHTML.createElement("p", { "class":`coverage`,});           }}
       }}else{ { svelteHTML.createElement("p", {});          }}
    }else{
       { svelteHTML.createElement("p", { "class":`hint`,});  detail.events[0]?.seq ?? 0; detail.events.at(-1)?.seq ?? 0;            }
       { svelteHTML.createElement("div", { "class":`events`,});   for(let event of __sveltets_2_ensureArray(detail.events)){event.seq;const payload = event.payload;
        if(tab === 'activity' || ['speech', 'chat', 'terminal', 'gap', 'capture', 'audio_pending'].includes(payload.type)){ { svelteHTML.createElement("article", { });payload.type === 'gap';
           { svelteHTML.createElement("header", {}); { svelteHTML.createElement("span", {}); event.seq;  new Date(event.created_at).toLocaleTimeString(); } { svelteHTML.createElement("span", {});sentenceCase(payload.type); } }
          if(payload.type === 'speech'){ { svelteHTML.createElement("strong", {});payload.member_name; }  for(let segment of __sveltets_2_ensureArray(payload.segments)){ { svelteHTML.createElement("p", {});segment.text; }}
          } else if (payload.type === 'chat'){ { svelteHTML.createElement("strong", {});payload.message.name; } { svelteHTML.createElement("p", {});payload.message.text; }
          } else if (payload.type === 'terminal'){ { svelteHTML.createElement("pre", {});terminalText(payload.data_base64); }
          } else if (payload.type === 'screen'){ { svelteHTML.createElement("strong", {});payload.title; } { svelteHTML.createElement("p", {});payload.text || 'No readable text detected; inspect the saved sample.'; } { const $$_egamIpaceR4C = __sveltets_2_ensureComponent(RecapImage); new $$_egamIpaceR4C({ target: __sveltets_2_any(), props: {     recapId,"imageId":payload.image_id,"title":payload.title,}});}
          } else if (payload.type === 'gap'){ { svelteHTML.createElement("p", {}); { svelteHTML.createElement("strong", {});  } payload.reason; }
          }else{ { svelteHTML.createElement("p", {});activity(payload); }}
         }}
      } }
      if(!detail.events.length){ { svelteHTML.createElement("p", {});    }}
       { svelteHTML.createElement("div", { "class":`pagination`,}); { svelteHTML.createElement("button", {     "class":`btn`,"disabled":!previous.length || loading,"onclick":() => { after = previous.at(-1) ?? 0; previous = previous.slice(0, -1); void load(); },});  } { svelteHTML.createElement("button", {     "class":`btn`,"disabled":detail.next_cursor === null || loading,"onclick":() => { previous = [...previous, after]; after = detail!.next_cursor!; void load(); },});  } }
    }
  }
   LoadState}
 }

};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RecapPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RecapPanel__SvelteComponent_ = ReturnType<typeof RecapPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default RecapPanel__SvelteComponent_;