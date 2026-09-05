<script lang="ts">
  // Add / edit cluster sheet. Add = two steps: (1) pick contexts from
  // `/k8s/discover` (multi-select), paste a kubeconfig, or jump to AWS/EKS;
  // (2) name (single pick), default namespace, environment → save, then a
  // best-effort connectivity test toasts per cluster. Edit = step 2 only,
  // PATCHing the row.
  import { untrack } from 'svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { router } from '../../lib/router.svelte';
  import { k8s } from '../../lib/stores/k8s.svelte';
  import { k8sApi } from '../../lib/api/k8s';
  import { toasts } from '../../lib/toast.svelte';
  import type { Environment, K8sCluster, K8sDiscoveredContext } from '../../lib/api/types';
  import { envBadge } from './k8s-util';

  interface Props {
    existing?: K8sCluster | null;
    onclose: () => void;
  }
  let { existing = null, onclose }: Props = $props();

  type Mode = 'contexts' | 'paste' | 'eks';
  const ENVS: Environment[] = ['dev', 'staging', 'prod'];

  // Initial values are snapshotted from the prop on purpose (the sheet is
  // remounted per open) — hence `untrack`.
  const init = untrack(() => existing);
  let step = $state<1 | 2>(init ? 2 : 1);
  let mode = $state<Mode>('contexts');
  let busy = $state(false);
  let error = $state('');

  // step 1 — contexts
  let contexts: K8sDiscoveredContext[] = $state([]);
  let discovering = $state(false);
  let discoverError = $state('');
  let ctxFilter = $state('');
  let picked = $state<Set<string>>(new Set());
  const ctxKey = (c: K8sDiscoveredContext): string => `${c.kubeconfig_path}::${c.name}`;
  const filteredContexts = $derived(
    contexts.filter((c) => {
      const q = ctxFilter.trim().toLowerCase();
      return !q || c.name.toLowerCase().includes(q) || c.cluster.toLowerCase().includes(q) || (c.server ?? '').toLowerCase().includes(q);
    }),
  );
  const pickedList = $derived(contexts.filter((c) => picked.has(ctxKey(c))));

  // step 1 — paste
  let yamlText = $state('');
  let pasteContext = $state('');

  // step 2
  let name = $state(init?.name ?? '');
  let contextName = $state(init?.context_name ?? '');
  let kubeconfigPath = $state(init?.kubeconfig_path ?? '');
  let defaultNs = $state(init?.default_namespace ?? '');
  let knownNsText = $state((init?.known_namespaces ?? []).join(', '));
  const knownNsList = (): string[] => [...new Set(knownNsText.split(/[,\s]+/).map((s) => s.trim().toLowerCase()).filter(Boolean))];
  let env = $state<Environment>(init?.environment ?? 'dev');

  $effect(() => {
    if (existing || !auth.isRoot) return;
    untrack(() => void discover());
  });

  async function discover(): Promise<void> {
    discovering = true;
    discoverError = '';
    try {
      const r = await k8sApi.discover();
      contexts = r.contexts;
    } catch (e) {
      discoverError = e instanceof Error ? e.message : String(e);
    } finally {
      discovering = false;
    }
  }

  function togglePick(c: K8sDiscoveredContext): void {
    const k = ctxKey(c);
    const next = new Set(picked);
    if (next.has(k)) next.delete(k);
    else next.add(k);
    picked = next;
  }

  const canNext = $derived(
    mode === 'contexts' ? pickedList.length > 0 : mode === 'paste' ? yamlText.trim().length > 0 : false,
  );

  function next(): void {
    error = '';
    if (mode === 'contexts' && pickedList.length === 1) {
      const c = pickedList[0];
      name = c.name;
      contextName = c.name;
      kubeconfigPath = c.kubeconfig_path;
      defaultNs = c.namespace ?? '';
    } else if (mode === 'paste') {
      name = name || pasteContext || '';
    }
    step = 2;
  }

  const singleName = $derived(existing !== null || mode === 'paste' || pickedList.length <= 1);
  const canSave = $derived(!busy && (singleName ? name.trim().length > 0 : true));

  async function testAndToast(c: K8sCluster): Promise<void> {
    try {
      const r = await k8sApi.testCluster(c.id);
      if (r.ok) toasts.success(`${c.name}: connected`, `${r.server_version ?? ''} · ${r.latency_ms} ms`.trim());
      else toasts.warn(`${c.name}: saved, but unreachable`, r.message);
      void k8s.loadCapabilities(c.id, true);
    } catch (e) {
      toasts.warn(`${c.name}: saved, test failed`, e instanceof Error ? e.message : String(e));
    }
  }

  async function save(): Promise<void> {
    if(!existing && !auth.isRoot)return;
    busy = true;
    error = '';
    try {
      if (existing) {
        const c = await k8s.updateCluster(existing.id, {
          name: name.trim(),
          known_namespaces: knownNsList(),
          ...(auth.isRoot ? {context_name:contextName.trim() || existing.context_name,default_namespace:defaultNs.trim().toLowerCase() || null,environment:env} : {}),
        });
        toasts.success('Cluster updated', c.name);
        onclose();
        return;
      }
      const created: K8sCluster[] = [];
      if (mode === 'paste') {
        created.push(
          await k8s.importCluster({
            name: name.trim(),
            kubeconfig_yaml: yamlText,
            context_name: pasteContext.trim() || null,
            default_namespace: defaultNs.trim().toLowerCase() || null,
            environment: env,
          }),
        );
      } else if (pickedList.length === 1) {
        created.push(
          await k8s.createCluster({
            name: name.trim(),
            source: 'kubeconfig',
            kubeconfig_path: kubeconfigPath || null,
            context_name: contextName.trim(),
            default_namespace: defaultNs.trim().toLowerCase() || null,
            environment: env,
            known_namespaces: knownNsList(),
          }),
        );
      } else {
        for (const c of pickedList) {
          created.push(
            await k8s.createCluster({
              name: c.name,
              source: 'kubeconfig',
              kubeconfig_path: c.kubeconfig_path,
              context_name: c.name,
              default_namespace: defaultNs.trim().toLowerCase() || c.namespace || null,
              environment: env,
            }),
          );
        }
      }
      onclose();
      // Best-effort tests after the sheet closes; each toasts its outcome.
      for (const c of created) void testAndToast(c);
      if (created.length === 1) router.go(`kubernetes/${encodeURIComponent(created[0].id)}`);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
</script>

<Modal title={existing ? 'Edit cluster' : step === 1 ? 'Add cluster' : 'Cluster details'} width={580} {onclose}>
  <div class="wiz" data-testid="k8s-cluster-wizard">
    {#if !auth.isRoot}<p class="hint">Owner manages credentials and native cluster settings. You can edit the name.</p>{/if}
    {#if step === 1}
      <div class="segmented modes" role="tablist" aria-label="Cluster source">
        <button role="tab" aria-selected={mode === 'contexts'} class:active={mode === 'contexts'} onclick={() => (mode = 'contexts')}>From kubeconfig</button>
        <button role="tab" aria-selected={mode === 'paste'} class:active={mode === 'paste'} onclick={() => (mode = 'paste')}>Paste kubeconfig</button>
        <button role="tab" aria-selected={mode === 'eks'} class:active={mode === 'eks'} onclick={() => (mode = 'eks')}>From EKS</button>
      </div>

      {#if mode === 'contexts'}
        <p class="hint">Contexts found in <span class="mono">~/.kube/config</span> and <span class="mono">$KUBECONFIG</span>. Otto reads them in place and never modifies the file. Pick one or more.</p>
        <div class="ctx-tools">
          <input class="input" placeholder="Filter contexts…" bind:value={ctxFilter} aria-label="Filter contexts" />
          <button class="btn ghost" onclick={() => void discover()} disabled={discovering} aria-label="Rescan"><Icon name="refresh" size={13} /></button>
        </div>
        {#if discovering}
          <Skeleton rows={3} height={40} />
        {:else if discoverError}
          <div class="err">{discoverError}</div>
        {:else if !contexts.length}
          <div class="hint">No contexts found. Paste a kubeconfig instead, or import from EKS.</div>
        {:else}
          <div class="ctx-list" role="listbox" aria-multiselectable="true" aria-label="Kubeconfig contexts">
            {#each filteredContexts as c (ctxKey(c))}
              {@const on = picked.has(ctxKey(c))}
              <label class="ctx" class:on>
                <input type="checkbox" checked={on} onchange={() => togglePick(c)} />
                <span class="ctx-main">
                  <span class="ctx-name">{c.name}</span>
                  <span class="ctx-meta mono">{c.cluster}{#if c.server} · {c.server}{/if}{#if c.namespace} · ns {c.namespace}{/if}</span>
                  <span class="ctx-meta mono dim">{c.kubeconfig_path}</span>
                </span>
              </label>
            {/each}
          </div>
        {/if}
      {:else if mode === 'paste'}
        <p class="hint">Otto stores the pasted file under its own data dir (0600) and never touches <span class="mono">~/.kube/config</span>.</p>
        <div class="field">
          <label for="k8s-yaml">kubeconfig YAML</label>
          <textarea id="k8s-yaml" class="input mono" rows="10" bind:value={yamlText} placeholder="apiVersion: v1&#10;kind: Config&#10;…" spellcheck="false"></textarea>
        </div>
        <div class="field">
          <label for="k8s-paste-ctx">Context name <span class="dim">(optional — defaults to the file's current-context)</span></label>
          <input id="k8s-paste-ctx" class="input mono" bind:value={pasteContext} />
        </div>
      {:else}
        <div class="eks">
          <Icon name="cloud" size={22} />
          <p>EKS clusters are imported from the AWS module: pick an account → EKS → “Open in Kubernetes”. Otto runs <span class="mono">aws eks update-kubeconfig</span> into its own kubeconfig and links the cluster to that account.</p>
          <button class="btn" onclick={() => { onclose(); router.go('aws'); }}><Icon name="external" size={13} /> Go to AWS</button>
        </div>
      {/if}
    {:else}
      {#if !existing && !singleName}
        <div class="picked-summary">
          <strong>{pickedList.length} contexts</strong> will be added, each named after its context:
          <ul>{#each pickedList as c (ctxKey(c))}<li class="mono">{c.name}</li>{/each}</ul>
        </div>
      {:else}
        <div class="field">
          <label for="k8s-name">Name</label>
          <input id="k8s-name" class="input" bind:value={name} placeholder="prod-eu-1" data-testid="k8s-wizard-name" />
        </div>
        {#if existing || mode === 'contexts'}
          <div class="field">
            <label for="k8s-ctx">Context</label>
            <input id="k8s-ctx" class="input mono" bind:value={contextName} readonly={!existing || !auth.isRoot} />
            {#if kubeconfigPath}<span class="hint mono">{kubeconfigPath}</span>{/if}
          </div>
        {/if}
      {/if}
      <div class="field">
        <label for="k8s-ns">Default namespace <span class="dim">(blank = all namespaces)</span></label>
        <input id="k8s-ns" disabled={!auth.isRoot} class="input mono" bind:value={defaultNs} placeholder="default" autocapitalize="off" autocorrect="off" spellcheck={false} />
      </div>
      <div class="field">
        <label for="k8s-known-ns">Namespaces <span class="dim">(comma-separated; offered in the picker even when the cluster forbids listing them — saved with the cluster)</span></label>
        <input id="k8s-known-ns" class="input mono" bind:value={knownNsText} placeholder="koala-staging, koala-jobs" autocapitalize="off" autocorrect="off" spellcheck={false} data-testid="k8s-known-ns" />
      </div>
      <div class="field">
        <span class="lbl">Environment</span>
        <div class="env-row" role="radiogroup" aria-label="Environment">
          {#each ENVS as e (e)}
            <button
              type="button"
              class="env-chip"
              class:selected={env === e}
              class:prod={e === 'prod'}
              role="radio"
              aria-checked={env === e}
              disabled={!auth.isRoot} onclick={() => (env = e)}
            >{e}</button>
          {/each}
        </div>
        {#if env === 'prod'}<span class="hint danger">Production clusters get the {envBadge('prod')} treatment everywhere: red pill, and every destructive action asks you to type the resource name.</span>{/if}
      </div>
    {/if}

    {#if error}<div class="err">{error}</div>{/if}
  </div>

  {#snippet footer()}
    {#if step === 2 && !existing}
      <button class="btn ghost" disabled={busy} onclick={() => (step = 1)}>Back</button>
    {/if}
    <span class="spacer"></span>
    <button class="btn" disabled={busy} onclick={onclose}>Cancel</button>
    {#if step === 1}
      <button class="btn primary" disabled={!canNext || busy} onclick={next}>Next</button>
    {:else}
      <button class="btn primary" disabled={!canSave} onclick={() => void save()} data-testid="k8s-wizard-save">
        {busy ? 'Saving…' : existing ? 'Save' : 'Save & test'}
      </button>
    {/if}
  {/snippet}
</Modal>

<style>
  .wiz {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .modes {
    align-self: flex-start;
  }
  .hint {
    font-size: 12px;
    color: var(--text-dim);
    line-height: 1.5;
    margin: 0;
  }
  .hint.danger {
    color: var(--status-exited);
  }
  .ctx-tools {
    display: flex;
    gap: 6px;
  }
  .ctx-tools .input {
    flex: 1;
  }
  .ctx-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: min(320px, 45vh);
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 4px;
  }
  .ctx {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .ctx:hover {
    background: var(--surface-2);
  }
  .ctx.on {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .ctx input {
    margin-top: 3px;
  }
  .ctx-main {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .ctx-name {
    font-weight: 500;
    font-size: 12.5px;
  }
  .ctx-meta {
    font-size: 11px;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .eks {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 10px;
    padding: 12px;
    border: 1px dashed var(--border);
    border-radius: var(--radius-m);
    color: var(--text-dim);
    font-size: 12.5px;
    line-height: 1.5;
  }
  .eks p {
    margin: 0;
  }
  .picked-summary {
    font-size: 12.5px;
    line-height: 1.5;
  }
  .picked-summary ul {
    margin: 6px 0 0;
    padding-left: 18px;
    max-height: 160px;
    overflow: auto;
  }
  .lbl {
    font-size: 11.5px;
    font-weight: 500;
    color: var(--text-dim);
  }
  .env-row {
    display: flex;
    gap: 6px;
  }
  .env-chip {
    height: 24px;
    padding: 0 13px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    font-size: 12px;
    color: var(--text-dim);
    cursor: pointer;
    text-transform: capitalize;
    transition: all 130ms ease-out;
  }
  .env-chip.selected {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
    color: var(--accent);
    font-weight: 500;
  }
  .env-chip.prod.selected {
    background: color-mix(in srgb, var(--status-exited) 18%, transparent);
    border-color: color-mix(in srgb, var(--status-exited) 55%, transparent);
    color: var(--status-exited);
  }
  .err {
    color: var(--status-exited);
    font-size: 12px;
    white-space: pre-wrap;
  }
  .spacer {
    flex: 1;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .dim {
    color: var(--text-dim);
    font-weight: 400;
  }
</style>
