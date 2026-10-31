///<reference types="svelte" />
;
import { onMount, tick, untrack } from 'svelte';
import { copyText } from '../../../lib/clipboard';
import { announce } from '../../../lib/announce.svelte';
import { GameClient, type GameConnection } from './client';
import type { GameRoomCredential, GameRoomEvent, GameRoomMember, GameRoomState } from '../../../lib/api/game-room-types';
import { gameFor, RACERS, FIGHTERS, type Difficulty } from './catalog';
import { kartSeats, shooterSeats, type HumanSeat } from './setup';
import KartGame, { type KartLink, type KartSetup } from './kart/KartGame.svelte';
import ShooterGame from './shooter/ShooterGame.svelte';
import type { ShooterLink, ShooterSetup, TeamSize } from './shooter/sim';
import * as kartNet from './kart/net';
import * as shooterNet from './shooter/net';
import { standings, type KartEvent, type KartInput, type KartState } from './kart/sim';
import type { RacerId } from './kart/roster';
import type { FighterId, ShooterMode } from './shooter/catalog';
import { shooterResults, type ShooterEvent, type ShooterInput, type ShooterState } from './shooter/sim';
;

  interface Props { credential: GameRoomCredential; origin: string; pick?: string; difficulty?: Difficulty; bots?: number; teamSize?: TeamSize; shooterMode?: ShooterMode; onexit: () => void };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { credential, origin, pick = '', difficulty = 'normal', bots = 9, teamSize = 1, shooterMode = 'royale', onexit }: Props = $props();
  const kind = $derived(credential.config.game);
  const game = $derived(gameFor(kind));
  const isHost = $derived(credential.role === 'host');

  let client: GameClient;
  let room = $state<GameRoomState | null>(null);
  let connection = $state<GameConnection>('connecting');
  let error = $state(''), closed = $state(''), copied = $state(false);
  let backButton = $state<HTMLButtonElement>();
  let choice = $state(untrack(() => pick));
  let cpuFill = $state(true);
  let botCount = $state(untrack(() => bots));
  let cpuLevel = $state<Difficulty>(untrack(() => difficulty));
  let kartSetup = $state<KartSetup | null>(null);
  let shooterSetup = $state<ShooterSetup | null>(null);
  let finished = $state(false);
  let seq = 0, lastSeq = new Map<number, number>();
  const kartLink: KartLink = { role: 'host', inputs: new Map(), snapshots: [] };
  const shooterLink: ShooterLink = { role: 'host', inputs: new Map(), snapshots: [] };

  const self = $derived(room?.members.find(m => m.id === credential.member_id));
  const selfSlot = $derived(self?.slot ?? (isHost ? 0 : 1));
  const active = $derived(room?.members.filter(m => !m.left) ?? []);
  const everyoneReady = $derived(active.length >= 2 && active.every(m => m.connected && m.ready));
  const memberName = (slot: number) => room?.members.find(m => m.slot === slot)?.name ?? 'Player';

  $effect(() => { if (!choice) choice = kind === 'kart' ? 'oobi' : 'male-a'; });

  function resetRound() {
    seq = 0; lastSeq = new Map(); finished = false;
    kartLink.inputs!.clear(); kartLink.snapshots!.length = 0;
    shooterLink.inputs!.clear(); shooterLink.snapshots!.length = 0;
  }
  function beginRound(state: GameRoomState) {
    // The host seats every connected member; guests only render snapshots.
    const humans: HumanSeat[] = state.members.filter(m => !m.left).map(m => ({ slot: m.slot, name: m.name, pick: m.slot === selfSlot ? choice : '' }));
    const seed = (Date.now() ^ Math.floor(Math.random() * 1e9)) >>> 0;
    if (kind === 'kart') {
      kartLink.role = isHost ? 'host' : 'guest';
      kartSetup = isHost ? { track: state.config.map as KartSetup['track'], difficulty: cpuLevel, seed, seats: kartSeats(humans.map(h => ({ ...h, pick: h.pick || RACERS[h.slot % RACERS.length].id })), cpuFill) } : null;
    } else {
      shooterLink.role = isHost ? 'host' : 'guest';
      shooterSetup = isHost ? { map: state.config.map as ShooterSetup['map'], difficulty: cpuLevel, seed, seats: shooterSeats(humans.map(h => ({ ...h, pick: h.pick || FIGHTERS[(h.slot * 3) % FIGHTERS.length].id })), botCount), teamSize, mode: shooterMode } : null;
    }
  }

  function handle(event: GameRoomEvent) {
    if (event.type === 'state') {
      if (event.room.id !== credential.room_id) return;
      const prev = room;
      if (!prev || prev.round !== event.room.round || prev.generation !== event.room.generation) { seq = 0; lastSeq = new Map(); }
      if (prev && prev.round !== event.room.round) { resetRound(); kartSetup = null; shooterSetup = null; }
      room = event.room;
      error = '';
      if (event.room.phase === 'playing' && (!prev || prev.phase === 'lobby')) { resetRound(); beginRound(event.room); }
      return;
    }
    if (event.type === 'closed') {
      // Closed mid-game: the canvas (and any focus in it) unmounts, so focus
      // moves to "Back to games" and the end is announced (a11y-6-09).
      closed = event.reason; announce(`Match ended. ${event.reason}`);
      void tick().then(() => backButton?.focus());
      return;
    }
    if (event.type === 'error') { if (!event.message.includes('Stale game round')) { error = event.message; announce(event.message, { assertive: true }); } return; }
    if (event.type === 'pong' || !room || !('round' in event) || event.round !== room.round || event.generation !== room.generation) return;
    if (event.type === 'snapshot' && !isHost) {
      if (event.seq <= (lastSeq.get(0) ?? 0)) return;
      lastSeq.set(0, event.seq);
      const at = performance.now();
      if (kind === 'kart') {
        const state = kartNet.decodeSnapshot(event.data, (slot, racer) => (slot >= 0 ? memberName(slot) : RACERS.find(r => r.id === racer)?.name ?? 'CPU'));
        if (state) { kartLink.snapshots!.push({ state, at }); if (kartLink.snapshots!.length > 4) kartLink.snapshots!.shift(); }
      } else {
        const prev = shooterLink.snapshots!.at(-1)?.state;
        const state = shooterNet.decodeSnapshot(event.data, slot => memberName(slot), prev?.loot ?? null, prev?.builds ?? null);
        if (state) { shooterLink.snapshots!.push({ state, at }); if (shooterLink.snapshots!.length > 4) shooterLink.snapshots!.shift(); }
      }
    }
    if (event.type === 'input' && isHost) {
      if (event.seq <= (lastSeq.get(event.slot) ?? 0)) return;
      lastSeq.set(event.slot, event.seq);
      if (kind === 'kart') { const d = kartNet.decodeInput(event.data); if (d) kartLink.inputs!.set(event.slot, d); }
      else { const d = shooterNet.decodeInput(event.data); if (d) shooterLink.inputs!.set(event.slot, d); }
    }
    if (event.type === 'finished') finished = true;
  }

  onMount(() => {
    client = new GameClient(origin, credential, handle, status => {
      connection = status;
      if (status !== 'connected') { kartLink.inputs!.clear(); shooterLink.inputs!.clear(); }
    });
    client.connect();
    return () => client.dispose();
  });

  const live = () => room && room.phase === 'playing' && !room.paused;
  function sendSnapshot(data: unknown) { if (live() && room) client.send({ type: 'snapshot', round: room.round, generation: room.generation, seq: ++seq, data }); }
  function kartFrame(state: KartState, events: KartEvent[]) { sendSnapshot(kartNet.encodeSnapshot(state, events)); }
  function kartInput(input: KartInput, racer: RacerId) { if (live() && room) client.send({ type: 'input', round: room.round, generation: room.generation, seq: ++seq, data: kartNet.encodeInput(input, racer) }); }
  function kartFinish(state: KartState) {
    if (!isHost || !room) return;
    kartFrame(state, state.events);
    const result = standings(state).map(r => ({ place: r.place, name: r.name.slice(0, 40), time: Math.round(r.time * 1000) / 1000 }));
    client.send({ type: 'finish', round: room.round, generation: room.generation, result: { game: 'kart', standings: result } });
  }
  function shooterFrame(state: ShooterState, events: ShooterEvent[], withLoot: boolean, withBuilds = false) { sendSnapshot(shooterNet.encodeSnapshot(state, events, withLoot, withBuilds)); }
  function shooterInput(input: ShooterInput, fighter: FighterId) { if (live() && room) client.send({ type: 'input', round: room.round, generation: room.generation, seq: ++seq, data: shooterNet.encodeInput(input, fighter) }); }
  function shooterFinish(state: ShooterState) {
    if (!isHost || !room) return;
    shooterFrame(state, state.events, true);
    const result = shooterResults(state).map(r => ({ place: r.place, name: r.name.slice(0, 40), kills: r.kills }));
    client.send({ type: 'finish', round: room.round, generation: room.generation, result: { game: 'shooter', standings: result } });
  }
  function rematch() { client.send({ type: 'rematch' }); }
  async function copyInvite() {
    if (await copyText(credential.invite_url ?? '')) { copied = true; announce('Invitation copied'); return; }
    error = 'Select the invitation link and copy it.'; announce(error, { assertive: true });
  }
  const statusOf = (m: GameRoomMember) => (m.left ? 'Left' : !m.connected ? 'Connecting…' : m.ready ? 'Ready' : 'Not ready');
  const rematchRequests = $derived(room?.members.filter(m => m.rematch && !m.left).length ?? 0);
  const lobbyLine = $derived(
    connection === 'connected' ? `${game.maps.find(m => m.id === credential.config.map)?.name ?? ''} · up to ${game.maxPlayers} people`
    : connection === 'connecting' ? 'Connecting to game room…'
    : connection === 'gave-up' ? 'Couldn’t reconnect to the game room.'
    : 'Connection lost. Reconnecting…');
  // One always-mounted polite status for the lobby: connection edges and readiness
  // changes reach screen readers (the visible line and roster are plain text).
  const readyCount = $derived(active.filter(m => m.ready).length);
  const lobbyAnnouncement = $derived(
    connection !== 'connected' ? lobbyLine
    : `${active.length} ${active.length === 1 ? 'player' : 'players'}, ${readyCount} ready.${everyoneReady ? (isHost ? ' Everyone is ready; you can start the match.' : ' Everyone is ready.') : ''}`);
  function reconnect() { error = ''; client.reconnect(); }
  const lostMidGame = $derived(connection === 'gave-up' && !!room && !closed && (room.phase === 'playing' || room.phase === 'finished'));
  $effect(() => { if (lostMidGame) announce('Couldn’t reconnect to the game room.', { assertive: true }); });
