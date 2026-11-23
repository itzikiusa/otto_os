<script lang="ts">
  // Settings → Clipboard history (desktop app only): on/off, pause, clear,
  // its global shortcut, and the two macOS permissions it depends on. Status
  // only — the history itself is readable in its own panel, never here
  // (the shell refuses content calls from any other window).
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { isTauri } from '../../lib/desktop';
  import { clip, type ClipStatus } from '../../lib/clipboardHistory';
  import { sectionLabel } from './sections';
  import SectionIntro from './SectionIntro.svelte';
  import ShortcutRow from './ShortcutRow.svelte';

  let status = $state<ClipStatus | null>(null);
  let loadError = $state('');
  let actionError = $state('');
  let busy = $state(false);

  async function load(): Promise<void> {
    loadError = '';
    try {
      status = await clip.status();
    } catch (e) {
      loadError = loadErrorText(e);
    }
  }

  $effect(() => {
    if (isTauri) void load();
  });

  async function act(what: () => Promise<ClipStatus | void>): Promise<void> {
    if (busy) return;
    busy = true;
    actionError = '';
    try {
      const s = await what();
      if (s) status = s;
    } catch (e) {
      actionError = loadErrorText(e);
    } finally {
      busy = false;
    }
  }

  async function turnOff(): Promise<void> {
    const ok = await confirmer.ask('Capture stops and every saved entry is deleted from this Mac.', {
      title: 'Turn off clipboard history?',
      confirmLabel: 'Turn off and delete',
      danger: true,
    });
    if (ok) await act(() => clip.set({ enabled: false }));
  }

  async function clearHistory(): Promise<void> {
    const ok = await confirmer.ask('Every entry, pinned ones too, is deleted from this Mac.', {
      title: 'Clear clipboard history?',
      confirmLabel: 'Clear history',
      danger: true,
    });
    if (ok) await act(() => clip.clear());
  }

</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('clipboard')} subtitle="Paste anything you copied again" />
  <PageBody width="readable">
  <SectionIntro>
    Otto can keep what you copy (text, links, file paths and images) on this Mac only, and bring it back with a
    shortcut. Copies marked as passwords are never kept, and the history is never shared with agents, the daemon
    or your phone.
  </SectionIntro>

  {#if !isTauri}
    <div class="card clip-card"><span class="row-desc">Clipboard history is available in the Otto desktop app.</span></div>
  {:else if loadError}
    <LoadState what="clipboard history" error={loadError} empty onretry={() => void load()} />
  {:else if !status}
    <Skeleton rows={3} height={48} label="clipboard history settings" />
  {:else}
    <div class="card clip-card">
      <div class="clip-row">
        <div class="row-text">
          <span class="row-title">Clipboard history</span>
          <span class="row-desc">
            {#if !status.enabled}Off. Turn it on here or from its panel.{:else if status.paused}Paused · {status.count} saved{:else}On · {status.count} saved{/if}
          </span>
        </div>
        <div class="row-controls">
          {#if !status.enabled}
            <button class="btn small primary" disabled={busy} onclick={() => void act(() => clip.set({ enabled: true }))}>Turn on</button>
          {:else}
            <button class="btn small" disabled={busy} onclick={() => void act(() => clip.set({ paused: !status?.paused }))}>
              {status.paused ? 'Resume capture' : 'Pause capture'}
            </button>
            <button class="btn small ghost" disabled={busy || status.count === 0} onclick={() => void clearHistory()}>Clear history…</button>
            <button class="btn small ghost" disabled={busy} onclick={() => void turnOff()}>Turn off…</button>
          {/if}
        </div>
      </div>
      {#if status.enabled && status.access !== 'allowed'}
        <div class="clip-row warn">
          <span class="row-desc">
            macOS {status.access === 'ask' ? 'asks before' : 'stops'} Otto reading the clipboard, so new copies aren’t kept.
            Set Otto to Allow in Privacy &amp; Security → Paste from Other Apps.
          </span>
          <button class="btn small" onclick={() => void act(() => clip.openSettings('paste'))}>Open System Settings</button>
        </div>
      {/if}
    </div>

    <div class="card clip-card">
      <ShortcutRow id="clipboard" desc="Opens the history over any app. Return pastes the selected entry into that app." />
      <div class="clip-row">
        <div class="row-text">
          <span class="row-title">Paste into apps</span>
          <span class="row-desc">
            {status.accessibility
              ? 'Allowed. Return pastes into the app you were in.'
              : 'Needs the Accessibility permission to press ⌘V for you. Until then Return copies the entry.'}
          </span>
        </div>
        {#if !status.accessibility}
          <div class="row-controls">
            <button class="btn small" onclick={() => void act(() => clip.openSettings('accessibility'))}>Open System Settings</button>
          </div>
        {/if}
      </div>
    </div>
    {#if actionError}
      <div class="error" use:announceShown={{ text: actionError, assertive: true }}>{actionError}</div>
    {/if}
  {/if}
  </PageBody>
</div>

<style>
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .clip-card {
    max-width: var(--settings-col);
    padding: 14px 16px;
    margin-bottom: 12px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .clip-row {
    display: flex;
    align-items: center;
    gap: 16px;
    flex-wrap: wrap;
  }
  .clip-row.warn {
    padding: 8px 10px;
    border-radius: var(--radius-s);
    background: var(--warning-soft);
  }
  .clip-row.warn .row-desc {
    flex: 1;
  }
  .row-text {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: 1;
    min-width: 240px;
  }
  .row-title {
    font-weight: 600;
    font-size: var(--fs-m);
  }
  .row-desc {
    color: var(--text-dim);
    font-size: var(--fs-s);
    line-height: 1.5;
  }
  .row-controls {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .error {
    color: var(--danger);
    font-size: var(--fs-s);
  }
</style>
