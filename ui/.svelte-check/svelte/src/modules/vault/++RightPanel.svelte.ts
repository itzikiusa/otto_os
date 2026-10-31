///<reference types="svelte" />
;// Right panel: Backlinks (linked mentions) · Outgoing links · Outline ·
// Properties (frontmatter) · OKF validation card (OKF vaults only).

import { scrollBehavior } from '../../lib/motion';
import { confirmer } from '../../lib/confirm.svelte';
import { vault } from './vault.svelte';
import PropertiesEditor from './PropertiesEditor.svelte';
import KnowledgeMetadata from './KnowledgeMetadata.svelte';
import { slugifyHeading } from '../../lib/render/markdown';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { plural } from '../../lib/plural';
import Badge from '../../lib/components/Badge.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  function openOrCreateOutgoing(dstPath: string | null, rawTarget: string): void {
    if (dstPath) {
      if (/\.md$/i.test(dstPath)) void vault.open(dstPath);
      return;
    }
    const raw = rawTarget.split('#')[0]?.trim() ?? '';
    if (!raw) return;
    const p = raw.endsWith('.md') ? raw : `${raw}.md`;
    void confirmer.ask(`Create “${p}”?`, { title: 'Create note', confirmLabel: 'Create', danger: false }).then((ok) => {
      if (ok) void vault.createNote(p, `# ${raw}\n\n`);
    });
  }

  let open = $state({ backlinks: true, outgoing: true, outline: false, props: false, okf: false });

  // V7: a hub note can have thousands of backlinks / outgoing links; mount a
  // page per list and grow on demand (reset when another note opens).
  const LIST_PAGE = 100;
  let shownOutgoing = $state(LIST_PAGE);
  $effect(() => {
    void vault.notePath;
    shownOutgoing = LIST_PAGE;
  });

  async function showMoreBacklinks(): Promise<void> {
    await vault.loadBacklinkContexts(vault.visibleBacklinks);
  }

  const props = $derived.by(() => {
    const fm = vault.note?.meta.frontmatter;
    if (!fm || typeof fm !== 'object' || Array.isArray(fm)) return [] as [string, string][];
    return Object.entries(fm as Record<string, unknown>).map(
      ([k, v]) => [k, typeof v === 'string' ? v : JSON.stringify(v)] as [string, string],
    );
  });

  const OKF_FIELDS = new Set(['type', 'title', 'description', 'resource', 'tags', 'timestamp', 'generated', 'verified', 'sources', 'usage_window', 'status', 'stale_after', 'computation', 'executor', 'attester']);

  function jumpToHeading(text: string): void {
    document
      .querySelector(`.read #h-${CSS.escape(slugifyHeading(text))}`)
      ?.scrollIntoView({ behavior: scrollBehavior(), block: 'start' });
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`right`,});
  if(vault.current?.okf && vault.note && !vault.note.meta.reserved){
     { svelteHTML.createElement("section", {}); { svelteHTML.createElement("h3", { "class":`hdr`,});  } { const $$_atadateMegdelwonK2C = __sveltets_2_ensureComponent(KnowledgeMetadata); new $$_atadateMegdelwonK2C({ target: __sveltets_2_any(), props: {  "frontmatter":vault.note.meta.frontmatter,}});} }
  }
   { svelteHTML.createElement("section", {});
     { svelteHTML.createElement("button", {     "class":`hdr`,"aria-expanded":open.backlinks,"onclick":() => (open.backlinks = !open.backlinks),});
       { svelteHTML.createElement("span", {  "class":`tri`,});open.backlinks; { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"noflip":true,"size":12,}});} }
      
       { svelteHTML.createElement("span", { "class":`hdr-count`,}); { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {  "label":String(vault.backlinks.length),}});} }
     }
    if(open.backlinks){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"what":`backlinks`,"variant":`compact`,"loading":vault.backlinksLoading,"error":vault.backlinksError || null,"empty":vault.backlinks.length === 0,"onretry":() => void vault.reloadBacklinks(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("div", { "class":`none`,});   }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
        
           for(let bl of __sveltets_2_ensureArray((vault.backlinks.slice(0, vault.visibleBacklinks)))){bl.path + bl.kind;
           { svelteHTML.createElement("button", {     "class":`item`,"title":bl.path,"onclick":() => void vault.open(bl.path),});
             { svelteHTML.createElement("div", { "class":`t`,});bl.title; }
            if(bl.context){ { svelteHTML.createElement("div", { "class":`ctx`,});bl.context; }}
           }
        }
        if(vault.backlinks.length > vault.visibleBacklinks){
           { svelteHTML.createElement("button", {     "class":`more`,"disabled":vault.loadingBacklinkContexts,"onclick":() => void showMoreBacklinks(),});  vault.backlinks.length - vault.visibleBacklinks;  }
        }
       LoadState}
    }
   }

   { svelteHTML.createElement("section", {});
     { svelteHTML.createElement("button", {     "class":`hdr`,"aria-expanded":open.outgoing,"onclick":() => (open.outgoing = !open.outgoing),});
       { svelteHTML.createElement("span", {  "class":`tri`,});open.outgoing; { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"noflip":true,"size":12,}});} }
       
       { svelteHTML.createElement("span", { "class":`hdr-count`,}); { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {  "label":String(vault.note?.outgoing.length ?? 0),}});} }
     }
    if(open.outgoing){
          for(let l of __sveltets_2_ensureArray(((vault.note?.outgoing ?? []).slice(0, shownOutgoing)))){let i = 1;i;
         { svelteHTML.createElement("button", {          "class":`item`,"title":l.dst_path ?? `${l.raw_target} (missing note — click to create)`,"disabled":Boolean(l.dst_path && !/\.md$/i.test(l.dst_path)),"onclick":() => openOrCreateOutgoing(l.dst_path, l.raw_target),});!l.dst_path;
           { svelteHTML.createElement("div", { "class":`t`,});
            if(l.kind === 'embed'){ { svelteHTML.createElement("span", {   "class":`embed-ic`,"title":`Embedded`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`image`,"size":12,}});} }}l.alias ?? l.raw_target;
            if(!l.dst_path){ { svelteHTML.createElement("span", { "class":`ghost`,});  }}
           }
         }
      }
      if((vault.note?.outgoing.length ?? 0) > shownOutgoing){
         { svelteHTML.createElement("button", {   "class":`more`,"onclick":() => (shownOutgoing += LIST_PAGE * 5),});  (vault.note?.outgoing.length ?? 0) - shownOutgoing;  }
      }
    }
   }

   { svelteHTML.createElement("section", {});
     { svelteHTML.createElement("button", {     "class":`hdr`,"aria-expanded":open.outline,"onclick":() => (open.outline = !open.outline),});
       { svelteHTML.createElement("span", {  "class":`tri`,});open.outline; { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"noflip":true,"size":12,}});} }
      
       { svelteHTML.createElement("span", { "class":`hdr-count`,}); { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {  "label":String(vault.note?.meta.headings.length ?? 0),}});} }
     }
    if(open.outline){
          for(let h of __sveltets_2_ensureArray(vault.note?.meta.headings ?? [])){let i = 1;i;
         { svelteHTML.createElement("button", {         "class":`item outline`,"style":`padding-inline-start: ${10 + (h.level - 1) * 12}px`,"title":h.text,"onclick":() => jumpToHeading(h.text),});
           { svelteHTML.createElement("div", { "class":`t`,});h.text; }
         }
      }
    }
   }

   { svelteHTML.createElement("section", {});
     { svelteHTML.createElement("button", {     "class":`hdr`,"aria-expanded":open.props,"onclick":() => (open.props = !open.props),});
       { svelteHTML.createElement("span", {  "class":`tri`,});open.props; { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"noflip":true,"size":12,}});} }
      
       { svelteHTML.createElement("span", { "class":`hdr-count`,}); { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {  "label":String(props.length),}});} }
     }
    if(open.props){
      if(vault.note?.meta.content_index_status === 'size_limited'){
         { svelteHTML.createElement("div", { "class":`none warn`,});                 }
      }
      if(vault.note?.meta.parse_error){
         { svelteHTML.createElement("div", { "class":`none warn`,});     }
      }
      vault.notePath; { { const $$_rotidEseitreporP2C = __sveltets_2_ensureComponent(PropertiesEditor); new $$_rotidEseitreporP2C({ target: __sveltets_2_any(), props: {}});}}
       { svelteHTML.createElement("table", { "class":`props`,});
         { svelteHTML.createElement("tbody", {});
             for(let [k, v] of __sveltets_2_ensureArray(props)){k;
             { svelteHTML.createElement("tr", { });vault.current?.okf && OKF_FIELDS.has(k);
               { svelteHTML.createElement("td", { "class":`k`,});k; }
               { svelteHTML.createElement("td", { "class":`v`,});v; }
             }
          }
         }
       }
    }
   }

  if(vault.current?.okf){
     { svelteHTML.createElement("section", {});
       { svelteHTML.createElement("button", {     "class":`hdr`,"aria-expanded":open.okf,"onclick":() => (open.okf = !open.okf),});
         { svelteHTML.createElement("span", {  "class":`tri`,});open.okf; { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"noflip":true,"size":12,}});} }
        
        if(vault.okfReport){
           { svelteHTML.createElement("span", { "class":`hdr-count`,});
            if(vault.okfReport.conformant){
               { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },"tone":`ok`,"title":`Conformant`,}}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});} Badge}
            }else{
               { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {      "tone":`bad`,"label":String(vault.okfReport.errors.length),"title":plural(vault.okfReport.errors.length, 'error'),}});}
            }
           }
        }
       }
      if(open.okf){
         { svelteHTML.createElement("div", { "class":`okf-actions`,});
           { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":vault.okfBusy,"onclick":() => void vault.validateOkf(),});
            vault.okfBusy ? 'Working…' : 'Validate';
           }
           { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":vault.okfBusy,"onclick":() => void vault.generateIndexes(),"title":`Write the index.md files OKF expects in each folder`,});
             
           }
         }
        if(vault.okfReport){
          if(vault.okfReport.conformant){
             { svelteHTML.createElement("div", { "class":`none ok`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}   vault.okfReport.checked_notes;  }
          }
              for(let f of __sveltets_2_ensureArray(vault.okfReport.errors)){let i = 1;i;
             { svelteHTML.createElement("button", {   "class":`item finding err`,"onclick":() => f.path.endsWith('.md') && void vault.open(f.path),});
               { svelteHTML.createElement("div", { "class":`t`,}); { svelteHTML.createElement("b", {});f.rule; } f.path; }
               { svelteHTML.createElement("div", { "class":`ctx`,});f.message; }
             }
          }
              for(let f of __sveltets_2_ensureArray((vault.okfReport.warnings.slice(0, 50)))){let i = 1;i;
             { svelteHTML.createElement("button", {   "class":`item finding warn`,"onclick":() => f.path.endsWith('.md') && void vault.open(f.path),});
               { svelteHTML.createElement("div", { "class":`t`,}); { svelteHTML.createElement("b", {});f.rule; } f.path; }
               { svelteHTML.createElement("div", { "class":`ctx`,});f.message; }
             }
          }
          if(vault.okfReport.warnings.length > 50){
             { svelteHTML.createElement("div", { "class":`none`,}); vault.okfReport.warnings.length - 50;   }
          }
        }
      }
     }
  }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RightPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RightPanel__SvelteComponent_ = ReturnType<typeof RightPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default RightPanel__SvelteComponent_;