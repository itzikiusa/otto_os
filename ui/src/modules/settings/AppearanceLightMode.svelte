<script lang="ts">
  // Settings ▸ Appearance: the phone's light version (lib/lightMode.svelte.ts).
  // Only on a phone-sized screen — tablet and desktop never change, so they
  // never show it. The bottom nav's More sheet and ⌘K switch it too.
  import SettingToggle from './SettingToggle.svelte';
  import { lightMode } from '../../lib/lightMode.svelte';
</script>

{#if lightMode.phoneSized}
  <div class="section-title">Phone</div>
  <SettingToggle
    label="Light version"
    hint="Only Threads, Connections, Git, API and Usage, opening on Threads. Remembered on this device."
    checked={lightMode.chosen}
    onchange={(v) => lightMode.set(v)}
    testid="appearance-light-version"
  />
{/if}
