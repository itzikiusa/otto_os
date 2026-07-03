<script lang="ts">
  // Settings → Snipping: the system-wide capture shortcut (desktop app only —
  // the chord is registered by the Tauri shell via tauri-plugin-global-shortcut
  // and persisted in the app config dir, so it works while Otto runs in the
  // background). In a plain browser only the in-app triggers exist.
  import { isTauri } from '../../lib/stores/ui.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { startSnip } from '../../lib/snip';

  const DEFAULT_ACCEL = 'Cmd+Ctrl+Shift+2';

  let accel = $state('');
  let loading = $state(true);
  let recording = $state(false);
  let saveError = $state('');

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
    let key = e.key.length === 1 ? e.key.toUpperCase() : e.key;
    if (key === ' ') key = 'Space';
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

<div class="page">
  <div class="page-header">
    <div>
      <h1>Snipping</h1>
      <div class="sub">
        One-gesture screenshots: capture a screen region, annotate it (text, boxes, arrows,
        colors), and the result is <strong>already on your clipboard</strong> at every step —
        paste it straight into an agent session.
      </div>
    </div>
  </div>

  {#if isTauri}
    <div class="card">
      <div class="row">
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
              class="recorder"
              autofocus
              readonly
              placeholder="Press keys… (Esc cancels)"
              onkeydown={onRecordKey}
              onblur={() => (recording = false)}
            />
          {:else}
            <code class="chord" data-accel={accel}>{loading ? '…' : pretty(accel)}</code>
            <button class="btn" onclick={() => (recording = true)}>Change</button>
            {#if accel !== DEFAULT_ACCEL}
              <button class="btn" onclick={() => void save(DEFAULT_ACCEL)}>Reset</button>
            {/if}
            {#if accel}
              <button class="btn" onclick={() => void save('')}>Disable</button>
            {/if}
          {/if}
        </div>
      </div>
      {#if saveError}
        <div class="error">Could not set shortcut: {saveError}</div>
      {/if}
    </div>
  {:else}
    <div class="card">
      <div class="row-text">
        <span class="row-title">Global shortcut</span>
        <span class="row-desc">
          Available in the Otto desktop app (default ⌘⌃⇧2). In the browser, use the in-app
          triggers below.
        </span>
      </div>
    </div>
  {/if}

  <div class="card">
    <div class="row">
      <div class="row-text">
        <span class="row-title">In-app triggers</span>
        <span class="row-desc">
          ⌘⇧S anywhere in Otto, “Take screenshot (snip)” in the ⌘K palette, or File → Take Snip.
          The first capture will ask macOS for Screen Recording permission for
          <span class="mono">ottod</span> (System Settings → Privacy &amp; Security).
        </span>
      </div>
      <div class="row-controls">
        <button class="btn primary" onclick={() => void startSnip()}>Take a snip now</button>
      </div>
    </div>
  </div>
</div>

<style>
  .page {
    padding: 24px;
    max-width: 760px;
  }
  .page-header h1 {
    margin: 0 0 6px;
    font-size: 20px;
  }
  .sub {
    color: var(--text-dim);
    font-size: 13px;
    line-height: 1.5;
    margin-bottom: 18px;
  }
  .card {
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface);
    padding: 14px 16px;
    margin-bottom: 14px;
  }
  .row {
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
    font-size: 13px;
  }
  .row-desc {
    color: var(--text-dim);
    font-size: 12px;
    line-height: 1.5;
  }
  .row-controls {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .chord {
    font-size: 15px;
    padding: 4px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    min-width: 64px;
    text-align: center;
  }
  .recorder {
    width: 220px;
    padding: 6px 10px;
    border: 1px solid var(--accent);
    border-radius: 6px;
    background: var(--bg);
    color: var(--text);
    font-size: 12px;
    outline: none;
  }
  .btn {
    border: 1px solid var(--border);
    background: none;
    color: var(--text);
    border-radius: 6px;
    padding: 5px 10px;
    font-size: 12px;
    cursor: pointer;
  }
  .btn:hover {
    border-color: var(--accent);
  }
  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
  }
  .error {
    margin-top: 10px;
    color: #e5484d;
    font-size: 12px;
  }
  .mono {
    font-family: ui-monospace, monospace;
  }
</style>
