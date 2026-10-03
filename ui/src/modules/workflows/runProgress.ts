import type {NodeRunState, WorkflowRun} from '../../lib/api/types';

/** Summary freshness is independent of lazily loaded checkpoint pages/bodies. */
export function mergeRunProgress(current: WorkflowRun, next: WorkflowRun): boolean {
  if (current.id !== next.id || (next.rev ?? 0) < (current.rev ?? 0)) return false;
  const checkpoints = current.checkpoints ?? [];
  const generation = current.checkpoint_generation;
  const nodes = new Map(current.nodes.map(node => [node.node_id, node]));
  const context = next.context_dir ?? current.context_dir;
  Object.assign(current, next);
  current.context_dir = context;
  current.nodes = next.nodes.map(node => {
    const old = nodes.get(node.node_id);
    if (!old) return node;
    // `detail_version` hashes the node's whole server state: equal version +
    // status ⇒ nothing changed, so skip the Object.assign that rewrote its
    // fresh logs/sessions/activity objects (and re-rendered the step) on every
    // merge (backlog B6 / W4).
    if (node.detail_version && old.detail_version === node.detail_version && old.status === node.status) return old;
    Object.assign(old, node);
    return old;
  });
  if (next.summary) current.checkpoints = generation === next.checkpoint_generation ? checkpoints : [];
  return true;
}

/** A live `workflow_run_updated` event as the run view sees it. */
export interface LiveRunEvent {
  rev: number;
  status: string;
  node: NodeRunState | null;
  waitingApproval: boolean;
}

/** Apply a live event's node SUMMARY in place (perf W5) — the event carries
 *  exactly the node shape `/progress` serves, so a contiguous event needs no
 *  refetch. Returns false (→ refetch) unless it is safe: a summary view of a
 *  running run, the very next `rev`, a node the view already lists, and no
 *  checkpointed (loop) pages, whose freshness an event cannot describe. */
export function applyLiveNode(run: WorkflowRun, ev: LiveRunEvent): boolean {
  const node = ev.node;
  if (!run.summary || !node || ev.rev <= 0 || ev.rev !== (run.rev ?? 0) + 1) return false;
  if (run.status !== 'running' || ev.status !== 'running') return false;
  if (run.checkpoint_rev) return false;
  const old = run.nodes.find((n) => n.node_id === node.node_id);
  if (!old) return false;
  // Same `detail_version` + status ⇒ nothing to rewrite (see mergeRunProgress).
  if (!(node.detail_version && old.detail_version === node.detail_version && old.status === node.status)) {
    Object.assign(old, node);
  }
  run.rev = ev.rev;
  run.waiting_approval = ev.waitingApproval;
  return true;
}

/** Bound inactive detail bodies; explicitly expanded bodies can stay pinned. */
export class RunBodyCache {
  private entries = new Map<string, {version: string; body: unknown; bytes: number}>();
  private pins = new Set<string>();
  private key(run: string, node: string): string {return JSON.stringify([run,node]);}
  pin(run: string, nodes: string[]): void {
    this.pins = new Set(nodes.map(node => this.key(run,node)));
    this.evict();
  }
  get<T = NodeRunState>(run: string, node: string, version: string): T | null {
    const key = this.key(run,node), entry = this.entries.get(key);
    if (!entry || entry.version !== version) return null;
    this.entries.delete(key); this.entries.set(key,entry);
    return entry.body as T;
  }
  put(run: string, node: string, version: string, body: unknown): void {
    const key=this.key(run,node);
    this.entries.delete(key);
    this.entries.set(key,{version,body,bytes:2*JSON.stringify(body).length});
    this.evict();
  }
  clear(): void {this.entries.clear();this.pins.clear();}
  private evict(): void {
    const inactive=[...this.entries].filter(([key])=>!this.pins.has(key));
    let bytes=inactive.reduce((sum,[,entry])=>sum+entry.bytes,0), count=inactive.length;
    for (const [key,entry] of inactive) {
      if (count<=16 && bytes<=16*1024*1024) break;
      this.entries.delete(key); count--; bytes-=entry.bytes;
    }
  }
}

