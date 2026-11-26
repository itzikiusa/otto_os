<script lang="ts">
  // Settings ▸ Updates (desktop app, root): the running version, the last
  // check, automatic download, release candidates, and the skipped version. The shell's updater
  // owns all of it (lib/stores/updates.svelte.ts); a source build only says
  // where its updates come from.
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import { updates } from '../../lib/stores/updates.svelte';
  import { sectionLabel } from './sections';
  import SectionIntro from './SectionIntro.svelte';
  import SettingToggle from './SettingToggle.svelte';

  const s = $derived(updates.state);
  const source = $derived(s?.channel === 'source');
  const lastCheck = $derived(s?.last_check ? new Date(s.last_check * 1000).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' }) : 'Not yet');
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('updates')} subtitle="Keep this Otto up to date" />
  <PageBody width="readable">
    <SectionIntro>
      Otto checks for a new release a minute after it starts and every six hours. A new version shows in the sidebar;
      restarting to install it is always your click, and Otto lists what keeps running first.
    </SectionIntro>
    <div class="card upd-card" data-testid="updates-settings">
      <div class="upd-row">
        <div class="grow">
          <div class="row-title">Otto {s?.current ?? ''}</div>
          <div class="row-desc">{source ? 'Source build — updates come from git: pull and run packaging/deploy.sh.' : `Release build · last checked: ${lastCheck}`}</div>
        </div>
        <button class="btn" onclick={() => updates.openDialog(true)}>Check for updates</button>
      </div>
      {#if s?.blocked}<div class="upd-blocked">{s.blocked}</div>{/if}
      {#if !source}
        <SettingToggle
          label="Install updates automatically"
          hint="Downloads and prepares a new version in the background. Otto never restarts on its own."
          checked={!!s?.auto_install}
          onchange={(v) => updates.setAuto(v)}
          testid="updates-auto"
        />
        <SettingToggle
          label="Include release candidates"
          hint="Also offers early builds of the next version, for trying it before it is final."
          checked={!!s?.prereleases}
          onchange={(v) => updates.setPrereleases(v)}
          testid="updates-prereleases"
        />
        {#if s?.skipped}
          <div class="upd-row">
            <div class="grow">
              <div class="row-title">Skipped version</div>
              <div class="row-desc">Otto {s.skipped} isn’t offered in the sidebar.</div>
            </div>
            <button class="btn" onclick={() => void updates.skip(null)}>Unskip</button>
          </div>
        {/if}
      {/if}
    </div>
  </PageBody>
</div>

<style>
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .upd-card {
    max-width: var(--settings-col);
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .upd-row {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .row-title {
    font-weight: 500;
  }
  .upd-blocked {
    color: var(--warning);
    font-size: var(--fs-s);
  }
  .row-desc {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
</style>
