/** History transport ownership, kept independent of Svelte for deferred-I/O tests. */
interface RefreshOptions<T> {
  workspace:()=>string|null;
  fetch:(workspace:string,signal:AbortSignal)=>Promise<T[]>;
  publish:(rows:T[])=>void;
  error:(error:unknown)=>void;
  schedule?:(work:()=>void)=>(()=>void);
}
export class HistoryRefresh<T extends {id:string}> {
  private workspace:string|null=null;
  private epoch=0;
  private flight:AbortController|null=null;
  private unschedule:(()=>void)|null=null;
  private ids=new Set<string>();
  private generic=false;
  private seen=new Set<string>();
  private waiters:Array<()=>void>=[];
  private activeWaiters:Array<()=>void>=[];
  private options:RefreshOptions<T>;
  constructor(options:RefreshOptions<T>) {this.options=options;}
  reset():void {
    this.epoch++;this.flight?.abort();this.flight=null;this.unschedule?.();this.unschedule=null;
    this.ids.clear();this.generic=false;this.seen.clear();
    for(const done of [...this.waiters,...this.activeWaiters])done();this.waiters=[];this.activeWaiters=[];
    this.workspace=this.options.workspace();
  }
  request(entryId?:string):Promise<void> {
    if(this.workspace!==this.options.workspace())this.reset();
    if(!this.workspace || (entryId && this.seen.has(entryId)))return Promise.resolve();
    if(entryId)this.ids.add(entryId);else this.generic=true;
    const done=new Promise<void>(resolve=>this.waiters.push(resolve));
    this.schedule();return done;
  }
  private schedule():void {
    if(this.flight||this.unschedule)return;
    const schedule=this.options.schedule??((work:()=>void)=>{const id=setTimeout(work,150);return ()=>clearTimeout(id);});
    this.unschedule=schedule(()=>{this.unschedule=null;void this.run();});
  }
  private async run():Promise<void> {
    const workspace=this.workspace;if(!workspace)return;
    const epoch=this.epoch;const controller=new AbortController();this.flight=controller;
    const waiters=this.waiters;this.waiters=[];this.activeWaiters=waiters;
    this.ids.clear();this.generic=false;
    const current=()=>this.epoch===epoch && this.options.workspace()===workspace && this.flight===controller;
    try {
      const rows=await this.options.fetch(workspace,controller.signal);
      if(!current())return;
      this.seen=new Set(rows.map(row=>row.id));this.options.publish(rows);
      // Entry IDs let a returned snapshot satisfy direct-send/event duplicates.
      const hasKnownSignals=this.ids.size>0;
      for(const id of this.ids)if(this.seen.has(id))this.ids.delete(id);
      if(hasKnownSignals&&this.ids.size===0)this.generic=false;
    }catch(error){if(current()&&!controller.signal.aborted)this.options.error(error);}
    finally {
      for(const done of waiters)done();
      if(current()){
        this.activeWaiters=[];this.flight=null;
        if(this.generic||this.ids.size)this.schedule();
        else {for(const done of this.waiters)done();this.waiters=[];}
      }
    }
  }
}

export interface HistoryOwner {workspace:string|null;tab:string|undefined;draft:object}
interface DetailOptions<T> {
  owner:()=>HistoryOwner;
  fetch:(id:string,signal:AbortSignal)=>Promise<T>;
  publish:(entry:T)=>void;
  pending:(id:string|null)=>void;
  error:(error:unknown)=>void;
}
export class HistoryDetail<T> {
  private controller:AbortController|null=null;
  private options:DetailOptions<T>;
  constructor(options:DetailOptions<T>){this.options=options;}
  cancel():void {if(this.controller){this.controller.abort();this.controller=null;this.options.pending(null);}}
  async select(id:string):Promise<void> {
    this.cancel();const controller=new AbortController();this.controller=controller;
    const owner=this.options.owner();this.options.pending(id);
    const current=()=>{const now=this.options.owner();return this.controller===controller&&!controller.signal.aborted&&owner.workspace===now.workspace&&owner.tab===now.tab&&owner.draft===now.draft;};
    try {const entry=await this.options.fetch(id,controller.signal);if(current())this.options.publish(entry);}
    catch(error){if(current())this.options.error(error);}
    finally {if(this.controller===controller){this.controller=null;this.options.pending(null);}}
  }
}
