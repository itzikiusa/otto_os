<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../../../lib/api/client';
  import { loadErrorText } from '../../../lib/loadError';
  import { pollWhileVisible } from '../../../lib/poll';
  import { announceShown } from '../../../lib/announce.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import type {
    ConfirmPassiveAudit, PassiveAuditTarget, ProjectPassiveAudit, ProjectScans, ScanSettings, SecurityCheck, SecurityFinding, SecurityManageAction,
    SecurityScanDetail, SecuritySchedule,
  } from '../../../lib/api/types';
  import { confirmer } from '../../../lib/confirm.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import PageBody from '../../../lib/components/PageBody.svelte';
  import PageHeader from '../../../lib/components/PageHeader.svelte';
  import type { IdeDiagnostic } from '../store/ideTypes';
  import { diagnostics, latestFileScan, projectSecurityPath } from './projectSecurity.svelte';

  interface Props {
    projectId: string;
    projectName: string;
    onBack: () => void;
    onOpen: (finding: SecurityFinding) => void;
    onDraft: (finding: SecurityFinding) => void;
    onDiagnostics: (items: IdeDiagnostic[]) => void;
  }

  let { projectId, projectName, onBack, onOpen, onDraft, onDiagnostics }: Props = $props();
  const checkLabels: [SecurityCheck, string][] = [
    ['vulnerability', 'Dependencies'], ['secret', 'Secrets'], ['misconfiguration', 'Misconfiguration'],
    ['license', 'Licenses'], ['sast', 'Source analysis'],
  ];
  let overview = $state<ProjectScans | null>(null);
  let detail = $state<SecurityScanDetail | null>(null);
  let selected = $state('');
  let settings = $state<ScanSettings | null>(null);
  let loading = $state(true);
  let error = $state('');
  let acting = $state(false);
  let showSettings = $state(false);
  let showSuppressed = $state(false);
  let severity = $state('all');
  let limit = $state(100);
  let cron = $state('');
  let timezone = $state('UTC');
  let excludes = $state('');
  let schedulePreview = $state<SecuritySchedule | null>(null);
  let passive = $state<ProjectPassiveAudit | null>(null);
  let passiveConfig = $state('');
  let passiveScheduled = $state(false);
  let passiveError = $state('');
  let passiveLoading = $state(false);
  let importInput = $state<HTMLInputElement>();
  let request = 0;
  let alive = true;

  const base = $derived(projectSecurityPath(projectId));
  const passiveCandidate = $derived(passive?.candidates.find((candidate) => candidate.run_config_id === passiveConfig));
  const passiveRunReady = $derived(!!passive?.confirmed && passive.candidates.some((candidate) =>
    candidate.run_config_id === passive?.confirmed?.run_config_id && candidate.url === passive?.confirmed?.url));
  const active = $derived(overview?.scans.find((scan) => scan.status === 'running' || scan.status === 'queued'));
  const visible = $derived((detail?.findings ?? []).filter((finding) =>
    (severity === 'all' || finding.severity === severity)
    && (showSuppressed || !detail?.annotations.suppressions[finding.fingerprint])));
  const existing = $derived(new Set(detail?.annotations.existing));
  const baseline = $derived(new Set(detail?.annotations.baseline));
  const completed = $derived((overview?.scans ?? []).filter((scan) => scan.status === 'completed'));
  const maxTrend = $derived(Math.max(1, ...completed.slice(0, 8).map((scan) => scan.count)));
  const when = (date: string, timeZone?: string) => new Date(date).toLocaleString(undefined, { timeZone });

  async function refresh(preferred?: string, signal?: AbortSignal): Promise<void> {
    const current = ++request;
    try {
      const next = await api.get<ProjectScans>(base, signal);
      if (!alive || current !== request) return;
      const id = preferred || selected;
      const choice = next.scans.find((item) => item.id === id) ?? next.scans[0];
      const last = latestFileScan(next.scans);
      overview = next;
      if (!settings) {
        settings = next.settings;
        excludes = next.settings.excludes.join('\n');
        cron = next.schedule?.cron ?? '';
        timezone = next.schedule?.timezone ?? Intl.DateTimeFormat().resolvedOptions().timeZone ?? 'UTC';
        schedulePreview = next.schedule;
      }
      selected = choice?.id ?? '';
      const scan = choice ? await api.get<SecurityScanDetail>(`${base}/scans/${choice.id}`, signal) : null;
      const diagnosticScan = !last ? null : last.id === scan?.scan.id ? scan
        : await api.bg.get<SecurityScanDetail>(`${base}/scans/${last.id}`, signal);
      if (!alive || current !== request) return;
      detail = scan;
      onDiagnostics(diagnosticScan ? diagnostics(diagnosticScan) : []);
      error = '';
    } catch (e) {
      if (alive && current === request) error = loadErrorText(e);
    } finally {
      if (alive && current === request) loading = false;
    }
  }

  onMount(() => {
    void refreshPassive();
    const poller = pollWhileVisible(async (signal) => {
      if (!overview || (active || overview.schedule) && !acting) await refresh(undefined, signal);
    }, { ms: 3500 });
    return () => {
      alive = false;
      poller.stop();
    };
  });

  async function action(run: () => Promise<void>) {
    acting = true;
    error = '';
    try { await run(); }
    catch (e) { error = loadErrorText(e); }
    finally { acting = false; }
  }

  function manage<T>(command: SecurityManageAction): Promise<T> {
    return api.post<T>(`${base}/manage`, command);
  }

  async function refreshPassive() {
    if (passiveLoading) return;
    passiveLoading = true;
    try {
      const result = await api.bg.get<ProjectPassiveAudit>(`${base}/dast`);
      if (!alive) return;
      passive = result;
      passiveConfig = result.confirmed?.run_config_id ?? result.candidates[0]?.run_config_id ?? '';
      passiveScheduled = result.confirmed?.scheduled ?? false;
      passiveError = '';
    } catch (e) {
      if (alive) passiveError = loadErrorText(e);
    } finally {
      if (alive) passiveLoading = false;
    }
  }

  async function confirmPassive() {
    const candidate = passiveCandidate;
    if (!candidate) return;
    const scheduled = passiveScheduled;
    const accepted = await confirmer.ask([
      `Audit destination ${candidate.url}`,
      'Otto sends one HTTP HEAD request without credentials, cookies, or a request body. Redirects are disabled.',
      candidate.public ? 'This is a public destination.' : '',
      scheduled ? 'Also allow scheduled audits while the saved run configuration is running.' : '',
    ].filter(Boolean).join('\n\n'), {
      title: candidate.public ? 'Confirm public audit target' : 'Confirm passive audit target',
      confirmLabel: candidate.public ? 'Confirm public URL' : 'Confirm URL',
    });
    if (!accepted) return;
    const command: ConfirmPassiveAudit = {
      run_config_id: candidate.run_config_id, url: candidate.url,
      allow_public: candidate.public, scheduled,
    };
    await action(async () => {
      await api.put<PassiveAuditTarget>(`${base}/dast`, command);
      await refreshPassive();
    });
  }

  function revokePassive() {
    void action(async () => {
      await api.del(`${base}/dast`);
      await refreshPassive();
    });
  }

  function runPassive() {
    void action(async () => {
      const scan = await api.post<{ id: string }>(`${base}/dast/run`);
      await refresh(scan.id);
    });
  }

  function scanNow() {
    void action(async () => {
      const result = await api.post<{ id: string }>(`${base}/scans`);
      await refresh(result.id);
    });
  }

  function cancel() {
    if (!active) return;
    const id = active.id;
    void action(async () => {
      await api.del(`${base}/scans/${id}`);
      await refresh(id);
    });
  }

  function changeCheck(check: SecurityCheck, checked: boolean) {
    if (!settings) return;
    settings.checks = checked ? [...settings.checks, check] : settings.checks.filter((item) => item !== check);
  }

  function preview() {
    void action(async () => {
      schedulePreview = await manage<SecuritySchedule>({ action: 'preview_schedule', schedule: { cron, timezone } });
    });
  }

  function saveSettings(event: SubmitEvent) {
    event.preventDefault();
    if (!settings) return;
    const config = { ...settings, excludes: excludes.split('\n').map((line) => line.trim()).filter(Boolean) };
    const schedule = cron.trim() ? { cron, timezone } : null;
    void action(async () => {
      if (schedule) await manage({ action: 'preview_schedule', schedule });
      settings = await api.put<ScanSettings>(base, config);
      schedulePreview = await manage<SecuritySchedule | null>({ action: 'set_schedule', schedule });
      await refresh();
      if (!error) showSettings = false;
    });
  }

  async function suppress(finding: SecurityFinding) {
    const old = detail?.annotations.suppressions[finding.fingerprint];
    const reason = old ? null : await confirmer.promptText('Why should this finding be suppressed?', {
      title: 'Suppress finding', confirmLabel: 'Suppress',
      validate: (value) => value.trim() && new TextEncoder().encode(value).length <= 800
        ? null : 'Enter a reason of 1–800 bytes.',
    });
    if (!old && reason === null) return;
    await action(async () => {
      await manage({ action: 'set_suppression', fingerprint: finding.fingerprint, reason });
      await refresh();
    });
  }

  function selectBaseline() {
    if (detail?.scan.status !== 'completed') return;
    const scan_id = overview?.baseline === detail.scan.id ? null : detail.scan.id;
    void action(async () => {
      await manage({ action: 'set_baseline', scan_id });
      await refresh();
    });
  }

  function exportSarif() {
    if (detail?.scan.status !== 'completed') return;
    const id = detail.scan.id;
    void action(async () => {
      const sarif = await api.get<unknown>(`${base}/scans/${id}/sarif`);
      const url = URL.createObjectURL(new Blob([JSON.stringify(sarif, null, 2)], { type: 'application/sarif+json' }));
      const link = document.createElement('a');
      link.href = url;
      link.download = `otto-security-${id}.sarif`;
      link.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    });
  }

  function importSarif(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    void action(async () => {
      if (file.size > 16 * 1024 * 1024) throw new Error('Choose a SARIF file smaller than 16 MiB.');
      const sarif: unknown = JSON.parse(await file.text());
      const result = await manage<{ scan_id: string }>({ action: 'import_sarif', sarif });
      await refresh(result.scan_id);
    });
  }
