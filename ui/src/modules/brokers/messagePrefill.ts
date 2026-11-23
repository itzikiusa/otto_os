// Pure helpers for Kafka message prefill flows ("Edit & Produce copy…" and
// "Replay to topic…") and topic partition-range math. Zero Svelte/DOM deps so
// plain-Node unit tests can exercise every edge case directly.

import type { DecodedPayload, KafkaMessage, MessageHeader, PartitionRange } from '../../lib/api/types';

export interface ProducePrefill {
  key: string;
  keyBase64: boolean;
  value: string;
  valueBase64: boolean;
  tombstone: boolean;
  partition: number;
  headers: MessageHeader[];
}

export interface ReplayPrefill {
  sourceTopic: string;
  partition: number;
  offset: number;
  /** Monotonic trigger counter so re-selecting the same message re-applies. */
  seq?: number;
}

export interface ReplayPrefillState {
  sourceTopic: string;
  selectorType: 'offset_range';
  partition: number;
  fromOffset: number;
  toOffset: number;
}

function decodeForProduce(payload: DecodedPayload | null): { text: string; base64: boolean } {
  if (!payload) return { text: '', base64: false };
  const binary = payload.format === 'base64' || payload.format === 'hex';
  if (binary && payload.raw_base64) return { text: payload.raw_base64, base64: true };
  if (payload.format === 'base64') return { text: payload.text, base64: true };
  return { text: payload.text, base64: false };
}

/** Build Produce-tab field values from a peeked message without mutating it. */
export function buildProducePrefill(msg: KafkaMessage): ProducePrefill {
  const key = decodeForProduce(msg.key);
  const tombstone = msg.value === null;
  const value = tombstone ? { text: '', base64: false } : decodeForProduce(msg.value);
  return {
    key: key.text,
    keyBase64: key.base64,
    value: value.text,
    valueBase64: value.base64,
    tombstone,
    partition: msg.partition,
    headers: msg.headers.map((h) => ({ key: h.key, value: h.value })),
  };
}

/** Convert a single-message Replay prefill into ReplayPanel form fields. */
export function buildReplayPrefill(prefill: ReplayPrefill): ReplayPrefillState {
  return {
    sourceTopic: prefill.sourceTopic,
    selectorType: 'offset_range',
    partition: prefill.partition,
    fromOffset: prefill.offset,
    toOffset: prefill.offset,
  };
}

/** Stable identity key for a ReplayPrefill so same-cluster re-renders don't overwrite edits. */
export function replayPrefillKey(prefill: ReplayPrefill | null | undefined): string | null {
  if (!prefill) return null;
  return `${prefill.seq ?? ''}:${prefill.sourceTopic}:${prefill.partition}:${prefill.offset}`;
}

export function shouldApplyReplayPrefill(
  prefill: ReplayPrefill | null | undefined,
  lastAppliedKey: string | null,
): boolean {
  const key = replayPrefillKey(prefill);
  return key !== null && key !== lastAppliedKey;
}

/** Merge two PartitionRange arrays, keeping the widest [low, high] per partition. */
export function mergePartitionRanges(a: PartitionRange[], b: PartitionRange[]): PartitionRange[] {
  const map = new Map<number, PartitionRange>();
  for (const r of a) map.set(r.partition, { ...r });
  for (const r of b) {
    const existing = map.get(r.partition);
    if (!existing) {
      map.set(r.partition, { ...r });
    } else {
      map.set(r.partition, {
        partition: r.partition,
        low: Math.min(existing.low, r.low),
        high: Math.max(existing.high, r.high),
      });
    }
  }
  return [...map.values()].sort((x, y) => x.partition - y.partition);
}

/** Compute the 0–100% position of `offset` within a partition's [low, high] watermark range. */
export function rangePct(msg: Pick<KafkaMessage, 'offset'>, range: PartitionRange | undefined): number | null {
  if (!range) return null;
  const span = range.high - range.low;
  if (span <= 0) return null;
  return Math.min(100, Math.max(0, ((msg.offset - range.low) / span) * 100));
}

export function offsetPct(msg: Pick<KafkaMessage, 'partition' | 'offset'>, partitions: PartitionRange[]): number | null {
  return rangePct(msg, partitions.find((r) => r.partition === msg.partition));
}
