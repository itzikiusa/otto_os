/// <reference lib="dom" />
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { RoomMediaClient, parseMediaEnvelope } from '../src/modules/rooms/room-media.ts';
import type { RoomMediaEnvironment } from '../src/modules/rooms/room-media.ts';
import type { RoomAction, RoomSnapshot } from '../src/lib/api/room-types.ts';

const snapshot: RoomSnapshot = { room_id: 'r', member_id: 'host', admission: 'admitted', host_member_id: 'host',
  members: [{ id: 'host', name: 'Host', role: 'host', admission: 'admitted', connected: true, generation: 1,
    control_requested: false, audio_joined: false, muted: true, room_muted: false, presenter_requested: false, presenter_allowed: true }], presentations: [] };

function fakeStream(kind: 'audio' | 'video') {
  const track = { kind, stopped: false, enabled: true, readyState: 'live', stop() { this.stopped = true; this.readyState = 'ended'; },
    getSettings: () => ({ width: 1280, height: 720 }), applyConstraints: async () => {}, addEventListener: () => {} };
  const stream = { getTracks: () => [track], getAudioTracks: () => kind === 'audio' ? [track] : [], getVideoTracks: () => kind === 'video' ? [track] : [] };
  return { track, stream: stream as unknown as MediaStream };
}

test('signal parser rejects oversized, malformed and unknown media envelopes', () => {
  assert.equal(parseMediaEnvelope({ version: 1, kind: 'description', description: { type: 'offer', sdp: 'x'.repeat(65537) }, bindings: [] }), null);
  assert.equal(parseMediaEnvelope({ version: 1, kind: 'description', description: { type: 'offer', sdp: 'm=video\r\n'.repeat(9) }, bindings: [] }), null);
  assert.equal(parseMediaEnvelope({ version: 1, kind: 'ice', candidate: { candidate: 'x'.repeat(2049) } }), null);
  assert.equal(parseMediaEnvelope({ version: 1, kind: 'bindings', bindings: [{ streamId: 's', sourceId: 'p', generation: -1 }] }), null);
  assert.equal(parseMediaEnvelope({ version: 1, kind: 'execute', command: 'anything' }), null);
  assert.ok(parseMediaEnvelope({ version: 1, kind: 'description', description: { type: 'offer', sdp: 'v=0\r\n' }, bindings: [] }));
});

test('ending room while screen permission is pending stops the late stream and publishes nothing', async () => {
  const captured = fakeStream('video'), actions: RoomAction[] = [];
  let resolve!: (stream: MediaStream) => void;
  const environment = { getDisplayMedia: () => new Promise<MediaStream>(r => { resolve = r; }) } as Partial<RoomMediaEnvironment>;
  const client = new RoomMediaClient({ memberId: 'host', send: a => actions.push(a), environment });
  client.update(snapshot);
  const pending = client.startPresentation();
  client.dispose();
  resolve(captured.stream);
  await pending;
  assert.equal(captured.track.stopped, true);
  assert.equal(actions.some(a => a.type === 'start_present'), false);
});

test('presenter permission is required before opening a screen picker', async () => {
  let pickerCalls = 0;
  const client = new RoomMediaClient({ memberId: 'guest', send: () => {}, environment: { getDisplayMedia: async () => { pickerCalls++; return fakeStream('video').stream; } } });
  client.update({ ...snapshot, member_id: 'guest', members: [{ ...snapshot.members![0], id: 'guest', role: 'viewer', presenter_allowed: false }] });
  await client.startPresentation();
  assert.equal(pickerCalls, 0);
  client.dispose();
});

