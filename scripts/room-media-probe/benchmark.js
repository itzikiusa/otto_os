// Synthetic canvas/audio only. Never calls getUserMedia or getDisplayMedia.
void (async () => {
const send=x=>window.webkit.messageHandlers.result.postMessage(JSON.stringify(x));
const wakeups=[];const sleep=ms=>new Promise(r=>wakeups.push({at:performance.now()+ms,r}));const draws=[];window.__nativeTick=()=>{for(const draw of draws)draw();for(let i=wakeups.length-1;i>=0;i--)if(performance.now()>=wakeups[i].at)wakeups.splice(i,1)[0].r();};
const contexts=[], tracks=[], timers=[], pcs=[], videos=[], latency=[];
let phase='baseline';
try {
 send({event:'baseline',secure:isSecureContext,getDisplayMedia:typeof navigator.mediaDevices?.getDisplayMedia});
 await sleep(5000);
 const ac=new AudioContext();contexts.push(ac);ac.resume().catch(()=>{});
 const sources=Array.from({length:4},(_,i)=>{
  const canvas=document.createElement('canvas');canvas.width=1920;canvas.height=1080;
  const cx=canvas.getContext('2d');let frame=0;
  const draw=()=>{
   cx.fillStyle='#111827';cx.fillRect(0,0,1920,1080);cx.font='14px monospace';cx.fillStyle='#e2e8f0';
   for(let n=0;n<52;n++)cx.fillText(`${n+1}   // Presenter ${i}, synthetic editor ${frame}    const result = await processRoom(${(n+frame)%137});`,24,100+n*18);
   // Large monochrome bit cells survive 320x180 previews, for same-clock latency measurement.
   const stamp=Math.floor(performance.now());for(let b=0;b<24;b++){cx.fillStyle=stamp&(1<<b)?'#ffffff':'#000000';cx.fillRect(b*60,0,60,60);}
   frame++;
  };draw();draws.push(draw);
  const stream=canvas.captureStream(15), track=stream.getVideoTracks()[0];track.contentHint='detail';tracks.push(track);
  const dest=ac.createMediaStreamDestination(), oscillator=ac.createOscillator();oscillator.frequency.value=220+i*110;oscillator.connect(dest);oscillator.start();tracks.push(...dest.stream.getTracks());
  return {stream,track,audio:dest.stream.getAudioTracks()[0],oscillator};
 });
 const links=[];
 function makePeer(){const p=new RTCPeerConnection({iceServers:[{urls:'stun:127.0.0.1:34789'}]});pcs.push(p);return p;}
 async function negotiate(a,b){await a.setLocalDescription(await a.createOffer());send({event:'offer',gathering:a.iceGatheringState,mlines:a.localDescription.sdp.split('\r\n').filter(x=>x.startsWith('m=')||x.startsWith('a=send')||x.startsWith('a=recv'))});await b.setRemoteDescription(a.localDescription);await b.setLocalDescription(await b.createAnswer());send({event:'answer',gathering:b.iceGatheringState,mlines:b.localDescription.sdp.split('\r\n').filter(x=>x.startsWith('m=')||x.startsWith('a=send')||x.startsWith('a=recv'))});await a.setRemoteDescription(b.localDescription);}
 function consume(track,label){
  const v=document.createElement('video');v.muted=true;v.autoplay=true;v.playsInline=true;v.width=320;v.height=180;v.srcObject=new MediaStream([track]);document.body.append(v);videos.push(v);v.play().catch(()=>{});
  const sample=document.createElement('canvas');sample.width=320;sample.height=180;const c=sample.getContext('2d',{willReadFrequently:true});
  const cb=()=>{try {c.drawImage(v,0,0,320,180);const data=c.getImageData(0,4,240,1).data;let stamp=0;for(let b=0;b<24;b++)if(data[(b*10+5)*4]>127)stamp|=1<<b;const lag=performance.now()-stamp;if(lag>=0&&lag<10000)latency.push({phase,label,lag});}catch{}v.requestVideoFrameCallback(cb);};v.requestVideoFrameCallback(cb);
 }
 for(let guest=1;guest<4;guest++){
  const host=makePeer(),remote=makePeer(),pendingH=[],pendingR=[];remote.createDataChannel('fixture-clock');
  host.onicecandidate=({candidate})=>{if(candidate){if(remote.remoteDescription)remote.addIceCandidate(candidate).catch(()=>{});else pendingR.push(candidate)}};
  remote.onicecandidate=({candidate})=>{if(candidate){if(host.remoteDescription)host.addIceCandidate(candidate).catch(()=>{});else pendingH.push(candidate)}};
  const link={guest,host,remote,hostSenders:[],incoming:null};links.push(link);
  host.ontrack=e=>{if(e.track.kind==='video'){link.incoming=e.track;consume(e.track,`host-from-${guest}`)}};
  remote.ontrack=e=>{if(e.track.kind==='video')consume(e.track,`guest-${guest}`)};
  remote.addTrack(sources[guest].track,sources[guest].stream);
  const mix=ac.createMediaStreamDestination();for(let i=0;i<4;i++)if(i!==guest)ac.createMediaStreamSource(new MediaStream([sources[i].audio])).connect(mix);
  tracks.push(...mix.stream.getTracks());host.addTrack(mix.stream.getAudioTracks()[0],mix.stream);remote.addTrack(sources[guest].audio,new MediaStream([sources[guest].audio]));
  const sender=host.addTrack(sources[0].track,sources[0].stream);link.hostSenders.push({source:0,sender});
  await negotiate(remote,host);for(const c of pendingH)await host.addIceCandidate(c);for(const c of pendingR)await remote.addIceCandidate(c);let waits=0;while(host.connectionState!=='connected'&&waits++<100)await sleep(100);send({event:'initial-connected',guest,state:host.connectionState,visibility:document.visibilityState});if(host.connectionState!=='connected')throw Error('Initial connection did not connect');
 }
 for(const link of links){for(const other of links)if(other.guest!==link.guest){if(!other.incoming)throw Error('Missing incoming track');const sender=link.host.addTrack(other.incoming,new MediaStream([other.incoming]));link.hostSenders.push({source:other.guest,sender});}await negotiate(link.host,link.remote);}
 const tune=async(mode)=>{phase=mode;for(const link of links)for(const {source,sender}of link.hostSenders){const p=sender.getParameters();const pin=(link.guest+1)%4;const full=source===pin;for(const e of p.encodings){e.active=mode!=='hidden';e.maxBitrate=mode==='grid'?600000:full?2000000:100000;e.maxFramerate=mode==='grid'?5:full?15:2;e.scaleResolutionDownBy=mode==='grid'?2:full?1:6;}await sender.setParameters(p);}send({event:'phase',phase,audioState:ac.state,connections:pcs.map(p=>p.connectionState)});};
 let previous=new Map();
 async function stats(){let hostBytes=0,guestBytes=0;const streams=[];for(const [idx,p]of pcs.entries()){const s=await p.getStats();for(const row of s.values())if(row.type==='outbound-rtp'){const key=idx+row.id,old=previous.get(key);if(old&&row.kind==='video'){const rate=(row.bytesSent-old.bytes)*8000/(row.timestamp-old.ts);if(idx%2===0)hostBytes+=rate;else guestBytes+=rate;streams.push({peer:idx,width:row.frameWidth,height:row.frameHeight,fps:row.framesPerSecond,bps:Math.round(rate),quality:row.qualityLimitationReason});}previous.set(key,{bytes:row.bytesSent,ts:row.timestamp});}}send({event:'stats',phase,hostVideoBps:Math.round(hostBytes),guestVideoBps:Math.round(guestBytes),streams});}
 for(const mode of ['pins','grid','hidden','repin']){await tune(mode);for(let i=0;i<(mode==='hidden'?2:4);i++){await sleep(5000);await stats();}}
 const summary={};for(const mode of ['pins','grid','hidden','repin']){const values=latency.filter(x=>x.phase===mode).map(x=>x.lag).sort((a,b)=>a-b);summary[mode]={count:values.length,p50:values[Math.floor(values.length*.5)],p95:values[Math.floor(values.length*.95)]};}
 send({event:'latency',note:'Synthetic pixel timestamp to JS decoded video frame; single webview same clock, not actual capture/network glass-to-glass acceptance',summary});
 draws.splice(0);for(const t of timers)clearInterval(t);for(const s of sources)s.oscillator.stop();for(const t of tracks)t.stop();for(const p of pcs)p.close();for(const v of videos){v.srcObject=null;v.remove();}for(const c of contexts)await c.close();send({event:'cleanup',tracks:tracks.map(t=>t.readyState),peers:pcs.map(p=>p.connectionState),audio:ac.state});await sleep(5000);send({event:'done'});
}catch(e){send({event:'error',message:e.stack||String(e)});for(const t of timers)clearInterval(t);for(const t of tracks)t.stop();for(const p of pcs)p.close();for(const c of contexts)c.close();send({event:'done'});}
})()
