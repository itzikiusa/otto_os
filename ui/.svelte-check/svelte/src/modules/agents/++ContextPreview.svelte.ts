///<reference types="svelte" />
;
import { formatBytes } from '../../lib/metric-format';
import { plural } from '../../lib/plural';
import Badge from '../../lib/components/Badge.svelte';
import { sentenceCase } from '../../lib/labels';
// Dry-run preview of what a session spawn would materialize for a workspace
// + provider — the exact skill files, soul, generated AGENTS.md / CLAUDE.md
// content and runtime hooks — WITHOUT spawning a session or touching disk.
//
// Crucially it labels each artifact advisory vs enforced:
//   • advisory — instruction files / skills: guidance the model MAY ignore.
//   • enforced — hooks / settings the runtime imposes regardless.
// See docs/contracts/api.md (POST /workspaces/{id}/context/preview).
import Icon from '../../lib/components/Icon.svelte';
import { toastError } from '../../lib/toastError';
import { onDestroy } from 'svelte';
import { contextApi } from '../../lib/api/context';
import type {
    ContextPreviewProvider,
    ContextPreviewReq,
  } from '../../lib/api/types';
;

  interface Props {
    /** Workspace to preview. */
    wsId: string;
    /** Provider to preview (claude / codex). Others materialize nothing. */
    provider: string;
    /** Optional overrides so a not-yet-saved selection can be previewed. */
    overrides?: Omit<ContextPreviewReq, 'provider'>;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { wsId, provider, overrides = {} }: Props = $props();

  let result: ContextPreviewProvider | null = $state(null);
  let loading = $state(false);
  const requestKey = $derived(JSON.stringify([wsId, provider, overrides]));
  let generation = 0;
  onDestroy(() => { generation++; });
  // Which artifact's full content is expanded (path), if any.
  let openFile: string | null = $state(null);

  async function run(): Promise<void> {
    if (!wsId || loading) return;
    const key = requestKey; const request = ++generation; const selectedProvider = provider;
    loading = true;
    try {
      const resp = await contextApi.preview(wsId, { provider: selectedProvider, ...overrides });
      if (request !== generation || requestKey !== key) return;
      result = resp.providers.find((p) => p.provider === selectedProvider) ?? resp.providers[0] ?? null;
      openFile = null;
    } catch (e) {
      if (request === generation && requestKey === key) toastError('Couldn’t preview the context', e);
    } finally {
      if (request === generation && requestKey === key) loading = false;
    }
  }

  // A preview belongs to the exact workspace, provider and draft that requested
  // it. Changing any input invalidates both displayed and in-flight results.
  $effect(() => {
    void requestKey;
    generation++;
    result = null; loading = false; openFile = null;
  });

  // Last path segment, for a compact file label (the absolute path is in title).
  function leaf(path: string): string {
    const parts = path.split('/');
    return parts[parts.length - 1] || path;
  }

  const kindLabels: Record<string, string> = {
    instructions: 'Instructions',
    skill: 'Skill',
    skill_asset: 'Skill asset',
    hooks: 'Hooks',
    manifest: 'Manifest',
  };

  function toggleFile(path: string): void {
    openFile = openFile === path ? null : path;
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`preview`,});
   { svelteHTML.createElement("div", { "class":`head`,});
     { svelteHTML.createElement("button", {     "class":`btn`,"disabled":loading,"onclick":run,});
      loading ? 'Previewing…' : result ? 'Refresh preview' : 'Preview context';
     }
     { svelteHTML.createElement("span", { "class":`hint`,});
          { svelteHTML.createElement("code", {});provider; }         
     }
   }

  if(result){
    if(result.skipped){
       { svelteHTML.createElement("div", { "class":`skipped dim`,});
         { svelteHTML.createElement("code", {});provider; }        
          
       }
    }else{
      
       { svelteHTML.createElement("div", { "class":`legend`,});
         { const $$_egdaB2C = __sveltets_2_ensureComponent(Badge); new $$_egdaB2C({ target: __sveltets_2_any(), props: {  "label":`Advisory`,}});}
         { svelteHTML.createElement("span", { "class":`legend-text`,});         }
         { const $$_egdaB2C = __sveltets_2_ensureComponent(Badge); new $$_egdaB2C({ target: __sveltets_2_any(), props: {    "tone":`accent`,"label":`Enforced`,}});}
         { svelteHTML.createElement("span", { "class":`legend-text`,});        }
       }

      
       { svelteHTML.createElement("div", { "class":`summary`,});
         { svelteHTML.createElement("div", { "class":`summary-row`,});
           { svelteHTML.createElement("span", { "class":`summary-lbl`,});  }
          if(result.soul){
             { const $$_egdaB3C = __sveltets_2_ensureComponent(Badge); new $$_egdaB3C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}}); { svelteHTML.createElement("span", { "class":`mono`,});result.soul; } Badge}
          }else{
             { svelteHTML.createElement("span", { "class":`dim`,});  }
          }
           { const $$_egdaB3C = __sveltets_2_ensureComponent(Badge); new $$_egdaB3C({ target: __sveltets_2_any(), props: {    "label":`Advisory`,"title":`The model reads the soul but may ignore it`,}});}
         }
         { svelteHTML.createElement("div", { "class":`summary-row`,});
           { svelteHTML.createElement("span", { "class":`summary-lbl`,});  }
          if(result.skills.length === 0){
             { svelteHTML.createElement("span", { "class":`dim`,});  }
          }else{
             { svelteHTML.createElement("span", { "class":`chips`,});
                 for(let s of __sveltets_2_ensureArray(result.skills)){s.name;
                 { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"title":s.description,}}); { svelteHTML.createElement("span", { "class":`mono`,});s.name; } { svelteHTML.createElement("span", { "class":`ver`,}); s.version; } Badge}
              }
             }
          }
           { const $$_egdaB3C = __sveltets_2_ensureComponent(Badge); new $$_egdaB3C({ target: __sveltets_2_any(), props: {  "label":`Advisory`,}});}
         }
       }

      
       { svelteHTML.createElement("div", { "class":`files-head`,});
         { svelteHTML.createElement("span", { "class":`files-title`,});plural(result.files.length, 'file'); }
       }
       { svelteHTML.createElement("ul", { "class":`files`,});
           for(let f of __sveltets_2_ensureArray(result.files)){f.path;
           { svelteHTML.createElement("li", {  "class":`file`,});f.enforcement === 'enforced';
             { svelteHTML.createElement("button", {         "class":`file-head`,"type":`button`,"onclick":() => toggleFile(f.path),"aria-expanded":openFile === f.path,});
               { svelteHTML.createElement("span", { "class":`file-kind kind-${f.kind}`,});kindLabels[f.kind] ?? f.kind; }
               { svelteHTML.createElement("span", {   "class":`file-name mono`,"title":f.path,});leaf(f.path); }
               { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {    "tone":f.enforcement === 'enforced' ? 'accent' : 'neutral',"label":sentenceCase(f.enforcement),}});}
               { svelteHTML.createElement("span", { "class":`file-size dim`,});formatBytes(f.size); }
               { svelteHTML.createElement("span", {  "class":`chevron`,});openFile === f.path; { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"noflip":true,"size":12,}});} }
             }
            if(openFile === f.path){
               { svelteHTML.createElement("pre", { "class":`file-body mono`,});f.first_lines;f.truncated ? '\n…' : ''; }
            }
           }
        }
       }

      
      if(result.instructions_file_name){
         { svelteHTML.createElement("details", { "class":`generated`,});
           { svelteHTML.createElement("summary", {});
             result.instructions_file_name;
             { const $$_egdaB3C = __sveltets_2_ensureComponent(Badge); new $$_egdaB3C({ target: __sveltets_2_any(), props: {  "label":`Advisory`,}});}
           }
           { svelteHTML.createElement("pre", { "class":`mono`,});result.generated_instructions; }
         }
      }

      
      if(result.generated_hooks){
         { svelteHTML.createElement("details", { "class":`generated`,});
           { svelteHTML.createElement("summary", {});
               
             { const $$_egdaB3C = __sveltets_2_ensureComponent(Badge); new $$_egdaB3C({ target: __sveltets_2_any(), props: {    "tone":`accent`,"label":`Enforced`,}});}
           }
           { svelteHTML.createElement("pre", { "class":`mono`,});result.generated_hooks; }
         }
      }
    }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ContextPreview__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ContextPreview__SvelteComponent_ = ReturnType<typeof ContextPreview__SvelteComponent_>;
/*Ωignore_endΩ*/export default ContextPreview__SvelteComponent_;