test('late microphone capture cannot reactivate after leaving audio', async () => {
  const captured = fakeStream('audio'); let resolve!: (stream: MediaStream) => void;
  const destination = { disconnect() {}, connect() {} };
  const context = { state: 'running', destination, resume: async () => {}, close: async () => {},
    createMediaStreamSource: () => destination } as unknown as AudioContext;
  const client = new RoomMediaClient({ memberId: 'host', send: () => {}, environment: {
    createAudioContext: () => context, getUserMedia: () => new Promise(r => { resolve = r; }),
  } });
  client.update(snapshot);
  await client.joinAudio();
  const pending = client.unmute();
  client.leaveAudio(); resolve(captured.stream); await pending;
  assert.equal(captured.track.stopped, true);
  client.dispose();
});

test('a host generation change invalidates a pending guest picker even if disconnect was missed', async () => {
  const captured = fakeStream('video'); let resolve!: (stream: MediaStream) => void;
  const guest = { ...snapshot.members![0], id: 'guest', role: 'viewer' as const };
  const client = new RoomMediaClient({ memberId: 'guest', send: () => {}, environment: { getDisplayMedia: () => new Promise(r => { resolve = r; }) } });
  client.update({ ...snapshot, member_id: 'guest', members: [...snapshot.members!, guest] });
  const pending = client.startPresentation();
  client.update({ ...snapshot, member_id: 'guest', members: [{ ...snapshot.members![0], generation: 2 }, guest] });
  resolve(captured.stream); await pending;
  assert.equal(captured.track.stopped, true);
  client.dispose();
});

test('server coordinator shutdown stops the host microphone and audio context', async () => {
  const captured = fakeStream('audio'); let closed = false;
  const node = { disconnect() {}, connect() {} };
  const context = { state: 'running', destination: node, resume: async () => {}, close: async () => { closed = true; }, createMediaStreamSource: () => node } as unknown as AudioContext;
  const client = new RoomMediaClient({ memberId: 'host', send: () => {}, environment: { createAudioContext: () => context,
    getUserMedia: async () => captured.stream, createStream: () => captured.stream } });
  client.update(snapshot); await client.joinAudio(); await client.unmute();
  client.update({ ...snapshot, members: [{ ...snapshot.members![0], audio_joined: true, muted: false }] });
  client.update(snapshot);
  assert.equal(captured.track.stopped, true); assert.equal(closed, true);
  client.dispose();
});

test('moderation mute then lift cannot revive a picker opened before moderation', async t => {
  const captured = fakeStream('audio'); let resolve!: (stream: MediaStream) => void;
  const node = { disconnect() {}, connect() {} };
  const context = { state: 'running', destination: node, resume: async () => {}, close: async () => {}, createMediaStreamSource: () => node } as unknown as AudioContext;
  const client = new RoomMediaClient({ memberId: 'host', send: () => {}, environment: { createAudioContext: () => context,
    getUserMedia: () => new Promise(r => { resolve = r; }), createStream: () => captured.stream } });
  t.after(() => client.dispose());
  client.update(snapshot); await client.joinAudio(); const pending = client.unmute();
  client.update({ ...snapshot, members: [{ ...snapshot.members![0], room_muted: true }] });
  client.update(snapshot); resolve(captured.stream); await pending;
  assert.equal(captured.track.stopped, true); client.dispose();
});

test('recap accessor exposes only the host raw live microphone and withdraws muted audio', async t => {
  const captured = fakeStream('audio'), node = { disconnect() {}, connect() {} };
  const context = { state: 'running', destination: node, resume: async () => {}, close: async () => {}, createMediaStreamSource: () => node } as unknown as AudioContext;
  const client = new RoomMediaClient({ memberId: 'host', send: () => {}, environment: { createAudioContext: () => context,
    getUserMedia: async () => captured.stream, createStream: () => captured.stream } });
  t.after(() => client.dispose()); client.update(snapshot);
  assert.equal(client.getRecapAudioSources().size, 0);
  await client.joinAudio(); await client.unmute();
  assert.equal(client.getRecapAudioSources().size, 0, 'pending local join/unmute is not authoritative consented audio');
  client.update({ ...snapshot, members: [{ ...snapshot.members![0], audio_joined: true, muted: false }] });
  const source = client.getRecapAudioSources().get('host');
  assert.equal(source?.stream, captured.stream); assert.equal(source?.generation, 1);
  client.mute(); assert.equal(client.getRecapAudioSources().size, 0);
});

