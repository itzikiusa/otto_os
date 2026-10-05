<script lang="ts">
  import { plural } from '../../lib/plural';
  import { toastError } from '../../lib/toastError';
  // Drawer "HTTP" tab (K-3): call an HTTP endpoint inside one pod or every pod
  // of a workload — e.g. Spring Boot actuator loggers — through the daemon's
  // kubectl-proxy gateway (port-forward fallback). Left: saved per-workload
  // actions + built-in actuator presets. Right: method / port / path /
  // headers / body with `{{var}}` inputs, a target pick and Run; results come
  // back one row per pod. Mutating calls on a prod (or read-only) cluster ask
  // for the target name typed back; the daemon enforces it too, and audits
  // every call (`k8s.pod_http`, body hash only).
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { copyText } from '../../lib/clipboard';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ApiError } from '../../lib/api/client';
  import { k8sApi } from '../../lib/api/k8s';
  import { k8s } from '../../lib/stores/k8s.svelte';
  import type { K8sPodAction, K8sPodHttpMethod, K8sPodHttpReq, K8sPodHttpResult } from '../../lib/api/types';
  import { confirmProd, isProdEnv } from '../../lib/confirmProd';
  import { clusterLabel } from './k8s-util';
  import { ACTUATOR_PRESETS, LOG_LEVELS, defaultPort, fillTemplate, pathProblem, prettyBody, templateVars } from './podHttp';

  interface Props {
    clusterId: string;
    ns: string;
    /** Set for a pod drawer. */
    pod?: string;
    /** The workload this drawer is (or the pod belongs to), when known —
     *  `kind` is the pod-http workload kind (deployment, statefulset, …). */
    workload?: { kind: string; name: string } | null;
    /** Pod or workload manifest — for the default port guess. */
    manifest?: unknown;
    /** May the person send mutating methods (Edit + the `exec` op — the daemon's rule)? */
    canMutate: boolean;
  }
  let { clusterId, ns, pod, workload = null, manifest = null, canMutate }: Props = $props();

  const METHODS: K8sPodHttpMethod[] = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE'];
  const cluster = $derived(k8s.clusters.find((c) => c.id === clusterId) ?? null);
  const guarded = $derived(isProdEnv(cluster?.environment) || !!(cluster as { read_only?: boolean } | null)?.read_only);

  let method = $state<K8sPodHttpMethod>('GET');
  let port = $state(untrack(() => defaultPort(manifest)));
  let path = $state('/actuator/health');
  let headersText = $state('');
  let body = $state('');
  let vars = $state<Record<string, string>>({ level: 'DEBUG' });
  let target = $state<'pod' | 'workload'>(untrack(() => (pod ? 'pod' : 'workload')));
  let loadedFrom = $state<string | null>(null);

  let running = $state(false);
  let runError = $state('');
  let results = $state<K8sPodHttpResult[] | null>(null);
  let openRow = $state<string | null>(null);

  // --- saved actions ---------------------------------------------------------
  let saved = $state<K8sPodAction[]>([]);
  let savedError = $state('');
  let savedLoading = $state(false);
  async function loadSaved(): Promise<void> {
    if (!workload) return;
    savedLoading = true;
    try {
      const r = await k8sApi.podActions(clusterId, { namespace: ns, workload_kind: workload.kind, workload: workload.name });
      saved = r.actions;
      savedError = '';
    } catch (e) {
      savedError = loadErrorText(e);
    } finally {
      savedLoading = false;
    }
  }
  $effect(() => {
    void clusterId;
    void ns;
    void workload?.name;
    untrack(() => void loadSaved());
  });

  const varNames = $derived(templateVars(path, method === 'GET' ? '' : body));
  const filledPath = $derived(fillTemplate(path, vars));
  const problem = $derived(pathProblem(filledPath) ?? (varNames.some((v) => !(vars[v] ?? '').trim()) ? 'Fill in every {{variable}}' : null));
  const mutating = $derived(method !== 'GET');
  const blockedByRole = $derived(mutating && !canMutate);
  const targetName = $derived(target === 'pod' && pod ? pod : (workload?.name ?? pod ?? ''));

  function parseHeaders(text: string): Record<string, string> {
    const out: Record<string, string> = {};
    for (const line of text.split('\n')) {
      const i = line.indexOf(':');
      if (i <= 0) continue;
      const k = line.slice(0, i).trim();
      if (k) out[k] = line.slice(i + 1).trim();
    }
    return out;
  }
  function headersToText(h: Record<string, string> | undefined): string {
    return Object.entries(h ?? {}).map(([k, v]) => `${k}: ${v}`).join('\n');
  }

  function applyPreset(p: { id: string; name: string; method: K8sPodHttpMethod; path: string; headers?: Record<string, string>; body?: string | null; port?: number }): void {
    method = p.method;
    path = p.path;
    headersText = headersToText(p.headers);
    body = p.body ?? '';
    if (p.port) port = p.port;
    loadedFrom = p.id;
    results = null;
    runError = '';
  }

  async function run(): Promise<void> {
    if (problem || running || blockedByRole) return;
    const req: K8sPodHttpReq = {
      namespace: ns,
      port: Number(port),
      method,
      path: filledPath,
      headers: parseHeaders(headersText),
      body: mutating && body.trim() ? fillTemplate(body, vars) : null,
    };
    if (target === 'pod' && pod) req.pod = pod;
    else if (workload) req.workload = workload;
    else if (pod) req.pod = pod;
    if (mutating && guarded) {
      const scope = req.pod ? `pod ${req.pod}` : `every pod of ${workload?.kind} ${workload?.name}`;
      const where = `${scope}${ns ? ` · namespace ${ns}` : ''} · ${clusterLabel(cluster)}${isProdEnv(cluster?.environment) ? '' : ' (read-only cluster)'}`;
      const ok = await confirmProd({
        env: cluster?.environment,
        where,
        verb: `Send ${method}`,
        what: `${method} ${req.path}`,
        typed: targetName,
        title: `${method} to pods?`,
        danger: true,
      });
      if (!ok) return;
      req.confirm_name = targetName;
    }
    running = true;
    runError = '';
    try {
      const r = await k8sApi.podHttp(clusterId, req);
      results = r.results;
      openRow = r.results.length === 1 ? r.results[0].pod : null;
      const failed = r.results.filter((x) => x.error || (x.status ?? 0) >= 400).length;
      if (mutating) {
        if (failed) toasts.warn(`${method} ${req.path}`, `${failed} of ${plural(r.results.length, 'pod')} failed`);
        else toasts.success(`${method} ${req.path}`, `${plural(r.results.length, 'pod')} answered`);
      }
    } catch (e) {
      runError = e instanceof ApiError && e.status === 409 ? `The daemon wants the target name confirmed: ${e.message}` : e instanceof Error ? e.message : String(e);
    } finally {
      running = false;
    }
  }

  async function saveAction(): Promise<void> {
    if (!workload) return;
    const existing = saved.find((s) => s.id === loadedFrom);
    const name = await confirmer.promptText(`Save this request for ${workload.kind} ${workload.name}. {{variables}} stay as placeholders.`, {
      title: existing ? 'Update saved action' : 'Save action',
      confirmLabel: 'Save',
      initial: existing?.name ?? ACTUATOR_PRESETS.find((p) => p.id === loadedFrom)?.name ?? '',
      placeholder: 'e.g. Billing logger → DEBUG',
    });
    if (!name) return;
    try {
      const a = await k8sApi.savePodAction(clusterId, {
        id: existing?.id,
        namespace: ns,
        workload_kind: workload.kind,
        workload: workload.name,
        name,
        method,
        port: Number(port),
        path,
        headers: parseHeaders(headersText),
        body_template: mutating && body.trim() ? body : null,
      });
      saved = [...saved.filter((s) => s.id !== a.id), a].sort((x, y) => x.name.localeCompare(y.name));
      loadedFrom = a.id;
      toasts.success('Action saved', a.name);
    } catch (e) {
      toastError("Couldn’t save the action", e);
    }
  }

  async function deleteAction(a: K8sPodAction): Promise<void> {
    const ok = await confirmer.ask(`Delete the saved action “${a.name}” for ${a.workload}?`, { title: 'Delete action', confirmLabel: 'Delete' });
    if (!ok) return;
    try {
      await k8sApi.deletePodAction(clusterId, a.id);
      saved = saved.filter((s) => s.id !== a.id);
      if (loadedFrom === a.id) loadedFrom = null;
    } catch (e) {
      toastError("Couldn’t delete the action", e);
    }
  }

  function statusTone(r: K8sPodHttpResult): string {
    if (r.error || r.status === null) return 'bad';
    if (r.status >= 500) return 'bad';
    if (r.status >= 400) return 'warn';
    return 'ok';
  }
  function shownBody(r: K8sPodHttpResult): string {
    if (r.body_base64) return `(binary body, ${r.body.length} base64 chars)`;
    return prettyBody(r.body);
  }
