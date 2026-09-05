<script lang="ts">
  // Per-cluster monitoring settings: enable, cadence, namespaces, probes
  // (preset-fillable; JSON probes carry field mappings, prometheus probes
  // carry include/exclude globs), exclusions, transport / concurrency /
  // retention, and a "Test probes" dry run that shows what one pod parses to.
  // Client-side validation mirrors the daemon's limits so most mistakes never
  // round-trip.
  import { untrack } from 'svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { ctxMenu } from '../../../lib/contextmenu.svelte';
  import { k8sApi } from '../../../lib/api/k8s';
  import type {
    K8sCluster,
    K8sExclusion,
    K8sMapping,
    K8sMonitorConfig,
    K8sMonitorPreset,
    K8sMonitorStatus,
    K8sMonitorTestResp,
    K8sProbe,
    K8sProbeFormat,
    K8sUnit,
  } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import Skeleton from '../../../lib/components/Skeleton.svelte';
  import { collectorLine, rbacMessage } from './monitor-util';

  interface Props {
    cluster: K8sCluster;
    canEdit: boolean;
    onsaved?: (cfg: K8sMonitorConfig, status: K8sMonitorStatus | null) => void;
  }
  let { cluster, canEdit, onsaved }: Props = $props();

  let cfg = $state<K8sMonitorConfig | null>(null);
  let status = $state<K8sMonitorStatus | null>(null);
  let presets = $state<K8sMonitorPreset[]>([]);
  let loading = $state(true);
  let loadError = $state('');
  let saving = $state(false);
  let running = $state(false);
  let errors = $state<Record<string, string>>({});
  let nsText = $state('');
  let testOpen = $state(false);
  let testBusy = $state(false);
  let testResult = $state<K8sMonitorTestResp | null>(null);
  let testError = $state('');
  let testPod = $state('');

  const FORMATS: K8sProbeFormat[] = ['json', 'prometheus', 'health'];
  const UNITS: K8sUnit[] = ['number', 'bytes', 'bytes_human', 'duration_human', 'percent'];
  const NEEDS_NS = $derived(!cluster.default_namespace);

  async function load(): Promise<void> {
    loading = true;
    try {
      const r = await k8sApi.monitor(cluster.id);
      cfg = r.config;
      status = r.status;
      presets = r.presets;
      nsText = r.config.namespaces.join(', ');
      loadError = '';
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    const id = cluster.id;
    void id;
    untrack(() => void load());
  });

  function validate(c: K8sMonitorConfig): Record<string, string> {
    const e: Record<string, string> = {};
    if (c.interval_secs < 15 || c.interval_secs > 3600) e.interval_secs = 'Interval must be 15..3600 seconds.';
    if (c.concurrency < 1 || c.concurrency > 32) e.concurrency = 'Concurrency must be 1..32.';
    if (c.retention_days < 1 || c.retention_days > 90) e.retention_days = 'Retention must be 1..90 days.';
    if (c.probes.length > 10) e.probes = 'At most 10 probes.';
    const names = new Set<string>();
    c.probes.forEach((p, i) => {
      if (!p.name.trim()) e[`probe.${i}.name`] = 'Name is required.';
      else if (names.has(p.name.trim())) e[`probe.${i}.name`] = 'Duplicate name.';
      names.add(p.name.trim());
      if (!p.path.startsWith('/')) e[`probe.${i}.path`] = 'Path must start with /.';
      if (p.port !== null && p.port !== undefined && (p.port < 1 || p.port > 65535)) e[`probe.${i}.port`] = 'Port must be 1..65535.';
      (p.mappings ?? []).forEach((m, j) => {
        if (!m.field.trim()) e[`probe.${i}.map.${j}`] = 'Field is required.';
        else if (!m.metric && !m.label) e[`probe.${i}.map.${j}`] = 'Set a metric name or a label.';
        else if (m.metric && !/^[A-Za-z0-9_:.]{1,128}$/.test(m.metric)) e[`probe.${i}.map.${j}`] = 'Metric: letters, digits, _ : . only.';
      });
    });
    c.exclusions.forEach((x, i) => {
      const v = x.kind === 'label' ? x.selector : x.match;
      if (!v.trim()) e[`ex.${i}`] = 'Pattern is required.';
    });
    if (NEEDS_NS && c.namespaces.length === 0) e.namespaces = 'This cluster has no default namespace — list the namespaces to monitor.';
    return e;
  }

  function syncNamespaces(): void {
    if (!cfg) return;
    cfg.namespaces = nsText
      .split(/[,\s]+/)
      .map((s) => s.trim())
      .filter(Boolean);
  }

  async function save(): Promise<void> {
    if (!cfg) return;
    syncNamespaces();
    errors = validate(cfg);
    if (Object.keys(errors).length) {
      toasts.error('Check the form', Object.values(errors)[0]);
      return;
    }
    saving = true;
    try {
      const r = await k8sApi.monitorSave(cluster.id, cfg);
      cfg = r.config;
      status = r.status;
      nsText = r.config.namespaces.join(', ');
      toasts.success('Monitoring saved', r.config.enabled ? 'The collector picks the change up within 15 s.' : 'Monitoring is off for this cluster.');
      onsaved?.(r.config, r.status);
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }

  async function runNow(): Promise<void> {
    running = true;
    try {
      status = await k8sApi.monitorRun(cluster.id);
      toasts.success('Cycle finished', collectorLine(status, true));
      if (cfg) onsaved?.(cfg, status);
    } catch (e) {
      toasts.error('Cycle failed', e instanceof Error ? e.message : String(e));
    } finally {
      running = false;
    }
  }

  async function runTest(): Promise<void> {
    testBusy = true;
    testError = '';
    testResult = null;
    try {
      // Test what is on screen: save first so the daemon sees the same probes.
      if (cfg) {
        syncNamespaces();
        const r = await k8sApi.monitorSave(cluster.id, cfg);
        cfg = r.config;
        status = r.status;
      }
      testResult = await k8sApi.monitorTest(cluster.id, { pod: testPod.trim() || undefined });
    } catch (e) {
      testError = e instanceof Error ? e.message : String(e);
    } finally {
      testBusy = false;
    }
  }

  // --- probe editing -----------------------------------------------------------
  function addProbe(): void {
    if (!cfg) return;
    cfg.probes = [...cfg.probes, { name: `probe${cfg.probes.length + 1}`, port: null, path: '/metrics', format: 'prometheus', mappings: [], include: [], exclude: [], timeout_ms: 3000 }];
  }
  function removeProbe(i: number): void {
    if (!cfg) return;
    cfg.probes = cfg.probes.filter((_, j) => j !== i);
  }
  function applyPreset(p: K8sMonitorPreset): void {
    if (!cfg) return;
    cfg.probes = p.probes.map((x) => ({ ...x, mappings: [...(x.mappings ?? [])], include: [...(x.include ?? [])], exclude: [...(x.exclude ?? [])] }));
    toasts.info('Preset applied', `${p.probes.length} probe(s) from “${p.title}”. Adjust ports if your services differ.`);
  }
  function presetMenu(e: MouseEvent): void {
    ctxMenu.show(
      e,
      presets.map((p) => ({ label: p.title, icon: 'layers', action: () => applyPreset(p) })),
    );
  }
  function formatMenu(e: MouseEvent, p: K8sProbe): void {
    ctxMenu.show(
      e,
      FORMATS.map((f) => ({ label: f, action: () => (p.format = f) })),
    );
  }
  function addMapping(p: K8sProbe): void {
    p.mappings = [...(p.mappings ?? []), { field: '', metric: '', label: null, unit: 'number' }];
  }
  function removeMapping(p: K8sProbe, j: number): void {
    p.mappings = (p.mappings ?? []).filter((_, k) => k !== j);
  }
  function unitMenu(e: MouseEvent, m: K8sMapping): void {
    ctxMenu.show(
      e,
      UNITS.map((u) => ({ label: u, action: () => (m.unit = u) })),
    );
  }
  function globs(list: string[] | undefined): string {
    return (list ?? []).join(', ');
  }
  function setGlobs(p: K8sProbe, key: 'include' | 'exclude', text: string): void {
    p[key] = text
      .split(/[,\s]+/)
      .map((s) => s.trim())
      .filter(Boolean);
  }

  // --- exclusions ---------------------------------------------------------------
  function addExclusion(kind: K8sExclusion['kind']): void {
    if (!cfg) return;
    const x: K8sExclusion = kind === 'label' ? { kind: 'label', selector: '' } : { kind, match: '' };
    cfg.exclusions = [...cfg.exclusions, x];
  }
  function exclusionMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Pod name glob', action: () => addExclusion('pod') },
      { label: 'Workload glob (kind:name)', action: () => addExclusion('workload') },
      { label: 'Namespace glob', action: () => addExclusion('namespace') },
      { label: 'Label selector', action: () => addExclusion('label') },
    ]);
  }
  function removeExclusion(i: number): void {
    if (!cfg) return;
    cfg.exclusions = cfg.exclusions.filter((_, j) => j !== i);
  }
  function transportMenu(e: MouseEvent): void {
    if (!cfg) return;
    const c = cfg;
    ctxMenu.show(e, [
      { label: 'auto — try the API-server proxy, else port-forward', action: () => (c.transport = 'auto') },
      { label: 'proxy — pods/proxy through the API server', action: () => (c.transport = 'proxy') },
      { label: 'port_forward — kubectl port-forward per pod', action: () => (c.transport = 'port_forward') },
    ]);
  }
