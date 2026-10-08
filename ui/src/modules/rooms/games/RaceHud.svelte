<script lang="ts">
 import type {GameState} from './types';
 import {trackFor} from './maps';
 import {courseOutline,courseBounds,raceClock,racePosition} from './race-hud';
 import {driverFor} from './roster';
 let {state,player,names}:{state:GameState;player:number;names:string[]}=$props();
 const track=$derived(trackFor(state.config.map));
 const self=$derived(state.players[player]);
 const place=$derived(racePosition(state,player));
 const itemNames:Record<string,string>={boost:'Turbo',pulse:'Shockwave',shield:'Bubble shield',seeker:'Seeker'};
 const itemSymbols:Record<string,string>={boost:'»',pulse:'◎',shield:'◇',seeker:'➤'};
</script>
<div class="race-position" aria-label={`Position ${place} of 2`}><strong>{place}<small>{place===1?'st':'nd'}</small></strong><span>{driverFor(self.character).name}</span></div>
<div class="race-lap"><span>Lap</span><strong>{Math.min(3,self.lap+1)}<small> / 3</small></strong><time>{raceClock(state.elapsed)}</time></div>
<div class="race-map" aria-label="Course map">
 <svg viewBox={courseBounds(track)} role="img" aria-label={`${track.name}, your position and opponent`}>
  <polyline points={courseOutline(track)} class="race-map-road"/>
  <circle cx={track.route[0].x} cy={track.route[0].z} r="3" class="race-map-start"/>
  {#each state.players as racer}<circle cx={racer.x} cy={racer.z} r={racer.id===player?4:3.2} class:race-map-self={racer.id===player} class:race-map-rival={racer.id!==player}><title>{names[racer.id]}</title></circle>{/each}
 </svg><span>{track.name}</span>
</div>
<div class="race-item" class:race-item-filled={self.item} aria-label={`Item: ${self.item?itemNames[self.item]:'Empty'}`}><b aria-hidden="true">{self.item?itemSymbols[self.item]:'?'}</b><span>{self.item?itemNames[self.item]:'Find an item box'}</span><kbd>E</kbd></div>
<div class="race-speed"><strong>{Math.round(Math.abs(self.speed)*3.6)}</strong><span>km/h</span><div class="race-charge"><progress value={self.boost>0?1.5:self.driftCharge} max="1.5" aria-label="Drift boost charge"></progress><span>{self.boost>0?'Turbo!':self.drifting?'Hold that drift':self.underwater?'Under the waves':!self.grounded?'Space · air trick':self.offTrack?'Off road':'Let’s race'}</span></div></div>
{#if self.shield>0}<div class="race-shield">Shield active</div>{/if}
<style>
 .race-position,.race-lap,.race-map,.race-item,.race-speed,.race-shield{position:absolute;color:var(--on-scrim);pointer-events:none;text-shadow:0 2px 5px var(--scrim-media);}
 .race-position{inset-block-start:72px;inset-inline-start:24px;display:grid;}.race-position strong{font-size:clamp(50px,6vw,84px);font-style:italic;line-height:1;letter-spacing:-0.01em;}.race-position small{font-size:var(--fs-2xl);letter-spacing:0;}.race-position>span{font-size:var(--fs-m);font-weight:600;margin-block-start:6px;}
 .race-lap{inset-block-start:74px;inset-inline-end:22px;display:grid;gap:4px;text-align:end;}.race-lap>span{font-size:var(--fs-m);}.race-lap strong{font-size:var(--fs-2xl);}.race-lap small{font-size:var(--fs-m);}.race-lap time{font-size:var(--fs-m);font-variant-numeric:tabular-nums;}
 .race-map{inset-inline-end:20px;inset-block-end:25px;width:150px;padding:10px;border-radius:var(--radius-l);background:var(--scrim-media);text-align:center;}.race-map svg{width:100%;height:125px;overflow:visible;}.race-map span{font-size:var(--fs-s);}.race-map-road{fill:none;stroke:var(--on-scrim);stroke-opacity:.35;stroke-width:7;stroke-linecap:round;stroke-linejoin:round;}.race-map-self{fill:var(--accent-solid);stroke:var(--on-scrim);stroke-width:1.2;}.race-map-rival{fill:var(--warning);stroke:var(--on-scrim);stroke-width:1;}.race-map-start{fill:var(--on-scrim);}
 .race-item{inset-block-start:48%;inset-inline-start:22px;display:grid;grid-template-columns:58px auto;align-items:center;gap:4px 10px;max-width:220px;background:var(--scrim-media);padding:10px 14px;border:1px solid color-mix(in srgb,var(--on-scrim) 25%,transparent);border-radius:var(--radius-l);}.race-item b{grid-row:span 2;font-size:calc(var(--fs-hero) * 1.6);text-align:center;}.race-item span{font-size:var(--fs-m);font-weight:600;}.race-item kbd{font-size:var(--fs-s);}.race-item-filled{border-color:var(--accent-solid);}
 .race-speed{inset-inline-start:24px;inset-block-end:25px;display:flex;align-items:baseline;gap:8px;}.race-speed>strong{font-size:clamp(38px,5vw,66px);font-style:italic;line-height:1;}.race-speed>span{font-size:var(--fs-m);}.race-charge{display:grid;gap:6px;margin-inline-start:16px;min-width:120px;font-size:var(--fs-m);}.race-charge progress{width:140px;height:7px;accent-color:var(--warning);}.race-shield{inset-inline-start:24px;inset-block-end:100px;font-size:var(--fs-m);}
 @media(max-width:640px){.race-position{inset-block-start:94px;inset-inline-start:14px;}.race-position strong{font-size:calc(var(--fs-hero) * 1.7);}.race-lap{inset-block-start:94px;inset-inline-end:14px;}.race-item{inset-inline-start:12px;inset-block-start:37%;grid-template-columns:36px auto;padding:8px;}.race-item b{font-size:calc(var(--fs-hero) * 1.15);}.race-item span{font-size:var(--fs-s);}.race-speed{inset-inline-start:14px;inset-block-end:18px;gap:4px;}.race-speed>strong{font-size:calc(var(--fs-hero) * 1.4);}.race-charge{margin-inline-start:8px;min-width:90px;font-size:var(--fs-s);}.race-charge progress{width:90px;}.race-map{width:82px;inset-inline-end:8px;inset-block-end:12px;padding:6px;}.race-map svg{height:65px;}.race-map span{display:none;}.race-shield{inset-block-end:70px;}}
</style>
