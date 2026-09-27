import type { RoomMember, RoomSubscriptionTier } from '../../lib/api/room-types';

export interface TrackOwner { getTracks(): { stop(): void }[] }

/** Owns a capture across asynchronous permission pickers and room revocations. */
export class CaptureSlot<T extends TrackOwner> {
  value: T | null = null;
  private epoch = 0;

  async acquire(capture: () => Promise<T>, allowed: () => boolean): Promise<T | null> {
    const epoch = ++this.epoch;
    const stream = await capture();
    if (epoch !== this.epoch || !allowed()) {
      for (const track of stream.getTracks()) track.stop();
      return null;
    }
    const previous = this.value;
    this.value = stream;
    if (previous && previous !== stream) for (const track of previous.getTracks()) track.stop();
    return stream;
  }

  stop(): void {
    this.epoch++;
    const previous = this.value;
    this.value = null;
    if (previous) for (const track of previous.getTracks()) track.stop();
  }
}

export interface VideoDemand {
  key: string; viewerId: string; tier: RoomSubscriptionTier; width: number; height: number;
}
export interface VideoBudget extends VideoDemand {
  active: boolean; maxBitrate: number; maxFramerate: number; scaleResolutionDownBy: number;
}
export const VIDEO_EGRESS_CEILING = 8_000_000;

/** Host playback is the host's minus-self mix too. Room mute is authoritative. */
export function audioContributors(members: readonly RoomMember[], recipientId: string): string[] {
  return members.filter(m => m.id !== recipientId && m.admission === 'admitted'
    && m.connected && m.audio_joined && !m.muted && !m.room_muted).map(m => m.id);
}

export const tierRank: Record<RoomSubscriptionTier, number> = { hidden: 0, preview: 1, grid: 2, full: 3 };
export function highestTier(tiers: readonly RoomSubscriptionTier[]): RoomSubscriptionTier {
  return tiers.reduce((best, tier) => tierRank[tier] > tierRank[best] ? tier : best, 'hidden');
}

/** Separates deliberate quality scaling from platform-observable source resizing. */
export class CaptureGeometry {
  private observed: { width: number; height: number };
  private content: { width: number; height: number };

  constructor(width: number, height: number) {
    this.observed = { width, height }; this.content = { width, height };
  }
  qualityChanged(width: number, height: number): void {
    if (this.valid(width, height)) this.observed = { width, height };
  }
  observe(width: number, height: number): { width: number; height: number } | null {
    if (!this.valid(width, height) || (width === this.observed.width && height === this.observed.height)) return null;
    this.content = { width: Math.max(1, Math.min(8192, Math.round(this.content.width * width / this.observed.width))),
      height: Math.max(1, Math.min(8192, Math.round(this.content.height * height / this.observed.height))) };
    this.observed = { width, height };
    return { ...this.content };
  }
  private valid(width: number, height: number): boolean {
    return Number.isFinite(width) && Number.isFinite(height) && width > 0 && height > 0;
  }
}

/** Per-recipient encodings, with a single full-detail pin and shared grid budget. */
export function allocateVideoBudget(demands: readonly VideoDemand[]): VideoBudget[] {
  const pins = new Set<string>();
  const gridCounts = new Map<string, number>();
  for (const d of demands) if (d.tier === 'grid') gridCounts.set(d.viewerId, (gridCounts.get(d.viewerId) ?? 0) + 1);
  const result = demands.map(d => {
    let tier = d.tier;
    if (tier === 'full') {
      if (pins.has(d.viewerId)) tier = 'preview';
      else pins.add(d.viewerId);
    }
    if (!Number.isFinite(d.width) || !Number.isFinite(d.height) || d.width <= 0 || d.height <= 0) tier = 'hidden';
    const [width, height, fps, bitrate] = tier === 'full' ? [1920, 1080, 15, 2_000_000]
      : tier === 'grid' ? [960, 540, 5, Math.floor(2_000_000 / (gridCounts.get(d.viewerId) ?? 1))]
      : tier === 'preview' ? [320, 180, 2, 100_000] : [1920, 1080, 0, 0];
    return { ...d, tier, active: tier !== 'hidden', maxBitrate: bitrate, maxFramerate: fps,
      scaleResolutionDownBy: tier === 'hidden' ? 1 : Math.max(1, d.width / width, d.height / height) };
  });
  let excess = result.reduce((sum, b) => sum + b.maxBitrate, 0) - VIDEO_EGRESS_CEILING;
  // Normally the four-person topology fits at 6.6 Mbps. Protect the aggregate
  // invariant even during pin transitions; previews yield before full detail.
  for (const tier of ['preview', 'grid', 'full'] as const) {
    const group = result.filter(b => b.tier === tier);
    if (excess <= 0 || group.length === 0) continue;
    const available = group.reduce((sum, b) => sum + b.maxBitrate, 0);
    const ratio = Math.max(0, (available - excess) / available);
    for (const b of group) {
      const reduced = Math.floor(b.maxBitrate * ratio);
      excess -= b.maxBitrate - reduced;
      b.maxBitrate = reduced;
      if (reduced === 0) { b.active = false; b.maxFramerate = 0; }
    }
  }
  return result;
}
