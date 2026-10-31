///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { plural } from '../../lib/plural';
// DLQ/Replay panel. Lets an operator re-publish selected messages from a
// source topic to a target topic, with optional key/header transforms.
// Evidence is recorded server-side (broker_replays table).
import { api } from '../../lib/api/client';
import { toastError } from '../../lib/toastError';
import { toasts } from '../../lib/toast.svelte';
import { confirmProd } from '../../lib/confirmProd';
import type { BrokerCluster } from '../../lib/api/types';
import type { ReplayResp, ReplaySelector } from './types';
;

  interface Props {
    cluster: BrokerCluster;
  };function $$render() {

  
  
  
  
  

  
  
  
  
  
  

  let { cluster }: Props = $props();

  const guarded = $derived(cluster.read_only || cluster.environment === 'prod');

  let sourceTopic = $state('');
  let targetTopic = $state('');
  let selectorType = $state<'latest' | 'offset_range' | 'timestamp'>('latest');
  let count = $state(10);
  let partition = $state(0);
  let fromOffset = $state(0);
  let toOffset = $state(0);
  let timestampMs = $state('');
  let tsLimit = $state(50);
  // Optional transform.
  let setKey = $state('');
  let addHeaderKey = $state('');
  let addHeaderVal = $state('');

  let running = $state(false);
  let result = $state<ReplayResp | null>(null);

  // Why Replay is disabled, in words (a disabled button must say why).
  function validInteger(value: number, min: number, max = Number.MAX_SAFE_INTEGER): boolean {
    return Number.isSafeInteger(value) && value >= min && value <= max;
  }
  const blockReason = $derived.by(() => {
    if (!sourceTopic.trim() || !targetTopic.trim()) return 'Enter a source and a target topic first';
    if (sourceTopic.trim() === targetTopic.trim()) return 'Source and target must be different topics';
    if (selectorType === 'latest' && !validInteger(count, 1, 5000)) return 'Count must be a whole number from 1 to 5000';
    if (selectorType === 'timestamp') {
      if (!timestampMs || !Number.isFinite(new Date(timestampMs).getTime())) return 'Choose a valid start time';
      if (!validInteger(tsLimit, 1, 5000)) return 'Limit must be a whole number from 1 to 5000';
    }
    if (selectorType === 'offset_range') {
      if (![partition, fromOffset, toOffset].every((n) => validInteger(n, 0))) return 'Partition and offsets must be non-negative whole numbers';
      if (toOffset < fromOffset) return '"To offset" must be at or after "From offset"';
      if (toOffset - fromOffset >= 5000) return 'Select at most 5000 offsets per replay';
    }
    return null;
  });

  function buildSelector(): ReplaySelector {
    if (selectorType === 'offset_range') {
      return { type: 'offset_range', partition, from: fromOffset, to: toOffset };
    }
    if (selectorType === 'timestamp') {
      return {
        type: 'timestamp',
        timestamp_ms: new Date(timestampMs).getTime(),
        limit: tsLimit,
      };
    }
    return { type: 'latest', count };
  }

  async function runReplay() {
    if (running || blockReason) {
      if (blockReason) toasts.error('Can’t replay yet', blockReason);
      return;
    }

    const body: Record<string, unknown> = {
      source_topic: sourceTopic.trim(),
      target_topic: targetTopic.trim(),
      selector: buildSelector(),
      confirm: guarded,
    };
    if (setKey.trim() || (addHeaderKey.trim() && addHeaderVal.trim())) {
      const transform: Record<string, unknown> = {};
      if (setKey.trim()) transform['set_key'] = setKey.trim();
      if (addHeaderKey.trim() && addHeaderVal.trim())
        transform['add_header'] = [addHeaderKey.trim(), addHeaderVal.trim()];
      body['transform'] = transform;
    }

    const clusterId = cluster.id;
    const prior = result;
    running = true;
    try {
      const ok = await confirmProd({
        env: cluster.environment,
        verb: 'Replay',
        title: cluster.environment === 'prod' ? 'Replay on production?' : 'Replay messages?',
        where: `${cluster.name} · ${body.source_topic} → ${body.target_topic}${cluster.read_only ? ' (read-only cluster)' : ''}`,
        what: prior?.error
          ? `${plural(prior.count, 'message')} already acknowledged in the previous replay. Retrying can duplicate messages, including a failed send whose outcome is unknown.`
          : 'Produces the selected messages to the target topic.',
        danger: guarded || !!prior?.error,
      });
      if (!ok) return;

      result = await api.post<ReplayResp>(`/brokers/clusters/${clusterId}/replay`, body);
      if (result.error) {
        toasts.warn('Replay stopped', result.error);
      } else {
        toasts.success('Replay complete', `${plural(result.count, 'message')} replayed to "${result.target_topic}"`);
      }
    } catch (e) {
      toastError('Couldn’t replay messages', e);
    } finally {
      running = false;
    }
  }
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  $effect(() => { if (result?.error) announce(result.error, { assertive: true }); });
;
async () => {

 { svelteHTML.createElement("div", { "class":`replay`,});
   { svelteHTML.createElement("h5", {});   }
   { svelteHTML.createElement("p", { "class":`muted small`,});
                 
                      
   }

   { svelteHTML.createElement("div", { "class":`form`,});
     { svelteHTML.createElement("label", {});
       
       { svelteHTML.createElement("input", {        "dir":`ltr`,"type":`text`,"bind:value":sourceTopic,"placeholder":`e.g. orders-dlq`,});/*Ωignore_startΩ*/() => sourceTopic = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
     { svelteHTML.createElement("label", {});
       
       { svelteHTML.createElement("input", {        "dir":`ltr`,"type":`text`,"bind:value":targetTopic,"placeholder":`e.g. orders`,});/*Ωignore_startΩ*/() => targetTopic = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }

     { svelteHTML.createElement("label", {});
      
       { svelteHTML.createElement("select", { "bind:value":selectorType,});/*Ωignore_startΩ*/() => selectorType = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", { "value":`latest`,});   }
         { svelteHTML.createElement("option", { "value":`offset_range`,});    }
         { svelteHTML.createElement("option", { "value":`timestamp`,});  }
       }
     }

    if(selectorType === 'latest'){
       { svelteHTML.createElement("label", {});
        
         { svelteHTML.createElement("input", {          "type":`number`,"bind:value":count,"min":`1`,"max":`5000`,"class":`narrow`,});/*Ωignore_startΩ*/() => count = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
    } else if (selectorType === 'offset_range'){
       { svelteHTML.createElement("div", { "class":`inline-row`,});
         { svelteHTML.createElement("label", {});
          
           { svelteHTML.createElement("input", {        "type":`number`,"bind:value":partition,"min":`0`,"class":`narrow`,});/*Ωignore_startΩ*/() => partition = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("label", {});
           
           { svelteHTML.createElement("input", {        "type":`number`,"bind:value":fromOffset,"min":`0`,"class":`narrow`,});/*Ωignore_startΩ*/() => fromOffset = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("label", {});
           
           { svelteHTML.createElement("input", {        "type":`number`,"bind:value":toOffset,"min":`0`,"class":`narrow`,});/*Ωignore_startΩ*/() => toOffset = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
       }
    }else{
       { svelteHTML.createElement("div", { "class":`inline-row`,});
         { svelteHTML.createElement("label", {});
            
           { svelteHTML.createElement("input", {    "type":`datetime-local`,"bind:value":timestampMs,});/*Ωignore_startΩ*/() => timestampMs = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("label", {});
          
           { svelteHTML.createElement("input", {          "type":`number`,"bind:value":tsLimit,"min":`1`,"max":`5000`,"class":`narrow`,});/*Ωignore_startΩ*/() => tsLimit = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
       }
    }

     { svelteHTML.createElement("details", { "class":`transform-details`,});
       { svelteHTML.createElement("summary", { "class":`small muted`,});  }
       { svelteHTML.createElement("div", { "class":`transform-body`,});
         { svelteHTML.createElement("label", {});
           
           { svelteHTML.createElement("input", {        "dir":`auto`,"type":`text`,"bind:value":setKey,"placeholder":`leave blank to keep original`,});/*Ωignore_startΩ*/() => setKey = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("div", { "class":`inline-row`,});
           { svelteHTML.createElement("label", {});
              
             { svelteHTML.createElement("input", {        "dir":`ltr`,"type":`text`,"bind:value":addHeaderKey,"placeholder":`x-replayed-from`,});/*Ωignore_startΩ*/() => addHeaderKey = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
           { svelteHTML.createElement("label", {});
            
             { svelteHTML.createElement("input", {        "dir":`ltr`,"type":`text`,"bind:value":addHeaderVal,"placeholder":`source-topic-name`,});/*Ωignore_startΩ*/() => addHeaderVal = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
         }
       }
     }

     { svelteHTML.createElement("button", {         "class":`btn primary`,"onclick":runReplay,"disabled":running || blockReason !== null,"title":blockReason ?? 'Re-publish the selected messages to the target topic',});
      running ? 'Replaying…' : 'Replay';
     }
    if(blockReason && sourceTopic.trim() && targetTopic.trim()){
       { svelteHTML.createElement("p", { "class":`field-err`,});blockReason; }
    }
   }

  
  if(result){
     { svelteHTML.createElement("div", { "class":`evidence`,});
      if(result.error){ { svelteHTML.createElement("p", { "class":`field-err`,});result.error; }}
       { svelteHTML.createElement("h5", {});result.evidence_saved === false ? 'Unsaved evidence' : 'Evidence';   result.replay_id.slice(0, 8);  }
       { svelteHTML.createElement("p", { "class":`muted small`,});
        plural(result.count, 'message'); 
          { svelteHTML.createElement("code", {});result.source_topic; }   { svelteHTML.createElement("code", {});result.target_topic; }
       }
       { svelteHTML.createElement("table", {});
         { svelteHTML.createElement("thead", {});
           { svelteHTML.createElement("tr", {});
             { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  }
             { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  }
           }
         }
         { svelteHTML.createElement("tbody", {});
             for(let e of __sveltets_2_ensureArray(result.evidence)){e.partition + '-' + e.offset;
             { svelteHTML.createElement("tr", {});
               { svelteHTML.createElement("td", {});e.partition; }
               { svelteHTML.createElement("td", { "class":`mono`,});e.offset; }
               { svelteHTML.createElement("td", { "class":`mono muted`,});e.key_preview ?? '—'; }
               { svelteHTML.createElement("td", {});e.target_partition; }
               { svelteHTML.createElement("td", { "class":`mono`,});e.target_offset; }
             }
          }
         }
       }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ReplayPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ReplayPanel__SvelteComponent_ = ReturnType<typeof ReplayPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default ReplayPanel__SvelteComponent_;