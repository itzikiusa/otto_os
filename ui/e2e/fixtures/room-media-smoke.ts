import { RoomMediaClient, type RoomMediaState } from '../../src/modules/rooms/room-media';
import type { RoomAction, RoomIceServer, RoomMember, RoomPresentation, RoomSnapshot } from '../../src/lib/api/room-types';

/** In-browser signaling harness, with synthetic pixels and oscillators only. */
export async function startMediaSmoke(options: {iceServers?: RoomIceServer[]; relayOnly?: boolean} = {}) {
  const ids = ['host', 'guest1', 'guest2', 'guest3'];
  const members: RoomMember[] = ids.map(id => ({id, name: id, role: id === 'host' ? 'host' : 'viewer', admission: 'admitted', connected: true, generation: 1, control_requested: false, audio_joined: false, muted: true, room_muted: false, presenter_requested: false, presenter_allowed: true}));
  const presentations: RoomPresentation[] = [];
  const clients = new Map<string, RoomMediaClient>();
  const seen = new Map<string, string[]>(), states = new Map<string, RoomMediaState>();
  const contexts: AudioContext[] = [];
  const ownedTracks: MediaStreamTrack[] = [];
  const peers: RTCPeerConnection[] = [];
  const sourceCounts = new Map<string, number>();
  const canvases = new Map<string, HTMLCanvasElement>();
  const paintTimers: ReturnType<typeof setInterval>[] = [];
  let audioEpoch = 1, enforced = 0;
  const snapshot = (memberId: string): RoomSnapshot => ({room_id: 'synthetic-media', member_id: memberId, admission: 'admitted', host_member_id: 'host', driver_member_id: 'host', grant_epoch: 1, members: structuredClone(members), presentations: structuredClone(presentations), audio_epoch: audioEpoch, audio_enforced_epoch: enforced});
  function broadcast() {
    for (const [id, client] of clients) client.update(snapshot(id));
    for (const client of clients.values()) client.setSubscriptions(presentations.map((source, index) => ({sourceId: source.id, generation: source.generation, tier: index === 0 ? 'full' : 'preview'})));
  }
  function send(from: string, action: RoomAction) {
    queueMicrotask(() => {
      if (action.type === 'request_ice') clients.get(from)?.handleEvent({type: 'ice', ice_servers: options.iceServers ?? [], relay_configured: !!options.iceServers?.length, relay_only: options.relayOnly ?? false, expires_at: '2099-01-01T00:00:00Z'});
      else if (action.type === 'signal') clients.get(action.to)?.handleEvent({type: 'signal', from, to: action.to, generation: 1, media: action.media});
      else if (action.type === 'subscribe') {
        const event = {type: 'subscription' as const, member_id: from, source_id: action.source_id, source_generation: action.source_generation, tier: action.tier};
        clients.get('host')?.handleEvent(event);
        const owner = presentations.find(p => p.id === action.source_id)?.member_id;
        if (owner && owner !== 'host') clients.get(owner)?.handleEvent(event);
      } else if (action.type === 'start_present') {
        const sequence = (sourceCounts.get(from) ?? 0) + 1; sourceCounts.set(from, sequence);
        presentations.push({id: `screen-${from}${sequence === 1 ? '' : `-${sequence}`}`, member_id: from, title: action.title, width: action.width, height: action.height, generation: 1, clear_epoch: 1}); broadcast();
      } else if (action.type === 'stop_present') {
        const index = presentations.findIndex(p => p.id === action.source_id && p.member_id === from);
        if (index >= 0) { presentations.splice(index, 1); broadcast(); }
      } else if (action.type === 'update_present') {
        const source = presentations.find(p => p.id === action.source_id && p.member_id === from);
        if (source && (source.width !== action.width || source.height !== action.height)) { source.width = action.width; source.height = action.height; source.generation++; source.clear_epoch++; broadcast(); }
      } else if (action.type === 'audio') {
        const member = members.find(m => m.id === from)!; member.audio_joined = action.joined; member.muted = action.muted; audioEpoch++; broadcast();
      } else if (action.type === 'audio_applied' && action.epoch > enforced) { enforced = action.epoch; broadcast(); }
    });
  }
  for (const [index, id] of ids.entries()) {
    const client = new RoomMediaClient({memberId: id, send: action => send(id, action), onStreams: streams => seen.set(id, [...streams.keys()]), onState: state => states.set(id, state), environment: {
      createAudioContext: () => {
        const context = new AudioContext(); contexts.push(context);
        const destination = context.createMediaStreamDestination.bind(context);
        context.createMediaStreamDestination = () => { const node = destination(); ownedTracks.push(...node.stream.getTracks()); return node; };
        return context;
      },
      createPeer: configuration => { const peer = new RTCPeerConnection(configuration); peers.push(peer); return peer; },
      getDisplayMedia: async () => {
        const canvas = document.createElement('canvas'); canvases.set(id, canvas); canvas.width = 640; canvas.height = 360;
        const ctx = canvas.getContext('2d')!; ctx.fillStyle = `hsl(${index * 80} 40% 30%)`; ctx.fillRect(0, 0, 640, 360);
        ctx.fillStyle = 'white'; ctx.font = '32px sans-serif'; ctx.fillText(id, 24, 60);
        // Ongoing synthetic frames let Chromium deliver resizing after its
        // frame-rate limiter; a single repaint can occur between frame slots.
        let tick = 0; paintTimers.push(setInterval(() => { ctx.fillStyle = `hsl(${tick++ % 360} 50% 50%)`; ctx.fillRect(0, 0, 16, 16); }, 100));
        const stream = canvas.captureStream(3); ownedTracks.push(...stream.getTracks()); return stream;
      },
      getUserMedia: async () => {
        const context = new AudioContext(); contexts.push(context);
        const oscillator = context.createOscillator(); oscillator.frequency.value = 220 + index * 110;
        const destination = context.createMediaStreamDestination(); oscillator.connect(destination); oscillator.start();
        ownedTracks.push(...destination.stream.getTracks()); return destination.stream;
      },
    }}); clients.set(id, client);
  }
  broadcast();
  for (const client of clients.values()) { await client.joinAudio(); await client.unmute(); }
  for (const client of clients.values()) await client.startPresentation();
  return {
    summary() { return {sources: presentations.length, sourceInfo: presentations, received: Object.fromEntries(seen), states: Object.fromEntries(states)}; },
    resize(id: string) { const canvas = canvases.get(id)!; canvas.width = 800; canvas.height = 450; const ctx = canvas.getContext('2d')!; ctx.fillStyle = 'navy'; ctx.fillRect(0, 0, 800, 450); },
    async startPresenting(id: string) { await clients.get(id)!.startPresentation(); },
    async statistics() {
      const reports = [];
      for (const peer of peers) {
        let framesDecoded = 0, bytesReceived = 0, selectedType = '';
        const stats = await peer.getStats();
        for (const row of stats.values()) {
          if (row.type === 'inbound-rtp') { framesDecoded += row.framesDecoded ?? 0; bytesReceived += row.bytesReceived ?? 0; }
          if (row.type === 'transport' && row.selectedCandidatePairId) {
            const pair = stats.get(row.selectedCandidatePairId);
            selectedType = stats.get(pair?.localCandidateId)?.candidateType ?? '';
          }
        }
        reports.push({state: peer.connectionState, signaling: peer.signalingState, gathering: peer.iceGatheringState, transceivers: peer.getTransceivers().length, mLines: peer.localDescription?.sdp.match(/^m=/gm)?.length ?? 0, framesDecoded, bytesReceived, selectedType, policy: peer.getConfiguration().iceTransportPolicy});
      }
      return {peers: reports, tracks: ownedTracks.map(track => track.readyState), contexts: contexts.map(context => context.state)};
    },
    stopPresenting(id: string) { clients.get(id)!.stopPresentation(); },
    remove(id: string) { const member = members.find(m => m.id === id)!; member.connected = false; member.audio_joined = false; for (let i = presentations.length - 1; i >= 0; i--) if (presentations[i].member_id === id) presentations.splice(i, 1); audioEpoch++; broadcast(); },
    async stop() { for (const timer of paintTimers) clearInterval(timer); for (const client of clients.values()) client.dispose(); await Promise.all(contexts.filter(context => context.state !== 'closed').map(context => context.close())); },
  };
}
