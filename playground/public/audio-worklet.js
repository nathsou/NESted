// A bounded, allocation-free playback ring. Audio is produced by the NES APU.
class NesAudio extends AudioWorkletProcessor {
  constructor() {
    super(); this.buffer = new Float32Array(32768); this.read = 0; this.write = 0; this.last = 0;
    this.port.onmessage = ({data}) => {
      if (data.reset) { this.read = this.write = 0; this.last = 0; return; }
      const samples = data.samples;
      if (!samples) return;
      if (this.write - this.read > 8192) this.read = this.write - 2048;
      for (let i = 0; i < samples.length; i++) this.buffer[(this.write++) & 32767] = samples[i];
    };
  }
  process(_inputs, outputs) {
    const out = outputs[0][0];
    for (let i = 0; i < out.length; i++) {
      if (this.read < this.write) this.last = this.buffer[(this.read++) & 32767];
      else this.last *= 0.995;
      out[i] = this.last * 0.65;
    }
    for (let channel = 1; channel < outputs[0].length; channel++) outputs[0][channel].set(out);
    return true;
  }
}
registerProcessor('nes-audio', NesAudio);
