import type { RoomAction, RoomEvent, RoomMember, RoomPresentation, RoomSnapshot, RoomSubscriptionTier } from '../../lib/api/room-types';
import { allocateVideoBudget, audioContributors, CaptureGeometry, CaptureSlot, highestTier } from './room-media-policy.ts';

export interface RoomMediaState {
  audioJoined: boolean; muted: boolean; presenting: boolean; error: string | null;
  connection: 'idle' | 'connecting' | 'connected' | 'failed'; relayConfigured: boolean;
}
export interface RoomMediaEnvironment {
  getUserMedia(constraints: MediaStreamConstraints): Promise<MediaStream>;
  getDisplayMedia(constraints: DisplayMediaStreamOptions): Promise<MediaStream>;
  createPeer(configuration: RTCConfiguration): RTCPeerConnection;
  createAudioContext(): AudioContext;
  createStream(tracks: MediaStreamTrack[]): MediaStream;
}
export interface RoomMediaOptions {
  memberId: string; send(action: RoomAction): void;
  onState?(state: RoomMediaState): void; onStreams?(streams: Map<string, MediaStream>): void;
  iceServers?: RTCIceServer[]; environment?: Partial<RoomMediaEnvironment>;
}
export interface RoomMediaSubscription { sourceId: string; generation: number; tier: RoomSubscriptionTier }
export interface RoomRecapAudioSource { memberId: string; generation: number; stream: MediaStream }
interface Binding { streamId: string; sourceId: string; generation: number }
type MediaEnvelope =
  | { version: 1; kind: 'description'; description: RTCSessionDescriptionInit; bindings: Binding[] }
  | { version: 1; kind: 'ice'; candidate: RTCIceCandidateInit | null }
  | { version: 1; kind: 'bindings'; bindings: Binding[] }
  | { version: 1; kind: 'demand'; sourceId: string; generation: number; tier: RoomSubscriptionTier };
const object = (v: unknown): v is Record<string, unknown> => v !== null && typeof v === 'object' && !Array.isArray(v);
const identifier = (v: unknown): v is string => typeof v === 'string' && v.length > 0 && v.length <= 256;
const generation = (v: unknown): v is number => Number.isSafeInteger(v) && (v as number) >= 0;
const tier = (v: unknown): v is RoomSubscriptionTier => v === 'hidden' || v === 'preview' || v === 'grid' || v === 'full';

/** Signaling is bounded data, never executable code or frame buffers. */
export function parseMediaEnvelope(v: unknown): MediaEnvelope | null {
  if (!object(v) || v.version !== 1) return null;
  if (v.kind === 'demand') return identifier(v.sourceId) && generation(v.generation) && tier(v.tier)
    ? { version: 1, kind: 'demand', sourceId: v.sourceId, generation: v.generation, tier: v.tier } : null;
  if (v.kind === 'ice') {
    if (v.candidate === null) return { version: 1, kind: 'ice', candidate: null };
    const c = v.candidate;
    if (!object(c) || typeof c.candidate !== 'string' || c.candidate.length > 2048
      || (c.sdpMid != null && (typeof c.sdpMid !== 'string' || c.sdpMid.length > 64))
      || (c.sdpMLineIndex != null && (!generation(c.sdpMLineIndex) || c.sdpMLineIndex > 32))
      || (c.usernameFragment != null && (typeof c.usernameFragment !== 'string' || c.usernameFragment.length > 256))) return null;
    return { version: 1, kind: 'ice', candidate: { candidate: c.candidate, sdpMid: c.sdpMid as string | null | undefined,
      sdpMLineIndex: c.sdpMLineIndex as number | null | undefined, usernameFragment: c.usernameFragment as string | undefined } };
  }
  if (v.kind !== 'description' && v.kind !== 'bindings') return null;
  if (!Array.isArray(v.bindings) || v.bindings.length > 4) return null;
  const bindings: Binding[] = [];
  for (const b of v.bindings) {
    if (!object(b) || !identifier(b.streamId) || !identifier(b.sourceId) || !generation(b.generation)) return null;
    bindings.push({ streamId: b.streamId, sourceId: b.sourceId, generation: b.generation });
  }
  if (new Set(bindings.map(b => b.streamId)).size !== bindings.length) return null;
  if (v.kind === 'bindings') return { version: 1, kind: 'bindings', bindings };
  const d = v.description;
  if (!object(d) || (d.type !== 'offer' && d.type !== 'answer') || typeof d.sdp !== 'string' || d.sdp.length > 65_536
    || (d.sdp.match(/^m=/gm)?.length ?? 0) > 8) return null;
  return { version: 1, kind: 'description', description: { type: d.type, sdp: d.sdp }, bindings };
}
interface VideoSlot { sender: RTCRtpSender; stream: MediaStream }
interface Outgoing extends VideoSlot { source: RoomPresentation }
interface Incoming { stream: MediaStream; generation: number }
interface Peer {
  id: string; generation: number; pc: RTCPeerConnection; makingOffer: boolean; ignoreOffer: boolean; settingAnswer: boolean;
  pendingIce: (RTCIceCandidateInit | null)[]; signals: Promise<void>; pendingSignals: number; bindings: Binding[];
  unbound: Map<string, MediaStream>; outgoing: Map<string, Outgoing>; incoming: Map<string, Incoming>;
  videoSlots: VideoSlot[]; audioWork: Promise<void>;
  audio: MediaStreamTrack | null; audioStream: MediaStream | null; audioSender: RTCRtpSender | null; mix: MediaStreamAudioDestinationNode | null;
  deadline: ReturnType<typeof setTimeout> | null; demandKey: string; bindingKey: string;
}

