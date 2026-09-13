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
    if (old) {Object.assign(old, node); return old;}
    return node;
  });
  if (next.summary) current.checkpoints = generation === next.checkpoint_generation ? checkpoints : [];
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