function mediaPeerFixture() {
  let delayed: {promise: Promise<void>; started: boolean} | null = null;
  function holdNextReplacement() {
    let release!: () => void;
    const gate = {promise: new Promise<void>(resolve => {release = resolve;}), started: false};
    delayed = gate;
    return {gate, release};
  }
  const senders: {track: MediaStreamTrack | null; replaceTrack(track: MediaStreamTrack | null): Promise<void>; getParameters(): RTCRtpSendParameters}[] = [];
  const pc = {
    signalingState: 'stable', connectionState: 'connected',
    ontrack: null as ((event: RTCTrackEvent) => void) | null,
    addTrack(track: MediaStreamTrack) {
      const sender = {track: track as MediaStreamTrack | null, async replaceTrack(next: MediaStreamTrack | null) {
        const gate = delayed; delayed = null;
        if (gate) {gate.started = true; await gate.promise;}
        this.track = next;
      }, getParameters: () => ({encodings: []} as unknown as RTCRtpSendParameters)};
      senders.push(sender); return sender;
    },
    removeTrack(sender: typeof senders[number]) {sender.track = null;},
    getReceivers: () => [], close() {},
  };
  let streamId = 0;
  const stream = (tracks: MediaStreamTrack[]) => ({id: `stream-${++streamId}`, getTracks: () => tracks,
    getAudioTracks: () => tracks.filter(t => t.kind === 'audio'), getVideoTracks: () => tracks.filter(t => t.kind === 'video')}) as unknown as MediaStream;
  const node = {disconnect() {}, connect() {}};
  const mixTracks: ReturnType<typeof fakeStream>['track'][] = [];
  const context = () => ({resume: async () => {}, close: async () => {}, destination: node,
    createMediaStreamSource: () => node, createMediaStreamDestination: () => {
      const track = fakeStream('audio').track; mixTracks.push(track);
      return {...node, stream: stream([track as unknown as MediaStreamTrack])};
    }}) as unknown as AudioContext;
  const environment: Partial<RoomMediaEnvironment> = {createPeer: () => pc as unknown as RTCPeerConnection,
    createStream: stream, createAudioContext: context, getDisplayMedia: async () => fakeStream('video').stream};
  return {pc, senders, environment, stream, holdNextReplacement, mixTracks};
}
async function flushMedia() {for (let i = 0; i < 100; i++) await Promise.resolve();}
const guestMember = {...snapshot.members![0], id: 'guest', role: 'editor' as const, audio_joined: true};

