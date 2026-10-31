///<reference types="svelte" />
;// One render item of the chat — two speakers, told apart at a glance:
//   • You: a right-aligned BLUE bubble (accent tint) with the time, Copy and
//     "Edit & resend" under it.
//   • The agent: a GREEN identity — its mark in a green disc + name + model +
//     time — and the response on a faint green surface hanging off a green
//     rail. A settled response keeps its FIRST and FINAL message visible and
//     folds the work between them (tool groups, narration, plan snapshots)
//     into "Worked for 2m 7s · 7 steps ›" (t3code); a live one stays flat.
//     Under it: the changed-files card and a footer with duration, model and
//     where the tokens went (in · thinking · out · cache).
// Thinking is its own violet, italic marker — never mistaken for the answer.
// Per-turn system notes stay a chip that expands in place.

import { copyTextOrThrow } from '../../../lib/clipboard';
import { getContext } from 'svelte';
import { toastError } from '../../../lib/toastError';
import Icon, { type IconName } from '../../../lib/components/Icon.svelte';
import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
import AgentChip from '../../../lib/components/AgentChip.svelte';
import Markdown from './Markdown.svelte';
import WorkSteps from './WorkSteps.svelte';
import TasksBlock from './TasksBlock.svelte';
import ImageBlock from './ImageBlock.svelte';
import ThinkingMarker from './ThinkingMarker.svelte';
import UsageLine from './UsageLine.svelte';
import ChangedFiles from './ChangedFiles.svelte';
import { transcript } from '../../../lib/stores/transcript.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { changedFiles, fmtClock, fmtDuration, foldResponse, foldStats, providerName, segment, type RenderItem, type Segment } from './format';
import type { Artifact, SystemNote } from '../../../lib/api/types';
import { CONV_CTX, type ConvContext } from './context';
;

  interface Props {
    item: RenderItem;
    /** True for the newest response while the agent works on it (flat, step groups start open). */
    live?: boolean;
    /** Newest response of a session that is still alive: calls without a
     *  result are in progress / waiting on you, not interrupted. */
    active?: boolean;
    /** Stopped on its open call, waiting for you (permission / question). */
    waiting?: boolean;
    /** Inside a subagent card — no attribution header, tighter chrome. */
    nested?: boolean;
    /** Matches the conversation search (soft highlight) / is the current match. */
    hit?: boolean;
    current?: boolean;
  };function $$render() {
 const pendingChip/*Ωignore_positionΩ*/ = (text: string)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   { svelteHTML.createElement("div", {   "class":`pending`,...__sveltets_2_empty({"data-pending":true}),"title":`Sent while the agent was busy — the CLI delivers it when the current turn ends`,});
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":12,}});}
     { svelteHTML.createElement("span", { "class":`pending-label`,});  }
     { svelteHTML.createElement("span", { "class":`pending-text`,});text; }
   }
};return __sveltets_2_any(0)};  const sysChip/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  if(sysNotes.length){
     { svelteHTML.createElement("button", {        "class":`sys-chip`,"onclick":() => (sysOpen = !sysOpen),"aria-expanded":sysOpen || showSystem,"title":`System notes attached to this turn (reminders, hooks, attachments)`,});sysOpen;
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":12,}});} sysNotes.length; 
     }
  }
};return __sveltets_2_any(0)}; const seg/*Ωignore_positionΩ*/ = (s: Segment, i: number)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  if(s.kind === 'steps'){
    if(s.steps.every((b) => b.kind === 'thinking')){
       { const $$_rekraMgniknihT0C = __sveltets_2_ensureComponent(ThinkingMarker); new $$_rekraMgniknihT0C({ target: __sveltets_2_any(), props: {     "count":s.steps.reduce((n, b) => n + (b.kind === 'thinking' ? b.count : 0), 0),"tokens":s === firstThink ? (item.usage?.thinking_tokens ?? null) : null,agentName,}});}
    }else{
       { const $$_spetSkroW0C = __sveltets_2_ensureComponent(WorkSteps); new $$_spetSkroW0C({ target: __sveltets_2_any(), props: {       "steps":s.steps,"durationMs":!fold && i === firstStepsIdx ? item.duration_ms : null,live,active,waiting,}});}
    }
  } else if (s.block.kind === 'text'){
     { const $$_nwodkraM0C = __sveltets_2_ensureComponent(Markdown); new $$_nwodkraM0C({ target: __sveltets_2_any(), props: {  "md":s.block.md,}});}
  } else if (s.block.kind === 'tasks'){
     { const $$_kcolBsksaT0C = __sveltets_2_ensureComponent(TasksBlock); new $$_kcolBsksaT0C({ target: __sveltets_2_any(), props: {    "tasks":s.block.tasks,"folded":s !== lastTasks,}});}
  } else if (s.block.kind === 'image'){
     { const $$_kcolBegamI0C = __sveltets_2_ensureComponent(ImageBlock); new $$_kcolBegamI0C({ target: __sveltets_2_any(), props: {      "id":s.block.id,"alt":s.block.alt,"mediaType":s.block.media_type,}});}
  } else if (s.block.kind === 'queued'){
    ;__sveltets_2_ensureSnippet(pendingChip(s.block.text));
  } else if (s.block.kind === 'artifact'){
    const a = s.block.artifact;
     { svelteHTML.createElement("button", {     "class":`chip artifact`,"onclick":() => openArtifact(a),"title":a.url ?? a.path ?? a.label,});
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":artifactIcon(a),"size":12,}});} a.label;
      if(a.path && !a.url){ { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`eye`,"size":12,}});}}
     }
  } else if (s.block.kind === 'notice'){
     { svelteHTML.createElement("div", {   "class":`notice`,"title":s.block.note.kind,});
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":12,}});}  { svelteHTML.createElement("strong", {});s.block.note.title; }
      if(s.block.note.body){ { svelteHTML.createElement("span", { "class":`dim`,});  s.block.note.body.slice(0, 400); }}
     }
  }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { item, live = false, active = live, waiting = false, nested = false, hit = false, current = false }: Props = $props();

  // Copy: the turn's prose as markdown; tool steps as one-line summaries so a
  // pasted response still reads. Images/chips are skipped.
  const copyText = $derived.by(() => {
    const parts: string[] = [];
    for (const b of item.blocks) {
      if (b.kind === 'text') parts.push(b.md.trim());
      else if (b.kind === 'tool_call') parts.push(`[${b.name}] ${b.title}`.trim());
      else if (b.kind === 'queued' && b.op === 'enqueue') parts.push(`Queued: ${b.text}`);
    }
    return parts.filter(Boolean).join('\n\n');
  });
  /** Your message's own words (Edit & resend puts them back in the composer). */
  const promptText = $derived(
    item.role === 'user' ? item.blocks.flatMap((b) => (b.kind === 'text' ? [b.md] : [])).join('\n\n').trim() : '',
  );
  // The composer sends attached images as `[Image: <path>]` lines after the
  // text; the bubble shows them as attachment chips, not as raw markers.
  const IMAGE_LINE = /^\[Image: (.+)\]\s*$/gm;
  function splitAttachments(md: string): { md: string; images: string[] } {
    const images: string[] = [];
    const rest = md.replace(IMAGE_LINE, (_m, p: string) => {
      images.push(p.trim());
      return '';
    });
    return { md: images.length ? rest.trim() : md, images };
  }
  async function copyPath(path: string): Promise<void> {
    try {
      await copyTextOrThrow(path);
      toasts.info('Path copied', path);
    } catch (e) {
      toastError('Couldn’t copy to the clipboard', e);
    }
  }
  async function copyTurn(): Promise<void> {
    try {
      await copyTextOrThrow(copyText);
      toasts.info('Copied', item.role === 'user' ? 'Your message' : 'The response');
    } catch (e) {
      toastError('Couldn’t copy to the clipboard', e);
    }
  }
  const ctx = getContext<ConvContext>(CONV_CTX);

  const segs = $derived(segment(item.blocks));
  const showSystem = $derived(transcript.showSystem);
  const isCodex = $derived(ctx.provider === 'codex');
  const agentName = $derived(providerName(ctx.provider));
  let sysOpen = $state(false);
  const sysNotes = $derived<SystemNote[]>(item.system);
  const visibleBlocks = $derived(
    segs.filter((s) => {
      if (s.kind !== 'block') return true;
      const b = s.block;
      if (b.kind === 'queued') return b.op === 'enqueue' && ctx.queuedLive.includes(b.text) && (showSystem || !b.injected);
      if (b.kind === 'notice') return showSystem;
      return true;
    }),
  );
  // Live / waiting responses stay flat: the reader follows the work as it happens.
  const fold = $derived(nested || live || waiting ? null : foldResponse(visibleBlocks));
  /** Your own open / close of this fold; null follows "Show all work". */
  let foldPref = $state<boolean | null>(null);
  const foldShown = $derived(!!fold && (current || ctx.revealId === item.id || (foldPref ?? !!ctx.expandAll)));
  function toggleFold(): void {
    const next = !foldShown;
    if (!next && ctx.revealId === item.id) ctx.revealId = null;
    foldPref = next;
  }
  const stats = $derived(fold ? foldStats(fold.body) : null);
  const firstStepsIdx = $derived(visibleBlocks.findIndex((s) => s.kind === 'steps'));
  // Only the response's LAST plan snapshot starts open; earlier ones fold.
  const lastTasks = $derived(visibleBlocks.findLast((s) => s.kind === 'block' && s.block.kind === 'tasks') ?? null);
  // The first thinking marker carries the response's thinking-token count.
  const firstThink = $derived(visibleBlocks.find((s) => s.kind === 'steps' && s.steps.every((b) => b.kind === 'thinking')) ?? null);
  const changed = $derived(item.role === 'assistant' && !nested && !live ? changedFiles(item.blocks) : []);
  const clock = $derived(fmtClock(item.ts));
  const fullTime = $derived(item.ts ? new Date(item.ts).toLocaleString() : '');
  const workedFor = $derived(item.duration_ms != null ? `Worked for ${fmtDuration(item.duration_ms)}` : 'Worked');

  function artifactIcon(a: Artifact): IconName {
    return a.kind === 'pr' ? 'pr' : a.kind === 'image' ? 'image' : a.kind === 'url' ? 'link' : a.kind === 'report' ? 'note' : 'file';
  }
  function openArtifact(a: Artifact): void {
    if (a.url) ctx?.openUrl?.(a.url);
    else if (a.path) ctx?.openPreview?.({ kind: 'file', path: a.path });
  }
