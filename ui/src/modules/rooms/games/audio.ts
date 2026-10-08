/** Gesture-started, synthesised effects: no remote audio or autoplay. */
export class GameAudio {
  private context: AudioContext | null = null;
  private master: GainNode | null = null;
  private engine: OscillatorNode | null = null;
  private engineGain: GainNode | null = null;
  muted = false;
  async start(): Promise<void> {
    if (!this.context) {
      this.context = new AudioContext(); this.master = this.context.createGain(); this.master.gain.value = this.muted ? 0 : .18; this.master.connect(this.context.destination);
      this.engine = this.context.createOscillator(); this.engine.type = 'sawtooth'; this.engineGain = this.context.createGain(); this.engineGain.gain.value = 0;
      const filter = this.context.createBiquadFilter(); filter.type = 'lowpass'; filter.frequency.value = 250;
      this.engine.connect(filter); filter.connect(this.engineGain); this.engineGain.connect(this.master); this.engine.start();
    }
    await this.context.resume();
  }
  mute(value:boolean):void { this.muted=value; if(this.master&&this.context)this.master.gain.setTargetAtTime(value?0:.18,this.context.currentTime,.04); }
  motor(speed:number):void {if(!this.context||!this.engine||!this.engineGain)return;this.engine.frequency.setTargetAtTime(35+Math.abs(speed)*3,this.context.currentTime,.1);this.engineGain.gain.setTargetAtTime(speed===0?0:.035,this.context.currentTime,.1);}
  effect(kind:string,pan=0):void {
    const c=this.context;if(!c||!this.master||c.state!=='running')return;
    const osc=c.createOscillator(),gain=c.createGain(),stereo=c.createStereoPanner();stereo.pan.value=Math.max(-1,Math.min(1,pan));
    osc.type=kind==='shot'?'square':'sine';const freq=kind==='shot'?170:kind==='hit'?680:kind==='finish'?880:kind==='boost'?350:530;
    osc.frequency.setValueAtTime(freq,c.currentTime);osc.frequency.exponentialRampToValueAtTime(kind==='shot'?40:freq*1.6,c.currentTime+.13);
    gain.gain.setValueAtTime(kind==='shot'?.12:.24,c.currentTime);gain.gain.exponentialRampToValueAtTime(.001,c.currentTime+.18);
    osc.connect(gain);gain.connect(stereo);stereo.connect(this.master);osc.start();osc.stop(c.currentTime+.2);osc.onended=()=>{osc.disconnect();gain.disconnect();stereo.disconnect();};
  }
  dispose():void {this.engine?.stop();this.engine?.disconnect();this.engineGain?.disconnect();this.master?.disconnect();void this.context?.close();this.context=null;}
}