test('25 screen source replacements and 25 audio rejoins reuse negotiated sender slots', async t => {
  const fixture = mediaPeerFixture(), actions: RoomAction[] = [];
  const client = new RoomMediaClient({memberId: 'host', send: action => actions.push(action), environment: fixture.environment});
  t.after(() => client.dispose());
  let room: RoomSnapshot = {...snapshot, members: [{...snapshot.members![0], audio_joined: true}, guestMember]};
  client.update(room); await client.joinAudio(); await flushMedia();
  let bindingStream: string | undefined;
  for (let cycle = 0; cycle < 25; cycle++) {
    await client.startPresentation();
    const source = {id: `screen-${cycle}`, member_id: 'host', title: 'Screen', generation: 1, width: 1280, height: 720, clear_epoch: 1};
    room = {...room, presentations: [source]}; client.update(room);
    client.handleEvent({type: 'subscription', member_id: 'guest', source_id: source.id, source_generation: 1, tier: 'grid'});
    await flushMedia();
    const signal = actions.filter(a => a.type === 'signal').at(-1);
    const envelope = signal?.type === 'signal' ? parseMediaEnvelope(signal.media) : null;
    assert.equal(envelope?.kind, 'bindings');
    if (envelope?.kind === 'bindings') {
      assert.equal(envelope.bindings[0].sourceId, source.id);
      bindingStream ??= envelope.bindings[0].streamId;
      assert.equal(envelope.bindings[0].streamId, bindingStream, 'receiver association survives a new source ID');
    }
    assert.equal(fixture.senders.length, 2, 'one audio and one video sender across every replacement');
    room = {...room, presentations: [{...source, generation: 2, width: 1920, height: 1080}]}; client.update(room);
    client.handleEvent({type: 'subscription', member_id: 'guest', source_id: source.id, source_generation: 2, tier: 'grid'});
    await flushMedia();
    const resizeSignal = actions.filter(a => a.type === 'signal').at(-1);
    const resized = resizeSignal?.type === 'signal' ? parseMediaEnvelope(resizeSignal.media) : null;
    assert.equal(resized?.kind, 'bindings');
    if (resized?.kind === 'bindings') assert.deepEqual(resized.bindings, [{streamId: bindingStream, sourceId: source.id, generation: 2}]);
    const pendingStop = fixture.holdNextReplacement();
    client.leaveAudio(); await client.joinAudio(); await flushMedia();
    assert.equal(pendingStop.gate.started, true, 'the prior audio stop is still pending when rejoining');
    pendingStop.release(); await flushMedia();
    assert.equal(fixture.senders.length, 2, 'voice leave/rejoin must not append SDP m-lines');
    assert.equal(fixture.senders[0].track?.readyState, 'live');
    client.stopPresentation(); room = {...room, presentations: []}; client.update(room); await flushMedia();
    assert.equal(fixture.senders[1].track, null, 'revoked video stops sending immediately');
  }
});

test('removing a member during a pending video replacement cannot recreate its audio mix', async t => {
  const fixture = mediaPeerFixture();
  const client = new RoomMediaClient({memberId: 'host', send: () => {}, environment: fixture.environment});
  t.after(() => client.dispose());
  let room: RoomSnapshot = {...snapshot, members: [{...snapshot.members![0], audio_joined: true}, guestMember]};
  client.update(room); await client.joinAudio(); await client.startPresentation();
  const source = {id: 'screen', member_id: 'host', title: 'Screen', generation: 1, width: 1280, height: 720, clear_epoch: 1};
  room = {...room, presentations: [source]}; client.update(room);
  client.handleEvent({type: 'subscription', member_id: 'guest', source_id: source.id, source_generation: 1, tier: 'grid'});
  await flushMedia(); assert.equal(fixture.mixTracks.length, 1);
  const pendingStop = fixture.holdNextReplacement();
  room = {...room, presentations: []}; client.update(room); await flushMedia();
  assert.equal(pendingStop.gate.started, true);
  client.update({...room, members: [room.members![0]]});
  assert.equal(fixture.mixTracks[0].stopped, true);
  pendingStop.release(); await flushMedia();
  assert.equal(fixture.mixTracks.length, 1, 'the stale reconcile must not allocate a new mix on a closed peer');
});

test('revoking a source during pending slot reuse clears the newly attached video track', async t => {
  const fixture = mediaPeerFixture();
  const client = new RoomMediaClient({memberId: 'host', send: () => {}, environment: fixture.environment});
  t.after(() => client.dispose());
  let room: RoomSnapshot = {...snapshot, members: [{...snapshot.members![0], audio_joined: true}, guestMember]};
  client.update(room); await client.joinAudio(); await flushMedia();
  for (let cycle = 0; cycle < 2; cycle++) {
    await client.startPresentation();
    const source = {id: `screen-${cycle}`, member_id: 'host', title: 'Screen', generation: 1, width: 1280, height: 720, clear_epoch: 1};
    const pending = cycle === 1 ? fixture.holdNextReplacement() : null;
    room = {...room, presentations: [source]}; client.update(room);
    client.handleEvent({type: 'subscription', member_id: 'guest', source_id: source.id, source_generation: 1, tier: 'grid'});
    await flushMedia();
    if (pending) assert.equal(pending.gate.started, true);
    room = {...room, presentations: []}; client.update(room);
    pending?.release(); await flushMedia();
    assert.equal(fixture.senders[1].track, null, 'even a slot attached during revocation must be tracked and cleared');
  }
});

