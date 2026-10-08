/** Gesture-started original procedural score and effects; no network audio. */
export class GameAudio {
 private context:AudioContext|null=null;
 private master:GainNode|null=null;
 private filter:BiquadFilterNode|null=null;
 private engine:OscillatorNode|null=null;
 private engineGain:GainNode|null=null;
 private noise:AudioBuffer|null=null;
 private nextBeat=0;
 private kind='shooter';private beat=0;private playing=true;
 muted=false;
 async start(kind='shooter'):Promise<void>{
  this.kind=kind;
  if(!this.context){
   const c=this.context=new AudioContext();this.master=c.createGain();this.master.gain.value=this.muted?0:.2;
   this.filter=c.createBiquadFilter();this.filter.type='lowpass';this.filter.frequency.value=12000;this.master.connect(this.filter);this.filter.connect(c.destination);
   this.engine=c.createOscillator();this.engine.type='sawtooth';this.engineGain=c.createGain();this.engineGain.gain.value=0;
   const exhaust=c.createBiquadFilter();exhaust.type='lowpass';exhaust.frequency.value=350;
   this.engine.connect(exhaust);exhaust.connect(this.engineGain);this.engineGain.connect(this.master);this.engine.start();
   this.noise=c.createBuffer(1,c.sampleRate*.4,c.sampleRate);const data=this.noise.getChannelData(0);for(let i=0;i<data.length;i++)data[i]=Math.random()*2-1;
  }
  await this.context.resume();
 }
 mute(value:boolean):void{this.muted=value;if(this.master&&this.context)this.master.gain.setTargetAtTime(value?0:.2,this.context.currentTime,.04);}
 activity(active:boolean,underwater=false):void{this.playing=active;if(active&&this.context&&this.context.currentTime>=this.nextBeat){this.nextBeat=this.context.currentTime+.19;this.music();}if(this.filter&&this.context)this.filter.frequency.setTargetAtTime(underwater?1600:12000,this.context.currentTime,.35);}
 motor(speed:number):void{if(!this.context||!this.engine||!this.engineGain)return;this.engine.frequency.setTargetAtTime(45+Math.abs(speed)*4.5,this.context.currentTime,.08);this.engineGain.gain.setTargetAtTime(speed===0?0:.12,this.context.currentTime,.08);}
 private tone(frequency:number,end:number,duration:number,volume:number,type:OscillatorType='sine',pan=0,delay=0):void{
  const c=this.context;if(!c||!this.master||c.state!=='running')return;
  const osc=c.createOscillator(),gain=c.createGain(),stereo=c.createStereoPanner(),at=c.currentTime+delay;stereo.pan.value=pan;osc.type=type;
  osc.frequency.setValueAtTime(frequency,at);osc.frequency.exponentialRampToValueAtTime(Math.max(20,end),at+duration);
  gain.gain.setValueAtTime(.001,at);gain.gain.linearRampToValueAtTime(volume,at+.006);gain.gain.exponentialRampToValueAtTime(.001,at+duration);
  osc.connect(gain);gain.connect(stereo);stereo.connect(this.master);osc.start(at);osc.stop(at+duration+.01);osc.onended=()=>{osc.disconnect();gain.disconnect();stereo.disconnect();};
 }
 private burst(duration:number,volume:number,frequency:number,pan=0):void{
  const c=this.context;if(!c||!this.master||!this.noise||c.state!=='running')return;
  const source=c.createBufferSource(),filter=c.createBiquadFilter(),gain=c.createGain(),stereo=c.createStereoPanner();source.buffer=this.noise;
  filter.type='bandpass';filter.frequency.value=frequency;filter.Q.value=.7;stereo.pan.value=pan;
  gain.gain.setValueAtTime(volume,c.currentTime);gain.gain.exponentialRampToValueAtTime(.001,c.currentTime+duration);
  source.connect(filter);filter.connect(gain);gain.connect(stereo);stereo.connect(this.master);source.start();source.stop(c.currentTime+duration);source.onended=()=>{source.disconnect();filter.disconnect();gain.disconnect();stereo.disconnect();};
 }
 private music():void{
  if(!this.playing||this.muted)return;
  const n=this.beat++;
  if(this.kind==='kart'){
   const notes=[329.63,392,493.88,587.33,493.88,392,440,392,293.66,369.99,440,523.25,440,369.99,392,293.66];
   if(n%2===0)this.tone(notes[Math.floor(n/2)%notes.length],notes[Math.floor(n/2)%notes.length],.23,.055,'triangle',n%4===0?-.25:.25);
   if(n%4===0)this.tone(n%32<16?82.4:73.4,65,.18,.12,'sine');
   if(n%2===1)this.burst(.045,.035,7000);
  }else if(n%16===0){this.tone(n%64<32?65.4:73.4,65.4,2.5,.025,'triangle',-.2);}
 }
 effect(kind:string,pan=0,weapon='rifle'):void{
  pan=Math.max(-1,Math.min(1,pan));
  if(kind==='shot'){
   const rail=weapon==='rail',scatter=weapon==='scatter';this.burst(scatter?.23:.10,scatter?.6:.32,rail?4000:1800,pan);this.tone(rail?1200:scatter?110:180,rail?220:35,rail?.3:.14,scatter?.42:.24,rail?'sine':'triangle',pan);return;
  }
  if(kind==='hit'){this.burst(.075,.25,2300,pan);this.tone(760,540,.08,.13,'sine',pan);return;}
  if(kind==='land'){this.burst(.15,.18,250,pan);return;}
  if(kind==='launch'){this.tone(180,700,.35,.16,'sine',pan);return;}
  if(kind==='dash'||kind==='boost'){this.burst(.3,.2,900,pan);this.tone(110,540,.32,.15,'sawtooth',pan);return;}
  if(kind==='countdown'){this.tone(440,440,.14,.2);return;}
  if(kind==='go'||kind==='finish'||kind==='lap'||kind==='trick'||kind==='kill'){
   [523.25,659.25,783.99,1046.5].forEach((f,i)=>this.tone(f,f,.25,.13,'triangle',pan,i*.085));return;
  }
  this.tone(kind==='shield'?330:660,990,.18,.15,'sine',pan);
 }
 dispose():void{this.engine?.stop();this.engine?.disconnect();this.engineGain?.disconnect();this.master?.disconnect();this.filter?.disconnect();void this.context?.close();this.context=null;}
}
