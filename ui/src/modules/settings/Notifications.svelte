<script lang="ts">
  // Notification preferences: expiry warning threshold + native/session toggles.
  // Bound to the shared notifications store; every change PUTs the full settings.
  import { notifications } from '../../lib/stores/notifications.svelte';
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
    left: 2px;
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
