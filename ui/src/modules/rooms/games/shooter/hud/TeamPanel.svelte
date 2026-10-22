<script lang="ts">
  // Storm Royale team panel: one row per teammate with health + shield, or the
  // bleed-out bar while knocked (team modes; ShooterGame decides when to show it).
  import { DOWN_HP, type Fighter } from '../sim';
  import { teamColour } from '../catalog';
  import { hex } from './minimap';

  interface Props { mates: Fighter[]; team: number; modeName: string }
  let { mates, team, modeName }: Props = $props();
</script>

<ul class="br-team" aria-label={`Your ${modeName.toLowerCase().replace(/s$/, '')} team`} style:--team={hex(teamColour(team))}>
  {#each mates as m (m.id)}
    <li class:down={m.mode === 'down'} class:out={m.mode === 'dead'}>
      <i aria-hidden="true"></i><span>{m.name}</span>
      {#if m.mode === 'dead'}<em>Out</em>
      {:else if m.mode === 'down'}<em>{m.revive > 0 ? 'Reviving…' : 'Knocked'}</em><b class="bleed" role="meter" aria-label={`${m.name} bleeding out`} aria-valuemin={0} aria-valuemax={DOWN_HP} aria-valuenow={Math.round(m.hp)}><span style:width={`${(m.hp / DOWN_HP) * 100}%`}></span></b>
      {:else}<b role="meter" aria-label={`${m.name} health and shield`} aria-valuemin={0} aria-valuemax={200} aria-valuenow={Math.round(m.hp + m.shield)}><span class="hp" style:width={`${m.hp / 2}%`}></span><span class="sh" style:width={`${m.shield / 2}%`}></span></b>{/if}
    </li>
  {/each}
</ul>

<style>
  /* Team panel: one row per teammate (health + shield, or the bleed-out bar when knocked). */
  .br-team { position: absolute; inset-block-start: 64px; inset-inline-start: 14px; margin: 0; padding: 6px; list-style: none; display: grid; gap: 4px; width: 210px; background: var(--scrim-media); color: var(--on-scrim); border-radius: var(--radius-m); border: 1px solid color-mix(in srgb, var(--on-scrim) 18%, transparent); pointer-events: none; font-size: var(--fs-s); }
  .br-team li { display: grid; grid-template-columns: 10px 1fr auto; grid-template-rows: auto 5px; column-gap: 6px; align-items: center; }
  .br-team li i { width: 10px; height: 10px; border-radius: 50%; background: var(--team); }
  .br-team li span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .br-team li em { font-style: normal; font-weight: 600; }
  .br-team li.down em { color: var(--danger); }
  .br-team li.out { opacity: .55; }
  .br-team b { grid-column: 2 / -1; grid-row: 2; display: flex; height: 5px; border-radius: 999px; overflow: hidden; background: color-mix(in srgb, var(--on-scrim) 15%, transparent); }
  .br-team b .hp { background: var(--success); } .br-team b .sh { background: var(--info); }
  .br-team b.bleed span { background: var(--danger); }
  @media (max-width: 640px) { .br-team { width: 150px; inset-block-start: 96px; } }
</style>