</script>

<div class="settings" data-testid="k8s-monitor-settings">
  {#if loading && !cfg}
    <Skeleton rows={6} height={40} />
  {:else if loadError && !cfg}
    <div class="error">{loadError} <button class="btn small" onclick={() => void load()}>Retry</button></div>
  {:else if cfg}
    <section class="card block">
      <div class="row between">
        <label class="toggle">
          <input type="checkbox" bind:checked={cfg.enabled} disabled={!canEdit} data-testid="k8s-monitor-enabled" />
          <span class="strong">Monitoring {cfg.enabled ? 'on' : 'off'}</span>
        </label>
        <div class="row">
          {#if canEdit}
            <button class="btn small" onclick={() => void runNow()} disabled={running || saving} title="Run one collection cycle now"><Icon name="play" size={12} /> {running ? 'Running…' : 'Run once'}</button>
            <button class="btn small" onclick={() => { testOpen = true; testResult = null; testError = ''; }} disabled={!cfg.probes.length}><Icon name="zap" size={12} /> Test probes</button>
            <button class="btn small primary" onclick={() => void save()} disabled={saving} data-testid="k8s-monitor-save">{saving ? 'Saving…' : 'Save'}</button>
          {/if}
        </div>
      </div>
      <div class="dim status">{collectorLine(status, cfg.enabled)}</div>
      {#if rbacMessage(status?.metrics_server)}
        <div class="rbac"><strong>metrics-server denied.</strong> Ask your cluster admin for: <code>{rbacMessage(status?.metrics_server)}</code></div>
      {/if}
      <div class="grid3">
        <label class="field">
          <span>Interval (s)</span>
          <input class="input" type="number" min="15" max="3600" bind:value={cfg.interval_secs} disabled={!canEdit} data-testid="k8s-monitor-interval" />
          {#if errors.interval_secs}<em class="err">{errors.interval_secs}</em>{/if}
        </label>
        <label class="field">
          <span>Concurrency</span>
          <input class="input" type="number" min="1" max="32" bind:value={cfg.concurrency} disabled={!canEdit} />
          {#if errors.concurrency}<em class="err">{errors.concurrency}</em>{/if}
        </label>
        <label class="field">
          <span>Retention (days)</span>
          <input class="input" type="number" min="1" max="90" bind:value={cfg.retention_days} disabled={!canEdit} />
          {#if errors.retention_days}<em class="err">{errors.retention_days}</em>{/if}
        </label>
      </div>
      <div class="grid2">
        <label class="field">
          <span>Namespaces {#if !NEEDS_NS}<span class="dim">(blank = {cluster.default_namespace})</span>{/if}</span>
          <input class="input" placeholder={NEEDS_NS ? 'required: e.g. groove, groove-jobs' : cluster.default_namespace ?? ''} bind:value={nsText} onblur={syncNamespaces} disabled={!canEdit} data-testid="k8s-monitor-namespaces" />
          {#if errors.namespaces}<em class="err">{errors.namespaces}</em>{/if}
        </label>
        <div class="field">
          <span>Transport</span>
          <button class="input picker" onclick={transportMenu} disabled={!canEdit}>{cfg.transport} <Icon name="dot" size={10} /></button>
        </div>
      </div>
    </section>

    <section class="card block">
      <div class="row between">
        <h3>Probes <span class="dim">({cfg.probes.length}/10)</span></h3>
        {#if canEdit}
          <div class="row">
            <button class="btn small" onclick={presetMenu} data-testid="k8s-monitor-preset"><Icon name="layers" size={12} /> Preset</button>
            <button class="btn small" onclick={addProbe} disabled={cfg.probes.length >= 10}><Icon name="plus" size={12} /> Probe</button>
          </div>
        {/if}
      </div>
      <p class="dim help">Each probe is an HTTP GET on every running pod (port defaults to the container's first declared port). <b>json</b> probes map fields to metrics or labels, <b>prometheus</b> probes ingest the text format (globs bound cardinality), <b>health</b> probes record <code>up</code>.</p>
      {#if !cfg.probes.length}
        <div class="dim">No probes yet — pick a preset or add one.</div>
      {/if}
      {#each cfg.probes as p, i (i)}
        <div class="probe" data-testid="k8s-monitor-probe">
          <div class="grid-probe">
            <label class="field"><span>Name</span><input class="input" bind:value={p.name} disabled={!canEdit} />{#if errors[`probe.${i}.name`]}<em class="err">{errors[`probe.${i}.name`]}</em>{/if}</label>
            <label class="field"><span>Port</span><input class="input" type="number" placeholder="first" bind:value={p.port} disabled={!canEdit} />{#if errors[`probe.${i}.port`]}<em class="err">{errors[`probe.${i}.port`]}</em>{/if}</label>
            <label class="field wide"><span>Path</span><input class="input mono" bind:value={p.path} disabled={!canEdit} />{#if errors[`probe.${i}.path`]}<em class="err">{errors[`probe.${i}.path`]}</em>{/if}</label>
            <div class="field"><span>Format</span><button class="input picker" onclick={(e) => formatMenu(e, p)} disabled={!canEdit}>{p.format}</button></div>
            <label class="field"><span>Timeout (ms)</span><input class="input" type="number" min="100" max="30000" bind:value={p.timeout_ms} disabled={!canEdit} /></label>
            {#if canEdit}<button class="icon-btn del" onclick={() => removeProbe(i)} title="Remove probe" aria-label="Remove probe"><Icon name="trash" size={13} /></button>{/if}
          </div>
          {#if p.format === 'json'}
            <div class="sub-block">
              <div class="row between"><span class="dim small">Field mappings</span>{#if canEdit}<button class="btn small ghost" onclick={() => addMapping(p)}><Icon name="plus" size={11} /> Mapping</button>{/if}</div>
              {#each p.mappings ?? [] as m, j (j)}
                <div class="map">
                  <input class="input mono" placeholder="memory_stats.sys" bind:value={m.field} disabled={!canEdit} title="Dotted path; numbers index arrays" />
                  <input class="input mono" placeholder="metric name" bind:value={m.metric} disabled={!canEdit} />
                  <input class="input mono" placeholder="or label" bind:value={m.label} disabled={!canEdit} />
                  <button class="input picker" onclick={(e) => unitMenu(e, m)} disabled={!canEdit}>{m.unit ?? 'number'}</button>
                  {#if canEdit}<button class="icon-btn" onclick={() => removeMapping(p, j)} aria-label="Remove mapping"><Icon name="x" size={12} /></button>{/if}
                  {#if errors[`probe.${i}.map.${j}`]}<em class="err span">{errors[`probe.${i}.map.${j}`]}</em>{/if}
                </div>
              {/each}
            </div>
          {:else if p.format === 'prometheus'}
            <div class="sub-block grid2">
              <label class="field"><span>Include globs <span class="dim">(blank = all)</span></span><input class="input mono" value={globs(p.include)} oninput={(e) => setGlobs(p, 'include', e.currentTarget.value)} placeholder="http_*, process_*" disabled={!canEdit} /></label>
              <label class="field"><span>Exclude globs</span><input class="input mono" value={globs(p.exclude)} oninput={(e) => setGlobs(p, 'exclude', e.currentTarget.value)} placeholder="*_bucket" disabled={!canEdit} /></label>
            </div>
          {/if}
        </div>
      {/each}
    </section>

    <section class="card block">
      <div class="row between">
        <h3>Exclusions <span class="dim">({cfg.exclusions.length})</span></h3>
        {#if canEdit}<button class="btn small" onclick={exclusionMenu} data-testid="k8s-monitor-add-exclusion"><Icon name="plus" size={12} /> Exclusion</button>{/if}
      </div>
      <p class="dim help">Excluded pods are still counted (phase, restarts) but never scraped. Globs use <code>*</code> and <code>?</code>; workload globs match <code>kind:name</code> (e.g. <code>cronjob:*</code>).</p>
      {#each cfg.exclusions as x, i (i)}
        <div class="ex">
          <span class="chip">{x.kind}</span>
          {#if x.kind === 'label'}
            <input class="input mono" placeholder="app=frb,tier!=web" bind:value={x.selector} disabled={!canEdit} />
          {:else}
            <input class="input mono" placeholder={x.kind === 'workload' ? 'cronjob:*' : '*-confsrv-*'} bind:value={x.match} disabled={!canEdit} />
          {/if}
          {#if canEdit}<button class="icon-btn" onclick={() => removeExclusion(i)} aria-label="Remove exclusion"><Icon name="x" size={12} /></button>{/if}
          {#if errors[`ex.${i}`]}<em class="err span">{errors[`ex.${i}`]}</em>{/if}
        </div>
      {/each}
    </section>
  {/if}
</div>

{#if testOpen && cfg}
  <Modal title="Test probes" width={720} onclose={() => (testOpen = false)}>
    <div class="test">
      <div class="row">
        <input class="input mono" placeholder="pod name (blank = first running pod)" bind:value={testPod} />
        <button class="btn primary" onclick={() => void runTest()} disabled={testBusy}>{testBusy ? 'Testing…' : 'Run'}</button>
      </div>
      <p class="dim help">Saves the current form, then fetches every probe from one pod and shows what parsed. Nothing is written to the metrics store.</p>
      {#if testError}<div class="error">{testError}</div>{/if}
      {#if testResult}
        <div class="dim small">Pod <b class="mono">{testResult.namespace}/{testResult.pod}</b> · transport <b>{testResult.transport}</b> · metrics-server <b>{testResult.metrics_server.split(':')[0]}</b></div>
        {#each testResult.probes as pr (pr.name)}
          <div class="tp" class:bad={!pr.ok} data-testid="k8s-monitor-test-probe">
            <div class="row between">
              <span><b>{pr.name}</b> {#if pr.port}<span class="dim">:{pr.port}</span>{/if}</span>
              <span class="dim small">{pr.ok ? `HTTP ${pr.status} · ${pr.ms} ms · ${pr.sample_count ?? 0} sample(s)` : (pr.error ?? `HTTP ${pr.status}`)}{#if pr.parse_errors} · {pr.parse_errors} parse error(s){/if}{#if pr.capped} · capped{/if}</span>
            </div>
            {#if pr.labels && Object.keys(pr.labels).length}
              <div class="row wrap">{#each Object.entries(pr.labels) as [k, v] (k)}<span class="chip accent">{k}={v}</span>{/each}</div>
            {/if}
            {#if pr.samples?.length}
              <table class="samples">
                <tbody>
                  {#each pr.samples as s, i (i)}
                    <tr><td class="mono">{s.metric}</td><td class="mono dim">{Object.entries(s.labels).map(([k, v]) => `${k}="${v}"`).join(' ')}</td><td class="mono num">{s.value}</td></tr>
                  {/each}
                </tbody>
              </table>
            {:else if pr.ok && pr.body_preview}
              <pre class="preview">{pr.body_preview}</pre>
            {/if}
          </div>
        {/each}
      {/if}
    </div>
  </Modal>
{/if}

<style>
  .settings {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .block {
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h3 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .row.between {
    justify-content: space-between;
  }
  .row.wrap {
    flex-wrap: wrap;
  }
  .toggle {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
  }
  .strong {
    font-weight: 600;
  }
  .status {
    font-size: 11.5px;
  }
  .rbac {
    font-size: 11.5px;
    padding: 8px;
    border-radius: 6px;
    background: color-mix(in srgb, orange 8%, var(--surface));
    border: 1px solid color-mix(in srgb, orange 30%, var(--border));
  }
  .rbac code {
    display: block;
    margin-top: 4px;
    font-family: var(--font-mono);
    font-size: 10.5px;
    white-space: pre-wrap;
    word-break: break-word;
    user-select: all;
  }
  .grid2 {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
  }
  .grid3 {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 10px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: 11.5px;
    min-width: 0;
  }
  .field > span {
    color: var(--text-dim);
  }
  .picker {
    text-align: left;
    cursor: pointer;
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .err {
    color: var(--status-exited);
    font-style: normal;
    font-size: 11px;
  }
  .err.span {
    grid-column: 1 / -1;
  }
  .help {
    margin: 0;
    font-size: 11.5px;
  }
  .probe {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .grid-probe {
    display: grid;
    grid-template-columns: 1.2fr 0.7fr 2fr 1fr 0.9fr auto;
    gap: 8px;
    align-items: end;
  }
  .del {
    margin-bottom: 4px;
  }
  .sub-block {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-top: 6px;
    border-top: 1px dashed var(--border);
  }
  .map {
    display: grid;
    grid-template-columns: 1.6fr 1.2fr 1fr 1fr auto;
    gap: 6px;
    align-items: center;
  }
  .ex {
    display: grid;
    grid-template-columns: auto 1fr auto;
    gap: 8px;
    align-items: center;
  }
  .small {
    font-size: 11px;
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }
  .error {
    color: var(--status-exited);
    font-size: 12px;
  }
  .test {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-height: 70vh;
    overflow: auto;
  }
  .test .row .input {
    flex: 1;
  }
  .tp {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 8px 10px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 12px;
  }
  .tp.bad {
    border-color: color-mix(in srgb, var(--status-exited) 40%, var(--border));
  }
  .samples {
    width: 100%;
    border-collapse: collapse;
    font-size: 11px;
  }
  .samples td {
    padding: 2px 6px;
    border-top: 1px solid var(--border);
    vertical-align: top;
    word-break: break-all;
  }
  .samples .num {
    text-align: right;
    white-space: nowrap;
  }
  .preview {
    margin: 0;
    font-size: 10.5px;
    white-space: pre-wrap;
    word-break: break-all;
    max-height: 120px;
    overflow: auto;
    color: var(--text-dim);
  }
  @media (max-width: 760px) {
    .grid3,
    .grid2,
    .grid-probe,
    .map {
      grid-template-columns: 1fr;
    }
  }
</style>
