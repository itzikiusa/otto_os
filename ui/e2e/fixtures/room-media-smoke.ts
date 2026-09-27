import { RoomMediaClient, type RoomMediaState } from '../../src/modules/rooms/room-media';
import type { RoomAction, RoomIceServer, RoomMember, RoomPresentation, RoomSnapshot } from '../../src/lib/api/room-types';

/** In-browser signaling harness, with synthetic pixels and oscillators only. */
export async function startMediaSmoke(options: {iceServers?: RoomIceServer[]; relayOnly?: boolean; taskSignaling?: boolean} = {}) {
  const ids = ['host', 'guest1', 'guest2', 'guest3'];
  const members: RoomMember[] = ids.map(id => ({id, name: id, role: id === 'host' ? 'host' : 'viewer', admission: 'admitted', connected: true, generation: 1, control_requested: false, audio_joined: false, muted: true, room_muted: false, presenter_requested: false, presenter_allowed: true}));
  const presentations: RoomPresentation[] = [];
  const clients = new Map<string, RoomMediaClient>();
  const seen = new Map<string, string[]>(), states = new Map<string, RoomMediaState>();
  const contexts: AudioContext[] = [];
  const ownedTracks: MediaStreamTrack[] = [];
  const peers: RTCPeerConnection[] = [];
  const history = new Map<RTCPeerConnection, object[]>();
  const identities = new Map<RTCPeerConnection, {owner: string; index: number}>();
  const diagnostics: object[] = [];
  const trace = (event: object) => { diagnostics.push({at: Math.round(performance.now()), ...event}); if (diagnostics.length > 400) diagnostics.splice(300, 1); };
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
    const deliver = () => {
      if (action.type === 'request_ice') clients.get(from)?.handleEvent({type: 'ice', ice_servers: options.iceServers ?? [], relay_configured: !!options.iceServers?.length, relay_only: options.relayOnly ?? false, expires_at: '2099-01-01T00:00:00Z'});
      else if (action.type === 'signal') {
        const media = action.media as {kind?: string; description?: RTCSessionDescriptionInit; candidate?: RTCIceCandidateInit | null; bindings?: unknown};
        trace({event: 'signal', from, to: action.to, kind: media.kind, sdpType: media.description?.type, ufrags: media.description?.sdp?.match(/^a=ice-ufrag:.+$/gm), candidateUfrag: media.candidate?.usernameFragment, candidateMid: media.candidate?.sdpMid, bindings: media.bindings});
        clients.get(action.to)?.handleEvent({type: 'signal', from, to: action.to, generation: 1, media: action.media});
      }
      else if (action.type === 'subscribe') {
        if (presentations.find(source => source.id === action.source_id)?.member_id === from) throw new Error('Backend rejects self-subscribe: use your local preview');
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
    };
    if (options.taskSignaling) setTimeout(deliver, 0); else queueMicrotask(deliver);
  }
  for (const [index, id] of ids.entries()) {
    const client = new RoomMediaClient({memberId: id, send: action => send(id, action), onStreams: streams => seen.set(id, [...streams.keys()]), onState: state => states.set(id, state), environment: {
      createAudioContext: () => {
        const context = new AudioContext(); contexts.push(context);
        const destination = context.createMediaStreamDestination.bind(context);
        context.createMediaStreamDestination = () => { const node = destination(); ownedTracks.push(...node.stream.getTracks()); return node; };
        return context;
      },
      createPeer: configuration => {
        const peer = new RTCPeerConnection(configuration), identity = {owner: id, index: peers.length}; peers.push(peer); identities.set(peer, identity);
        const events: object[] = []; history.set(peer, events);
        const operation = (name: string, call: () => Promise<void>, detail: object) => {
          trace({...identity, event: 'invoke', name, signaling: peer.signalingState, ...detail});
          const promise = call();
          void promise.then(() => trace({...identity, event: 'resolved', name, signaling: peer.signalingState, localUfrags: peer.localDescription?.sdp.match(/^a=ice-ufrag:.+$/gm), remoteUfrags: peer.remoteDescription?.sdp.match(/^a=ice-ufrag:.+$/gm)}),
            error => trace({...identity, event: 'rejected', name, error: String(error)}));
          // Return the original RTC promise: adding an async wrapper changes
          // the microtask ordering of the glare condition this trace diagnoses.
          return promise;
        };
        const local = peer.setLocalDescription.bind(peer), remote = peer.setRemoteDescription.bind(peer), candidate = peer.addIceCandidate.bind(peer);
        peer.setLocalDescription = description => operation('setLocalDescription', () => local(description), {type: description?.type, ufrags: description?.sdp?.match(/^a=ice-ufrag:.+$/gm)});
        peer.setRemoteDescription = description => operation('setRemoteDescription', () => remote(description), {type: description.type, ufrags: description.sdp?.match(/^a=ice-ufrag:.+$/gm)});
        peer.addIceCandidate = value => operation('addIceCandidate', () => candidate(value), {ufrag: value?.usernameFragment, mid: value?.sdpMid});
        for (const event of ['signalingstatechange', 'connectionstatechange', 'iceconnectionstatechange', 'icegatheringstatechange']) peer.addEventListener(event, () => {
          events.push({event, elapsed: Math.round(performance.now()), state: peer.connectionState, ice: peer.iceConnectionState, signaling: peer.signalingState,
            localType: peer.localDescription?.type, remoteType: peer.remoteDescription?.type,
            localUfrags: peer.localDescription?.sdp.match(/^a=ice-ufrag:.+$/gm), remoteUfrags: peer.remoteDescription?.sdp.match(/^a=ice-ufrag:.+$/gm)});
          if (events.length > 40) events.shift();
        });
        return peer;
      },
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
  const awaitMembership = async (id: string, muted: boolean) => {
    const until = performance.now() + 5000;
    while (performance.now() < until) {
      const member = members.find(member => member.id === id)!;
      if (member.audio_joined && member.muted === muted) return;
      await new Promise(resolve => setTimeout(resolve, 10));
    }
    throw new Error(`Synthetic audio membership did not settle: ${id}`);
  };
  for (const [id, client] of clients) {
    await client.joinAudio(); await awaitMembership(id, true);
    await client.unmute(); await awaitMembership(id, false);
  }
  for (const client of clients.values()) await client.startPresentation();
  return {
    summary() { return {diagnostics, sources: presentations.length, sourceInfo: presentations, received: Object.fromEntries(seen), states: Object.fromEntries(states)}; },
    resize(id: string) { const canvas = canvases.get(id)!; canvas.width = 800; canvas.height = 450; const ctx = canvas.getContext('2d')!; ctx.fillStyle = 'navy'; ctx.fillRect(0, 0, 800, 450); },
    async startPresenting(id: string) { await clients.get(id)!.startPresentation(); },
    async statistics() {
      const reports = [];
      for (const peer of peers) {
        let framesDecoded = 0, bytesReceived = 0, audioBytesReceived = 0, audioPacketsReceived = 0, selectedType = '';
        const stats = await peer.getStats();
        for (const row of stats.values()) {
          if (row.type === 'inbound-rtp') {
            framesDecoded += row.framesDecoded ?? 0; bytesReceived += row.bytesReceived ?? 0;
            if (row.kind === 'audio' || row.mediaType === 'audio') { audioBytesReceived += row.bytesReceived ?? 0; audioPacketsReceived += row.packetsReceived ?? 0; }
          }
          if (row.type === 'transport' && row.selectedCandidatePairId) {
            const pair = stats.get(row.selectedCandidatePairId);
            selectedType = stats.get(pair?.localCandidateId)?.candidateType ?? '';
          }
        }
        reports.push({...identities.get(peer), history: history.get(peer), state: peer.connectionState, signaling: peer.signalingState, gathering: peer.iceGatheringState, transceivers: peer.getTransceivers().length, mLines: peer.localDescription?.sdp.match(/^m=/gm)?.length ?? 0, framesDecoded, bytesReceived, audioBytesReceived, audioPacketsReceived, selectedType, policy: peer.getConfiguration().iceTransportPolicy});
      }
      return {peers: reports, tracks: ownedTracks.map(track => track.readyState), contexts: contexts.map(context => context.state)};
    },
    stopPresenting(id: string) { clients.get(id)!.stopPresentation(); },
    remove(id: string) { const member = members.find(m => m.id === id)!; member.connected = false; member.audio_joined = false; for (let i = presentations.length - 1; i >= 0; i--) if (presentations[i].member_id === id) presentations.splice(i, 1); audioEpoch++; broadcast(); },
    async stop() { for (const timer of paintTimers) clearInterval(timer); for (const client of clients.values()) client.dispose(); await Promise.all(contexts.filter(context => context.state !== 'closed').map(context => context.close())); },
  };
}
