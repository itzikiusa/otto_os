import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import ts from 'typescript';
function setup(extra:Record<string,unknown>={}) {
  const sockets: any[]=[];
  class Socket {onopen:any;onmessage:any;onclose:any;onerror:any;sent:string[]=[];url:string;constructor(url:string){this.url=url;sockets.push(this);}send(s:string){this.sent.push(s);}close(){}}
  const context:any={exports:{} as any,$state:Object.assign((v:any)=>v,{raw:(v:any)=>v}),URL,WebSocket:Socket,require:()=>({baseUrl:()=> 'http://localhost:7700',getToken:()=> 'token'}),...extra};
  runInNewContext(ts.transpileModule(readFileSync(new URL('../src/lib/stores/apiStream.svelte.ts',import.meta.url),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,context);
  return {v:context.exports.apiStream,sockets};
}
test('late callbacks from a replaced socket cannot mutate current stream',()=>{
  const {v,sockets}=setup();const req={url:'https://example.test',method:'GET',headers:[]};
  v.connect('A','sse',req);v.connect('B','sse',req);
  sockets[0].onclose();sockets[0].onerror();sockets[0].onmessage({data:JSON.stringify({type:'event',data:'stale'})});
  assert.equal(v.status,'connecting');assert.equal(v.items.length,0);
  sockets[1].onopen();assert.match(sockets[1].url,/workspace_id=B/);
  assert.equal(JSON.parse(sockets[1].sent[0]).request.url,req.url);
});
test('switching request tabs closes the previous relay and clears its console',()=>{
  const {v,sockets}=setup();
  v.connect('A','websocket',{url:'https://a.test',method:'GET',headers:[]},'tab-a');
  sockets[0].onopen();
  sockets[0].onmessage({data:JSON.stringify({type:'open',detail:'A'})});
  sockets[0].onmessage({data:JSON.stringify({type:'message',data:'only A'})});
  v.retainOwner('A','tab-b');
  assert.equal(v.active,false);
  assert.equal(v.items.length,0);
  const before=sockets[0].sent.length;
  v.send('must not reach A');
  assert.equal(sockets[0].sent.length,before);
  sockets[0].onmessage({data:JSON.stringify({type:'message',data:'late A'})});
  assert.equal(v.items.length,0);
});
test('stream console retains a bounded tail and counts discarded messages',()=>{
 const {v,sockets}=setup();v.connect('A','sse',{url:'https://example.test',method:'GET',headers:[]});
 for(let n=0;n<2100;n++) sockets[0].onmessage({data:JSON.stringify({type:'event',data:String(n)})});
 assert.equal(v.items.length,1000);assert.equal(v.dropped,1100);assert.equal(v.items.at(-1).data,'2099');
});
test('a message burst updates the console once per animation frame',()=>{
 const frames:(()=>void)[]=[];
 const {v,sockets}=setup({requestAnimationFrame:(cb:()=>void)=>{frames.push(cb);return frames.length;}});
 v.connect('A','sse',{url:'https://example.test',method:'GET',headers:[]});
 const before=v.items;
 for(let n=0;n<1500;n++) sockets[0].onmessage({data:JSON.stringify({type:'event',data:String(n)})});
 assert.equal(v.items,before);assert.equal(frames.length,1);
 frames.shift()!();
 assert.equal(v.items.length,1000);assert.equal(v.dropped,500);assert.equal(v.items[0].data,'500');assert.equal(v.items.at(-1).data,'1499');
 v.clear();assert.equal(v.items.length,0);assert.equal(v.dropped,0);
});
