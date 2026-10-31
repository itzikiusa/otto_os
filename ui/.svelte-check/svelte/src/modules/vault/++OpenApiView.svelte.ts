///<reference types="svelte" />
;// Compact structured OpenAPI viewer (v2/v3, json or yaml source) — info
// header, servers, and operations grouped by tag with expandable details:
// parameters, request body, responses, and a depth-bounded schema tree with
// $ref resolution. Deliberately dependency-free (no swagger-ui) so it stays
// light and matches the app's look.
/* eslint-disable @typescript-eslint/no-explicit-any */

import EmptyState from '../../lib/components/EmptyState.svelte';
;

  interface Op {
    method: string;
    path: string;
    summary: string;
    deprecated: boolean;
    op: any;
    /** Owning path item — carries shared (path-level) parameters. */
    item: any;
  };
;type $$ComponentProps =  { spec: Record<string, any> };function $$render() {
 const schemaTree/*Ωignore_positionΩ*/ = (raw: any, depth: number, seen: Set<string>)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  const r = deref(raw, seen);
  if(r.schema && typeof r.schema === 'object' && depth <= 5){
    const props = (r.schema.properties ?? {}) as Record<string, any>;
    const required = new Set((r.schema.required ?? []) as string[]);
    if(Object.keys(props).length > 0){
       { svelteHTML.createElement("ul", { "class":`schema`,});
           for(let [k, v] of __sveltets_2_ensureArray(Object.entries(props))){k;
          const child = deref(v, new Set(seen));
           { svelteHTML.createElement("li", {});
             { svelteHTML.createElement("span", {  "class":`pname`,});required.has(k);k; }
             { svelteHTML.createElement("span", { "class":`ptype`,});typeLabel(v); }
            if(v?.description){ { svelteHTML.createElement("span", { "class":`pdesc`,}); v.description; }}
            if(child.schema?.properties || v?.items){
              ;__sveltets_2_ensureSnippet(schemaTree(v?.items ?? v, depth + 1, new Set(seen)));
            }
           }
        }
       }
    } else if (r.schema.items){
      ;__sveltets_2_ensureSnippet(schemaTree(r.schema.items, depth + 1, new Set(seen)));
    } else if (r.name){
       { svelteHTML.createElement("span", { "class":`ptype`,});r.name; }
    }
  } else if (r.name){
     { svelteHTML.createElement("span", { "class":`ptype`,});r.name; }
  }
};return __sveltets_2_any(0)};
  
  
  
  
  

  
  

  let { spec }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  // Operation bodies (parameter tables, request/response schema trees up to 5
  // deep) are built only for the operations the user has OPENED. A 300–500
  // operation spec used to build and style every body inside its collapsed
  // `<details>` up front — 20k–100k+ nodes (r3-03-04).
  let openOps = $state(new Set<string>());
  function toggleOp(key: string, e: Event): void {
    const open = (e.currentTarget as HTMLDetailsElement).open;
    if (open === openOps.has(key)) return;
    const next = new Set(openOps);
    if (open) next.add(key);
    else next.delete(key);
    openOps = next;
  }

  const METHODS = ['get', 'post', 'put', 'patch', 'delete', 'options', 'head'] as const;


  const info = $derived((spec.info ?? {}) as Record<string, any>);
  const servers = $derived.by(() => {
    if (Array.isArray(spec.servers)) return spec.servers.map((s: any) => String(s?.url ?? ''));
    if (spec.host) return [`${(spec.schemes?.[0] as string) ?? 'https'}://${spec.host}${spec.basePath ?? ''}`];
    return [];
  });

  /** tag → operations (spec order preserved; untagged land in 'default'). */
  const groups = $derived.by(() => {
    const out = new Map<string, Op[]>();
    const paths = (spec.paths ?? {}) as Record<string, any>;
    for (const [p, item] of Object.entries(paths)) {
      if (!item || typeof item !== 'object') continue;
      for (const m of METHODS) {
        const op = item[m];
        if (!op || typeof op !== 'object') continue;
        const tag = Array.isArray(op.tags) && op.tags.length ? String(op.tags[0]) : 'default';
        if (!out.has(tag)) out.set(tag, []);
        out.get(tag)!.push({
          method: m,
          path: p,
          summary: String(op.summary ?? op.operationId ?? ''),
          deprecated: !!op.deprecated,
          op,
          item,
        });
      }
    }
    return out;
  });

  /** Resolve a local $ref against components.schemas / definitions. */
  function deref(s: any, seen: Set<string>): { schema: any; name: string | null } {
    if (s && typeof s === 'object' && typeof s.$ref === 'string') {
      const name = s.$ref.split('/').pop() ?? '';
      if (seen.has(s.$ref)) return { schema: null, name }; // cycle — show the name only
      const target = spec.components?.schemas?.[name] ?? spec.definitions?.[name];
      if (target) {
        seen.add(s.$ref);
        return { schema: target, name };
      }
      return { schema: null, name };
    }
    return { schema: s, name: null };
  }

  function typeLabel(s: any): string {
    if (!s || typeof s !== 'object') return '';
    if (s.$ref) return String(s.$ref).split('/').pop() ?? 'object';
    let t = String(s.type ?? (s.properties ? 'object' : s.items ? 'array' : ''));
    if (t === 'array' && s.items) t = `${typeLabel(s.items)}[]`;
    if (s.format) t += ` (${s.format})`;
    if (Array.isArray(s.enum)) t += ` ∈ {${s.enum.slice(0, 6).join(', ')}${s.enum.length > 6 ? ', …' : ''}}`;
    return t;
  }

  function bodySchemas(op: any): { mime: string; schema: any }[] {
    const out: { mime: string; schema: any }[] = [];
    const content = op.requestBody?.content;
    if (content && typeof content === 'object') {
      for (const [mime, c] of Object.entries(content as Record<string, any>)) {
        out.push({ mime, schema: c?.schema });
      }
    }
    // swagger 2: body parameter
    for (const raw of op.parameters ?? []) {
      const p = derefParam(raw);
      if (p?.in === 'body' && p.schema) out.push({ mime: 'body', schema: p.schema });
    }
    return out;
  }

  function responses(op: any): { code: string; desc: string; schema: any }[] {
    const out: { code: string; desc: string; schema: any }[] = [];
    for (const [code, r] of Object.entries((op.responses ?? {}) as Record<string, any>)) {
      let schema = r?.schema ?? null; // swagger 2
      const content = r?.content;
      if (!schema && content && typeof content === 'object') {
        const first = Object.values(content as Record<string, any>)[0];
        schema = first?.schema ?? null;
      }
      out.push({ code, desc: String(r?.description ?? ''), schema });
    }
    return out;
  }

  /** Resolve a parameter-level $ref (components.parameters / swagger-2 root
   *  `parameters`) — schema deref alone left ref'd params as blank rows with
   *  the ref tail leaking into the type column. */
  function derefParam(p: any): any {
    if (p && typeof p === 'object' && typeof p.$ref === 'string') {
      const name = p.$ref.split('/').pop() ?? '';
      return spec.components?.parameters?.[name] ?? spec.parameters?.[name] ?? p;
    }
    return p;
  }

  /** Operation + path-item level parameters, $ref-resolved; op-level wins on
   *  (name, in) collisions per the OpenAPI merge rule. */
  function params(op: any, pathItem: any): any[] {
    const own = ((op.parameters ?? []) as any[]).map(derefParam);
    const shared = ((pathItem?.parameters ?? []) as any[])
      .map(derefParam)
      .filter((s: any) => s && !own.some((o: any) => o?.name === s.name && o?.in === s.in));
    return [...own, ...shared].filter((p: any) => p && p.in !== 'body');
  }

  function example(op: any): string | null {
    const bodies = bodySchemas(op);
    for (const b of bodies) {
      const c = op.requestBody?.content?.[b.mime];
      const ex = c?.example ?? (c?.examples ? Object.values(c.examples)[0] : null);
      if (ex !== null && ex !== undefined) {
        const v = (ex as any)?.value ?? ex;
        try {
          return JSON.stringify(v, null, 2);
        } catch {
          return String(v);
        }
      }
    }
    return null;
  }
