<script lang="ts">
  // Storm Royale "Knocked" overlay (ShooterGame's HUD): the bleed-out bar and,
  // while a teammate holds E, the revive progress. Visual only — ShooterGame's
  // screen-reader status announces the knock.
  import { DOWN_HP, REVIVE_TIME, type Fighter } from '../sim';

  interface Props { me: Fighter }
  let { me }: Props = $props();
</script>

<div class="br-knocked">
  <h2>Knocked</h2>
  <p>{me.revive > 0 ? 'A teammate is reviving you — hold on' : 'Crawl to a teammate; they can revive you (E)'}</p>
  <div class="br-reload" aria-label="Bleeding out"><span style:width={`${(me.hp / DOWN_HP) * 100}%`}></span><em>Bleeding out</em></div>
  {#if me.revive > 0}<div class="br-reload use" aria-label="Revive progress"><span style:width={`${Math.min(1, me.revive / REVIVE_TIME) * 100}%`}></span><em>Revive</em></div>{/if}
</div>

<style>
  .br-reload { position: absolute; inset-block-start: 56%; inset-inline-start: 50%; transform: translateX(-50%); width: 140px; height: 6px; border-radius: var(--radius-s); background: var(--scrim-media); pointer-events: none; }
  .br-reload span { display: block; height: 100%; border-radius: var(--radius-s); background: var(--on-scrim); }
  .br-reload.use span { background: var(--success); }
  .br-reload em { position: absolute; inset-block-start: 8px; inset-inline: 0; text-align: center; font-size: var(--fs-s); font-style: normal; color: var(--on-scrim); white-space: nowrap; }
  .br-knocked { position: absolute; inset-block-start: 30%; inset-inline: 12px; margin-inline: auto; max-width: 360px; text-align: center; color: var(--on-scrim); pointer-events: none; }
  .br-knocked h2 { margin: 0; color: var(--danger); font-size: var(--fs-2xl); text-shadow: 0 2px 8px var(--scrim-media); }
  .br-knocked p { margin: 4px 0 8px; font-size: var(--fs-m); text-shadow: 0 1px 4px var(--scrim-media); }
  .br-knocked .br-reload { position: relative; inset: auto; transform: none; width: 220px; margin: 0 auto 22px; }
  .br-knocked .br-reload span { background: var(--danger); }
  .br-knocked .br-reload.use span { background: var(--success); }
</style>