test('a retained receiver stream binds new source IDs without another ontrack event', async t => {
  const fixture = mediaPeerFixture(); let shown = new Map<string, MediaStream>();
  const client = new RoomMediaClient({memberId: 'guest', send: () => {}, environment: fixture.environment, onStreams: streams => {shown = streams;}});
  t.after(() => client.dispose());
  const members = [{...snapshot.members![0], audio_joined: true}, guestMember];
  const stream = fixture.stream([fakeStream('video').track as unknown as MediaStreamTrack]);
  let room: RoomSnapshot = {...snapshot, member_id: 'guest', members};
  client.update(room); await client.joinAudio(); await flushMedia();
  for (let cycle = 0; cycle < 25; cycle++) {
    const source = {id: `remote-${cycle}`, member_id: 'host', title: `Screen ${cycle}`, generation: cycle + 1, width: 1280, height: 720, clear_epoch: 1};
    room = {...room, presentations: [source]}; client.update(room);
    client.setSubscriptions([{sourceId: source.id, generation: source.generation, tier: 'grid'}]); await flushMedia();
    client.handleEvent({type: 'signal', from: 'host', to: 'guest', generation: 1, media: {version: 1, kind: 'bindings', bindings: [{streamId: stream.id, sourceId: source.id, generation: source.generation}]}});
    await flushMedia();
    if (cycle === 0) fixture.pc.ontrack!({track: stream.getVideoTracks()[0], streams: [stream]} as unknown as RTCTrackEvent);
    await flushMedia(); assert.deepEqual([...shown.keys()], [source.id]);
    room = {...room, presentations: []}; client.update(room);
    client.handleEvent({type: 'signal', from: 'host', to: 'guest', generation: 1, media: {version: 1, kind: 'bindings', bindings: []}});
    await flushMedia(); assert.equal(shown.size, 0, 'old source disappears when its grant is removed');
  }
});

test('a binding-only publisher replacement triggers forwarding to another guest', async t => {
  const publisher = mediaPeerFixture(), viewer = mediaPeerFixture(), actions: RoomAction[] = [];
  let peerCount = 0;
  const client = new RoomMediaClient({memberId: 'host', send: action => actions.push(action), environment: {
    ...publisher.environment, createPeer: () => (peerCount++ === 0 ? publisher.pc : viewer.pc) as unknown as RTCPeerConnection,
  }});
  t.after(() => client.dispose());
  let room: RoomSnapshot = {...snapshot, members: [{...snapshot.members![0], audio_joined: true}, guestMember, {...guestMember, id: 'viewer'}]};
  const stream = publisher.stream([fakeStream('video').track as unknown as MediaStreamTrack]);
  client.update(room); await client.joinAudio(); await flushMedia();
  for (let cycle = 0; cycle < 25; cycle++) {
    const source = {id: `publisher-${cycle}`, member_id: 'guest', title: 'Screen', generation: 1, width: 1280, height: 720, clear_epoch: 1};
    room = {...room, presentations: [source]}; client.update(room);
    client.handleEvent({type: 'subscription', member_id: 'viewer', source_id: source.id, source_generation: 1, tier: 'grid'});
    await flushMedia();
    client.handleEvent({type: 'signal', from: 'guest', to: 'host', generation: 1, media: {version: 1, kind: 'bindings', bindings: [{streamId: stream.id, sourceId: source.id, generation: 1}]}});
    if (cycle === 0) publisher.pc.ontrack!({track: stream.getVideoTracks()[0], streams: [stream]} as unknown as RTCTrackEvent);
    await flushMedia();
    const signal = actions.filter(a => a.type === 'signal' && a.to === 'viewer').at(-1);
    const envelope = signal?.type === 'signal' ? parseMediaEnvelope(signal.media) : null;
    assert.equal(envelope?.kind, 'bindings');
    if (envelope?.kind === 'bindings') assert.deepEqual(envelope.bindings.map(b => b.sourceId), [source.id], 'new binding must relay without another ontrack or room event');
    room = {...room, presentations: []}; client.update(room); await flushMedia();
  }
  assert.equal(viewer.senders.length, 2, 'relay replacements keep a bounded audio/video sender pair');
});

