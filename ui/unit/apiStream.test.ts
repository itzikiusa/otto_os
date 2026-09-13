import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import ts from 'typescript';
function setup() {
  const sockets: any[]=[];
  class Socket {onopen:any;onmessage:any;onclose:any;onerror:any;sent:string[]=[];url:string;constructor(url:string){this.url=url;sockets.push(this);}send(s:string){this.sent.push(s);}close(){}}
  const context={exports:{} as any,$state:(v:any)=>v,URL,WebSocket:Socket,require:()=>({baseUrl:()=> 'http://localhost:7700',getToken:()=> 'token'})};
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
test('stream console retains a bounded tail and counts discarded messages',()=>{
 const {v,sockets}=setup();v.connect('A','sse',{url:'https://example.test',method:'GET',headers:[]});
 for(let n=0;n<2100;n++) sockets[0].onmessage({data:JSON.stringify({type:'event',data:String(n)})});
 assert.equal(v.items.length,1000);assert.equal(v.dropped,1100);assert.equal(v.items.at(-1).data,'2099');
});