;
async () => {



 { svelteHTML.createElement("div", { "class":`oas`,});
   { svelteHTML.createElement("div", { "class":`head`,});
     { svelteHTML.createElement("h2", {});info.title ?? 'API'; }
     { svelteHTML.createElement("span", { "class":`ver`,});spec.openapi ? `OpenAPI ${spec.openapi}` : `Swagger ${spec.swagger ?? ''}`; }
    if(info.version){ { svelteHTML.createElement("span", { "class":`ver`,}); info.version; }}
   }
  if(info.description){
     { svelteHTML.createElement("p", { "class":`desc`,});info.description; }
  }
  if(servers.length){
     { svelteHTML.createElement("div", { "class":`servers`,});
          for(let s of __sveltets_2_ensureArray(servers)){let i = 1;i; { svelteHTML.createElement("code", {});s; }}
     }
  }

  if(groups.size === 0){
    
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {      "icon":`file`,"title":`No operations in this spec`,"body":`It declares no paths, so there is nothing to browse. Its info and servers are shown above.`,}});}
  }
     for(let [tag, ops] of __sveltets_2_ensureArray([...groups.entries()])){tag;
     { svelteHTML.createElement("section", {});
       { svelteHTML.createElement("h3", {});tag; }
         for(let o of __sveltets_2_ensureArray(ops)){o.method + o.path;
        const key = `${tag}\u0001${o.method} ${o.path}`;
         { svelteHTML.createElement("details", {   "class":`op`,"ontoggle":(e) => toggleOp(key, e),});
           { svelteHTML.createElement("summary", {});
             { svelteHTML.createElement("span", { "class":`method ${o.method}`,});o.method.toUpperCase(); }
             { svelteHTML.createElement("code", {  "class":`path`,});o.deprecated;o.path; }
             { svelteHTML.createElement("span", { "class":`sum`,});o.summary; }
           }
          if(openOps.has(key)){
          const ps = params(o.op, o.item);
           { svelteHTML.createElement("div", { "class":`op-body`,});
            if(o.op.description){ { svelteHTML.createElement("p", { "class":`desc`,});o.op.description; }}

            if(ps.length){
               { svelteHTML.createElement("h4", {});  }
               { svelteHTML.createElement("table", {});
                 { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } } }
                 { svelteHTML.createElement("tbody", {});
                      for(let p of __sveltets_2_ensureArray(ps)){let i = 1;i;
                     { svelteHTML.createElement("tr", {});
                       { svelteHTML.createElement("td", {}); { svelteHTML.createElement("code", {});p.name; } }
                       { svelteHTML.createElement("td", {});p.in; }
                       { svelteHTML.createElement("td", {});typeLabel(p.schema ?? p); }
                       { svelteHTML.createElement("td", {});p.required ? 'Yes' : ''; }
                       { svelteHTML.createElement("td", { });p.description ?? '';p.example !== undefined
                          ? `${p.description ? ' — ' : ''}e.g. ${p.example}`
                          : ''; }
                     }
                  }
                 }
               }
            }

               for(let b of __sveltets_2_ensureArray(bodySchemas(o.op))){b.mime;
               { svelteHTML.createElement("h4", {});   { svelteHTML.createElement("code", { "class":`mime`,});b.mime; } }
              ;__sveltets_2_ensureSnippet(schemaTree(b.schema, 0, new Set()));
            }

            if(example(o.op)){
               { svelteHTML.createElement("h4", {});  }
               { svelteHTML.createElement("pre", { "class":`ex`,});example(o.op); }
            }

             { svelteHTML.createElement("h4", {});  }
               for(let r of __sveltets_2_ensureArray(responses(o.op))){r.code;
               { svelteHTML.createElement("div", { "class":`resp`,});
                 { svelteHTML.createElement("span", {   "class":`code`,});r.code.startsWith('2');r.code.startsWith('4') || r.code.startsWith('5');r.code; }
                 { svelteHTML.createElement("span", { "class":`rdesc`,});r.desc; }
                if(r.schema){;__sveltets_2_ensureSnippet(schemaTree(r.schema, 0, new Set()));}
               }
            }
           }
          }
         }
      }
     }
  }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const OpenApiView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type OpenApiView__SvelteComponent_ = ReturnType<typeof OpenApiView__SvelteComponent_>;
/*Ωignore_endΩ*/export default OpenApiView__SvelteComponent_;