<script lang="ts">
  // Notification preferences: expiry warning threshold + native/session toggles.
  // Also exposes the `channels.notify_self_improvement` opt-in flag so the user
  // can turn on Slack/Telegram self-improvement pings from one place (T6).
  import { notifications } from '../../lib/stores/notifications.svelte';
  import { api } from '../../lib/api/client';
  import { auth } from '../../lib/stores/auth.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { NotificationSettings } from '../../lib/api/types';

  // Load once on mount if the store hasn't fetched yet.
  $effect(() => {
    if (!notifications.loaded) void notifications.load();
  });

  function save(patch: Partial<NotificationSettings>): void {
    void notifications.saveSettings({ ...notifications.settings, ...patch });
  }

  function onThreshold(e: Event & { currentTarget: HTMLInputElement }): void {
    const n = Math.round(Number(e.currentTarget.value));
    if (!Number.isFinite(n)) return;
    save({ expiry_threshold_days: Math.min(30, Math.max(1, n)) });
  }

  // ---------------------------------------------------------------------------
  // channels.notify_self_improvement — persisted in the daemon settings store.
  // The backend `improve_notify` task reads it live from the global SettingsRepo.
  // Root-only (matches GET/PUT /api/v1/settings permission gate).
  // ---------------------------------------------------------------------------
  // Per-event channel notify toggles (all off by default, root-only).
  // ---------------------------------------------------------------------------

  type NotifyFlag = {
    key: string;
    label: string;
    sub: string;
    value: boolean;
    loading: boolean;
  };

  const CHANNEL_NOTIFY_FLAGS: Array<{ key: string; label: string; sub: string }> = [
    {
      key: 'channels.notify_self_improvement',
      label: 'Push self-improvement events to Slack / Telegram',
      sub: 'Posts a one-line summary when a run finishes or an approval is pending.',
    },
    {
      key: 'channels.notify_review_done',
      label: 'Code-review completed',
      sub: 'Sends a message when a code-review run finishes or fails.',
    },
    {
      key: 'channels.notify_swarm_done',
      label: 'Agent swarm completed',
      sub: 'Sends a message when a swarm run finishes, is aborted, or fails.',
    },
    {
      key: 'channels.notify_insight_ready',
      label: 'Insights report ready',
      sub: 'Sends a message when a daily / weekly / monthly insights report becomes available.',
    },
    {
      key: 'channels.notify_budget_exceeded',
      label: 'Budget cap exceeded',
      sub: 'Sends a message when a spend cap is crossed (requires budget enforcement to be on).',
    },
  ];

  // Reactive state for each flag: keyed by the settings key.
  let flagValues: Record<string, boolean> = $state(
    Object.fromEntries(CHANNEL_NOTIFY_FLAGS.map((f) => [f.key, false]))
  );
  let flagLoading: Record<string, boolean> = $state(
    Object.fromEntries(CHANNEL_NOTIFY_FLAGS.map((f) => [f.key, false]))
  );

  $effect(() => {
    if (auth.isRoot) void loadChannelFlags();
  });

  async function loadChannelFlags(): Promise<void> {
    for (const f of CHANNEL_NOTIFY_FLAGS) {
      flagLoading[f.key] = true;
    }
    try {
      const all = await api.get<Record<string, unknown>>('/settings');
      for (const f of CHANNEL_NOTIFY_FLAGS) {
        flagValues[f.key] = all?.[f.key] === true;
      }
    } catch {
      // Not root or settings not reachable — keep defaults.
    } finally {
      for (const f of CHANNEL_NOTIFY_FLAGS) {
        flagLoading[f.key] = false;
      }
    }
  }

  async function toggleFlag(key: string, checked: boolean): Promise<void> {
    flagValues[key] = checked;
    try {
      await api.put('/settings', { [key]: checked });
    } catch (e) {
      flagValues[key] = !checked; // revert
      toasts.error('Could not save setting', e instanceof Error ? e.message : String(e));
    }
  }