;
async () => {







  { svelteHTML.createElement("article", {           "class":`turn ${item.role}`,...__sveltets_2_empty({"data-turn-id":item.id}),...__sveltets_2_empty({"data-role":item.role}),"aria-label":item.role === 'user' ? `You${clock ? `, ${clock}` : ''}` : `${agentName}${clock ? `, ${clock}` : ''}`,});nested;hit;current;
  if(item.role === 'user'){
     { svelteHTML.createElement("div", {   "class":`bubble chat-bubble-end`,"dir":`auto`,});
          for(let b of __sveltets_2_ensureArray(item.blocks)){let i = 1;i;
        if(b.kind === 'text'){
          const parts = splitAttachments(b.md);
          if(parts.md){ { const $$_nwodkraM2C = __sveltets_2_ensureComponent(Markdown); new $$_nwodkraM2C({ target: __sveltets_2_any(), props: {  "md":parts.md,}});}}
          if(parts.images.length){
             { svelteHTML.createElement("div", {   "class":`attach`,...__sveltets_2_empty({"data-attachments":parts.images.length}),});
                 for(let p of __sveltets_2_ensureArray(parts.images)){p;
                 { svelteHTML.createElement("button", {     "class":`attach-chip`,"onclick":() => void copyPath(p),"title":`Attached image — ${p} (click to copy the path)`,});
                   { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`image`,"size":12,}});}  { svelteHTML.createElement("span", { "class":`attach-name`,});p.split('/').pop(); }
                 }
              }
             }
          }
        } else if (b.kind === 'image'){
           { const $$_kcolBegamI2C = __sveltets_2_ensureComponent(ImageBlock); new $$_kcolBegamI2C({ target: __sveltets_2_any(), props: {      "id":b.id,"alt":b.alt,"mediaType":b.media_type,}});}
        } else if (b.kind === 'queued' && b.op === 'enqueue' && ctx.queuedLive.includes(b.text) && (showSystem || !b.injected)){
          ;__sveltets_2_ensureSnippet(pendingChip(b.text));
        } else if (b.kind === 'notice' && showSystem){
           { svelteHTML.createElement("span", {   "class":`chip note`,"title":b.note.body ?? '',});b.note.title; }
        }
      }
     }
     { svelteHTML.createElement("div", { "class":`meta`,});
      ;__sveltets_2_ensureSnippet(sysChip());
      if(ctx.reusePrompt && promptText){
         { svelteHTML.createElement("button", {       "class":`act-btn`,"onclick":() => ctx.reusePrompt?.(promptText),"title":`Edit & resend — put this message back in the composer`,"aria-label":`Edit and resend`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});} }
      }
      if(copyText){
         { svelteHTML.createElement("button", {       "class":`act-btn copy-btn`,"onclick":() => void copyTurn(),"title":`Copy message`,"aria-label":`Copy message`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});} }
      }
      if(clock){ { svelteHTML.createElement("time", {     "class":`ts`,"datetime":item.ts ?? undefined,"title":fullTime,});clock; }}
     }
  }else{
    if(!nested){
       { svelteHTML.createElement("header", { "class":`agent-head`,});
         { svelteHTML.createElement("span", {   "class":`avatar`,"aria-hidden":`true`,}); { const $$_nocIredivorP3C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP3C({ target: __sveltets_2_any(), props: {    "provider":ctx.provider,"size":13,}});} }
         { svelteHTML.createElement("span", { "class":`agent-name`,});agentName; }
         { const $$_pihCtnegA2C = __sveltets_2_ensureComponent(AgentChip); new $$_pihCtnegA2C({ target: __sveltets_2_any(), props: {}});}
        if(item.model){ { svelteHTML.createElement("span", {   "class":`agent-model mono`,"title":`Model`,});item.model; }}
        if(clock){ { svelteHTML.createElement("time", {     "class":`ts`,"datetime":item.ts ?? undefined,"title":fullTime,});clock; }}
       }
    }
     { svelteHTML.createElement("div", { "class":`resp`,});
      if(isCodex && item.reasoning_steps > 0){
         { const $$_rekraMgniknihT2C = __sveltets_2_ensureComponent(ThinkingMarker); new $$_rekraMgniknihT2C({ target: __sveltets_2_any(), props: {     "codex":true,"count":item.reasoning_steps,"tokens":item.usage?.thinking_tokens ?? null,agentName,}});}
      }
      if(fold){
            for(let s of __sveltets_2_ensureArray(fold.head)){let i = 1;i;;__sveltets_2_ensureSnippet(seg(s, i));}
         { svelteHTML.createElement("div", {  "class":`fold`,});foldShown;
           { svelteHTML.createElement("button", {       "class":`fold-row`,"onclick":toggleFold,"aria-expanded":foldShown,...__sveltets_2_empty({"data-fold-toggle":true}),"title":foldShown ? 'Hide the work' : 'Show the work: tool calls, narration and plan updates',});
             { svelteHTML.createElement("span", { "class":`fold-label`,});workedFor; }
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":foldShown ? 'chevronDown' : 'chevronRight',"size":12,}});}
            if(stats){
               { svelteHTML.createElement("span", { "class":`fold-meta`,});`${stats.steps} ${stats.steps === 1 ? 'step' : 'steps'}${stats.thinking ? ` · thought ${stats.thinking}×` : ''}`; }
              if(stats.failed){ { svelteHTML.createElement("span", { "class":`fold-fail`,});stats.failed;  }}
            }
           }
          if(foldShown){
             { svelteHTML.createElement("div", { "class":`fold-body`,});
                  for(let s of __sveltets_2_ensureArray(fold.body)){let i = 1;i;;__sveltets_2_ensureSnippet(seg(s, i));}
             }
          }
         }
            for(let s of __sveltets_2_ensureArray(fold.tail)){let i = 1;i;;__sveltets_2_ensureSnippet(seg(s, i));}
      }else{
            for(let s of __sveltets_2_ensureArray(visibleBlocks)){let i = 1;i;;__sveltets_2_ensureSnippet(seg(s, i));}
      }
      if(!visibleBlocks.length && !(isCodex && item.reasoning_steps > 0)){
         { svelteHTML.createElement("div", { "class":`dim empty`,});  sysNotes.length ? ' — system notes only' : '';  }
      }
      if(changed.length){
         { const $$_seliFdegnahC2C = __sveltets_2_ensureComponent(ChangedFiles); new $$_seliFdegnahC2C({ target: __sveltets_2_any(), props: {  "files":changed,}});}
      }
     }
     { svelteHTML.createElement("footer", { "class":`meta`,});
       { const $$_eniLegasU2C = __sveltets_2_ensureComponent(UsageLine); new $$_eniLegasU2C({ target: __sveltets_2_any(), props: {      "usage":item.usage ?? null,"durationMs":fold ? null : item.duration_ms,"model":nested ? item.model : null,}});}
       { svelteHTML.createElement("span", { "class":`meta-grow`,}); }
      ;__sveltets_2_ensureSnippet(sysChip());
      if(nested && clock){ { svelteHTML.createElement("time", {     "class":`ts`,"datetime":item.ts ?? undefined,"title":fullTime,});clock; }}
      if(copyText){
         { svelteHTML.createElement("button", {       "class":`act-btn copy-btn`,"onclick":() => void copyTurn(),"title":`Copy response`,"aria-label":`Copy response`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});} }
      }
     }
  }
  if(sysNotes.length && (sysOpen || showSystem)){
     { svelteHTML.createElement("div", {   "class":`sys-list`,...__sveltets_2_empty({"data-system-notes":sysNotes.length}),});
          for(let n of __sveltets_2_ensureArray(sysNotes)){let i = 1;i;
         { svelteHTML.createElement("details", { "class":`sys-note`,});
           { svelteHTML.createElement("summary", {}); { svelteHTML.createElement("span", { "class":`sys-kind mono`,});n.kind; } n.title; }
          if(n.body){ { svelteHTML.createElement("pre", {   "class":`mono`,"dir":`ltr`,});n.body.length > 4000 ? n.body.slice(0, 4000) + '\n…' : n.body; }}
         }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const TurnItem__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TurnItem__SvelteComponent_ = ReturnType<typeof TurnItem__SvelteComponent_>;
/*Ωignore_endΩ*/export default TurnItem__SvelteComponent_;