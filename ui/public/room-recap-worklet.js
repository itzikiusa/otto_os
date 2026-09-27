/* Runs only after host capture consent. Owns copied PCM, never microphone tracks. */
class RoomRecapProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.buffer = new Float32Array(Math.floor(sampleRate * 30));
    this.position = 0;
    this.port.onmessage = event => { if (event.data === 'flush') { this.flush(); this.port.postMessage({flushed: true}); } };
  }
  flush() {
    if (!this.position) return;
    const samples = this.buffer.slice(0, this.position);
    this.port.postMessage({samples, sampleRate}, [samples.buffer]);
    this.position = 0;
  }
  process(inputs) {
    const channels = inputs[0];
    if (!channels?.length) return true;
    for (let frame = 0; frame < channels[0].length; frame++) {
      let mono = 0;
      for (const channel of channels) mono += channel[frame] || 0;
      this.buffer[this.position++] = mono / channels.length;
      if (this.position === this.buffer.length) this.flush();
    }
    return true;
  }
}
registerProcessor('room-recap-pcm', RoomRecapProcessor);
