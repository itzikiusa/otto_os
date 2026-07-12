<script lang="ts">
  // Compact structured OpenAPI viewer (v2/v3, json or yaml source) — info
  // header, servers, and operations grouped by tag with expandable details:
  // parameters, request body, responses, and a depth-bounded schema tree with
  // $ref resolution. Deliberately dependency-free (no swagger-ui) so it stays
  // light and matches the app's look.

  /* eslint-disable @typescript-eslint/no-explicit-any */
  let { spec }: { spec: Record<string, any> } = $props();

  const METHODS = ['get', 'post', 'put', 'patch', 'delete', 'options', 'head'] as const;

  interface Op {
    method: string;
    path: string;
    summary: string;
    deprecated: boolean;
    op: any;
  }

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
    for (const p of op.parameters ?? []) {
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

  function params(op: any): any[] {
    return (op.parameters ?? []).filter((p: any) => p && p.in !== 'body');
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
</script>

{#snippet schemaTree(raw: any, depth: number, seen: Set<string>)}
  {@const r = deref(raw, seen)}
  {#if r.schema && typeof r.schema === 'object' && depth <= 5}
    {@const props = (r.schema.properties ?? {}) as Record<string, any>}
    {@const required = new Set((r.schema.required ?? []) as string[])}
    {#if Object.keys(props).length > 0}
      <ul class="schema">
        {#each Object.entries(props) as [k, v] (k)}
          {@const child = deref(v, new Set(seen))}
          <li>
            <span class="pname" class:req={required.has(k)}>{k}</span>
            <span class="ptype">{typeLabel(v)}</span>
            {#if v?.description}<span class="pdesc">— {v.description}</span>{/if}
            {#if child.schema?.properties || v?.items}
              {@render schemaTree(v?.items ?? v, depth + 1, new Set(seen))}
            {/if}
          </li>
        {/each}
      </ul>
    {:else if r.schema.items}
      {@render schemaTree(r.schema.items, depth + 1, new Set(seen))}
    {:else if r.name}
      <span class="ptype">{r.name}</span>
    {/if}
  {:else if r.name}
    <span class="ptype">{r.name}</span>
  {/if}
{/snippet}

<div class="oas">
  <div class="head">
    <h2>{info.title ?? 'API'}</h2>
    <span class="ver">{spec.openapi ? `OpenAPI ${spec.openapi}` : `Swagger ${spec.swagger ?? ''}`}</span>
    {#if info.version}<span class="ver">v{info.version}</span>{/if}
  </div>
  {#if info.description}
    <p class="desc">{info.description}</p>
  {/if}
  {#if servers.length}
    <div class="servers">
      {#each servers as s, i (i)}<code>{s}</code>{/each}
    </div>
  {/if}

  {#each [...groups.entries()] as [tag, ops] (tag)}
    <section>
      <h3>{tag}</h3>
      {#each ops as o (o.method + o.path)}
        <details class="op">
          <summary>
            <span class="method {o.method}">{o.method.toUpperCase()}</span>
            <code class="path" class:deprecated={o.deprecated}>{o.path}</code>
            <span class="sum">{o.summary}</span>
          </summary>
          <div class="op-body">
            {#if o.op.description}<p class="desc">{o.op.description}</p>{/if}

            {#if params(o.op).length}
              <h4>Parameters</h4>
              <table>
                <thead><tr><th>name</th><th>in</th><th>type</th><th>req</th><th>description</th></tr></thead>
                <tbody>
                  {#each params(o.op) as p, i (i)}
                    <tr>
                      <td><code>{p.name}</code></td>
                      <td>{p.in}</td>
                      <td>{typeLabel(p.schema ?? p)}</td>
                      <td>{p.required ? '✓' : ''}</td>
                      <td>{p.description ?? ''}</td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            {/if}

            {#each bodySchemas(o.op) as b (b.mime)}
              <h4>Request body <code class="mime">{b.mime}</code></h4>
              {@render schemaTree(b.schema, 0, new Set())}
            {/each}

            {#if example(o.op)}
              <h4>Example</h4>
              <pre class="ex">{example(o.op)}</pre>
            {/if}

            <h4>Responses</h4>
            {#each responses(o.op) as r (r.code)}
              <div class="resp">
                <span class="code" class:ok={r.code.startsWith('2')} class:bad={r.code.startsWith('4') || r.code.startsWith('5')}>{r.code}</span>
                <span class="rdesc">{r.desc}</span>
                {#if r.schema}{@render schemaTree(r.schema, 0, new Set())}{/if}
              </div>
            {/each}
          </div>
        </details>
      {/each}
    </section>
  {/each}
</div>

<style>
  .oas {
    padding: 18px 26px 60px;
    max-width: min(1240px, 96%);
    width: 100%;
    margin: 0 auto;
    font-size: 13px;
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: 10px;
    flex-wrap: wrap;
  }
  .head h2 {
    margin: 0;
  }
  .ver {
    font-size: 11px;
    color: var(--accent, #9ab4ff);
    border: 1px solid var(--accent, #9ab4ff);
    border-radius: 5px;
    padding: 1px 7px;
    white-space: nowrap;
  }
  .desc {
    color: var(--text-dim);
    line-height: 1.5;
    white-space: pre-wrap;
  }
  .servers {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    margin: 8px 0 4px;
  }
  .servers code {
    background: var(--panel-2, #222);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 2px 8px;
    font-size: 12px;
  }
  section h3 {
    margin: 22px 0 8px;
    text-transform: capitalize;
    border-bottom: 1px solid var(--border);
    padding-bottom: 4px;
  }
  .op {
    border: 1px solid var(--border);
    border-radius: 8px;
    margin: 6px 0;
    overflow: hidden;
  }
  .op summary {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 10px;
    cursor: pointer;
    list-style: none;
  }
  .op summary::-webkit-details-marker {
    display: none;
  }
  .op summary:hover {
    background: var(--hover, rgba(127, 127, 127, 0.08));
  }
  .method {
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: 0.4px;
    border-radius: 5px;
    padding: 2px 7px;
    min-width: 46px;
    text-align: center;
    color: #fff;
    flex-shrink: 0;
  }
  .method.get { background: #2f6feb; }
  .method.post { background: #2da44e; }
  .method.put { background: #b58a2c; }
  .method.patch { background: #8250df; }
  .method.delete { background: #cf4436; }
  .method.options, .method.head { background: #6e7781; }
  .path {
    font-size: 12.5px;
  }
  .path.deprecated {
    text-decoration: line-through;
    opacity: 0.6;
  }
  .sum {
    color: var(--text-dim);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .op-body {
    padding: 10px 14px 14px;
    border-top: 1px solid var(--border);
  }
  .op-body h4 {
    margin: 12px 0 6px;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.4px;
    color: var(--text-dim);
  }
  .mime {
    text-transform: none;
    letter-spacing: 0;
    font-size: 11px;
    color: var(--accent, #9ab4ff);
  }
  table {
    border-collapse: collapse;
    font-size: 12px;
    width: 100%;
  }
  th, td {
    border: 1px solid var(--border);
    padding: 3px 8px;
    text-align: start;
  }
  th {
    color: var(--text-dim);
    font-weight: 600;
  }
  ul.schema {
    list-style: none;
    margin: 4px 0;
    padding-inline-start: 16px;
    border-inline-start: 1px solid var(--border);
  }
  ul.schema li {
    padding: 1px 0;
    font-size: 12px;
  }
  .pname {
    font-family: var(--mono, ui-monospace, monospace);
  }
  .pname.req::after {
    content: '*';
    color: #e88;
  }
  .ptype {
    color: var(--accent, #9ab4ff);
    margin-inline-start: 8px;
    font-size: 11.5px;
  }
  .pdesc {
    color: var(--text-dim);
    font-size: 11.5px;
  }
  .ex {
    background: var(--panel-2, #1d1d1f);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 10px 12px;
    font-size: 11.5px;
    overflow-x: auto;
  }
  .resp {
    margin: 4px 0;
  }
  .resp .code {
    font-weight: 700;
    font-size: 12px;
    margin-inline-end: 8px;
  }
  .resp .code.ok { color: #7fc97f; }
  .resp .code.bad { color: #e88; }
  .rdesc {
    color: var(--text-dim);
    font-size: 12px;
  }
</style>
