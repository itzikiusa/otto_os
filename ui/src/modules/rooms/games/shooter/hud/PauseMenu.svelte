<script lang="ts">
  // Storm Royale pause menu (Esc / Menu): the full key reference plus Resume
  // and Leave. It covers the Menu button, so focus moves onto Resume.
  import { tick } from 'svelte';

  interface Props { networked: boolean; onresume: () => void; onexit: () => void }
  let { networked, onresume, onexit }: Props = $props();
  let resumeButton = $state<HTMLButtonElement>();
  $effect(() => { const b = resumeButton; if (b) void tick().then(() => b.focus()); });
</script>

<div class="br-cover"><h2>Paused</h2><p>{networked ? 'The match continues for everyone else while this menu is open.' : 'The island waits for you.'}</p>
  <p class="br-keys">WASD move · Mouse aim · Left click fire (hold to use heals, shields and launch pads) · Right click / Z aim down sights (zooms; attachments: scope, grip, extended mag, suppressor) · pull the mouse down against the recoil · 1–5 or wheel switch hotbar slots · F pickaxe (harvest materials; Zombies: repair barricades) · Q build mode: 1 wall, 2 floor, 3 ramp, R material, G edit your piece (door / window, flip ramp), click to place · G frag / molotov · T smoke / flash · X call in a killstreak (UAV, airstrike at the crosshair, attack drone) · C crouch, or slide out of a sprint · Shift sprint, double-tap for a tactical sprint · Space jump, or mantle a ledge ahead · E pick up / hold to open chests and supply drops / hold next to a knocked teammate to revive / buy from a wall (Zombies) · R reload (fire to cancel) · 6–9 respawn class (TDM, Domination) · M full map · Esc release the mouse</p>
  <button class="btn primary" bind:this={resumeButton} onclick={onresume}>Resume</button><button class="btn" onclick={onexit}>Leave match</button></div>

<style>
  .br-cover { position: absolute; inset: 0; z-index: 6; display: flex; flex-direction: column; align-items: center; justify-content: safe center; overflow-y: auto; gap: 12px; padding: 24px; text-align: center; background: color-mix(in srgb, var(--surface) 90%, transparent); }
  .br-cover h2 { margin: 0; font-size: var(--fs-2xl); }
  .br-cover p { margin: 0; max-width: 600px; color: var(--text-dim); line-height: 1.6; }
  .br-keys { font-size: var(--fs-s); }
  @media (max-width: 640px) { .br-cover { padding: 16px; gap: 8px; } }
</style>
