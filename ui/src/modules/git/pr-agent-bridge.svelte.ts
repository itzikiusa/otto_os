// The PR workspace's agent thread (PrAgentPanel) is asked from places that
// do not hold it: the PR header's ⋯ (PrDetail), the Files panel and a
// selection in the diff (PrAgentLinePick). They post the ask here; the
// workspace brings the panel forward (the left column, or the sheet on a
// phone) and the panel of that PR carries it out once it knows whether the PR
// already has a thread — so an ask never starts a second one.
import type { PrAgentAsk } from './pr-agent-context';
import { prAgentOriginId } from './pr-agent-thread';

export type { PrAgentKind } from './pr-agent';

class PrAgentBridge {
  /** The ask waiting for its PR's panel (a newer ask replaces it). */
  pending = $state.raw<{ key: string; ask: PrAgentAsk; seq: number } | null>(null);
  /** Threads moved out to Threads ("Open here again"): origin id → thread id. */
  detached = $state.raw<Readonly<Record<string, string>>>({});
  #seq = 0;

  ask(repoId: string, n: number, ask: PrAgentAsk): void {
    this.#seq += 1;
    this.pending = { key: prAgentOriginId(repoId, n), ask, seq: this.#seq };
  }

  /** The waiting ask for `key`, consumed. */
  take(key: string): PrAgentAsk | null {
    const p = this.pending;
    if (!p || p.key !== key) return null;
    this.pending = null;
    return p.ask;
  }

  detach(key: string, id: string): void {
    this.detached = { ...this.detached, [key]: id };
  }

  reattach(key: string): void {
    if (!(key in this.detached)) return;
    const next = { ...this.detached };
    delete next[key];
    this.detached = next;
  }
}

export const prAgentBridge = new PrAgentBridge();
