<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import PageBody from '../../lib/components/PageBody.svelte';
  // Notification preferences: expiry warning threshold + native/session toggles.
  // Also exposes the `channels.notify_self_improvement` opt-in flag so the user
  // can turn on Slack/Telegram self-improvement pings from one place (T6).
  import { notifications } from '../../lib/stores/notifications.svelte';
  import { api } from '../../lib/api/client';
  import { auth } from '../../lib/stores/auth.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import type { NotificationSettings } from '../../lib/api/types';
  import { router } from '../../lib/router.svelte';
  import SettingToggle from './SettingToggle.svelte';

  // Load once on mount if the store hasn't fetched yet.
  $effect(() => {
    if (!notifications.loaded) void notifications.load();
  });

  function save(patch: Partial<NotificationSettings>): void {
    void notifications.saveSettings({ ...notifications.settings, ...patch });
  }

  function onThreshold(e: Event & { currentTarget: HTMLInputElement }): void {
    const el = e.currentTarget;
    const n = Math.round(Number(el.value));
    const cur = notifications.settings.expiry_threshold_days;
    // An empty/invalid entry, or one clamped back to the value already saved,
    // changes no state — so put the saved number back in the box explicitly
    // (otherwise it keeps showing e.g. "45" while 30 is in effect).
    const next = el.value.trim() === '' || !Number.isFinite(n) ? cur : Math.min(30, Math.max(1, n));
    el.value = String(next);
    if (next !== cur) save({ expiry_threshold_days: next });
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
      label: 'Self-improvement events',
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

  // Set when the flags couldn't be read: the toggles would otherwise all show
  // OFF (the defaults) even if some are on, so they're disabled until a retry.
  let flagsError = $state('');

  async function loadChannelFlags(): Promise<void> {
    for (const f of CHANNEL_NOTIFY_FLAGS) {
      flagLoading[f.key] = true;
    }
    flagsError = '';
    try {
      const all = await api.get<Record<string, unknown>>('/settings');
      for (const f of CHANNEL_NOTIFY_FLAGS) {
        flagValues[f.key] = all?.[f.key] === true;
      }
    } catch (e) {
      flagsError = loadErrorText(e);
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
      toasts.error("Couldn't save the channel notification", loadErrorText(e));
    }
  }
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('notifications')} subtitle="Alerts and credential-expiry warnings" />
  <PageBody width="readable">

  <div class="section-title">Alerts</div>
  <div class="card s-card">
    <SettingToggle
      label="Native macOS notifications for important alerts"
      hint="Show system notifications for warnings and errors."
      checked={notifications.settings.native_enabled}
      onchange={(v) => save({ native_enabled: v })}
    />
    <SettingToggle
      label="Notify on session events"
      hint="A heads-up when a session finishes or is waiting for your input."
      checked={notifications.settings.session_events}
      onchange={(v) => save({ session_events: v })}
    />
  </div>

  <div class="section-title">Credential expiry</div>
  <div class="card s-card">
    <div class="field threshold">
      <label for="nt-threshold">Warn me this many days before a credential expires</label>
      <input
        id="nt-threshold"
        class="input num"
        type="number"
        min="1"
        max="30"
        value={notifications.settings.expiry_threshold_days}
        onchange={onThreshold}
      />
      <span class="hint">1–30 days (default 3). Covers Git and Jira account tokens and agent CLI sign-ins.</span>
    </div>
  </div>

  {#if auth.isRoot}
    <div class="section-title">Channel notifications</div>
    <p class="section-note">
      Each one posts a one-line message to this workspace's Slack or Telegram channel (set up in
      <button class="link" onclick={() => router.go('settings/channels')}>Channels</button>). All are off by default.
    </p>
    {#if flagsError}
      <div class="flags-error" role="alert">
        <span>Couldn't load these settings: {flagsError}</span>
        <button class="btn small" onclick={() => void loadChannelFlags()}>Retry</button>
      </div>
    {/if}
    <div class="card s-card">
      {#each CHANNEL_NOTIFY_FLAGS as flag (flag.key)}
        <SettingToggle
          label={flag.label}
          hint={flag.sub}
          checked={flagValues[flag.key]}
          disabled={flagLoading[flag.key] || !!flagsError}
          title={flagsError ? "Couldn't read the current value — Retry above" : undefined}
          onchange={(v) => toggleFlag(flag.key, v)}
        />
      {/each}
    </div>
  {/if}
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
  .s-card {
    padding: 6px 16px;
    max-width: var(--settings-col);
    margin-bottom: 8px;
  }
  .threshold {
    margin: 8px 0;
  }
  .num {
    width: 96px;
  }
  .section-note {
    margin: 0 0 8px;
    max-width: var(--settings-col);
    font-size: var(--fs-s);
    line-height: 1.5;
    color: var(--text-dim);
  }
  .link {
    border: none;
    background: none;
    padding: 0;
    font: inherit;
    color: var(--accent-text);
    text-decoration: underline;
    cursor: pointer;
  }
  .flags-error {
    display: flex;
    align-items: center;
    gap: 10px;
    max-width: var(--settings-col);
    margin-bottom: 8px;
    font-size: var(--fs-s);
    color: var(--danger);
  }
</style>
