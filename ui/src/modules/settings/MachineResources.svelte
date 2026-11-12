<script lang="ts">
  // Settings ▸ Daemon ▸ Machine resources: how many background agents (code
  // reviews, workflow steps, scheduled tasks…) may run at once, when they
  // wait for a busy Mac, how hard their builds may push it, and when free
  // disk is low. Edits the page's form (saved by the page's one Save); the
  // live line reads the same poller as the status-bar chip.
  import SettingToggle from './SettingToggle.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import { machineStatus } from './machineStatus.svelte';
  import { statusLine } from './machineStatusText';
  import {
    DEFAULT_BG_ARCHIVE_DAYS,
    DEFAULT_LOW_DISK_GB,
    DEFAULT_MAX_LOAD_PER_CORE,
    DEFAULT_MAX_WAIT_SECS,
    defaultMaxLive,
    type MachineErrors,
    type MachineForm,
  } from './machineSettings';

  let { form = $bindable(), errors }: { form: MachineForm; errors: MachineErrors } = $props();

  $effect(() => machineStatus.watch());
  const status = $derived(machineStatus.status);
  const autoLive = $derived(
    status ? `${defaultMaxLive(status.cores)} on this Mac` : 'three quarters of the cores, at least 2',
  );
  /** The load the threshold stands for on this Mac, once the cores are known. */
  const loadLimit = $derived.by(() => {
    const per = typeof form.maxLoadPerCore === 'number' && form.maxLoadPerCore > 0 ? form.maxLoadPerCore : DEFAULT_MAX_LOAD_PER_CORE;
    return status ? ` — on ${status.cores} cores, a 1-minute load above ${(status.cores * per).toFixed(1)}` : '';
  });
</script>

