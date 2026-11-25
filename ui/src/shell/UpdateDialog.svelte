<script lang="ts">
  // The Update dialog — opened by the sidebar row, Otto ▸ Check for Updates…
  // and Settings ▸ Updates. One primary button that follows the state
  // (Download and install → Downloading… → Restart to finish); the rarely used
  // choices sit under ⋯. The restart step lists what keeps running and what
  // ends (the daemon's count) and asks once more only when something ends.
  // Release notes are untrusted: rendered through the sanitising markdown
  // renderer; their links open in the browser (the shell's link handler).
  import Modal from '../lib/components/Modal.svelte';
  import Icon from '../lib/components/Icon.svelte';
  import LoadState from '../lib/components/LoadState.svelte';
  import { ctxMenu } from '../lib/contextmenu.svelte';
  import { renderMarkdownGfm } from '../lib/md';
  import { formatBytes } from '../lib/metric-format';
  import { openExternal } from '../lib/external';
  import { announceShown } from '../lib/announce.svelte';
  import { updates, dialogView, restartPlan } from '../lib/stores/updates.svelte';

  const s = $derived(updates.state);
  const view = $derived(dialogView(s, updates.failed, updates.checkError));
  const plan = $derived(restartPlan(updates.impact));
  // Notes come from an unsigned manifest: no remote images (a tracking pixel).
  const notes = $derived(s?.notes ? renderMarkdownGfm(s.notes).replace(/<img\b[^>]*>/gi, '') : '');
  const meta = $derived(
    [s?.date ? new Date(s.date).toLocaleDateString(undefined, { dateStyle: 'medium' }) : '', s?.size ? formatBytes(s.size) : '']
      .filter(Boolean)
      .join(' · '),
  );
  const title = $derived(
    {
      checking: 'Checking for updates…',
      uptodate: 'You’re up to date',
      source: 'Updates',
      failed: 'Update failed and was undone',
      error: s?.version && !updates.checkError ? `Otto ${s.version}` : 'Check for updates',
    }[view.kind as string] ?? `Otto ${s?.version ?? ''}`,
  );
  const busy = $derived(updates.restarting);

  // Entering the staged step (a download finishing while the dialog is open)
  // counts what a restart would end.
  $effect(() => {
    if (view.kind === 'staged' && !updates.impact && !updates.impactError) void updates.loadImpact();
  });

  function primary(): void {
    const a = view.primary.action;
    if (a === 'download') void updates.download();
    else if (a === 'restart') void updates.restart();
    else if (view.kind === 'failed') updates.dismissFailed();
    else updates.close();
  }

  function more(e: MouseEvent): void {
    const v = s?.version;
    ctxMenu.showAt(e.currentTarget as Element, [
      { label: 'Skip this version', icon: 'x', disabled: !v, action: () => updates.skip() },
      { label: 'View release on GitHub', icon: 'external', disabled: !v, action: () => void openExternal(`https://github.com/itzikiusa/otto_os/releases/tag/v${v}`) },
      { separator: true },
      { label: 'Install updates automatically', checked: updates.prefs.auto_install, title: 'Downloads in the background; you choose when to restart', action: () => void updates.setPrefs({ auto_install: !updates.prefs.auto_install }) },
    ]);
  }
</script>

<Modal {title} width={520} onclose={() => updates.close()} dismissable={!busy}>
  <div class="upd" data-testid="update-dialog" data-kind={view.kind}>
    {#if view.kind === 'checking'}
      <p class="dim" use:announceShown={'Checking for a newer Otto'}>Checking for a newer Otto…</p>
    {:else if view.kind === 'uptodate'}
      <p>Otto {s?.current} is the newest version.</p>
    {:else if view.kind === 'source'}
      <p>This is a source build — updates come from git.</p>
      <p class="dim">Pull the latest changes and run <code>packaging/deploy.sh</code>. Release builds update themselves.</p>
    {:else if view.kind === 'failed'}
      <p>Otto {updates.failed?.from} is running again; {updates.failed?.to} was skipped so it isn’t tried again.</p>
      {#if updates.failed?.reason}<p class="dim">{updates.failed.reason}</p>{/if}
      <button class="btn small" onclick={() => void updates.showLog()}><Icon name="file" size={12} /> Show the update log</button>
    {:else if view.kind === 'error' && (updates.checkError || !s?.version)}
      <LoadState what="updates" errorTitle="Couldn’t check for updates" error={updates.checkError ?? s?.error ?? 'Unknown cause'} empty onretry={() => void updates.check()} />
    {:else}
      <div class="head">
        <span>You have {s?.current}</span>
        {#if meta}<span class="dim">{meta}</span>{/if}
      </div>
      {#if view.kind === 'error' && s?.error}<p class="err" use:announceShown={{ text: s.error, assertive: true }}>{s.error}</p>{/if}
      {#if view.kind === 'downloading'}
        <progress max="100" value={s?.pct ?? undefined} aria-label="Download progress"></progress>
      {/if}
      {#if view.kind === 'staged'}
        <div class="impact" data-testid="restart-impact">
          {#if updates.impactError}
            <p class="err" use:announceShown={{ text: `Couldn’t count what a restart ends: ${updates.impactError}`, assertive: true }}>Couldn’t count what a restart ends: {updates.impactError}</p>
          {:else if !updates.impact}
            <p class="dim">Counting what keeps running…</p>
          {:else}
            {#if plan.keeps.length}<p><strong>Keeps running:</strong> {plan.keeps.join(', ')}.</p>{/if}
            {#if plan.ends.length}<p class="ends"><strong>Ends:</strong> {plan.ends.join(', ')}.</p>{:else}<p>Nothing ends — Otto restarts and picks up where it was.</p>{/if}
          {/if}
        </div>
      {/if}
      {#if notes}
        <!-- renderMarkdownGfm output goes through the allowlist sanitizer. -->
        <div class="md-body notes" data-testid="update-notes">{@html notes}</div>
      {/if}
    {/if}
    {#if updates.actionError}<p class="err" use:announceShown={{ text: updates.actionError, assertive: true }}>{updates.actionError}</p>{/if}
  </div>
  {#snippet footer()}
    {#if s?.version && s.channel === 'release' && view.kind !== 'failed'}
      <button class="icon-btn" onclick={more} title="More update options" aria-label="More update options" aria-haspopup="menu" data-testid="update-more"><Icon name="more" size={16} /></button>
    {/if}
    <span class="grow"></span>
    {#if view.primary.action !== 'close'}<button class="btn" onclick={() => updates.close()} disabled={busy}>Later</button>{/if}
    <button class="btn primary" onclick={primary} disabled={!view.primary.action && view.kind !== 'failed' || busy} data-testid="update-primary">
      {busy ? 'Restarting…' : view.kind === 'staged' && updates.impact ? plan.label : view.primary.label}
    </button>
  {/snippet}
</Modal>

<style>
  .upd {
    display: flex;
    flex-direction: column;
    gap: 10px;
    font-size: var(--fs-m);
  }
  .upd p {
    margin: 0;
  }
  .dim {
    color: var(--text-dim);
  }
  .head {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }
  .err {
    color: var(--danger);
  }
  .ends {
    color: var(--warning);
  }
  progress {
    width: 100%;
    accent-color: var(--accent-solid);
  }
  .impact {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px 12px;
    border-radius: var(--radius-m);
    background: var(--hover);
  }
  .notes {
    max-height: min(320px, 45vh);
    overflow-y: auto;
    border-top: 1px solid var(--separator);
    padding-top: 10px;
  }
  .grow {
    flex: 1;
  }
</style>