;
async () => {

 { svelteHTML.createElement("div", { "class":`match`,});
  if(closed){
     { svelteHTML.createElement("div", { "class":`match-wait`,}); { svelteHTML.createElement("h2", {});  } { svelteHTML.createElement("p", {});closed; } { const $$_button2 = svelteHTML.createElement("button", {    "class":`btn primary`,"onclick":onexit,});backButton = $$_button2;   } }
  } else if (room && (room.phase === 'playing' || room.phase === 'finished')){
    room.round; {
      if(kind === 'kart'){
         { const $$_emaGtraK1C = __sveltets_2_ensureComponent(KartGame); new $$_emaGtraK1C({ target: __sveltets_2_any(), props: {                        "setup":kartSetup,selfSlot,"racer":choice as RacerId,"link":kartLink,"names":memberName,"paused":connection !== 'connected' || room.paused,"onframe":isHost ? kartFrame : undefined,"oninput":isHost ? undefined : kartInput,"onfinish":kartFinish,onexit,"onrematch":rematch,"rematchLabel":isHost ? (rematchRequests ? `Back to lobby (${rematchRequests} want a rematch)` : 'Back to lobby') : self?.rematch ? 'Rematch requested' : 'Ask for a rematch',"rematchDisabled":!finished || (!isHost && !!self?.rematch),}});}
      }else{
         { const $$_emaGretoohS1C = __sveltets_2_ensureComponent(ShooterGame); new $$_emaGretoohS1C({ target: __sveltets_2_any(), props: {                        "setup":shooterSetup,selfSlot,"fighter":choice as FighterId,"link":shooterLink,"names":memberName,"paused":connection !== 'connected' || room.paused,"onframe":isHost ? shooterFrame : undefined,"oninput":isHost ? undefined : shooterInput,"onfinish":shooterFinish,onexit,"onrematch":rematch,"rematchLabel":isHost ? (rematchRequests ? `Back to lobby (${rematchRequests} want a rematch)` : 'Back to lobby') : self?.rematch ? 'Rematch requested' : 'Ask for a rematch',"rematchDisabled":!finished || (!isHost && !!self?.rematch),}});}
      }
    }
  }else{
     { svelteHTML.createElement("div", { "class":`match-wait`,});
       { svelteHTML.createElement("h2", {});game.title; }
       { svelteHTML.createElement("p", {});lobbyLine; }
      if(connection === 'gave-up'){ { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":reconnect,});  }}
      if(credential.invite_url){
         { svelteHTML.createElement("div", { "class":`match-invite`,});
           { svelteHTML.createElement("label", { "for":`game-invite`,});  }
           { svelteHTML.createElement("div", { "class":`match-invite-row`,}); { svelteHTML.createElement("input", {        "dir":`ltr`,"id":`game-invite`,"readonly":true,"value":credential.invite_url,"aria-label":`Game invitation`,});} { svelteHTML.createElement("button", {   "class":`btn`,"onclick":copyInvite,});copied ? 'Copied' : 'Copy invitation'; } }
           { svelteHTML.createElement("p", {});     game.maxPlayers - 1;                  }
         }
      }
       { svelteHTML.createElement("fieldset", { "class":`match-roster`,}); { svelteHTML.createElement("legend", {});kind === 'kart' ? 'Your racer' : 'Your survivor'; }
         { svelteHTML.createElement("div", {});
          if(kind === 'kart'){
               for(let r of __sveltets_2_ensureArray(RACERS)){r.id; { svelteHTML.createElement("button", {     "aria-pressed":choice === r.id,"onclick":() => (choice = r.id),"title":r.name,}); { svelteHTML.createElement("img", {    "src":`/room-games/portraits/racer-${r.id}.webp`,"alt":r.name,});} }}
          }else{
               for(let f of __sveltets_2_ensureArray(FIGHTERS)){f.id; { svelteHTML.createElement("button", {     "aria-pressed":choice === f.id,"onclick":() => (choice = f.id),"title":f.name,}); { svelteHTML.createElement("img", {    "src":`/room-games/portraits/fighter-${f.id}.webp`,"alt":f.name,});} }}
          }
         }
       }
      if(isHost){
         { svelteHTML.createElement("div", { "class":`match-settings`,});
           { svelteHTML.createElement("label", {});  { svelteHTML.createElement("select", { "bind:value":cpuLevel,});/*Ωignore_startΩ*/() => cpuLevel = __sveltets_2_any(null);/*Ωignore_endΩ*/ { svelteHTML.createElement("option", { "value":`easy`,});  } { svelteHTML.createElement("option", { "value":`normal`,});  } { svelteHTML.createElement("option", { "value":`hard`,});  } } }
          if(kind === 'kart'){ { svelteHTML.createElement("label", { "class":`match-check`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":cpuFill,});/*Ωignore_startΩ*/() => cpuFill = __sveltets_2_any(null);/*Ωignore_endΩ*/}       }
          }else{ { svelteHTML.createElement("label", {}); botCount; { svelteHTML.createElement("input", {        "type":`range`,"min":`0`,"max":`15`,"bind:value":botCount,});/*Ωignore_startΩ*/() => botCount = __sveltets_2_any(null);/*Ωignore_endΩ*/} }}
         }
      }
       { svelteHTML.createElement("ul", {   "class":`match-members`,"aria-label":`Players`,});
           for(let member of __sveltets_2_ensureArray(room?.members ?? [])){member.id;
           { svelteHTML.createElement("li", { });member.left; { svelteHTML.createElement("strong", {});member.name;member.id === credential.member_id ? ' (you)' : ''; } { svelteHTML.createElement("span", {});member.role === 'host' ? 'Host · ' : '';statusOf(member); } }
        }
        if(active.length < 2){ { svelteHTML.createElement("li", { "class":`match-empty`,});     }}
       }
       { svelteHTML.createElement("div", { "class":`match-actions`,});
         { svelteHTML.createElement("button", {     "class":`btn`,"disabled":connection !== 'connected' || !room,"onclick":() => client.send({ type: 'ready', ready: !self?.ready }),});self?.ready ? 'Not ready' : 'Ready'; }
        if(isHost){ { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":!everyoneReady || connection !== 'connected',"onclick":() => room && client.send({ type: 'start', round: room.round, generation: room.generation }),});  }}
         { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onexit,});  }
       }
     }
  }
  if(connection === 'gave-up' && room && !closed && (room.phase === 'playing' || room.phase === 'finished')){
     { svelteHTML.createElement("div", { "class":`match-lost`,}); { svelteHTML.createElement("span", {});      } { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":reconnect,});  } }
  }
  
   { svelteHTML.createElement("p", {     "class":`sr-only`,"role":`status`,"aria-live":`polite`,});!closed && !(room && (room.phase === 'playing' || room.phase === 'finished')) ? lobbyAnnouncement : ''; }
  if(error){ { svelteHTML.createElement("p", { "class":`match-error`,});error; }}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const GameMatch__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type GameMatch__SvelteComponent_ = ReturnType<typeof GameMatch__SvelteComponent_>;
/*Ωignore_endΩ*/export default GameMatch__SvelteComponent_;