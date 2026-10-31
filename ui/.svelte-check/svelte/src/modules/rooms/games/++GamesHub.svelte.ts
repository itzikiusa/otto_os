///<reference types="svelte" />
;
import { tick } from 'svelte';
import { announceShown } from '../../../lib/announce.svelte';
import PageHeader from '../../../lib/components/PageHeader.svelte';
import PageBody from '../../../lib/components/PageBody.svelte';
import { router } from '../../../lib/router.svelte';
import { api, baseUrl } from '../../../lib/api/client';
import type { GameRoomCredential, GameRoomKind, GameRoomMap } from '../../../lib/api/game-room-types';
import { GAMES, gameFor, RACERS, FIGHTERS, type Difficulty } from './catalog';
import { kartSeats, shooterSeats } from './setup';
import type { RacerId } from './kart/roster';
import { SHOOTER_MODES, TEAM_MODES, type FighterId, type ShooterMode } from './shooter/catalog';
import type { KartSetup } from './kart/KartGame.svelte';
import type { ShooterSetup, TeamSize } from './shooter/sim';
import KartGame from './kart/KartGame.svelte';
import ShooterGame from './shooter/ShooterGame.svelte';
import GameMatch from './GameMatch.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let kind = $state<GameRoomKind>('kart');
  let mode = $state<'solo' | 'room'>('solo');
  let difficulty = $state<Difficulty>('normal');
  let map = $state<GameRoomMap>('meadow');
  let racer = $state<RacerId>('oobi');
  let fighter = $state<FighterId>('male-a');
  let bots = $state(11);
  /** Storm Royale: solo, duos or squads; the game mode (royale, TDM, Domination, Zombies, Apocalypse). */
  let teamSize = $state<TeamSize>(1);
  let shooterMode = $state<ShooterMode>('royale');
  const modeInfo = $derived(SHOOTER_MODES.find(m => m.id === shooterMode) ?? SHOOTER_MODES[0]);
  let name = $state('Player');
  let kartSetup = $state<KartSetup | null>(null);
  let shooterSetup = $state<ShooterSetup | null>(null);
  let credential = $state<GameRoomCredential | null>(null);
  let busy = $state(false), error = $state(''), round = $state(0);
  const game = $derived(gameFor(kind));
  const playing = $derived(!!kartSetup || !!shooterSetup || !!credential);

  function choose(next: GameRoomKind) { kind = next; map = gameFor(next).maps[0].id; }
  async function start() {
    busy = true; error = '';
    try {
      const seed = (Date.now() ^ Math.floor(Math.random() * 1e9)) >>> 0;
      if (mode === 'solo') {
        if (kind === 'kart') kartSetup = { track: map as KartSetup['track'], difficulty, seed, seats: kartSeats([{ slot: 0, name: 'You', pick: racer }], true) };
        else shooterSetup = { map: map as ShooterSetup['map'], difficulty, seed, seats: shooterSeats([{ slot: 0, name: 'You', pick: fighter }], bots), teamSize, mode: shooterMode };
      } else {
        credential = await api.post<GameRoomCredential>('/game-rooms', { name: name.trim() || 'Player', config: { game: kind, map } });
      }
    } catch (e) { error = e instanceof Error ? e.message : 'Try again.'; }
    finally { busy = false; }
  }
  let titles = $state<HTMLElement>();
  /** Leaving a game unmounts every control that had focus; land it on the selected game's title. */
  async function exit() {
    kartSetup = null; shooterSetup = null; credential = null; round++;
    await tick();
    titles?.querySelector<HTMLButtonElement>('button[aria-pressed="true"]')?.focus();
  }
  function again() {
    const seed = (Date.now() ^ Math.floor(Math.random() * 1e9)) >>> 0;
    if (kartSetup) kartSetup = { ...kartSetup, seed };
    if (shooterSetup) shooterSetup = { ...shooterSetup, seed };
    round++;
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`games-page`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "title":playing ? game.title : 'Games',"subtitle":playing ? 'Rooms' : 'Play together between builds',actions:() => { async ()/*Ωignore_positionΩ*/ => {
      if(playing){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":exit,});   }}
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => { void exit(); router.go('rooms'); },});  }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
   PageHeader}
  if(kartSetup){
     { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}});round; { { const $$_emaGtraK2C = __sveltets_2_ensureComponent(KartGame); new $$_emaGtraK2C({ target: __sveltets_2_any(), props: {         "setup":kartSetup,"selfSlot":0,racer,"onexit":exit,"onrematch":again,}});}} PageBody}
  } else if (shooterSetup){
     { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}});round; { { const $$_emaGretoohS2C = __sveltets_2_ensureComponent(ShooterGame); new $$_emaGretoohS2C({ target: __sveltets_2_any(), props: {         "setup":shooterSetup,"selfSlot":0,fighter,"onexit":exit,"onrematch":again,}});}} PageBody}
  } else if (credential){
     { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}}); { const $$_hctaMemaG2C = __sveltets_2_ensureComponent(GameMatch); new $$_hctaMemaG2C({ target: __sveltets_2_any(), props: {           credential,"origin":baseUrl(),"pick":kind === 'kart' ? racer : fighter,difficulty,bots,teamSize,shooterMode,"onexit":exit,}});} PageBody}
  }else{
     { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
       { svelteHTML.createElement("div", { "class":`games-chooser`,});
         { const $$_div3 = svelteHTML.createElement("div", {      "class":`games-titles`,"role":`group`,"aria-label":`Choose a game`,});titles = $$_div3;
             for(let g of __sveltets_2_ensureArray(GAMES)){g.kind;
             { svelteHTML.createElement("button", {     "aria-pressed":kind === g.kind,"onclick":() => choose(g.kind),});kind === g.kind;
               { svelteHTML.createElement("img", {    "src":g.cover,"alt":g.coverAlt,});}
               { svelteHTML.createElement("div", {}); { svelteHTML.createElement("h2", {});g.title; } { svelteHTML.createElement("p", {});g.tagline; } { svelteHTML.createElement("span", {});g.summary; } }
             }
          }
         }
         { svelteHTML.createElement("div", { "class":`games-setup`,});
           { svelteHTML.createElement("section", {   "class":`games-options`,"aria-labelledby":`games-setup-title`,});
             { svelteHTML.createElement("h3", { "id":`games-setup-title`,});    }
             { svelteHTML.createElement("fieldset", {}); { svelteHTML.createElement("legend", {});  }
               { svelteHTML.createElement("div", { "class":`games-segments`,});
                 { svelteHTML.createElement("button", {   "aria-pressed":mode === 'solo',"onclick":() => (mode = 'solo'),});   }
                 { svelteHTML.createElement("button", {   "aria-pressed":mode === 'room',"onclick":() => (mode = 'room'),});  }
               }
             }
             { svelteHTML.createElement("label", {}); 
               { svelteHTML.createElement("select", { "bind:value":difficulty,});/*Ωignore_startΩ*/() => difficulty = __sveltets_2_any(null);/*Ωignore_endΩ*/ { svelteHTML.createElement("option", { "value":`easy`,});  } { svelteHTML.createElement("option", { "value":`normal`,});  } { svelteHTML.createElement("option", { "value":`hard`,});  } }
             }
            if(kind === 'shooter'){
               { svelteHTML.createElement("fieldset", {}); { svelteHTML.createElement("legend", {});  }
                 { svelteHTML.createElement("div", { "class":`games-segments`,});
                     for(let m of __sveltets_2_ensureArray(SHOOTER_MODES)){m.id; { svelteHTML.createElement("button", {     "aria-pressed":shooterMode === m.id,"title":m.blurb,"onclick":() => { shooterMode = m.id; bots = m.bots; },});m.short; }}
                 }
               }
               { svelteHTML.createElement("p", { "class":`games-note`,});modeInfo.blurb; }
              if(modeInfo.teams){
               { svelteHTML.createElement("fieldset", {}); { svelteHTML.createElement("legend", {});  }
                 { svelteHTML.createElement("div", { "class":`games-segments`,});
                     for(let t of __sveltets_2_ensureArray(TEAM_MODES)){t.size; { svelteHTML.createElement("button", {   "aria-pressed":teamSize === t.size,"onclick":() => (teamSize = t.size),});t.name; }}
                 }
               }
              }
              if(modeInfo.teams && teamSize > 1){ { svelteHTML.createElement("p", { "class":`games-note`,});mode === 'solo' ? `Bots fill your ${teamSize === 2 ? 'duo' : 'squad'} and the other teams.` : 'People who join play on the same team first; bots fill the rest.';      }}
               { svelteHTML.createElement("label", {});    bots;
                 { svelteHTML.createElement("input", {        "type":`range`,"min":mode === 'solo' ? 1 : 0,"max":`15`,"bind:value":bots,});/*Ωignore_startΩ*/() => bots = __sveltets_2_any(null);/*Ωignore_endΩ*/}
               }
            }
            if(mode === 'room'){
               { svelteHTML.createElement("label", {});   { svelteHTML.createElement("input", {      "dir":`auto`,"maxlength":40,"bind:value":name,});/*Ωignore_startΩ*/() => name = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
               { svelteHTML.createElement("p", { "class":`games-note`,});  game.maxPlayers;      kind === 'kart' ? 'CPU racers fill the remaining grid slots' : 'bots fill the island';  }
            }
             { svelteHTML.createElement("fieldset", {}); { svelteHTML.createElement("legend", {});kind === 'kart' ? 'Choose your racer' : 'Choose your survivor'; }
               { svelteHTML.createElement("div", { "class":`games-roster`,});
                if(kind === 'kart'){
                     for(let r of __sveltets_2_ensureArray(RACERS)){r.id;
                     { svelteHTML.createElement("button", {       "aria-pressed":racer === r.id,"onclick":() => (racer = r.id),"title":r.tagline,});racer === r.id;  { svelteHTML.createElement("img", {  "src":`/room-games/portraits/racer-${r.id}.webp`,"alt":"",});} { svelteHTML.createElement("strong", {});r.name; } { svelteHTML.createElement("small", {});r.tagline; } }
                  }
                }else{
                     for(let f of __sveltets_2_ensureArray(FIGHTERS)){f.id;
                     { svelteHTML.createElement("button", {     "aria-pressed":fighter === f.id,"onclick":() => (fighter = f.id),});fighter === f.id;  { svelteHTML.createElement("img", {  "src":`/room-games/portraits/fighter-${f.id}.webp`,"alt":"",});} { svelteHTML.createElement("strong", {});f.name; } }
                  }
                }
               }
             }
             { svelteHTML.createElement("div", { "class":`games-start`,});
               { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":busy,"onclick":start,});busy ? 'Creating room…' : mode === 'solo' ? (kind === 'kart' ? 'Start race' : 'Drop in') : 'Create game room'; }
              if(error){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: `${mode === 'room' ? 'Couldn’t create the game room.' : 'Couldn’t start the game.'} ${error}`, assertive: true })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`field-error`,});mode === 'room' ? 'Couldn’t create the game room.' : 'Couldn’t start the game.'; error; }}}
             }
           }
           { svelteHTML.createElement("section", {   "class":`games-environments`,"aria-labelledby":`games-map-title`,});
             { svelteHTML.createElement("h3", { "id":`games-map-title`,});kind === 'kart' ? 'Choose a track' : 'Choose an island'; }
               for(let item of __sveltets_2_ensureArray(game.maps)){item.id;
               { svelteHTML.createElement("button", {     "aria-pressed":map === item.id,"onclick":() => (map = item.id),});map === item.id;
                  { svelteHTML.createElement("img", {  "src":`/room-games/map-${item.id}.webp`,"alt":"",});}
                 { svelteHTML.createElement("span", {}); { svelteHTML.createElement("strong", {});item.name; } { svelteHTML.createElement("small", {});item.blurb; } }
               }
            }
             { svelteHTML.createElement("p", { "class":`games-note`,});kind === 'kart' ? 'W/S gas & brake · A/D steer · Space drift & trick · E item · Q look back' : 'WASD move · Mouse aim · Click fire · Right-click aim down sights · 1–5 hotbar · G grenade · T smoke / flash · X killstreak · C crouch / slide · E interact · R reload · Space jump / mantle'; }
           }
         }
       }
     PageBody}
  }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const GamesHub__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type GamesHub__SvelteComponent_ = ReturnType<typeof GamesHub__SvelteComponent_>;
/*Ωignore_endΩ*/export default GamesHub__SvelteComponent_;