</script>

<div class="page">
  <div class="page-header">
    <div>
      <h1>Notifications</h1>
      <div class="sub">Control credential-expiry warnings and how Otto alerts you.</div>
    </div>
  </div>

  <div class="section-title">Credential expiry</div>
  <div class="card pad">
    <div class="field" style="max-width: 320px">
      <label for="nt-threshold">Warn me this many days before a credential expires</label>
      <input
        id="nt-threshold"
        class="input"
        type="number"
        min="1"
        max="30"
        value={notifications.settings.expiry_threshold_days}
        onchange={onThreshold}
      />
    </div>
  </div>

  <div class="section-title">Alerts</div>
  <div class="card pad">
    <label class="opt">
      <span class="opt-text">
        <span class="opt-title">Native macOS notifications for important alerts</span>
        <span class="opt-sub dim">Show system notifications for warnings and errors.</span>
      </span>
      <span class="toggle">
        <input
          type="checkbox"
          checked={notifications.settings.native_enabled}
          onchange={(e) => save({ native_enabled: e.currentTarget.checked })}
        />
        <span class="toggle-track"></span>
      </span>
    </label>

    <label class="opt">
      <span class="opt-text">
        <span class="opt-title">Notify on session events (finished / awaiting input)</span>
        <span class="opt-sub dim">Get a heads-up when a session finishes or needs you.</span>
      </span>
      <span class="toggle">
        <input
          type="checkbox"
          checked={notifications.settings.session_events}
          onchange={(e) => save({ session_events: e.currentTarget.checked })}
        />
        <span class="toggle-track"></span>
      </span>
    </label>
  </div>

  {#if auth.isRoot}
    <div class="section-title">Channel notifications</div>
    <div class="sub dim" style="max-width:520px;margin-bottom:8px">
      Each toggle below sends a one-line push notification to your configured
      Slack / Telegram integration. All are off by default.
    </div>
    <div class="card pad">
      {#each CHANNEL_NOTIFY_FLAGS as flag (flag.key)}
        <label class="opt">
          <span class="opt-text">
            <span class="opt-title">{flag.label}</span>
            <span class="opt-sub dim">{flag.sub}</span>
          </span>
          <span class="toggle">
            <input
              type="checkbox"
              checked={flagValues[flag.key]}
              disabled={flagLoading[flag.key]}
              onchange={(e) => void toggleFlag(flag.key, e.currentTarget.checked)}
            />
            <span class="toggle-track"></span>
          </span>
        </label>
      {/each}
    </div>
  {/if}
</div>

<style>
  .card.pad {
    padding: 14px 16px;
    max-width: 520px;
    margin-bottom: 8px;
  }
  .opt {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 8px 0;
    cursor: pointer;
  }
  .opt + .opt {
    border-top: 1px solid var(--border);
  }
  .opt-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .opt-title {
    font-size: 12.5px;
  }
  .opt-sub {
    font-size: 11.5px;
  }

  /* Toggle switch (matches Channels) */
  .toggle {
    display: flex;
    align-items: center;
    cursor: pointer;
    position: relative;
    flex-shrink: 0;
  }
  .toggle input {
    position: absolute;
    opacity: 0;
    width: 0;
    height: 0;
  }
  .toggle-track {
    width: 30px;
    height: 17px;
    border-radius: 9px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    position: relative;
    transition: background 140ms ease-out;
  }
  .toggle-track::after {
    content: '';
    position: absolute;
    top: 2px;
    inset-inline-start: 2px;
    width: 11px;
    height: 11px;
    border-radius: 50%;
    background: var(--text-dim);
    transition: transform 140ms ease-out, background 140ms ease-out;
  }
  .toggle input:checked ~ .toggle-track {
    background: var(--accent);
    border-color: var(--accent);
  }
  .toggle input:checked ~ .toggle-track::after {
    transform: translateX(13px);
    background: #fff;
  }
</style>