test('offer glare still accepts ICE for the current remote generation', async t => {
  const fixture = mediaPeerFixture(), added: string[] = [], errors: (string | null)[] = [];
  Object.assign(fixture.pc, {
    remoteDescription: {type: 'answer', sdp: 'v=0\r\na=ice-ufrag:current\r\n'},
    async setRemoteDescription() {fixture.pc.signalingState = 'stable';},
    async addIceCandidate(candidate: RTCIceCandidateInit) {
      if (candidate.usernameFragment !== 'current') throw new Error('Unknown ICE generation');
      added.push(candidate.usernameFragment);
    },
  });
  const client = new RoomMediaClient({memberId: 'guest', send: () => {}, environment: fixture.environment, onState: state => errors.push(state.error)});
  t.after(() => client.dispose());
  client.update({...snapshot, member_id: 'guest', members: [{...snapshot.members![0], audio_joined: true}, guestMember]});
  await client.joinAudio(); await flushMedia();
  fixture.pc.signalingState = 'have-local-offer';
  const signal = (media: unknown) => client.handleEvent({type: 'signal', from: 'host', to: 'guest', generation: 1, media});
  signal({version: 1, kind: 'description', description: {type: 'offer', sdp: 'v=0\r\na=ice-ufrag:ignored\r\n'}, bindings: []});
  signal({version: 1, kind: 'ice', candidate: {candidate: 'candidate:current', usernameFragment: 'current', sdpMid: '0'}});
  signal({version: 1, kind: 'ice', candidate: {candidate: 'candidate:ignored', usernameFragment: 'ignored', sdpMid: '0'}});
  await flushMedia();
  assert.deepEqual(added, ['current'], 'ignoring an offer must not discard candidates matching the accepted remote description');
  assert.equal(errors.some(Boolean), false, 'unknown candidates from the ignored offer remain harmless');
  signal({version: 1, kind: 'description', description: {type: 'answer', sdp: 'v=0\r\na=ice-ufrag:current\r\n'}, bindings: []});
  signal({version: 1, kind: 'ice', candidate: {candidate: 'candidate:unrelated', usernameFragment: 'unrelated', sdpMid: '0'}});
  await flushMedia();
  assert.equal(errors.some(Boolean), true, 'an unrelated candidate outside ignored-offer glare must still report negotiation failure');
});