/**
 * One node-body fetch path shared by the step list (RunSteps) and the node
 * inspector (WorkflowsPage). Both used to fetch the same `…/nodes/{id}` body
 * independently — twice per `detail_version` for a node that was expanded AND
 * selected. Concurrent reads of the same (run, node) share one request; the
 * newest body per (run, node) is kept (bounded) so the second consumer gets it
 * without a request. A shared request is aborted only when EVERY consumer
 * that joined it has aborted (the inspector aborts superseded reads).
 */
export class SharedNodeBodies {
  private done = new Map<string, {version: string; body: unknown}>();
  private inflight = new Map<string, {promise: Promise<{detail_version: string; body: unknown}>; ctl: AbortController; refs: number}>();
  private cap: number;
  constructor(cap = 48) {this.cap = cap;}
  private key(run: string, node: string): string {return `${run}\u0000${node}`;}
  /** The cached body for exactly this version, else null. */
  peek<T = unknown>(run: string, node: string, version: string): T | null {
    const hit = this.done.get(this.key(run, node));
    return hit && hit.version === version ? (hit.body as T) : null;
  }
  fetch<T extends {detail_version: string; body: unknown}>(
    run: string, node: string, load: (signal: AbortSignal) => Promise<T>, signal?: AbortSignal,
  ): Promise<T> {
    const key = this.key(run, node);
    let entry = this.inflight.get(key);
    if (!entry) {
      const ctl = new AbortController();
      const promise = load(ctl.signal).then((result) => {
        this.done.delete(key);
        this.done.set(key, {version: result.detail_version, body: result.body});
        while (this.done.size > this.cap) this.done.delete(this.done.keys().next().value as string);
        return result;
      });
      const settle = () => { if (this.inflight.get(key)?.promise === promise) this.inflight.delete(key); };
      promise.then(settle, settle);
      entry = {promise, ctl, refs: 0};
      this.inflight.set(key, entry);
    }
    const e = entry;
    e.refs++;
    if (signal) {
      const leave = () => {
        e.refs--;
        if (e.refs <= 0) {
          e.ctl.abort();
          if (this.inflight.get(key) === e) this.inflight.delete(key);
        }
      };
      if (signal.aborted) leave();
      else signal.addEventListener('abort', leave, {once: true});
    }
    return e.promise as Promise<T>;
  }
  clear(): void {this.done.clear();}
}

/** The app-wide instance (module scope: survives the inspector re-mounting). */
export const sharedNodeBodies = new SharedNodeBodies();

/** Page membership remains useful while later checkpoint revisions arrive.
 * Preserve a row already fetched at a newer revision, independent of run.rev. */
export function mergeCheckpointPage<T extends {node_id:string}>(
  incoming: {items:T[];checkpoint_rev:number}, known:T[], versions:Map<string,number>,
): {items:T[];known:T[]} {
  const rows=new Map(known.map(row=>[row.node_id,row]));
  const items=incoming.items.map(row=>{
    const old=rows.get(row.node_id);
    if(old && (versions.get(row.node_id)??-1)>incoming.checkpoint_rev) return old;
    versions.set(row.node_id,incoming.checkpoint_rev);rows.set(row.node_id,row);return row;
  });
  return {items,known:[...rows.values()]};
}

/** A step/run duration a person can read at a glance: "840ms", "12.4s",
 *  "3m 05s", "1h 12m". Seconds alone ("4325.0s") hid how long a long agent
 *  step really ran. Empty for a missing value. */
export function fmtStepMs(ms?: number | null): string {
  if (ms == null || !Number.isFinite(ms)) return '';
  if (ms < 1000) return `${Math.max(0, Math.round(ms))}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  const totalS = Math.floor(ms / 1000);
  const h = Math.floor(totalS / 3600);
  const m = Math.floor((totalS % 3600) / 60);
  const s = totalS % 60;
  return h > 0 ? `${h}h ${String(m).padStart(2, '0')}m` : `${m}m ${String(s).padStart(2, '0')}s`;
}