</script>

<div class="ph" data-testid="k8s-pod-http">
  <aside class="ph-side" aria-label="Saved actions and presets">
    {#if workload}
      <div class="ph-sec">Saved for {workload.name}</div>
      <LoadState what="saved actions" variant="compact" loading={savedLoading} error={savedError} empty={!saved.length} onretry={() => void loadSaved()}>
        {#snippet emptyView()}<div class="dim small pad">None yet — build a request and Save it.</div>{/snippet}
        {#each saved as a (a.id)}
          <div class="ph-item-row">
            <button class="ph-item" class:active={loadedFrom === a.id} onclick={() => applyPreset({ ...a, body: a.body_template })} title="{a.method} :{a.port}{a.path}">
              <span class="m mono {a.method.toLowerCase()}">{a.method}</span><span class="ell">{a.name}</span>
            </button>
            {#if canMutate}<button class="icon-btn" onclick={() => void deleteAction(a)} aria-label="Delete {a.name}" title="Delete saved action"><Icon name="trash" size={12} /></button>{/if}
          </div>
        {/each}
      </LoadState>
    {/if}
    <div class="ph-sec">Spring Boot actuator</div>
    {#each ACTUATOR_PRESETS as p (p.id)}
      <button class="ph-item" class:active={loadedFrom === p.id} onclick={() => applyPreset(p)} title={p.hint} data-testid="k8s-pod-http-preset-{p.id}">
        <span class="m mono {p.method.toLowerCase()}">{p.method}</span><span class="ell">{p.name}</span>
      </button>
    {/each}
  </aside>

  <section class="ph-main">
    <div class="ph-line">
      <select class="input meth" bind:value={method} aria-label="HTTP method">
        {#each METHODS as m (m)}<option value={m}>{m}</option>{/each}
      </select>
      <input class="input port mono" type="number" min="1" max="65535" bind:value={port} aria-label="Container port" title="Container port" />
      <input class="input path mono" bind:value={path} placeholder="/actuator/health" aria-label="Path" data-testid="k8s-pod-http-path" onkeydown={(e) => { if (e.key === 'Enter') void run(); }} />
    </div>

    {#if varNames.length}
      <div class="ph-vars">
        {#each varNames as v (v)}
          <label class="var">
            <span class="dim small mono">{v}</span>
            {#if v === 'level'}
              <select class="input" bind:value={vars[v]} aria-label="Log level">
                {#each LOG_LEVELS as l (l)}<option value={l}>{l === 'RESET' ? 'RESET (clear override)' : l}</option>{/each}
              </select>
            {:else}
              <input class="input mono" bind:value={vars[v]} placeholder={v === 'logger' ? 'com.example.service' : v} aria-label={v} />
            {/if}
          </label>
        {/each}
      </div>
    {/if}

    <details class="ph-more" open={!!headersText || mutating}>
      <summary>Headers{mutating ? ' & body' : ''}</summary>
      <textarea class="input mono" rows="2" bind:value={headersText} placeholder="Accept: application/json" aria-label="Headers, one per line"></textarea>
      {#if mutating}
        <textarea class="input mono" rows="4" bind:value={body} placeholder={'{"configuredLevel":"DEBUG"}'} aria-label="Request body"></textarea>
      {/if}
    </details>

    <div class="ph-run">
      {#if pod && workload}
        <div class="segmented" role="group" aria-label="Target">
          <button class:active={target === 'pod'} aria-pressed={target === 'pod'} onclick={() => (target = 'pod')}>This pod</button>
          <button class:active={target === 'workload'} aria-pressed={target === 'workload'} onclick={() => (target = 'workload')}>All pods of {workload.name}</button>
        </div>
      {:else}
        <span class="dim small">{pod ? `Pod ${pod}` : workload ? `Every pod of ${workload.name}` : ''}</span>
      {/if}
      <span class="spacer"></span>
      {#if workload && canMutate}<button class="btn small" onclick={() => void saveAction()}>Save…</button>{/if}
      <button class="btn small primary" class:danger={mutating && guarded} onclick={() => void run()} disabled={!!problem || running || blockedByRole} title={blockedByRole ? 'Sending a mutating request needs Edit on this cluster' : (problem ?? 'Send (Enter in the path)')} data-testid="k8s-pod-http-run">
        <Icon name="send" size={12} /> {running ? 'Sending…' : 'Send'}
      </button>
    </div>
    {#if problem && path}<div class="dim small">{problem}</div>{/if}
    {#if mutating && guarded}<div class="guard small" role="note"><Icon name="warning" size={12} /> {isProdEnv(cluster?.environment) ? 'Production' : 'Read-only'} cluster — you’ll be asked to type the target name.</div>{/if}

    {#if runError}
      <div class="err" role="alert">{runError}</div>
    {:else if results && !results.length}
      <EmptyState icon="box" title="No pods matched" body="The workload has no running pods to call." />
    {:else if results}
      <ul class="ph-results" data-testid="k8s-pod-http-results">
        {#each results as r (r.pod)}
          <li>
            <button class="res-row" aria-expanded={openRow === r.pod} onclick={() => (openRow = openRow === r.pod ? null : r.pod)}>
              <span class="ph-status {statusTone(r)} mono">{r.status ?? 'ERR'}</span>
              <span class="mono ell">{r.pod}</span>
              <span class="spacer"></span>
              <span class="dim small mono">{r.duration_ms} ms{r.via === 'port_forward' ? ' · port-forward' : ''}</span>
            </button>
            {#if openRow === r.pod}
              <div class="res-body">
                {#if r.error}<div class="err small">{r.error}</div>{/if}
                {#if r.body}
                  <div class="res-tools">
                    <span class="dim small">{r.truncated ? 'Truncated at 256 KB' : ''}</span>
                    <button class="btn small" onclick={() => void copyText(r.body)}><Icon name="copy" size={12} /> Copy</button>
                  </div>
                  <pre class="mono">{shownBody(r)}</pre>
                {/if}
                {#if Object.keys(r.headers).length}
                  <details><summary class="small">Response headers</summary><pre class="mono">{headersToText(r.headers)}</pre></details>
                {/if}
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {:else}
      <div class="dim small">Pick a preset or type a path, then Send. Calls go through the cluster API server’s pod proxy; nothing is exposed outside this Mac.</div>
    {/if}
  </section>
</div>

<style>
  .ph {
    display: grid;
    grid-template-columns: minmax(150px, 190px) minmax(0, 1fr);
    min-height: 100%;
    font-size: var(--fs-m);
  }
  .ph-side {
    border-inline-end: 1px solid var(--border);
    padding: 8px 6px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    overflow-y: auto;
  }
  .ph-sec {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
    padding: 8px 6px 4px;
  }
  .ph-item-row {
    display: flex;
    align-items: center;
  }
  .ph-item {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    min-width: 0;
    padding: 4px 6px;
    border: 0;
    background: transparent;
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-s);
    text-align: start;
    cursor: pointer;
  }
  .ph-item:hover {
    background: var(--hover);
  }
  .ph-item.active {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .m {
    font-size: var(--fs-xs);
    min-inline-size: 38px;
    color: var(--text-dim);
  }
  .m.post,
  .m.put,
  .m.patch {
    color: var(--warning);
  }
  .m.delete {
    color: var(--danger);
  }
  .ph-main {
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  .ph-line {
    display: flex;
    gap: 6px;
  }
  .meth {
    inline-size: 92px;
  }
  .port {
    inline-size: 84px;
  }
  .path {
    flex: 1;
    min-width: 0;
  }
  .ph-vars {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .var {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-inline-size: 160px;
    flex: 1;
  }
  .ph-more {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .ph-more summary {
    font-size: var(--fs-s);
    color: var(--text-dim);
    cursor: pointer;
  }
  .ph-more textarea {
    inline-size: 100%;
    margin-block-start: 6px;
    resize: vertical;
    font-size: var(--fs-s);
  }
  .ph-run {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .spacer {
    flex: 1;
  }
  .guard {
    color: var(--warning);
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .ph-results {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .res-row {
    display: flex;
    align-items: center;
    gap: 8px;
    inline-size: 100%;
    padding: 4px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    cursor: pointer;
    text-align: start;
  }
  .res-row[aria-expanded='true'] {
    border-color: var(--accent-solid);
  }
  .res-body {
    padding: 6px 2px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .res-tools {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  pre {
    margin: 0;
    padding: 8px;
    background: var(--surface-2);
    border-radius: var(--radius-s);
    font-size: var(--fs-s);
    max-block-size: 360px;
    overflow: auto;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .ph-status {
    font-size: var(--fs-xs);
    padding: 1px 6px;
    border-radius: 999px;
    border: 1px solid currentColor;
    min-inline-size: 34px;
    text-align: center;
  }
  .ph-status.ok {
    color: var(--success);
  }
  .ph-status.warn {
    color: var(--warning);
  }
  .ph-status.bad {
    color: var(--danger);
  }
  .ell {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .dim {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-xs);
  }
  .pad {
    padding: 2px 6px;
  }
  .err {
    color: var(--danger);
  }
  .mono {
    font-family: var(--font-mono);
  }
  @media (max-width: 640px) {
    .ph {
      grid-template-columns: minmax(0, 1fr);
    }
    .ph-side {
      border-inline-end: 0;
      border-block-end: 1px solid var(--border);
      max-block-size: 180px;
    }
  }
</style>