/** Host-routed media; ordinary owner credentials and media bytes never enter signaling. */
export class RoomMediaClient {
  private options: RoomMediaOptions;
  private env: RoomMediaEnvironment;
  private snapshot: RoomSnapshot | null = null;
  private disposed = false;
  private lifecycle = 0;
  private mic = new CaptureSlot<MediaStream>();
  private screen = new CaptureSlot<MediaStream>();
  private peers = new Map<string, Peer>();
  private subscriptions = new Map<string, Map<string, RoomMediaSubscription>>();
  private publisherDemand: RoomMediaSubscription | null = null;
  private context: AudioContext | null = null;
  private graphNodes: AudioNode[] = [];
  private state: RoomMediaState = { audioJoined: false, muted: true, presenting: false, error: null, connection: 'idle', relayConfigured: false };
  private rtc: RTCConfiguration;
  private work: Promise<void> = Promise.resolve();
  private queued = false;
  private captureDeadline: ReturnType<typeof setTimeout> | null = null;
  private iceRefresh: ReturnType<typeof setTimeout> | null = null;
  private ackTimer: ReturnType<typeof setInterval> | null = null;
  private ownSource: string | null = null;
  private replacingSource: string | null = null;
  private geometry: CaptureGeometry | null = null;
  private geometryTimer: ReturnType<typeof setInterval> | null = null;
  private qualityChanging = false;
  private lastStreams = new Map<string, MediaStream>();
  private constraintKey = '';
  private joinEpoch = 0;

  constructor(options: RoomMediaOptions) {
    this.options = options;
    this.env = {
      getUserMedia: c => globalThis.navigator?.mediaDevices?.getUserMedia(c) ?? Promise.reject(new Error('Microphone capture unavailable')),
      getDisplayMedia: c => globalThis.navigator?.mediaDevices?.getDisplayMedia?.(c) ?? Promise.reject(new Error('Screen sharing unavailable')),
      createPeer: c => new RTCPeerConnection(c), createAudioContext: () => new AudioContext(),
      createStream: tracks => new MediaStream(tracks), ...options.environment,
    };
    this.rtc = { iceServers: options.iceServers ?? [] };
    this.state.relayConfigured = (options.iceServers ?? []).some(s => (Array.isArray(s.urls) ? s.urls : [s.urls]).some(url => /^turns?:/.test(url)));
    this.emit();
  }
  private emit(): void { this.options.onState?.({ ...this.state }); }
  private error(message: string): void { this.state.error = message; this.emit(); }
  private send(action: RoomAction): void { if (!this.disposed && this.snapshot?.admission === 'admitted') this.options.send(action); }
  private self(): RoomMember | undefined { return this.snapshot?.members?.find(m => m.id === this.options.memberId); }
  private get isHost(): boolean { return this.snapshot?.host_member_id === this.options.memberId; }
  private source(id: string): RoomPresentation | undefined { return this.snapshot?.presentations?.find(p => p.id === id); }
  private canPresent(): boolean { const m = this.self(); return !this.disposed && !!m?.connected && (m.role === 'host' || m.presenter_allowed); }

  /** Host-only raw sources for separately consented recap processing. Callers
   * must re-check this view on every room/media change and never stop its tracks.
   * Guests have only a combined host mix, which cannot identify its speakers. */
  getRecapAudioSources(): ReadonlyMap<string, RoomRecapAudioSource> {
    const sources = new Map<string, RoomRecapAudioSource>();
    if (this.disposed || !this.isHost || !this.state.audioJoined) return sources;
    for (const member of this.snapshot?.members ?? []) {
      const local = member.id === this.options.memberId;
      const muted = member.muted || (local && this.state.muted);
      const joined = member.audio_joined && (!local || this.state.audioJoined);
      if (member.admission !== 'admitted' || !member.connected || !joined || muted || member.room_muted) continue;
      const peer = this.peers.get(member.id);
      const stream = local ? this.mic.value : peer?.generation === member.generation ? peer.audioStream : null;
      if (stream?.getAudioTracks().some(track => track.readyState === 'live')) sources.set(member.id,
        { memberId: member.id, generation: member.generation, stream });
    }
    return sources;
  }