</script>

<PageHeader title="Security" subtitle={projectName} icon="shield">
  {#snippet leading()}
    <button type="button" class="btn small ghost" onclick={onBack}>Back to IDE</button>
  {/snippet}
  {#snippet actions()}
    <button type="button" class="btn small ghost" onclick={() => {
      showSettings = !showSettings;
      if (showSettings) void refreshPassive();
    }} disabled={!settings || acting} aria-expanded={showSettings}>Settings</button>
    {#if passiveRunReady}
      <button type="button" class="btn small ghost" onclick={runPassive} disabled={acting || passiveLoading || !!active}>Run passive audit</button>
    {/if}
    <button type="button" class="btn small ghost" onclick={() => importInput?.click()} disabled={acting}>Import SARIF</button>
    <button type="button" class="btn small ghost" onclick={exportSarif} disabled={acting || detail?.scan.status !== 'completed'}>Export SARIF</button>
    <button type="button" class="btn small ghost" onclick={selectBaseline} disabled={acting || detail?.scan.status !== 'completed'}>
      {overview?.baseline && overview.baseline === detail?.scan.id ? 'Clear baseline' : 'Set baseline'}
    </button>
    {#if active}
      <button type="button" class="btn small" onclick={cancel} disabled={acting}>Cancel scan</button>
    {:else if overview?.scans.length}
      <button type="button" class="btn small primary" onclick={scanNow} disabled={acting}>Scan now</button>
    {/if}
  {/snippet}
</PageHeader>

<PageBody width="full" fill>
  <input hidden type="file" accept=".sarif,.json,application/sarif+json" bind:this={importInput} onchange={importSarif} />
  <div class="security" data-testid="ide-security-page">
    {#if loading}
      <LoadState what="project security" variant="page" loading empty />
    {:else if error && !overview}
      <EmptyState variant="page" headingLevel={2} tone="error" title="Couldn’t load security scans"
        body={error} actionLabel="Retry" onaction={() => { loading = true; void refresh(); }} />
    {:else}
      {#if error}<p class="error" use:announceShown={{ text: error, assertive: true }}>{error}</p>{/if}
      {#if showSettings && settings}
        <form class="settings" onsubmit={saveSettings}>
          <fieldset>
            <legend>Checks</legend>
            <div class="checks">
              {#each checkLabels as [check, label] (check)}
                <label><input type="checkbox" checked={settings.checks.includes(check)}
                  onchange={(event) => changeCheck(check, event.currentTarget.checked)} /> {label}</label>
              {/each}
            </div>
          </fieldset>
          <div class="settings-grid">
            <label>Workers<input type="number" min="1" max="2" required bind:value={settings.parallelism} /></label>
            <label>Memory limit (MiB)<input type="number" min="256" max="4096" required bind:value={settings.memory_mib} /></label>
            <label>Time limit (seconds)<input type="number" min="10" max="900" required bind:value={settings.timeout_secs} /></label>
          </div>
          <label>Excludes, one relative glob per line<textarea dir="ltr" rows="3" bind:value={excludes}></textarea></label>
          <div class="settings-grid">
            <label>Cron schedule<input dir="ltr" placeholder="0 9 * * 1-5" bind:value={cron} /></label>
            <label>Timezone<input dir="ltr" placeholder="UTC" bind:value={timezone} /></label>
          </div>
          <div class="actions">
            <button type="button" class="btn small ghost" onclick={preview} disabled={acting || !cron.trim()}>Preview next runs</button>
            <button type="submit" class="btn small" disabled={acting}>Save settings</button>
          </div>
          {#if schedulePreview?.next_runs.length}
            <ul class="moments" aria-label="Next scheduled runs">
              {#each schedulePreview.next_runs as run (run)}<li>{when(run, schedulePreview.timezone)}</li>{/each}
            </ul>
          {/if}
        </form>
        <section class="settings" aria-label="Passive HTTP audit">
          <h2>Passive HTTP audit</h2>
          {#if !passiveLoading && passiveError}
            <LoadState what="audit targets" variant="compact" error={passiveError} empty onretry={() => void refreshPassive()} />
          {:else if passiveLoading || !passive}
            <LoadState what="audit targets" variant="compact" loading empty />
          {:else if passive.candidates.length}
            <label>Saved run configuration<select bind:value={passiveConfig}>
              {#each passive.candidates as candidate (candidate.run_config_id)}
                <option value={candidate.run_config_id}>{candidate.run_config_name}</option>
              {/each}
            </select></label>
            {#if passiveCandidate}
              <p class="audit-url" dir="ltr">{passiveCandidate.url}</p>
            {/if}
            <label class="audit-toggle"><input type="checkbox" bind:checked={passiveScheduled} /> Include in scheduled scans</label>
            <div class="actions">
              <button type="button" class="btn small" onclick={() => void confirmPassive()} disabled={acting || !passiveCandidate}>
                {passive?.confirmed ? 'Update confirmation' : 'Review and confirm'}
              </button>
              {#if passive?.confirmed}
                <button type="button" class="btn small ghost" onclick={revokePassive} disabled={acting}>Revoke confirmation</button>
              {/if}
            </div>
            {#if passive?.confirmed}
              <p class="muted">Confirmed {passive.confirmed.url} · {when(passive.confirmed.confirmed_at)}</p>
            {/if}
          {:else}
            <p class="muted">Set OTTO_SECURITY_URL in a saved run configuration for this project.</p>
            {#if passive.confirmed}
              <button type="button" class="btn small ghost" onclick={revokePassive} disabled={acting}>Revoke confirmation</button>
            {/if}
          {/if}
        </section>
      {/if}

      {#if overview?.schedule && !showSettings}
        <p class="muted">Next scan {overview.schedule.next_runs[0] ? when(overview.schedule.next_runs[0], overview.schedule.timezone) : 'unavailable'} · {overview.schedule.timezone}</p>
      {/if}
      {#if overview?.engine || overview?.opengrep}
        <div class="engine-list">
          {#each [['Trivy', overview.engine], ['Opengrep', overview.opengrep]] as [name, metadata] (name)}
            {#if typeof metadata === 'object' && metadata}
              <span>{name} {metadata.version} · updated {when(metadata.updated_at)}</span>
            {/if}
          {/each}
        </div>
      {/if}

      {#if !overview?.scans.length}
        <EmptyState variant="page" headingLevel={2} icon="shield" title="No security scans yet"
          body="Run a scan to review findings and build a project history." actionLabel="Scan now" onaction={scanNow} />
      {:else}
        <div class="scan-controls">
          <label>Scan<select value={selected} onchange={(event) => { limit = 100; void refresh(event.currentTarget.value); }}>
            {#each overview.scans as scan (scan.id)}
              <option value={scan.id}>{when(scan.created_at)} · {scan.status} · {scan.count} findings</option>
            {/each}
          </select></label>
          {#if detail?.scan.status === 'completed'}
            <label>Severity<select bind:value={severity}>
              <option value="all">All severities</option>
              {#each ['critical', 'high', 'medium', 'low', 'unknown'] as level (level)}
                <option value={level}>{level}</option>
              {/each}
            </select></label>
            <label class="toggle"><input type="checkbox" bind:checked={showSuppressed} /> Show suppressed</label>
          {/if}
        </div>

        {#if detail?.scan.status === 'failed'}
          <EmptyState tone="error" title="Scan failed" body={detail.scan.error ?? 'Start a new scan to retry.'} />
        {:else if detail?.scan.status === 'queued' || detail?.scan.status === 'running'}
          {@const message = detail.scan.status === 'queued' ? 'Waiting for the scan queue and machine load' : 'Scanning project'}
          <p use:announceShown={message}>{message}</p>
        {:else if detail?.scan.status === 'cancelled'}
          <p use:announceShown={'Scan canceled'}>Scan canceled</p>
        {:else if detail}
          {#if detail.scan.engines.length}
            <p class="muted">{detail.scan.engines.map((run) =>
              `${run.name} · ${run.mode} · ${(run.metrics.wall_ms / 1000).toFixed(1)} s`).join(' / ')}</p>
          {/if}
          {#if visible.length === 0}
            <EmptyState icon="shield" title={detail.findings.length ? 'No matching findings' : 'No findings in this scan'} />
          {:else}
            <ul class="findings" aria-label="Security findings">
              {#each visible.slice(0, limit) as finding (finding.fingerprint)}
                {@const reason = detail.annotations.suppressions[finding.fingerprint]}
                <li>
                  <div class="finding-head">
                    {#if finding.check === 'dast'}
                      <span dir="ltr">{finding.path}</span>
                    {:else}
                      <button type="button" class="location" onclick={() => onOpen(finding)}>
                        {finding.path}{finding.line ? `:${finding.line}` : ''}
                      </button>
                    {/if}
                    <span class="severity" data-level={finding.severity}>{finding.severity}</span>
                  </div>
                  <div class="muted">{finding.rule} · {finding.engine} · {existing.has(finding.fingerprint) ? 'Existing' : 'New'}
                    {baseline.has(finding.fingerprint) ? ' · In baseline' : ''}</div>
                  <p>{finding.description || finding.title}</p>
                  {#if finding.fix}<p class="muted">{finding.fix}</p>{/if}
                  {#if reason}<p class="muted">Suppressed · {reason}</p>{/if}
                  <div class="actions">
                    <button type="button" class="btn small ghost" onclick={() => onDraft(finding)}>Ask an agent to fix</button>
                    <button type="button" class="btn small ghost" onclick={() => void suppress(finding)} disabled={acting}>
                      {reason ? 'Restore finding' : 'Suppress'}
                    </button>
                  </div>
                </li>
              {/each}
            </ul>
            {#if visible.length > limit}
              <button type="button" class="btn small" onclick={() => limit += 100}>Show more findings</button>
            {/if}
          {/if}
        {/if}

        {#if completed.length > 1}
          <details class="trend"><summary>Finding trend</summary>
            <ul aria-label="Findings in recent completed scans">
              {#each completed.slice(0, 8).reverse() as scan (scan.id)}
                <li><span>{when(scan.created_at)}</span>
                  <meter value={scan.count} min="0" max={maxTrend} aria-label={`${scan.count} findings`}></meter>
                  <span>{scan.count}</span></li>
              {/each}
            </ul>
          </details>
        {/if}
      {/if}
    {/if}
  </div>
</PageBody>

<style>
  .security { display: grid; gap: 12px; min-inline-size: 0; max-inline-size: 980px; margin-inline: auto; }
  .settings, .findings, .trend { border: 1px solid var(--border); border-radius: var(--radius-m); }
  .settings { display: grid; gap: 12px; padding: 12px; }
  .settings fieldset { margin: 0; padding: 8px; border: 1px solid var(--border); }
  .settings-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(130px, 1fr)); gap: 10px; }
  .settings label, .scan-controls label { display: grid; gap: 4px; min-inline-size: 0; }
  .settings input:not([type='checkbox']), .settings textarea, .settings select, .scan-controls select {
    inline-size: 100%; min-inline-size: 0; color: var(--text); background: var(--surface-2);
    border: 1px solid var(--border); border-radius: var(--radius-s); padding: 6px;
  }
  .checks, .scan-controls, .actions, .finding-head, .engine-list { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .checks label, .scan-controls .toggle { display: flex; align-items: center; }
  .scan-controls label:first-child { flex: 1 1 210px; }
  .engine-list, .muted { font-size: var(--fs-s); color: var(--text-dim); }
  .findings, .trend ul { list-style: none; margin: 0; padding: 0; }
  .findings li { padding: 10px 12px; overflow-wrap: anywhere; }
  .findings li + li { border-block-start: 1px solid var(--border); }
  .findings p { margin-block: 6px; }
  .location { padding: 2px; border: 0; background: transparent; color: var(--accent-text); text-align: start; overflow-wrap: anywhere; }
  .severity { font-size: var(--fs-s); text-transform: capitalize; }
  .severity[data-level='high'], .severity[data-level='critical'], .error { color: var(--danger); }
  .severity[data-level='medium'] { color: var(--warning); }
  .trend { padding: 10px; }
  .trend li { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; padding-block: 4px; }
  .trend meter { min-inline-size: 50px; flex: 1; }
  .moments { margin: 0; }
  .settings h2 { margin: 0; font-size: var(--fs-m); }
  .settings .audit-toggle { display: flex; align-items: center; }
  .audit-url { margin: 0; overflow-wrap: anywhere; }
</style>
