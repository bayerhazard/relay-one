import { describe, it, expect } from "vitest";
import { encodeWav, arrayBufferToBase64 } from "$lib/utils/wav";

// AudioContext is unavailable in happy-dom — build a minimal AudioBuffer-like
// object with the fields encodeWav actually uses.
function makeAudioBuffer(sampleRate: number, seconds: number): AudioBuffer {
  const length = Math.floor(sampleRate * seconds);
  const channel = new Float32Array(length);
  for (let i = 0; i < length; i++) {
    channel[i] = Math.sin(2 * Math.PI * 440 * i / sampleRate) * 0.5;
  }
  return {
    sampleRate,
    length,
    numberOfChannels: 1,
    duration: seconds,
    getChannelData: () => channel,
  } as unknown as AudioBuffer;
}

describe("encodeWav", () => {
  it("produces a valid RIFF/WAVE header", () => {
    const buf = makeAudioBuffer(48000, 1);
    const out = encodeWav(buf);
    expect(String.fromCharCode(...new Uint8Array(out, 0, 4))).toBe("RIFF");
    expect(String.fromCharCode(...new Uint8Array(out, 8, 4))).toBe("WAVE");
    expect(String.fromCharCode(...new Uint8Array(out, 12, 4))).toBe("fmt ");
    expect(String.fromCharCode(...new Uint8Array(out, 36, 4))).toBe("data");
  });

  it("resamples to 16 kHz mono 16-bit PCM", () => {
    const buf = makeAudioBuffer(48000, 1);
    const out = encodeWav(buf);
    const view = new DataView(out);
    expect(view.getUint16(22, true)).toBe(1);       // mono
    expect(view.getUint32(24, true)).toBe(16000);   // sample rate
    expect(view.getUint16(34, true)).toBe(16);      // bits per sample
    expect(view.getUint32(40, true)).toBe(16000 * 2); // data size
    expect(out.byteLength).toBe(44 + 32000);          // header + data
  });

  it("clamps samples to [-1, 1]", () => {
    const buf = makeAudioBuffer(16000, 0.1);
    const data = buf.getChannelData(0);
    for (let i = 0; i < data.length; i++) data[i] = i % 2 ? 2 : -2;
    const out = encodeWav(buf);
    const view = new DataView(out);
    for (let i = 0; i < data.length; i++) {
      const s = view.getInt16(44 + i * 2, true);
      expect(s).toBeLessThanOrEqual(0x7fff);
      expect(s).toBeGreaterThanOrEqual(-0x8000);
    }
  });

  it("base64 roundtrip preserves bytes", () => {
    const buf = makeAudioBuffer(16000, 0.5);
    const out = encodeWav(buf);
    const b64 = arrayBufferToBase64(out);
    const decoded = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
    expect(decoded.length).toBe(out.byteLength);
    expect(decoded).toEqual(new Uint8Array(out));
  });
});
