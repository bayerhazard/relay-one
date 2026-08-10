// WAV conversion for Voice-to-Mail.
//
// MediaRecorder produces browser-specific containers (WebM/Opus in Chrome,
// MP4 in Safari) regardless of the type label we set on the Blob. Whisper /
// vLLM audio endpoints expect real PCM WAV, so we decode the recorded audio
// with the Web Audio API and re-encode it as 16 kHz / 16-bit / mono WAV.

/** Decode any audio Blob (webm/mp4/wav) into 16 kHz mono PCM WAV. */
export async function blobToWavBase64(blob: Blob): Promise<string> {
  const arrayBuffer = await blob.arrayBuffer();
  const AudioCtx = window.AudioContext ?? (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
  const ctx = new AudioCtx();
  try {
    const audioBuffer = await ctx.decodeAudioData(arrayBuffer);
    const wav = encodeWav(audioBuffer);
    return arrayBufferToBase64(wav);
  } finally {
    void ctx.close();
  }
}

/** Encode an AudioBuffer as 16 kHz / 16-bit / mono WAV (downmixed). */
export function encodeWav(buffer: AudioBuffer): ArrayBuffer {
  const targetRate = 16000;
  const input = buffer.getChannelData(0);
  // Resample to 16 kHz with linear interpolation.
  const ratio = buffer.sampleRate / targetRate;
  const outLen = Math.max(1, Math.floor(input.length / ratio));
  const samples = new Float32Array(outLen);
  for (let i = 0; i < outLen; i++) {
    const pos = i * ratio;
    const i0 = Math.floor(pos);
    const i1 = Math.min(i0 + 1, input.length - 1);
    const frac = pos - i0;
    samples[i] = input[i0] * (1 - frac) + input[i1] * frac;
  }

  const bytesPerSample = 2; // 16-bit PCM
  const dataSize = outLen * bytesPerSample;
  const bufferOut = new ArrayBuffer(44 + dataSize);
  const view = new DataView(bufferOut);

  // RIFF header
  writeAscii(view, 0, "RIFF");
  view.setUint32(4, 36 + dataSize, true);
  writeAscii(view, 8, "WAVE");
  // fmt chunk
  writeAscii(view, 12, "fmt ");
  view.setUint32(16, 16, true);          // fmt chunk size
  view.setUint16(20, 1, true);           // PCM
  view.setUint16(22, 1, true);           // mono
  view.setUint32(24, targetRate, true);  // sample rate
  view.setUint32(28, targetRate * bytesPerSample, true); // byte rate
  view.setUint16(32, bytesPerSample, true); // block align
  view.setUint16(34, 16, true);          // bits per sample
  // data chunk
  writeAscii(view, 36, "data");
  view.setUint32(40, dataSize, true);

  let offset = 44;
  for (let i = 0; i < outLen; i++) {
    const s = Math.max(-1, Math.min(1, samples[i]));
    const int = s < 0 ? s * 0x8000 : s * 0x7fff;
    view.setInt16(offset, int, true);
    offset += 2;
  }
  return bufferOut;
}

function writeAscii(view: DataView, offset: number, text: string) {
  for (let i = 0; i < text.length; i++) {
    view.setUint8(offset + i, text.charCodeAt(i));
  }
}

export function arrayBufferToBase64(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}