  update(snapshot: RoomSnapshot): void {
    if (this.disposed) return;
    const previousSelf = this.self(), previousHost = this.snapshot?.members?.find(m => m.id === this.snapshot?.host_member_id);
    this.snapshot = snapshot;
    const self = this.self(), host = snapshot.members?.find(m => m.id === snapshot.host_member_id);
    if (snapshot.admission !== 'admitted' || !self?.connected || !host?.connected) { this.disconnect(); return; }
    if (previousSelf && (previousSelf.generation !== self.generation || previousHost?.id !== host.id || previousHost.generation !== host.generation)) this.reset();
    if (!previousSelf || previousSelf.generation !== self.generation) this.send({ type: 'request_ice' });
    if (self.room_muted && (!previousSelf?.room_muted || !this.state.muted || this.mic.value)) this.mute();
    if (previousSelf?.audio_joined && !self.audio_joined && this.state.audioJoined) this.leaveAudio();
    if (!this.isHost && !host.audio_joined && (this.state.audioJoined || previousHost?.audio_joined)) this.leaveAudio();
    if (!this.canPresent() || (this.ownSource !== null && !this.source(this.ownSource))) this.stopPresentation();
    const own = snapshot.presentations?.find(p => p.member_id === self.id && p.id !== this.replacingSource);
    if (own && this.screen.value) {
      this.ownSource = own.id; this.replacingSource = null;
      if (this.captureDeadline) { clearTimeout(this.captureDeadline); this.captureDeadline = null; }
    }
    for (const [id, peer] of this.peers) {
      const member = snapshot.members?.find(m => m.id === id);
      if (!member?.connected || member.admission !== 'admitted' || member.generation !== peer.generation) this.closePeer(id);
      else for (const [id, incoming] of peer.incoming) if (this.source(id)?.generation !== incoming.generation) peer.incoming.delete(id);
    }
    for (const [viewer, subs] of this.subscriptions) {
      if (!snapshot.members?.some(m => m.id === viewer && m.connected)) this.subscriptions.delete(viewer);
      else for (const [id, sub] of subs) if (this.source(id)?.generation !== sub.generation) subs.delete(id);
    }
    this.rebuildGraph(); this.publishStreams(); this.schedule();
  }

