///<reference types="svelte" />
;// Storm Royale match results cover (ShooterGame's end screen): the headline,
// the mode summary line and the scoreboard, plus rematch / leave.

import type { Fighter, ShooterResult, ShooterState } from '../sim';
import { modeName as modeTitle, teamColour } from '../catalog';
import { isRoyale, respawns } from '../modes/index';
import { hex } from './minimap';
;

  interface Props {
    hud: ShooterState; me: Fighter; results: ShooterResult[]; headline: string; mapLabel: string; modeName: string;
    ours: number; theirs: number; rematchLabel: string; rematchDisabled: boolean; onrematch: () => void; onexit: () => void;
  };function $$render() {

  
  
  
  
  
  

  let { hud, me, results, headline, mapLabel, modeName, ours, theirs, rematchLabel, rematchDisabled, onrematch, onexit }: Props = $props();
  const mode = $derived(hud.mode);
  const arena = $derived(respawns(mode));
  const obj = $derived(hud.obj);
  const squad = $derived(hud.squad ?? 1);
  const mm = (s: number) => `${Math.floor(Math.max(0, s) / 60)}:${String(Math.floor(Math.max(0, s) % 60)).padStart(2, '0')}`;
;
async () => {

 { svelteHTML.createElement("div", { "class":`br-cover br-results`,});
   { svelteHTML.createElement("p", { "class":`br-kicker`,});mapLabel;  modeTitle(mode);isRoyale(mode) && squad > 1 ? ` · ${modeName}` : '';     mm(hud.clock); }
  if(arena && obj){
     { svelteHTML.createElement("h2", { });hud.winTeam === me.team;headline; }
     { svelteHTML.createElement("p", {});  ours;   theirs;  me.kills;   me.deaths;   Math.round(me.damage);  }
  } else if (mode === 'zombies' && obj){
     { svelteHTML.createElement("h2", { });me.place === 1;headline; }
     { svelteHTML.createElement("p", {});obj.aboard > 0 ? `${obj.aboard} made the helicopter after ${obj.round} rounds` : `The crew survived ${Math.max(0, obj.round - 1)} of ${obj.lastRound} rounds`;  me.kills;    me.points.toLocaleString();  }
  }else{
     { svelteHTML.createElement("h2", { });me.place === 1;headline; }
     { svelteHTML.createElement("p", {});me.kills;   Math.round(me.damage);    mm(me.aliveTime); }
  }
   { svelteHTML.createElement("table", {});
     { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", { "scope":`col`,});arena ? 'Team' : 'Place'; } { svelteHTML.createElement("th", { "scope":`col`,});  } { svelteHTML.createElement("th", { "scope":`col`,});mode === 'zombies' ? 'Kills' : 'Elims'; }if(arena){ { svelteHTML.createElement("th", { "scope":`col`,});  }} { svelteHTML.createElement("th", { "scope":`col`,});mode === 'zombies' ? 'Points' : 'Damage'; } } }
     { svelteHTML.createElement("tbody", {});   for(let r of __sveltets_2_ensureArray((results.filter((r, i) => i < 8 || (squad > 1 ? r.team === me.team : r.id === me.id))))){r.id; { svelteHTML.createElement("tr", {  });r.id === me.id;squad > 1 && r.team === me.team; { svelteHTML.createElement("td", {});arena ? (r.team === me.team ? 'Yours' : 'Enemy') : `#${r.place}`; } { svelteHTML.createElement("td", {});if(squad > 1){ { svelteHTML.createElement("i", {    "class":`br-swatch`,"title":`Team ${r.team + 1}`,});__sveltets_2_ensureType(String, Number, hex(teamColour(r.team))); }}  { svelteHTML.createElement("img", {  "src":`/room-games/portraits/fighter-${r.fighter}.webp`,"alt":"",});} { svelteHTML.createElement("span", {});r.name;r.human ? '' : ' (bot)'; } } { svelteHTML.createElement("td", {});r.kills; }if(arena){ { svelteHTML.createElement("td", {});r.deaths; }} { svelteHTML.createElement("td", {});mode === 'zombies' ? r.points : r.damage; } }} }
   }
   { svelteHTML.createElement("div", { "class":`br-actions`,}); { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":rematchDisabled,"onclick":onrematch,});rematchLabel; } { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onexit,});   } }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MatchResults__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MatchResults__SvelteComponent_ = ReturnType<typeof MatchResults__SvelteComponent_>;
/*Ωignore_endΩ*/export default MatchResults__SvelteComponent_;