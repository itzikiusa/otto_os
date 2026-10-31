///<reference types="svelte" />
;// Brand Kit → Voice / Imagery: a one-line summary and paired do / don't
// examples (≤ 20 each). Agents read these in every design turn's brief, so
// concrete examples beat adjectives.

import type { BrandDoc, BrandProse } from '../../../lib/api/types';
import Icon from '../../../lib/components/Icon.svelte';
;

  interface Props {
    doc: BrandDoc;
    group: 'voice' | 'imagery';
    readonly?: boolean;
  };function $$render() {

  
  
  
  
  

  let { doc = $bindable(), group, readonly = false }: Props = $props()/*Ωignore_startΩ*/;doc;/*Ωignore_endΩ*/;

  const COPY = {
    voice: {
      title: 'Voice',
      meta: 'How the brand sounds — agents write copy from these examples',
      summary: 'Warm, confident, never salesy.',
      doPh: 'Earn on every purchase, spend anywhere.',
      dontPh: 'Unlock INCREDIBLE savings NOW!!!',
    },
    imagery: {
      title: 'Imagery',
      meta: 'What pictures to use, and what to avoid',
      summary: 'Product renders and bold type over stock photography.',
      doPh: 'Product renders with soft light',
      dontPh: 'Stock photos of people shopping',
    },
  } as const;
  const c = $derived(COPY[group]);
  const prose = $derived<BrandProse>(doc[group] ?? {});
  const rows = $derived(Math.max(prose.do?.length ?? 0, prose.dont?.length ?? 0));

  function ensure(): BrandProse {
    if (!doc[group]) doc[group] = { summary: '', do: [], dont: [] };
    return doc[group]!;
  }

  function setSummary(v: string): void {
    ensure().summary = v;
  }

  function setLine(kind: 'do' | 'dont', i: number, v: string): void {
    const p = ensure();
    const list = [...(p[kind] ?? [])];
    while (list.length <= i) list.push('');
    list[i] = v;
    p[kind] = list;
  }

  function addPair(): void {
    const p = ensure();
    const n = rows;
    const d = [...(p.do ?? [])];
    const x = [...(p.dont ?? [])];
    while (d.length <= n) d.push('');
    while (x.length <= n) x.push('');
    p.do = d.slice(0, 20);
    p.dont = x.slice(0, 20);
  }

  function removePair(i: number): void {
    const p = ensure();
    p.do = (p.do ?? []).filter((_, j) => j !== i);
    p.dont = (p.dont ?? []).filter((_, j) => j !== i);
  }
;
async () => {

 { svelteHTML.createElement("section", {     "id":`brand-${group}`,"class":`bsec`,"aria-labelledby":`brand-${group}-h`,});
   { svelteHTML.createElement("div", { "class":`sec-h`,});
     { svelteHTML.createElement("h2", { "id":`brand-${group}-h`,});c.title; }
     { svelteHTML.createElement("span", { "class":`meta`,});c.meta; }
   }

   { svelteHTML.createElement("div", {   "class":`card`,...__sveltets_2_empty({"data-testid":`brand-${group}`}),});
    if(readonly){
       { svelteHTML.createElement("p", {  "class":`vq`,});group === 'voice';prose.summary || '—'; }
    }else{
       { svelteHTML.createElement("textarea", {                 "dir":`auto`,"class":`vq input`,"rows":group === 'voice' ? 1 : 2,"value":prose.summary ?? '',"oninput":(e) => setSummary(e.currentTarget.value),"placeholder":c.summary,"aria-label":`${c.title} summary`,"maxlength":2000,});group === 'voice'; }
    }

     { svelteHTML.createElement("div", {     "class":`dd`,"role":`table`,"aria-label":`${c.title} do and don’t examples`,});
       { svelteHTML.createElement("div", {   "class":`ddrow head`,"role":`row`,});
         { svelteHTML.createElement("span", { "role":`columnheader`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}  }
         { svelteHTML.createElement("span", { "role":`columnheader`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}  }
       }
          for(let _ of __sveltets_2_ensureArray(Array(rows))){let i = 1;i;
         { svelteHTML.createElement("div", {   "class":`ddrow`,"role":`row`,});
           { svelteHTML.createElement("span", {   "class":`cell do`,"role":`cell`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}
             { svelteHTML.createElement("input", {                "dir":`auto`,"class":`line`,"value":prose.do?.[i] ?? '',"oninput":(e) => setLine('do', i, e.currentTarget.value),"disabled":readonly,"placeholder":c.doPh,"aria-label":`Do example ${i + 1}`,"maxlength":2000,});}
           }
           { svelteHTML.createElement("span", {   "class":`cell dont`,"role":`cell`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
             { svelteHTML.createElement("input", {                "dir":`auto`,"class":`line`,"value":prose.dont?.[i] ?? '',"oninput":(e) => setLine('dont', i, e.currentTarget.value),"disabled":readonly,"placeholder":c.dontPh,"aria-label":`Don’t example ${i + 1}`,"maxlength":2000,});}
            if(!readonly){
               { svelteHTML.createElement("button", {       "class":`icon-btn`,"aria-label":`Remove example ${i + 1}`,"title":`Remove`,"onclick":() => removePair(i),}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});} }
            }
           }
         }
      }
     }
    if(!readonly && rows < 20){
       { svelteHTML.createElement("button", {   "class":`btn small add`,"onclick":addPair,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('doc'), slots: {}, events: {} }}
const ProseSection__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ProseSection__SvelteComponent_ = ReturnType<typeof ProseSection__SvelteComponent_>;
/*Ωignore_endΩ*/export default ProseSection__SvelteComponent_;