<script lang="ts">
  // Storm Royale match results cover (ShooterGame's end screen): the headline,
  // the mode summary line and the scoreboard, plus rematch / leave.
  import type { Fighter, ShooterResult, ShooterState } from '../sim';
  import { modeName as modeTitle, teamColour } from '../catalog';
  import { isRoyale, respawns } from '../modes/index';
  import { hex } from './minimap';

  interface Props {
    hud: ShooterState; me: Fighter; results: ShooterResult[]; headline: string; mapLabel: string; modeName: string;
    ours: number; theirs: number; rematchLabel: string; rematchDisabled: boolean; onrematch: () => void; onexit: () => void;
  }
  let { hud, me, results, headline, mapLabel, modeName, ours, theirs, rematchLabel, rematchDisabled, onrematch, onexit }: Props = $props();
  const mode = $derived(hud.mode);
  const arena = $derived(respawns(mode));
  const obj = $derived(hud.obj);
  const squad = $derived(hud.squad ?? 1);
  const mm = (s: number) => `${Math.floor(Math.max(0, s) / 60)}:${String(Math.floor(Math.max(0, s) % 60)).padStart(2, '0')}`;
</script>

<div class="br-cover br-results">
  <p class="br-kicker">{mapLabel} · {modeTitle(mode)}{isRoyale(mode) && squad > 1 ? ` · ${modeName}` : ''} · Match results · {mm(hud.clock)}</p>
  {#if arena && obj}
    <h2 class:victory={hud.winTeam === me.team}>{headline}</h2>
    <p>Your team {ours} · enemy {theirs} · {me.kills} eliminations · {me.deaths} deaths · {Math.round(me.damage)} damage</p>
  {:else if mode === 'zombies' && obj}
    <h2 class:victory={me.place === 1}>{headline}</h2>
    <p>{obj.aboard > 0 ? `${obj.aboard} made the helicopter after ${obj.round} rounds` : `The crew survived ${Math.max(0, obj.round - 1)} of ${obj.lastRound} rounds`} · {me.kills} infected killed · {me.points.toLocaleString()} points</p>
  {:else}
    <h2 class:victory={me.place === 1}>{headline}</h2>
    <p>{me.kills} eliminations · {Math.round(me.damage)} damage · survived {mm(me.aliveTime)}</p>
  {/if}
  <table>
    <thead><tr><th scope="col">{arena ? 'Team' : 'Place'}</th><th scope="col">Survivor</th><th scope="col">{mode === 'zombies' ? 'Kills' : 'Elims'}</th>{#if arena}<th scope="col">Deaths</th>{/if}<th scope="col">{mode === 'zombies' ? 'Points' : 'Damage'}</th></tr></thead>
    <tbody>{#each results.filter((r, i) => i < 8 || (squad > 1 ? r.team === me.team : r.id === me.id)) as r (r.id)}<tr class:me={r.id === me.id} class:mate={squad > 1 && r.team === me.team}><td>{arena ? (r.team === me.team ? 'Yours' : 'Enemy') : `#${r.place}`}</td><td>{#if squad > 1}<i class="br-swatch" style:background={hex(teamColour(r.team))} title={`Team ${r.team + 1}`}></i>{/if}<img src={`/room-games/portraits/fighter-${r.fighter}.webp`} alt="" /><span>{r.name}{r.human ? '' : ' (bot)'}</span></td><td>{r.kills}</td>{#if arena}<td>{r.deaths}</td>{/if}<td>{mode === 'zombies' ? r.points : r.damage}</td></tr>{/each}</tbody>
  </table>
  <div class="br-actions"><button class="btn primary" disabled={rematchDisabled} onclick={onrematch}>{rematchLabel}</button><button class="btn" onclick={onexit}>Back to games</button></div>
</div>

<style>
  .br-cover { position: absolute; inset: 0; z-index: 6; display: flex; flex-direction: column; align-items: center; justify-content: safe center; overflow-y: auto; gap: 12px; padding: 24px; text-align: center; background: color-mix(in srgb, var(--surface) 90%, transparent); }
  .br-cover h2 { margin: 0; font-size: var(--fs-2xl); }
  .br-cover p { margin: 0; max-width: 600px; color: var(--text-dim); line-height: 1.6; }
  .br-cover h2.victory { color: var(--warning); font-size: clamp(28px, 5vw, 52px); letter-spacing: -0.01em; }
  .br-results table { border-collapse: collapse; min-width: min(560px, 100%); font-size: var(--fs-m); font-variant-numeric: tabular-nums; }
  .br-results th { text-align: start; font-size: var(--fs-s); color: var(--text-dim); font-weight: 500; padding: 4px 10px; }
  .br-results td { padding: 4px 10px; border-block-start: 1px solid var(--border); text-align: start; }
  .br-results td:nth-child(2) { display: flex; align-items: center; gap: 8px; }
  .br-results td img { width: 32px; height: 32px; object-fit: contain; }
  .br-results tr.me td { background: var(--accent-soft); font-weight: 600; }
  .br-results tr.mate td:first-child { box-shadow: inset 3px 0 0 var(--accent-solid); }
  .br-swatch { width: 10px; height: 10px; border-radius: 50%; flex: none; }
  .br-kicker { text-transform: uppercase; letter-spacing: .06em; font-size: var(--fs-s); }
  .br-actions { display: flex; gap: 8px; flex-wrap: wrap; justify-content: center; }
  @media (max-width: 640px) { .br-cover { padding: 16px; gap: 8px; } .br-results table { min-width: 0; width: 100%; font-size: var(--fs-s); } .br-results th, .br-results td { padding: 2px 6px; } .br-results td img { width: 22px; height: 22px; } }
</style>