  async joinAudio(): Promise<void> {
    if (this.disposed || !this.self()?.connected) return;
    if (!this.isHost && !this.snapshot?.members?.find(m => m.id === this.snapshot?.host_member_id)?.audio_joined) {
      this.error('The host needs to join audio before guests can join.'); return;
    }
    const epoch = ++this.joinEpoch;
    try {
      const context = this.context ?? this.env.createAudioContext(); this.context = context;
      await context.resume();
      if (this.disposed || epoch !== this.joinEpoch || (!this.isHost && !this.snapshot?.members?.find(m => m.id === this.snapshot?.host_member_id)?.audio_joined)) {
        if (this.context === context) this.context = null;
        void context.close(); return;
      }
      this.state.audioJoined = true; this.state.muted = true; this.state.error = null;
      this.send({ type: 'audio', joined: true, muted: true });
      if (!this.ackTimer) this.ackTimer = setInterval(() => this.ackAudio(true), 2000);
      this.emit(); this.schedule();
    } catch { this.error('Audio could not start. Select Join audio again, or check the output device.'); }
  }
  async unmute(deviceId?: string): Promise<void> {
    if (!this.state.audioJoined || this.self()?.room_muted || this.disposed) return;
    const gen = this.self()?.generation;
    try {
      const stream = await this.mic.acquire(() => this.env.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true,
        ...(deviceId ? { deviceId: { exact: deviceId } } : {}) }, video: false }),
      () => !this.disposed && this.state.audioJoined && !this.self()?.room_muted && this.self()?.generation === gen);
      if (!stream) return;
      const track = stream.getAudioTracks()[0];
      if (!track) { this.mic.stop(); this.error('No microphone track was available. Choose another microphone.'); return; }
      track.addEventListener('ended', () => { if (this.mic.value === stream) this.mute(); }, { once: true });
      this.state.muted = false; this.state.error = null;
      this.send({ type: 'audio', joined: true, muted: false }); this.emit(); this.schedule();
    } catch { if (!this.disposed && this.state.audioJoined) this.error('Microphone access was not granted. Check Otto microphone permission in System Settings, then select Unmute.'); }
  }
  mute(): void {
    this.mic.stop(); this.state.muted = true; this.rebuildGraph();
    if (this.state.audioJoined) this.send({ type: 'audio', joined: true, muted: true });
    this.emit(); this.schedule();
  }
  leaveAudio(): void {
    const joined = this.state.audioJoined;
    this.joinEpoch++; this.mic.stop(); this.state.audioJoined = false; this.state.muted = true;
    if (this.ackTimer) { clearInterval(this.ackTimer); this.ackTimer = null; }
    this.clearGraph(); for (const p of this.peers.values()) this.removeAudio(p);
    if (this.context) { const context = this.context; this.context = null; void context.close().catch(() => {}); }
    if (joined) this.send({ type: 'audio', joined: false, muted: true });
    this.emit(); this.schedule();
  }
  async startPresentation(): Promise<void> {
    if (!this.canPresent()) { this.error('Ask the host for permission to present first.'); return; }
    const gen = this.self()?.generation;
    try {
      const stream = await this.screen.acquire(() => this.env.getDisplayMedia({ video: { width: { ideal: 1920 },
        height: { ideal: 1080 }, frameRate: { ideal: 15, max: 15 } }, audio: false }), () => this.canPresent() && this.self()?.generation === gen);
      if (!stream) return;
      const track = stream.getVideoTracks()[0];
      if (!track) { this.screen.stop(); this.error('The selected source did not provide video. Try another window.'); return; }
      for (const audio of stream.getAudioTracks()) audio.stop();
      track.contentHint = 'detail';
      await track.applyConstraints({ width: { max: 1920 }, height: { max: 1080 }, frameRate: { max: 15 } }).catch(() => {});
      if (this.screen.value !== stream || !this.canPresent()) { for (const t of stream.getTracks()) t.stop(); return; }
      track.addEventListener('ended', () => { if (this.screen.value === stream) this.stopPresentation(); }, { once: true });
      const settings = track.getSettings();
      this.state.presenting = true; this.state.error = null; this.constraintKey = '';
      this.replacingSource = this.ownSource;
      if (this.ownSource) this.send({ type: 'stop_present', source_id: this.ownSource });
      this.ownSource = null;
      this.geometry = new CaptureGeometry(settings.width ?? 1920, settings.height ?? 1080);
      if (this.geometryTimer) clearInterval(this.geometryTimer);
      this.geometryTimer = setInterval(() => this.checkGeometry(), 500);
      this.send({ type: 'start_present', title: track.label?.slice(0, 120) || 'Shared screen', width: Math.min(8192, settings.width ?? 1920), height: Math.min(8192, settings.height ?? 1080) });
      if (this.captureDeadline) clearTimeout(this.captureDeadline);
      this.captureDeadline = setTimeout(() => { if (!this.ownSource && this.screen.value) {
        this.stopPresentation(); this.error('The room did not accept the presentation. Ask for permission and try again.');
      } }, 10_000);
      this.emit(); this.schedule();
    } catch { if (!this.disposed) this.error('Screen sharing was cancelled or is unavailable. Choose a supported window or display and check macOS screen-recording permission.'); }
  }
  stopPresentation(): void {
    if (this.captureDeadline) { clearTimeout(this.captureDeadline); this.captureDeadline = null; }
    const previous = this.ownSource; this.ownSource = null; this.replacingSource = null; this.screen.stop(); this.publisherDemand = null;
    if (this.geometryTimer) clearInterval(this.geometryTimer); this.geometryTimer = null; this.geometry = null;
    this.state.presenting = false; this.constraintKey = '';
    if (previous && this.source(previous)) this.send({ type: 'stop_present', source_id: previous });
    this.publishStreams(); this.emit(); this.schedule();
  }
  setSubscriptions(values: readonly RoomMediaSubscription[]): void {
    const previous = this.subscriptions.get(this.options.memberId) ?? new Map<string, RoomMediaSubscription>();
    const current = new Map<string, RoomMediaSubscription>();
    for (const sub of values.slice(0, 4)) if (this.source(sub.sourceId)?.generation === sub.generation) current.set(sub.sourceId, sub);
    // Release old pins before upgrading new ones.
    for (const [id, sub] of previous) if (!current.has(id) || (sub.tier === 'full' && current.get(id)?.tier !== 'full'))
      this.send({ type: 'subscribe', source_id: id, source_generation: sub.generation, tier: 'hidden' });
    for (const [id, sub] of current) if (previous.get(id)?.tier !== sub.tier || previous.get(id)?.generation !== sub.generation)
      this.send({ type: 'subscribe', source_id: id, source_generation: sub.generation, tier: sub.tier });
    this.subscriptions.set(this.options.memberId, current); this.publishStreams(); this.schedule();
  }
  handleEvent(event: RoomEvent): void {
    if (this.disposed) return;
    if (event.type === 'ended') { this.dispose(); return; }
    if (event.type === 'ice') {
      this.rtc = { iceServers: event.ice_servers, iceTransportPolicy: event.relay_only ? 'relay' : 'all' };
      this.state.relayConfigured = event.relay_configured;
      for (const peer of this.peers.values()) try { peer.pc.setConfiguration(this.rtc); } catch { this.closePeer(peer.id); }
      if (this.iceRefresh) clearTimeout(this.iceRefresh);
      const remaining = Date.parse(event.expires_at) - Date.now() - 60_000;
      this.iceRefresh = setTimeout(() => this.send({ type: 'request_ice' }), Math.max(30_000, Math.min(remaining, 30 * 60_000)));
      this.emit(); this.schedule(); return;
    }
    if (event.type === 'subscription' && this.isHost) {
      const source = this.source(event.source_id);
      if (!source || source.generation !== event.source_generation || !this.snapshot?.members?.some(m => m.id === event.member_id && m.connected && m.admission === 'admitted')) return;
      let subs = this.subscriptions.get(event.member_id);
      if (!subs) { subs = new Map(); this.subscriptions.set(event.member_id, subs); }
      subs.set(source.id, { sourceId: source.id, generation: source.generation, tier: event.tier }); this.schedule(); return;
    }
    if (event.type !== 'signal' || event.to !== this.options.memberId) return;
    const sender = this.snapshot?.members?.find(m => m.id === event.from);
    if (!sender?.connected || sender.admission !== 'admitted' || sender.generation !== event.generation
      || (!this.isHost && sender.id !== this.snapshot?.host_member_id)) return;
    const media = parseMediaEnvelope(event.media);
    if (!media) { this.error('An invalid media message was rejected.'); return; }
    if (media.kind === 'demand') {
      const source = this.source(media.sourceId);
      if (!this.isHost && source?.member_id === this.options.memberId && source.generation === media.generation
        && (this.publisherDemand?.sourceId !== source.id || this.publisherDemand?.generation !== source.generation || this.publisherDemand?.tier !== media.tier)) {
        this.publisherDemand = { sourceId: source.id, generation: source.generation, tier: media.tier }; this.schedule();
      }
      return;
    }
    const peer = this.getPeer(sender); if (!peer) return;
    if (peer.pendingSignals >= 32) { this.closePeer(peer.id); this.error('Too many pending media messages. Rejoin the media connection.'); return; }
    peer.pendingSignals++;
    peer.signals = peer.signals.then(() => this.applySignal(peer, media)).catch(() => {
      if (this.peers.get(peer.id) === peer) this.error('Media negotiation failed. Rejoin audio or restart sharing; chat and terminal remain available.');
    }).finally(() => { peer.pendingSignals--; });
  }
  private signal(peer: Peer, media: MediaEnvelope): void {
    if (this.peers.get(peer.id) === peer) this.send({ type: 'signal', to: peer.id, generation: peer.generation, media });
  }
  private bindings(peer: Peer): Binding[] {
    return [...peer.outgoing.values()].map(v => ({ streamId: v.stream.id, sourceId: v.source.id, generation: v.source.generation }));
  }
  private getPeer(member: RoomMember): Peer | null {
    const existing = this.peers.get(member.id); if (existing) return existing;
    if (this.peers.size >= 3 || member.id === this.options.memberId) return null;
    try {
      const pc = this.env.createPeer(this.rtc);
      const peer: Peer = { id: member.id, generation: member.generation, pc, makingOffer: false, ignoreOffer: false, settingAnswer: false,
        pendingIce: [], signals: Promise.resolve(), pendingSignals: 0, bindings: [], unbound: new Map(), outgoing: new Map(), incoming: new Map(),
        videoSlots: [], audioWork: Promise.resolve(), audio: null, audioStream: null, audioSender: null, mix: null, deadline: null, demandKey: '', bindingKey: '' };
      this.peers.set(member.id, peer);
      pc.onicecandidate = e => this.signal(peer, { version: 1, kind: 'ice', candidate: e.candidate?.toJSON() ?? null });
      pc.onnegotiationneeded = async () => {
        try {
          peer.makingOffer = true;
          if (pc.signalingState !== 'stable') return;
          await pc.setLocalDescription(await pc.createOffer());
          if (pc.localDescription) this.signal(peer, { version: 1, kind: 'description', description: pc.localDescription.toJSON(), bindings: this.bindings(peer) });
        } catch { if (this.peers.get(peer.id) === peer) this.error('Media negotiation could not start. Check the host connection setup.'); }
        finally { peer.makingOffer = false; }
      };
      pc.ontrack = e => {
        if (e.track.kind === 'audio') { peer.audio = e.track; peer.audioStream = this.env.createStream([e.track]); this.rebuildGraph(); }
        else { const stream = e.streams[0]; if (stream && peer.unbound.size < 4) peer.unbound.set(stream.id, stream); this.bindIncoming(peer); }
        this.schedule();
      };
      pc.onconnectionstatechange = () => {
        if (pc.connectionState === 'failed' || pc.connectionState === 'disconnected') {
          this.closePeer(peer.id); this.state.connection = 'failed';
          this.error('Media connection was lost. Check the host STUN/TURN setup, then rejoin audio or restart sharing.');
        } else {
          if (pc.connectionState === 'connected' && peer.deadline) { clearTimeout(peer.deadline); peer.deadline = null; }
          this.updateConnection();
        }
      };
      peer.deadline = setTimeout(() => { if (pc.connectionState !== 'connected' && this.peers.get(peer.id) === peer) {
        this.closePeer(peer.id); this.state.connection = 'failed'; this.error('Media could not connect. Ask the host to configure STUN/TURN for this network.');
      } }, 20_000);
      this.updateConnection(); return peer;
    } catch { this.error('WebRTC media is unavailable in this client. Chat and terminal remain available.'); return null; }
  }
  private async applySignal(peer: Peer, media: Exclude<MediaEnvelope, { kind: 'demand' }>): Promise<void> {
    if (this.peers.get(peer.id) !== peer) return;
    if (media.kind === 'ice') {
      if (peer.ignoreOffer) return;
      if (!peer.pc.remoteDescription) {
        if (peer.pendingIce.length >= 128) { this.closePeer(peer.id); return; }
        peer.pendingIce.push(media.candidate);
      } else await peer.pc.addIceCandidate(media.candidate ?? undefined);
      return;
    }
    peer.bindings = media.bindings.filter(b => {
      const source = this.source(b.sourceId);
      return source?.generation === b.generation && (!this.isHost || source.member_id === peer.id);
    });
    if (media.kind === 'bindings') {
      this.bindIncoming(peer);
      // A reused receiver emits no new ontrack event. Relay its newly bound
      // source to downstream viewers even when no other room event follows.
      this.schedule(); return;
    }
    const pc = peer.pc;
    const collision = media.description.type === 'offer' && (peer.makingOffer || (pc.signalingState !== 'stable' && !peer.settingAnswer));
    peer.ignoreOffer = collision && this.options.memberId < peer.id;
    if (peer.ignoreOffer) return;
    if (collision && pc.signalingState !== 'stable') await pc.setLocalDescription({ type: 'rollback' });
    peer.settingAnswer = media.description.type === 'answer';
    try { await pc.setRemoteDescription(media.description); } finally { peer.settingAnswer = false; }
    if (this.peers.get(peer.id) !== peer) return;
    this.bindIncoming(peer);
    for (const candidate of peer.pendingIce.splice(0)) await pc.addIceCandidate(candidate ?? undefined);
    if (media.description.type === 'offer') {
      await pc.setLocalDescription(await pc.createAnswer());
      if (pc.localDescription) this.signal(peer, { version: 1, kind: 'description', description: pc.localDescription.toJSON(), bindings: this.bindings(peer) });
    }
    this.schedule();
  }
  private bindIncoming(peer: Peer): void {
    for (const [id, incoming] of peer.incoming) if (!peer.bindings.some(b => b.sourceId === id && b.generation === incoming.generation)) peer.incoming.delete(id);
    for (const b of peer.bindings) {
      const source = this.source(b.sourceId), stream = peer.unbound.get(b.streamId);
      if (!source || source.generation !== b.generation || !stream || (this.isHost && source.member_id !== peer.id)) continue;
      peer.incoming.set(source.id, { stream, generation: source.generation });
    }
    // replaceTrack reuses the receiver without firing ontrack. Keep its bounded
    // stream association while no source is bound, so a later source ID or
    // generation can reuse the slot. Only bindings expose it to the UI.
    for (const [id, stream] of peer.unbound) if (stream.getVideoTracks().every(track => track.readyState === 'ended')) peer.unbound.delete(id);
    this.publishStreams();
  }
  private trackFor(source: RoomPresentation): MediaStreamTrack | undefined {
    if (source.member_id === this.options.memberId) return source.id === this.ownSource ? this.screen.value?.getVideoTracks()[0] : undefined;
    for (const peer of this.peers.values()) { const incoming = peer.incoming.get(source.id);
      if (incoming?.generation === source.generation) return incoming.stream.getVideoTracks()[0]; }
    return undefined;
  }
  private subscription(viewer: string, source: RoomPresentation): RoomSubscriptionTier {
    const sub = this.subscriptions.get(viewer)?.get(source.id); return sub?.generation === source.generation ? sub.tier : 'hidden';
  }
  private aggregate(source: RoomPresentation): RoomSubscriptionTier {
    return highestTier((this.snapshot?.members ?? []).filter(m => m.connected && m.id !== source.member_id).map(m => this.subscription(m.id, source)));
  }
  private schedule(): void {
    if (this.disposed || this.queued) return;
    this.queued = true;
    this.work = this.work.then(async () => { this.queued = false; if (!this.disposed) await this.reconcile(); })
      .catch(() => { if (!this.disposed) this.error('A media update failed. Stop and restart the affected stream.'); });
  }
  private async reconcile(): Promise<void> {
    const self = this.self(), snapshot = this.snapshot, lifecycle = this.lifecycle;
    // A snapshot may revoke a source or remove a later target while a sender
    // operation is pending. update() queues a fresh pass; abandon this one.
    const stale = () => this.lifecycle !== lifecycle || this.snapshot !== snapshot;
    if (!self?.connected || !snapshot?.host_member_id) return;
    const members = snapshot.members ?? [], sources = snapshot.presentations ?? [];
    const targets = this.isHost ? members.filter(m => m.id !== self.id && m.connected && m.admission === 'admitted')
      : members.filter(m => m.id === snapshot.host_member_id && m.connected);
    for (const member of targets) {
      const wantAudio = this.state.audioJoined && (this.isHost ? member.audio_joined : true);
      const wanted = sources.filter(s => s.member_id !== member.id && (this.isHost ? this.subscription(member.id, s) !== 'hidden'
        : s.member_id === self.id && this.publisherDemand?.sourceId === s.id && this.publisherDemand.generation === s.generation && this.publisherDemand.tier !== 'hidden'));
      const needIncoming = this.isHost ? sources.some(s => s.member_id === member.id && this.aggregate(s) !== 'hidden')
        : sources.some(s => s.member_id !== self.id && this.subscription(self.id, s) !== 'hidden');
      let peer = this.peers.get(member.id);
      if (!peer && (wantAudio || wanted.length || needIncoming)) peer = this.getPeer(member) ?? undefined;
      if (!peer) continue;
      if (this.isHost) for (const s of sources.filter(s => s.member_id === member.id)) {
        const tier = this.aggregate(s), key = `${s.id}:${s.generation}:${tier}`;
        if (key !== peer.demandKey) { peer.demandKey = key; this.signal(peer, { version: 1, kind: 'demand', sourceId: s.id, generation: s.generation, tier }); }
      }
      for (const [id, outgoing] of peer.outgoing) if (!wanted.some(s => s.id === id && s.generation === outgoing.source.generation)) {
        await outgoing.sender.replaceTrack(null); peer.outgoing.delete(id);
        if (stale() || this.peers.get(peer.id) !== peer) return;
      }
      for (const source of wanted) {
        const track = this.trackFor(source); if (!track || track.readyState === 'ended') continue;
        const current = peer.outgoing.get(source.id);
        if (current) { if (current.sender.track !== track) await current.sender.replaceTrack(track); }
        else {
          // removeTrack/addTrack cannot reliably reuse a previously negotiated
          // sending transceiver. Reserve at most one slot per possible remote
          // presentation and replace its track while retaining its SDP stream ID.
          let slot = peer.videoSlots.find(slot => ![...peer.outgoing.values()].some(v => v.sender === slot.sender));
          if (slot) await slot.sender.replaceTrack(track);
          else {
            if (peer.videoSlots.length >= (this.isHost ? 3 : 1)) continue;
            const stream = this.env.createStream([track]);
            slot = {stream, sender: peer.pc.addTrack(track, stream)};
            peer.videoSlots.push(slot);
          }
          // Record an attached track even if its source was revoked during the
          // await; the fresh pass must be able to find and clear that sender.
          peer.outgoing.set(source.id, {source, ...slot});
        }
        if (stale() || this.peers.get(peer.id) !== peer) return;
      }
      if (wantAudio) await this.setAudio(peer); else this.removeAudio(peer);
      if (stale() || this.peers.get(peer.id) !== peer) return;
      const bindings = this.bindings(peer), key = JSON.stringify(bindings);
      if (key !== peer.bindingKey) { peer.bindingKey = key; this.signal(peer, { version: 1, kind: 'bindings', bindings }); }
      if (!wantAudio && !wanted.length && !needIncoming) this.closePeer(peer.id);
    }
    this.rebuildGraph();
    const outgoing = [...this.peers.values()].flatMap(peer => [...peer.outgoing.values()].map(video => ({ peer, video })));
    const budgets = allocateVideoBudget(outgoing.map(({ peer, video }) => {
      const settings = video.sender.track?.getSettings();
      // Incoming publishers may already be downscaled to aggregate demand.
      const upstream = allocateVideoBudget([{ key: video.source.id, viewerId: self.id, tier: this.aggregate(video.source), width: video.source.width, height: video.source.height }])[0];
      const divisor = this.isHost && video.source.member_id !== self.id ? upstream.scaleResolutionDownBy : 1;
      return { key: `${peer.id}:${video.source.id}`, viewerId: peer.id,
        tier: this.isHost ? this.subscription(peer.id, video.source) : this.publisherDemand?.tier ?? 'hidden',
        width: settings?.width ?? Math.floor(video.source.width / divisor), height: settings?.height ?? Math.floor(video.source.height / divisor) };
    }));
    for (let i = 0; i < outgoing.length; i++) {
      const { peer, video } = outgoing[i], sender = video.sender, b = budgets[i], parameters = sender.getParameters();
      // A newly added sender can expose encodings before it has negotiated
      // codecs. Chromium can leave setParameters pending indefinitely there,
      // blocking every later source in this client's update queue. The remote
      // description handler schedules another pass once negotiation completes.
      if (!parameters.encodings?.length || !parameters.codecs?.length || peer.pc.signalingState !== 'stable') continue;
      parameters.degradationPreference = 'maintain-resolution';
      for (const encoding of parameters.encodings) {
        encoding.active = b.active; encoding.maxBitrate = Math.floor(b.maxBitrate / parameters.encodings.length);
        // WebKit can exceed this cadence despite accepting the parameter. The
        // native probe documents the unmet target; never canvas-recapture.
        encoding.maxFramerate = b.maxFramerate || 1; encoding.scaleResolutionDownBy = b.scaleResolutionDownBy;
      }
      await sender.setParameters(parameters);
      if (stale() || this.peers.get(peer.id) !== peer) return;
    }
    const own = sources.find(s => s.member_id === self.id), track = this.screen.value?.getVideoTracks()[0];
    if (own && track) {
      const demand = highestTier([this.subscription(self.id, own), this.isHost ? this.aggregate(own) : this.publisherDemand?.tier ?? 'hidden']);
      const b = allocateVideoBudget([{ key: own.id, viewerId: self.id, tier: demand, width: own.width, height: own.height }])[0];
      const key = `${own.id}:${demand}`;
      if (key !== this.constraintKey) {
        this.constraintKey = key;
        this.qualityChanging = true;
        try {
          // Keep source geometry independent of encoding quality. Resolution
          // constraints can crop a canvas/window and feed a metadata resize
          // back into another resize. Each RTP sender already scales its copy.
          await track.applyConstraints({ frameRate: { max: b.maxFramerate || 1 } });
        } catch { /* Some capture sources cannot change resolution in place. */ }
        finally {
          this.qualityChanging = false;
        }
      }
    }
    if (stale()) return;
    this.publishStreams(); this.appliedAudioEpoch = snapshot.audio_epoch ?? null; this.ackAudio();
  }
  private async setAudio(peer: Peer): Promise<void> {
    let stream: MediaStream | null = null;
    if (this.isHost && this.context) { peer.mix ??= this.context.createMediaStreamDestination(); stream = peer.mix.stream; }
    else if (!this.state.muted) stream = this.mic.value;
    const track = stream?.getAudioTracks()[0] ?? null;
    if (peer.audioSender) await this.replaceAudio(peer, track);
    else if (track && stream) peer.audioSender = peer.pc.addTrack(track, stream);
  }
  private replaceAudio(peer: Peer, track: MediaStreamTrack | null): Promise<void> {
    // leaveAudio is synchronous; serialize its null replacement with an
    // immediate rejoin so an older stop cannot mute the replacement capture.
    const sender = peer.audioSender;
    const update = peer.audioWork.then(async () => {
      if (this.peers.get(peer.id) === peer && sender && sender.track !== track) await sender.replaceTrack(track);
    });
    peer.audioWork = update.catch(() => {});
    return update;
  }
  private removeAudio(peer: Peer): void {
    if (peer.audioSender) void this.replaceAudio(peer, null).catch(() => {});
    if (peer.mix) { for (const t of peer.mix.stream.getTracks()) t.stop(); peer.mix.disconnect(); peer.mix = null; }
  }
  private clearGraph(): void { for (const node of this.graphNodes) node.disconnect(); this.graphNodes = []; }
  private rebuildGraph(): void {
    this.clearGraph();
    if (!this.context || !this.state.audioJoined) return;
    if (!this.isHost) {
      const host = this.peers.get(this.snapshot?.host_member_id ?? '');
      if (host?.audio?.readyState === 'live') {
        const source = this.context.createMediaStreamSource(this.env.createStream([host.audio]));
        source.connect(this.context.destination); this.graphNodes.push(source);
      }
      return;
    }
    const members = (this.snapshot?.members ?? []).map(m => m.id === this.options.memberId
      ? { ...m, audio_joined: this.state.audioJoined, muted: this.state.muted } : m);
    for (const member of members) {
      const track = member.id === this.options.memberId ? this.mic.value?.getAudioTracks()[0] : this.peers.get(member.id)?.audio;
      if (!track || track.readyState !== 'live') continue;
      const destinations: AudioNode[] = [];
      if (audioContributors(members, this.options.memberId).includes(member.id)) destinations.push(this.context.destination);
      for (const peer of this.peers.values()) if (peer.mix && audioContributors(members, peer.id).includes(member.id)) destinations.push(peer.mix);
      if (destinations.length) {
        const source = this.context.createMediaStreamSource(this.env.createStream([track]));
        for (const destination of destinations) source.connect(destination);
        this.graphNodes.push(source);
      }
    }
  }
  private appliedAudioEpoch: number | null = null;
  private lastSentAudioEpoch: number | null = null;
  private ackAudio(periodic = false): void {
    if (this.isHost && this.appliedAudioEpoch !== null && this.appliedAudioEpoch === this.snapshot?.audio_epoch
      && (periodic || this.lastSentAudioEpoch !== this.appliedAudioEpoch)) {
      this.lastSentAudioEpoch = this.appliedAudioEpoch;
      this.send({ type: 'audio_applied', epoch: this.appliedAudioEpoch });
    }
  }
  private checkGeometry(): void {
    if (this.qualityChanging || !this.ownSource || !this.geometry) return;
    const settings = this.screen.value?.getVideoTracks()[0]?.getSettings();
    if (!settings?.width || !settings.height) return;
    const changed = this.geometry.observe(settings.width, settings.height);
    if (changed) {
      this.constraintKey = '';
      this.send({ type: 'update_present', source_id: this.ownSource, ...changed });
    }
  }
  private publishStreams(): void {
    const streams = new Map<string, MediaStream>();
    for (const source of this.snapshot?.presentations ?? []) {
      if (source.member_id === this.options.memberId && source.id === this.ownSource && this.screen.value) streams.set(source.id, this.screen.value);
      else if (this.subscription(this.options.memberId, source) !== 'hidden') for (const peer of this.peers.values()) {
        const incoming = peer.incoming.get(source.id); if (incoming?.generation === source.generation) streams.set(source.id, incoming.stream);
      }
    }
    if (streams.size !== this.lastStreams.size || [...streams].some(([id, stream]) => this.lastStreams.get(id) !== stream)) {
      this.lastStreams = streams; this.options.onStreams?.(new Map(streams));
    }
  }
  private updateConnection(): void {
    const peers = [...this.peers.values()];
    this.state.connection = !peers.length ? 'idle' : peers.every(p => p.pc.connectionState === 'connected') ? 'connected' : 'connecting'; this.emit();
  }
  private closePeer(id: string): void {
    const peer = this.peers.get(id); if (!peer) return;
    this.peers.delete(id); if (peer.deadline) clearTimeout(peer.deadline);
    peer.pc.ontrack = null; peer.pc.onicecandidate = null; peer.pc.onnegotiationneeded = null; peer.pc.onconnectionstatechange = null;
    this.removeAudio(peer); for (const receiver of peer.pc.getReceivers()) receiver.track.stop();
    peer.pc.close(); peer.pendingIce = []; peer.unbound.clear(); peer.incoming.clear(); peer.outgoing.clear();
    this.rebuildGraph(); this.publishStreams(); this.updateConnection();
  }
  private reset(): void {
    this.lifecycle++; this.joinEpoch++; this.mic.stop(); this.screen.stop(); this.ownSource = null; this.replacingSource = null; this.publisherDemand = null; this.appliedAudioEpoch = null; this.lastSentAudioEpoch = null;
    if (this.geometryTimer) clearInterval(this.geometryTimer); this.geometryTimer = null; this.geometry = null; this.qualityChanging = false;
    if (this.captureDeadline) clearTimeout(this.captureDeadline); this.captureDeadline = null;
    if (this.iceRefresh) clearTimeout(this.iceRefresh); this.iceRefresh = null;
    if (this.ackTimer) clearInterval(this.ackTimer); this.ackTimer = null;
    this.state.audioJoined = false; this.state.muted = true; this.state.presenting = false;
    this.clearGraph(); for (const id of [...this.peers.keys()]) this.closePeer(id);
    if (this.context) { void this.context.close().catch(() => {}); this.context = null; }
    this.subscriptions.clear(); this.constraintKey = ''; this.publishStreams(); this.emit();
  }
  /** Socket loss stops all devices immediately; reconnect never restarts capture. */
  disconnect(): void { this.reset(); this.snapshot = null; }
  dispose(): void { this.disposed = true; this.reset(); }
}