test('initial ignored-offer candidates are not replayed into a different accepted generation', async t => {
  const fixture = mediaPeerFixture(), attempted: string[] = [], errors: (string | null)[] = [];
  Object.assign(fixture.pc, {remoteDescription: null as RTCSessionDescriptionInit | null,
    async setRemoteDescription(description: RTCSessionDescriptionInit) {Object.assign(fixture.pc, {remoteDescription: description, signalingState: 'stable'});},
    async addIceCandidate(candidate: RTCIceCandidateInit) {attempted.push(candidate.usernameFragment!); throw new Error('Unknown ICE generation');},
  });
  const client = new RoomMediaClient({memberId: 'guest', send: () => {}, environment: fixture.environment, onState: state => errors.push(state.error)});
  t.after(() => client.dispose());
  client.update({...snapshot, member_id: 'guest', members: [{...snapshot.members![0], audio_joined: true}, guestMember]});
  await client.joinAudio(); await flushMedia(); fixture.pc.signalingState = 'have-local-offer';
  const signal = (media: unknown) => client.handleEvent({type: 'signal', from: 'host', to: 'guest', generation: 1, media});
  signal({version: 1, kind: 'description', description: {type: 'offer', sdp: 'v=0\r\na=ice-ufrag:ignored\r\n'}, bindings: []});
  signal({version: 1, kind: 'ice', candidate: {candidate: 'candidate:ignored', usernameFragment: 'ignored', sdpMid: '0'}});
  signal({version: 1, kind: 'description', description: {type: 'answer', sdp: 'v=0\r\na=ice-ufrag:accepted\r\n'}, bindings: []});
  await flushMedia();
  assert.deepEqual(attempted, []);
  assert.equal(errors.some(Boolean), false);
});

test('negotiation chooses the local description after a pending remote offer changes signaling state', async t => {
  const fixture = mediaPeerFixture(), errors: (string | null)[] = [];
  let release!: () => void;
  const gate = new Promise<void>(resolve => {release = resolve;});
  let chosen: RTCSdpType | undefined;
  Object.assign(fixture.pc, {
    localDescription: null,
    async createOffer() {await gate; return {type: 'offer', sdp: 'v=0\r\n'};},
    async setLocalDescription(description?: RTCSessionDescriptionInit) {
      await gate;
      const type = description?.type ?? (fixture.pc.signalingState === 'have-remote-offer' ? 'answer' : 'offer');
      if (type === 'offer' && fixture.pc.signalingState === 'have-remote-offer') throw new Error('Cannot set an offer while a remote offer is pending');
      chosen = type;
      Object.assign(fixture.pc, {localDescription: {toJSON: () => ({type, sdp: 'v=0\r\n'})}});
    },
  });
  const client = new RoomMediaClient({memberId: 'host', send: () => {}, environment: fixture.environment, onState: state => errors.push(state.error)});
  t.after(() => client.dispose());
  client.update({...snapshot, members: [{...snapshot.members![0], audio_joined: true}, guestMember]});
  await client.joinAudio(); await flushMedia();
  const negotiation = (fixture.pc as unknown as {onnegotiationneeded(): Promise<void>}).onnegotiationneeded();
  fixture.pc.signalingState = 'have-remote-offer'; release(); await negotiation;
  assert.equal(errors.some(Boolean), false, 'remote-offer overlap must not raise the observed InvalidStateError');
  assert.equal(chosen, 'answer');
});

test('local preview demand never subscribes to the publisher through the server', async t => {
  const fixture = mediaPeerFixture(), actions: RoomAction[] = [];
  const client = new RoomMediaClient({memberId: 'host', send: action => actions.push(action), environment: fixture.environment});
  t.after(() => client.dispose());
  const source = {id: 'local', member_id: 'host', title: 'Local', generation: 1, width: 640, height: 360, clear_epoch: 1};
  client.update({...snapshot, members: [...snapshot.members!, guestMember], presentations: [source, {...source, id: 'remote', member_id: 'guest'}]});
  client.setSubscriptions([{sourceId: 'local', generation: 1, tier: 'full'}, {sourceId: 'remote', generation: 1, tier: 'grid'}]);
  client.setSubscriptions([{sourceId: 'local', generation: 1, tier: 'preview'}, {sourceId: 'remote', generation: 1, tier: 'grid'}]);
  client.setSubscriptions([{sourceId: 'remote', generation: 1, tier: 'grid'}]);
  assert.deepEqual(actions.filter(action => action.type === 'subscribe'), [{type: 'subscribe', source_id: 'remote', source_generation: 1, tier: 'grid'}]);
});
