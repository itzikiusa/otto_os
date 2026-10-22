<script lang="ts">
  // Storm Royale vitals panel: shield and health bars, grenades, killstreaks and
  // tactical-sprint stamina, and the building materials (ShooterGame's HUD).
  import { TAC_TIME, type Fighter } from '../sim';
  import { MATERIAL_COLOURS, MATERIAL_NAMES, NADES, STREAKS } from '../weapons';
  import type { ShooterMode } from '../catalog';
  import { hex } from './minimap';

  interface Props { me: Fighter; mode: ShooterMode; uavUp: boolean; building: boolean; material: number }
  let { me, mode, uavUp, building, material }: Props = $props();
</script>

<div class="br-vitals" aria-label="Health and shield">
  <div class="br-bar shield" role="meter" aria-label="Shield" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(me.shield)}><span style:width={`${me.shield}%`}></span><b><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 2 4 5v6c0 5 3.4 9.4 8 11 4.6-1.6 8-6 8-11V5z" /></svg>Shield {Math.round(me.shield)}</b></div>
  <div class="br-bar health" role="meter" aria-label="Health" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(me.hp)}><span style:width={`${me.hp}%`}></span><b><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M10 3h4v7h7v4h-7v7h-4v-7H3v-4h7z" /></svg>Health {Math.round(me.hp)}</b></div>
  <div class="br-gear" aria-label="Grenades and killstreaks">
    <span class:empty={me.lethalN === 0} title={`${NADES[me.lethal].name} (G)`}><kbd>G</kbd>{NADES[me.lethal].name} <b>{me.lethalN}</b></span>
    <span class:empty={me.tacN === 0} title={`${NADES[me.tacKind].name} (T)`}><kbd>T</kbd>{NADES[me.tacKind].name} <b>{me.tacN}</b></span>
    {#each STREAKS as st, k (st.id)}<span class="br-streak" class:ready={(me.ready & (1 << k)) > 0} title={(me.ready & (1 << k)) > 0 ? `${st.name} ready — press X` : `${st.name}: ${mode === 'zombies' ? st.kills * 4 : st.kills} eliminations in a row`}>{#if (me.ready & (1 << k)) > 0}<kbd>X</kbd>{/if}{st.name}{#if !((me.ready & (1 << k)) > 0)}<small> {Math.min(Math.floor(me.streak), st.kills)}/{st.kills}</small>{/if}</span>{/each}
    {#if uavUp}<span class="br-streak ready" title="Your team’s UAV shows every enemy on the map">UAV up</span>{/if}
    {#if me.tactical || me.tacT < TAC_TIME}<span class="br-tac" title="Tactical sprint (double-tap Shift)"><i style:width={`${(me.tacT / TAC_TIME) * 100}%`}></i></span>{/if}
  </div>
  <div class="br-mats" aria-label="Building materials">
    {#each MATERIAL_NAMES as name, m (name)}<span class:active={building && m === material} style:--mat={hex(MATERIAL_COLOURS[m])} title={name}><i aria-hidden="true"></i>{name} <b>{me.mats[m]}</b></span>{/each}
  </div>
</div>

<style>
  .br-vitals { background: var(--scrim-media); color: var(--on-scrim); border: 1px solid color-mix(in srgb, var(--on-scrim) 18%, transparent); border-radius: var(--radius-m); }
  .br-vitals { position: absolute; inset-block-end: 18px; inset-inline-start: 18px; padding: 10px; width: min(300px, 40vw); display: grid; gap: 6px; pointer-events: none; }
  .br-bar { position: relative; height: 22px; border-radius: var(--radius-s); background: color-mix(in srgb, var(--on-scrim) 15%, transparent); overflow: hidden; }
  .br-bar span { position: absolute; inset-block: 0; inset-inline-start: 0; }
  .br-bar.health span { background: var(--success); }
  .br-bar.shield span { background: var(--info); }
  .br-bar b { position: relative; display: inline-flex; align-items: center; gap: 6px; padding-inline-start: 8px; font-size: var(--fs-m); font-weight: 600; line-height: 22px; font-variant-numeric: tabular-nums; text-shadow: 0 1px 2px var(--scrim-media); }
  .br-bar svg { width: 14px; height: 14px; fill: var(--on-scrim); }
  .br-mats { display: flex; flex-wrap: wrap; gap: 4px; font-size: var(--fs-s); --mat: var(--on-scrim); }
  .br-mats span { display: inline-flex; align-items: center; gap: 4px; padding: 2px 6px; border-radius: var(--radius-s); background: color-mix(in srgb, var(--on-scrim) 10%, transparent); font-variant-numeric: tabular-nums; }
  .br-mats span.active { box-shadow: inset 0 0 0 2px var(--mat); }
  .br-mats i { width: 10px; height: 10px; border-radius: 2px; background: var(--mat); }
  /* Grenades, killstreaks and tactical-sprint stamina under the vitals. */
  .br-gear { display: flex; flex-wrap: wrap; gap: 4px; font-size: var(--fs-s); }
  .br-gear span { display: inline-flex; align-items: center; gap: 4px; padding: 2px 6px; border-radius: var(--radius-s); background: color-mix(in srgb, var(--on-scrim) 10%, transparent); font-variant-numeric: tabular-nums; }
  .br-gear span.empty { opacity: .5; }
  .br-gear kbd { font: inherit; font-size: var(--fs-xs); padding: 0 4px; border-radius: var(--radius-s); background: color-mix(in srgb, var(--on-scrim) 22%, transparent); }
  .br-gear .br-streak.ready { background: color-mix(in srgb, var(--warning) 45%, transparent); font-weight: 600; }
  .br-gear .br-tac { width: 60px; height: 6px; padding: 0; overflow: hidden; }
  .br-gear .br-tac i { display: block; height: 100%; background: var(--warning); }
</style>
