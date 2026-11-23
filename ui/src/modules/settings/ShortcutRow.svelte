<script lang="ts">
  // One system-wide shortcut's Settings row (desktop app only): the chord,
  // Change… (records the next chord; Esc cancels), Reset to the default, and
  // why it isn't active (another app holds it, a clash). Shared by the
  // Settings pages that own a global shortcut (shortcuts.rs).
  import { announceShown } from '../../lib/announce.svelte';
  import { focusOnMount } from '../../lib/focusOnMount';
  import { loadErrorText } from '../../lib/loadError';
  import { shortcuts, type ShortcutBinding } from '../../lib/desktop';
  import { chordFromKey, prettyChord } from '../../lib/chords';

  interface Props {
    id: ShortcutBinding['id'];
    /** What the chord does, under the "Shortcut" title. */
    desc: string;
  }
  let { id, desc }: Props = $props();

  let chord = $state<ShortcutBinding | null>(null);
  let loaded = $state(false);
  let error = $state('');
  let busy = $state(false);
  let recording = $state(false);

  $effect(() => {
    void shortcuts.list().then(
      (all) => {
        chord = all.find((s) => s.id === id) ?? null;
        loaded = true;
      },
      (e) => {
        error = loadErrorText(e);
        loaded = true;
      },
    );
  });

  async function setChord(accel: string): Promise<void> {
    if (busy) return;
    busy = true;
    error = '';
    try {
      await shortcuts.set(id, accel);
      chord = chord && { ...chord, accel, active: !!accel, error: null };
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      busy = false;
    }
  }

  function onRecordKey(e: KeyboardEvent): void {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === 'Escape') {
      recording = false;
      return;
    }
    const next = chordFromKey(e);
    if (!next) return;
    recording = false;
    void setChord(next);
  }
</script>

<div class="sc-row">
  <div class="row-text">
    <span class="row-title">Shortcut</span>
    <span class="row-desc">{desc}</span>
  </div>
  <div class="row-controls">
    {#if recording}
      <input
        class="input recorder"
        dir="ltr"
        use:focusOnMount
        readonly
        aria-label="Press the new shortcut"
        placeholder="Press keys… (Esc cancels)"
        onkeydown={onRecordKey}
        onblur={() => (recording = false)}
      />
    {:else if chord}
      <kbd class="chord" title={chord.accel || 'No shortcut'}>{chord.accel ? prettyChord(chord.accel) : 'Off'}</kbd>
      <button class="btn small" disabled={busy} onclick={() => (recording = true)}>Change…</button>
      {#if chord.accel !== chord.default}
        <button class="btn small ghost" disabled={busy} onclick={() => void setChord(chord?.default ?? '')}>Reset to {prettyChord(chord.default)}</button>
      {/if}
    {:else if loaded && !error}
      <span class="row-desc">This version of the app can’t change it — update Otto.</span>
    {/if}
  </div>
</div>
{#if error || chord?.error}
  <span class="error" use:announceShown={{ text: error || chord?.error || '', assertive: true }}>{error || chord?.error}</span>
{/if}

<style>
  .sc-row {
    display: flex;
    align-items: center;
    gap: 16px;
    flex-wrap: wrap;
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
  .chord {
    font-family: var(--font-ui);
    font-size: var(--fs-m);
    padding: 2px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    min-width: 56px;
    text-align: center;
  }
  .recorder {
    width: 220px;
    border-color: var(--accent);
  }
  .error {
    color: var(--danger);
    font-size: var(--fs-s);
  }
</style>