<div class="card mr-card" data-testid="machine-resources">
  <!-- Not a live region: it changes every few seconds. -->
  <p class="live" class:warn={status?.busy || status?.low_disk} data-testid="machine-status-line">
    {#if status}
      {statusLine(status)}{status.busy && status.busy_reason ? ` — busy: ${status.busy_reason}` : ''}
    {:else if machineStatus.error}
      Live status unavailable: {machineStatus.error}
    {:else}
      Reading the machine…
    {/if}
  </p>

  <SettingToggle
    label="Queue background agents when this Mac is busy"
    checked={form.admission}
    testid="agent-admission-enabled"
    onchange={(v) => { form.admission = v; }}
  >
    Code reviews, workflow steps, scheduled tasks and other agents Otto starts on its own wait
    while the load or memory pressure is high, then start a few seconds apart. Sessions you start
    never wait. Turning it off starts anything queued at once.
  </SettingToggle>

  <div class="field num">
    <label for="mr-max-live">Background agents running at once</label>
    <input
      id="mr-max-live"
      class="input"
      type="number"
      min="0"
      step="1"
      placeholder="Automatic"
      bind:value={form.maxLive}
      disabled={!form.admission}
      aria-invalid={!!errors.maxLive}
      aria-describedby="mr-max-live-hint{errors.maxLive ? ' mr-max-live-err' : ''}"
      data-testid="agent-background-max-live"
    />
    {#if errors.maxLive}<span class="hint err" id="mr-max-live-err" use:announceShown={errors.maxLive}>{errors.maxLive}</span>{/if}
  </div>
  <p class="hint-line" id="mr-max-live-hint">Blank = automatic ({autoLive}); 0 = no limit. More wait their turn.</p>

  <SettingToggle
    label="Run background agents at lower priority"
    checked={form.lowPriority}
    testid="agent-background-low-priority"
    onchange={(v) => { form.lowPriority = v; }}
  >
    Their builds and tests yield the CPU to you and the app when the Mac is busy, and cost nothing
    when it is idle. On by default.
  </SettingToggle>

  <div class="field num">
    <label for="mr-build">Build workers per agent session</label>
    <div class="mr-row">
      <select id="mr-build" class="input" bind:value={form.buildMode} data-testid="agent-build-parallelism">
        <option value="auto">Automatic</option>
        <option value="fixed">A fixed number</option>
        <option value="off">Off (tools decide)</option>
      </select>
      {#if form.buildMode === 'fixed'}
        <input
          class="input workers"
          type="number"
          min="1"
          step="1"
          aria-label="Build workers"
          bind:value={form.buildWorkers}
          aria-invalid={!!errors.buildWorkers}
          data-testid="agent-build-workers"
        />
      {/if}
    </div>
    {#if errors.buildWorkers}<span class="hint err" use:announceShown={errors.buildWorkers}>{errors.buildWorkers}</span>{/if}
  </div>
  <p class="hint-line">
    How many jobs cargo, go, make and cmake inside an agent session may run. Automatic shares the
    cores between the agents running (at least 2). A value you set in your shell profile still wins.
  </p>

  <div class="field num">
    <label for="mr-bg-archive">Archive finished background agent sessions after (days)</label>
    <input
      id="mr-bg-archive"
      class="input"
      type="number"
      min="0"
      step="1"
      bind:value={form.bgArchiveDays}
      aria-invalid={!!errors.bgArchiveDays}
      aria-describedby="mr-bg-archive-hint{errors.bgArchiveDays ? ' mr-bg-archive-err' : ''}"
      data-testid="session-auto-archive-background-days"
    />
    {#if errors.bgArchiveDays}<span class="hint err" id="mr-bg-archive-err" use:announceShown={errors.bgArchiveDays}>{errors.bgArchiveDays}</span>{/if}
  </div>
  <p class="hint-line" id="mr-bg-archive-hint">
    Default {DEFAULT_BG_ARCHIVE_DAYS}; 0 = never. Reviews, workflow steps and other sessions Otto
    started on its own leave the sidebar once idle this long. Archiving deletes nothing: history
    stays and the session can be unarchived and resumed.
  </p>

  <div class="field num">
    <label for="mr-low-disk">Warn when free disk drops below (GB)</label>
    <input
      id="mr-low-disk"
      class="input"
      type="number"
      min="0"
      step="1"
      placeholder={String(DEFAULT_LOW_DISK_GB)}
      bind:value={form.lowDiskGb}
      aria-invalid={!!errors.lowDiskGb}
      aria-describedby="mr-low-disk-hint{errors.lowDiskGb ? ' mr-low-disk-err' : ''}"
      data-testid="machine-low-disk-gb"
    />
    {#if errors.lowDiskGb}<span class="hint err" id="mr-low-disk-err" use:announceShown={errors.lowDiskGb}>{errors.lowDiskGb}</span>{/if}
  </div>
  <p class="hint-line" id="mr-low-disk-hint">
    Blank = {DEFAULT_LOW_DISK_GB}; 0 = no warning. The status bar and the notifications say so;
    nothing is held back or deleted.
  </p>

  <details class="advanced">
    <summary>Advanced</summary>
    <div class="field num">
      <label for="mr-load">Queue while the load per core is above</label>
      <input
        id="mr-load"
        class="input"
        type="number"
        min="0.1"
        step="0.1"
        placeholder={String(DEFAULT_MAX_LOAD_PER_CORE)}
        bind:value={form.maxLoadPerCore}
        disabled={!form.admission}
        aria-invalid={!!errors.maxLoadPerCore}
        aria-describedby="mr-load-hint{errors.maxLoadPerCore ? ' mr-load-err' : ''}"
        data-testid="agent-admission-max-load-per-core"
      />
      {#if errors.maxLoadPerCore}<span class="hint err" id="mr-load-err" use:announceShown={errors.maxLoadPerCore}>{errors.maxLoadPerCore}</span>{/if}
    </div>
    <p class="hint-line" id="mr-load-hint">
      Blank = {DEFAULT_MAX_LOAD_PER_CORE}. Background agents wait while the load per core is above
      this{loadLimit}. High memory pressure always makes them wait.
    </p>
    <div class="field num">
      <label for="mr-wait">Longest wait for a busy Mac (seconds)</label>
      <input
        id="mr-wait"
        class="input"
        type="number"
        min="0"
        step="1"
        placeholder={String(DEFAULT_MAX_WAIT_SECS)}
        bind:value={form.maxWaitSecs}
        disabled={!form.admission}
        aria-invalid={!!errors.maxWaitSecs}
        aria-describedby="mr-wait-hint{errors.maxWaitSecs ? ' mr-wait-err' : ''}"
        data-testid="agent-admission-max-wait-secs"
      />
      {#if errors.maxWaitSecs}<span class="hint err" id="mr-wait-err" use:announceShown={errors.maxWaitSecs}>{errors.maxWaitSecs}</span>{/if}
    </div>
    <p class="hint-line" id="mr-wait-hint">
      Blank = {DEFAULT_MAX_WAIT_SECS}. After this an agent starts anyway, so nothing waits forever; at
      the running-agents limit it waits four times as long.
    </p>
  </details>
</div>

<style>
  .mr-card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px 16px 14px;
    max-width: var(--settings-col);
  }
  .mr-card .field {
    margin-block: 0;
  }
  .field.num {
    max-width: 360px;
  }
  .mr-row {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .mr-row select {
    flex: 1 1 180px;
    min-width: 0;
  }
  .workers {
    flex: 0 0 96px;
  }
  .live {
    margin: 4px 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }
  .live.warn {
    color: var(--warning);
  }
  .hint-line {
    font-size: var(--fs-xs);
    line-height: 1.5;
    color: var(--text-dim);
    margin: 0;
  }
  .err {
    color: var(--danger);
  }
  .advanced summary {
    cursor: pointer;
    font-size: var(--fs-s);
    font-weight: 600;
  }
  .advanced .field {
    margin-block-start: 8px;
  }
  .advanced .hint-line {
    margin-block-start: 6px;
  }
</style>
