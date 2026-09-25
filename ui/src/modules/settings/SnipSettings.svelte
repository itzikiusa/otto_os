<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import PageBody from '../../lib/components/PageBody.svelte';
  // Settings → Snipping: the system-wide capture shortcut (desktop app only —
  // the chord is registered by the Tauri shell via tauri-plugin-global-shortcut
  // and persisted in the app config dir, so it works while Otto runs in the
  // background). In a plain browser only the in-app triggers exist.
  import { isTauri } from '../../lib/stores/ui.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { startSnip } from '../../lib/snip';
  import SectionIntro from './SectionIntro.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  const DEFAULT_ACCEL = 'Cmd+Ctrl+Shift+2';

  let accel = $state('');
  let loading = $state(true);
  let recording = $state(false);
  let saveError = $state('');
  // The shell couldn't report the chord (an older app build): '' would read
  // as "disabled" and offer Reset/Disable against a value we never saw.
  let unavailable = $state(false);

  $effect(() => {
    if (!isTauri) {
      loading = false;
      return;
    }
    void (async () => {
      try {
        const { invoke } = await import('@tauri-apps/api/core');
        accel = await invoke<string>('snip_get_shortcut');
      } catch {
        // Older shell without the command; leave the section read-only.
        unavailable = true;
      } finally {
        loading = false;
      }
    })();
  });

  async function save(next: string): Promise<void> {
    saveError = '';
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('snip_set_shortcut', { accel: next });
      accel = next;
      toasts.success(next ? `Snip shortcut set to ${pretty(next)}` : 'Snip shortcut disabled');
    } catch (e) {
      saveError = e instanceof Error ? e.message : String(e);
    }
  }

  function onRecordKey(e: KeyboardEvent): void {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === 'Escape') {
      recording = false;
      return;
    }
    // Wait for a non-modifier key; require at least one modifier so a bare
    // letter can't shadow normal typing system-wide.
    if (['Meta', 'Control', 'Alt', 'Shift'].includes(e.key)) return;
    const mods = [
      e.metaKey ? 'Cmd' : null,
      e.ctrlKey ? 'Ctrl' : null,
      e.altKey ? 'Alt' : null,
      e.shiftKey ? 'Shift' : null,
    ].filter(Boolean) as string[];
    if (!mods.length) return;
    // The PHYSICAL key: with ⇧ or ⌥ held, `e.key` is the shifted/option
    // character ("@", "™"), which the shell can't parse as a chord.
    const c = e.code;
    let key = /^Key[A-Z]$/.test(c)
      ? c.slice(3)
      : /^Digit[0-9]$/.test(c)
        ? c.slice(5)
        : /^F\d{1,2}$/.test(c)
          ? c
          : e.key.length === 1
            ? e.key.toUpperCase()
            : e.key;
    if (key === ' ' || c === 'Space') key = 'Space';
    recording = false;
    void save([...mods, key].join('+'));
  }

  /** "Cmd+Ctrl+Shift+2" → "⌘⌃⇧2" for display. */
  function pretty(a: string): string {
    if (!a) return '—';
    return a
      .split('+')
      .map((part) => ({ Cmd: '⌘', Ctrl: '⌃', Alt: '⌥', Shift: '⇧' })[part] ?? part)
      .join('');
  }
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('snipping')} subtitle="One-gesture screenshots: capture, annotate, paste" />
  <PageBody width="readable">
  <SectionIntro>Capture a screen region, annotate it (text, boxes, arrows, colours), and the result is <strong>already on your clipboard</strong> at every step — paste it straight into an agent session.</SectionIntro>

  {#if isTauri}
    <div class="card snip-card">
      <div class="snip-row">
        <div class="row-text">
          <span class="row-title">Global shortcut</span>
          <span class="row-desc">
            Works system-wide while Otto is running. Press it, drag a region (Space toggles
            window mode, Esc cancels), and the annotation editor opens with the capture already
            copied.
          </span>
        </div>
        <div class="row-controls">
          {#if recording}
            <!-- svelte-ignore a11y_autofocus -->
            <input
              class="input recorder"
              autofocus
              readonly
              aria-label="Press the new shortcut"
              placeholder="Press keys… (Esc cancels)"
              onkeydown={onRecordKey}
              onblur={() => (recording = false)}
            />
          {:else if unavailable}
            <span class="row-desc">This version of the app can't change the shortcut — update Otto.</span>
          {:else}
            <kbd class="chord" data-accel={accel} title={accel || 'No shortcut'}>{loading ? '…' : accel ? pretty(accel) : 'Off'}</kbd>
            <button class="btn small" onclick={() => (recording = true)}>{accel ? 'Change…' : 'Set…'}</button>
            {#if accel !== DEFAULT_ACCEL}
              <button class="btn small ghost" onclick={() => void save(DEFAULT_ACCEL)}>Reset to {pretty(DEFAULT_ACCEL)}</button>
            {/if}
            {#if accel}
              <button class="btn small ghost" onclick={() => void save('')}>Turn off</button>
            {/if}
          {/if}
        </div>
      </div>
      {#if saveError}
        <div class="error" role="alert">Couldn't set the shortcut: {saveError}. It may already belong to another app — try a different chord.</div>
      {/if}
    </div>
  {:else}
    <div class="card snip-card">
      <div class="row-text">
        <span class="row-title">Global shortcut</span>
        <span class="row-desc">
          Available in the Otto desktop app (default ⌘⌃⇧2). In the browser, use the in-app
          triggers below.
        </span>
      </div>
    </div>
  {/if}

  <div class="card snip-card">
    <div class="snip-row">
      <div class="row-text">
        <span class="row-title">In-app triggers</span>
        <span class="row-desc">
          ⌘⇧S anywhere in Otto, “Take screenshot (snip)” in the ⌘K palette, or File → Take Snip.
          The first capture asks macOS for Screen Recording permission for
          <span class="mono">ottod</span> (System Settings → Privacy &amp; Security).
        </span>
      </div>
      <div class="row-controls">
        <button class="btn primary" onclick={() => void startSnip()}><Icon name="image" size={13} /> Take a snip</button>
      </div>
    </div>
  </div>
  </PageBody>
</div>

<style>
  /* Section chrome: shared PageHeader bar + scrolling PageBody. */
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .snip-card {
    max-width: 720px;
    padding: 14px 16px;
    margin-bottom: 12px;
  }
  .snip-row {
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
    margin-top: 10px;
    color: var(--danger);
    font-size: var(--fs-s);
  }
</style>
