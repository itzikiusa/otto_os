///<reference types="svelte" />
;// Run detail drawer: everything one swarm turn produced — the brief that was
// sent, the cwd/worktree it ran in, the parsed artifacts as clickable rows
// (open file / open PR), the board posts tagged with this run, tokens + cost,
// and the raw result JSON. Mirrors workflows/RunSteps.svelte.

import Modal from '../../lib/components/Modal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import Badge from '../../lib/components/Badge.svelte';
import { runStatus, sentenceCase, type BadgeTone } from '../../lib/status';
import { loadErrorText } from '../../lib/loadError';
import { isAbortError } from '../../lib/api/client';
import { toasts } from '../../lib/toast.svelte';
import { openExternal, isExternalUrl } from '../../lib/external';
import { swarm } from '../../lib/stores/swarm.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { canOpenRunSession, RUN_SESSION_NOT_YOURS } from './access';
import type { SwarmRun, SwarmMessage, TurnResult, TurnArtifact } from './types';
import { copyTextOrThrow } from '../../lib/clipboard';
import { formatCount, formatUsd } from '../../lib/metric-format';
;

  interface Props {
    run: SwarmRun;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  /** Finding severity → Badge tone (a warning is not a selection). */
  const SEV_TONE: Record<string, BadgeTone> = { error: 'bad', warn: 'warn', info: 'neutral' };

  let { run, onclose }: Props = $props();

  // access-control-6-04: same rule as the Runs list.
  const canOpen = $derived(!!run.session_id && canOpenRunSession(
    { isRoot: auth.isRoot, myRole: ws.workspaces.find((w) => w.id === run.workspace_id)?.my_role },
    !!ws.getSession(run.session_id),
  ));

  const agent = $derived(swarm.agentById(run.agent_id));
  // The runs list is `lite` (no per-run result blob); a live event's run does
  // carry it. Otherwise fetch this one run in full, again when its status
  // changes (a finished turn writes the result).
  let fetched = $state.raw<SwarmRun | null>(null);
  let resultError = $state<string | null>(null);
  let resultAttempt = $state(0);
  $effect(() => {
    const id = run.id;
    const status = run.status;
    const has = run.result != null;
    const attempt = resultAttempt;
    void status;
    void attempt;
    if (has) {
      resultError = null;
      return;
    }
    const ctl = new AbortController();
    resultError = null;
    swarm
      .getRun(id, ctl.signal)
      .then((r) => {
        if (!ctl.signal.aborted) fetched = r;
      })
      .catch((e: unknown) => {
        if (ctl.signal.aborted || isAbortError(e)) return;
        resultError = loadErrorText(e);
      });
    return () => ctl.abort();
  });
  const result = $derived(
    (run.result ?? (fetched?.id === run.id ? fetched.result : null) ?? null) as TurnResult | null,
  );
  const brief = $derived(typeof result?.brief === 'string' ? result.brief : null);
  const cwd = $derived(typeof result?.cwd === 'string' ? result.cwd : null);
  const artifacts = $derived<TurnArtifact[]>(
    Array.isArray(result?.artifacts) ? (result!.artifacts as TurnArtifact[]) : [],
  );
  // Board posts tagged with this run (the store keeps the open swarm's board).
  const posts = $derived(swarm.board.filter((m) => m.run_id === run.id));

  const hasTokens = $derived(
    run.tokens_input != null || run.tokens_output != null || run.cost_usd != null,
  );

  /** Absolute local date-time ("21/10/2026, 14:03") — NOT the shared
   *  relative `rel()`; named apart so the two are never confused. */
  function fmtWhen(ts?: string | null): string {
    if (!ts) return '—';
    return new Date(ts).toLocaleString();
  }
  function author(m: SwarmMessage): string {
    if (m.author_agent_id) return swarm.agentById(m.author_agent_id)?.name ?? 'agent';
    if (m.author_user_id) return 'you';
    return 'system';
  }

  function artifactKind(a: TurnArtifact): 'pr' | 'url' | 'file' {
    if (a.type === 'pr') return 'pr';
    if (a.path) return 'file';
    if (a.url) return isExternalUrl(a.url) ? 'url' : 'file';
    return 'file';
  }
  function artifactTarget(a: TurnArtifact): string | null {
    return a.url || a.path || null;
  }
  async function openArtifact(a: TurnArtifact): Promise<void> {
    const target = artifactTarget(a);
    if (!target) return;
    if (isExternalUrl(target)) {
      await openExternal(target);
      return;
    }
    // A local file path — copy it so the user can paste/open it (the daemon has
    // no generic "reveal in Finder" route; copying is the safe, faithful action).
    await copy(target, 'path');
  }

  async function copy(text: string, label = 'output'): Promise<void> {
    try {
      await copyTextOrThrow(text);
      toasts.success(`Copied ${label}`);
    } catch {
      toasts.error('Couldn’t copy to the clipboard', 'Select the text and copy it manually.');
    }
  }

  const rawJson = $derived(result ? JSON.stringify(result, null, 2) : null);
  const runTitle = $derived.by(() => {
    const kind = sentenceCase(run.kind);
    return agent?.name ? `Run detail — ${agent.name} (${kind})` : `Run detail — ${kind}`;
  });
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":runTitle,"width":640,onclose,}});
   { svelteHTML.createElement("div", { "class":`insp`,});
    
     { svelteHTML.createElement("div", { "class":`hdr`,});
       { svelteHTML.createElement("span", { "class":`agent`,});agent?.name ?? run.agent_id.slice(0, 8); }
       { svelteHTML.createElement("span", { "class":`dim`,}); sentenceCase(run.kind); }
      
       { const $$_egdaBsutatS3C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS3C({ target: __sveltets_2_any(), props: {  "status":runStatus(run.status),}});}
       { svelteHTML.createElement("span", { "class":`grow`,}); }
      if(run.session_id && canOpen){
         { svelteHTML.createElement("button", {     "class":`btn small ghost`,"aria-label":`Open ${agent?.name ?? 'this run'}’s session`,"onclick":() => { swarm.selectedSessionId = run.session_id!; onclose(); },});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":12,}});}  
         }
      } else if (run.session_id){
         { svelteHTML.createElement("button", {     "class":`btn small ghost`,"disabled":true,"title":RUN_SESSION_NOT_YOURS,"aria-label":`Open session: ${RUN_SESSION_NOT_YOURS}`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":12,}});}  
         }
      }
     }
     { svelteHTML.createElement("div", { "class":`meta`,});
       { svelteHTML.createElement("span", { "class":`dim`,}); fmtWhen(run.enqueued_at); }
      if(run.started_at){ { svelteHTML.createElement("span", { "class":`dim`,});  fmtWhen(run.started_at); }}
      if(run.finished_at){ { svelteHTML.createElement("span", { "class":`dim`,});  fmtWhen(run.finished_at); }}
     }

    if(run.error){
       { svelteHTML.createElement("div", { "class":`err`,});run.error; }
    }

    
     { svelteHTML.createElement("div", { "class":`stats`,});
       { svelteHTML.createElement("div", { "class":`stat`,});
         { svelteHTML.createElement("span", { "class":`k`,});  }
         { svelteHTML.createElement("span", {   "class":`v mono`,"title":run.tokens_input?.toLocaleString(),});run.tokens_input == null ? '—' : formatCount(run.tokens_input); }
       }
       { svelteHTML.createElement("div", { "class":`stat`,});
         { svelteHTML.createElement("span", { "class":`k`,});  }
         { svelteHTML.createElement("span", {   "class":`v mono`,"title":run.tokens_output?.toLocaleString(),});run.tokens_output == null ? '—' : formatCount(run.tokens_output); }
       }
       { svelteHTML.createElement("div", { "class":`stat`,});
         { svelteHTML.createElement("span", { "class":`k`,});  }
         { svelteHTML.createElement("span", { "class":`v mono`,});formatUsd(run.cost_usd); }
       }
     }
    if(!hasTokens){
       { svelteHTML.createElement("div", { "class":`muted small`,});             }
    }

    
    if(result?.concerns?.length){
       { svelteHTML.createElement("section", {});
         { svelteHTML.createElement("h3", {});  { svelteHTML.createElement("span", { "class":`count`,});result.concerns.length; } }
         { svelteHTML.createElement("div", { "class":`findings`,});
              for(let c of __sveltets_2_ensureArray(result.concerns ?? [])){let i = 1;i;
             { svelteHTML.createElement("div", { "class":`finding ${c.severity}`,});
               { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {    "tone":SEV_TONE[c.severity] ?? 'neutral',"label":sentenceCase(c.severity),}});}
               { svelteHTML.createElement("span", { "class":`finding-text`,});c.text; }
             }
          }
         }
       }
    }

    
    if(result?.summary){
       { svelteHTML.createElement("section", {});
         { svelteHTML.createElement("h3", {});  }
         { svelteHTML.createElement("p", { "class":`summary`,});result.summary; }
       }
    }

    
    if(cwd){
       { svelteHTML.createElement("section", {});
         { svelteHTML.createElement("h3", {});  }
         { svelteHTML.createElement("div", { "class":`path-row`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":13,}});}
           { svelteHTML.createElement("span", { "class":`path mono`,});cwd; }
           { svelteHTML.createElement("span", { "class":`grow`,}); }
           { svelteHTML.createElement("button", {     "class":`copy-btn`,"title":`Copy path`,"onclick":() => copy(cwd!, 'path'),});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});} 
           }
         }
       }
    }

    
     { svelteHTML.createElement("section", {});
       { svelteHTML.createElement("h3", {}); if(artifacts.length){ { svelteHTML.createElement("span", { "class":`count`,});artifacts.length; }} }
      if(artifacts.length === 0 && result == null){
         { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {            "what":`artifacts`,"variant":`compact`,"loading":!resultError,"error":resultError,"empty":false,"onretry":() => (resultAttempt += 1),}});}
      } else if (artifacts.length === 0){
         { svelteHTML.createElement("div", { "class":`muted small`,});   }
      }else{
         { svelteHTML.createElement("div", { "class":`arts`,});
              for(let a of __sveltets_2_ensureArray(artifacts)){let i = 1;i;
            const kind = artifactKind(a);
            const target = artifactTarget(a);
             { svelteHTML.createElement("button", {      "class":`art`,"disabled":!target,"onclick":() => openArtifact(a),});!!target;
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":kind === 'pr' ? 'pr' : kind === 'url' ? 'external' : 'file',"size":13,}});}
               { svelteHTML.createElement("span", { "class":`art-label`,});a.label || target || a.type; }
              if(target){ { svelteHTML.createElement("span", { "class":`art-target mono dim`,});target; }}
              if(target && isExternalUrl(target)){ { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":12,}});}} else if (target){ { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`link`,"size":12,}});}}
             }
          }
         }
      }
     }

    
    if(brief){
       { svelteHTML.createElement("section", {});
         { svelteHTML.createElement("div", { "class":`sec-h`,});
           { svelteHTML.createElement("h3", {});  }
           { svelteHTML.createElement("span", { "class":`grow`,}); }
           { svelteHTML.createElement("button", {     "class":`copy-btn`,"title":`Copy brief`,"onclick":() => copy(brief!, 'brief'),});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});} 
           }
         }
         { svelteHTML.createElement("pre", { "class":`block scrolly`,});brief; }
       }
    }

    
     { svelteHTML.createElement("section", {});
       { svelteHTML.createElement("h3", {});  if(posts.length){ { svelteHTML.createElement("span", { "class":`count`,});posts.length; }} }
      if(posts.length === 0){
         { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {      "icon":`comment`,"title":`No posts`,"body":`This run did not post to the team board.`,}});}
      }else{
         { svelteHTML.createElement("div", { "class":`posts`,});
             for(let m of __sveltets_2_ensureArray(posts)){m.id;
             { svelteHTML.createElement("div", { "class":`post`,});
               { svelteHTML.createElement("div", { "class":`post-h`,});
                 { const $$_egdaB6C = __sveltets_2_ensureComponent(Badge); new $$_egdaB6C({ target: __sveltets_2_any(), props: {    "variant":`outline`,"label":sentenceCase(m.kind),}});}
                 { svelteHTML.createElement("span", { "class":`who`,});author(m); }
                 { svelteHTML.createElement("span", { "class":`grow`,}); }
                 { svelteHTML.createElement("span", { "class":`dim time`,});fmtWhen(m.created_at); }
               }
               { svelteHTML.createElement("div", { "class":`post-body`,});m.body; }
             }
          }
         }
      }
     }

    
    if(rawJson){
       { svelteHTML.createElement("section", {});
         { svelteHTML.createElement("div", { "class":`sec-h`,});
           { svelteHTML.createElement("h3", {});  }
           { svelteHTML.createElement("span", { "class":`grow`,}); }
           { svelteHTML.createElement("button", {     "class":`copy-btn`,"title":`Copy JSON`,"onclick":() => copy(rawJson!, 'JSON'),});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});} 
           }
         }
         { svelteHTML.createElement("pre", { "class":`block json scrolly`,});rawJson; }
       }
    }
   }
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RunInspector__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RunInspector__SvelteComponent_ = ReturnType<typeof RunInspector__SvelteComponent_>;
/*Ωignore_endΩ*/export default RunInspector__SvelteComponent_;