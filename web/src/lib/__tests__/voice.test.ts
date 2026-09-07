import { describe, it, expect, beforeEach, vi } from "vitest";
import { getVoiceSettings, saveVoiceSettings, voiceTranscribe, voiceSpeak } from "$lib/services/tauri";
import type { VoiceSettings } from "$lib/services/tauri";

const fetchMock = vi.hoisted(() => vi.fn());

function jsonResponse(body: unknown, ok = true) {
  return {
    ok,
    status: ok ? 200 : 500,
    json: async () => body,
  };
}

describe("Voice Settings", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    (globalThis as any).fetch = fetchMock;
  });

  describe("getVoiceSettings", () => {
    it("returns VoiceSettings when backend succeeds", async () => {
      const expected: VoiceSettings = {
        enabled: true,
        sttUrl: "https://stt.example.com/v1",
        sttKey: "sk-test-key",
        sttModel: "whisper-1",
        ttsEnabled: true,
        ttsUrl: "https://tts.example.com/v1",
        ttsKey: "sk-tts-key",
        ttsModel: "tts-1",
        ttsAuto: false,
      };
      fetchMock.mockResolvedValue(jsonResponse(expected));

      const result = await getVoiceSettings();

      expect(result).toEqual(expected);
      expect(fetchMock).toHaveBeenCalledWith("/api/v1/voice/config", expect.anything());
    });

    it("returns null when backend fails", async () => {
      fetchMock.mockRejectedValue(new Error("Backend unavailable"));

      const result = await getVoiceSettings();

      expect(result).toBeNull();
    });

    it("returns null when backend returns null", async () => {
      fetchMock.mockResolvedValue(jsonResponse(null));

      const result = await getVoiceSettings();

      expect(result).toBeNull();
    });

    it("handles partial settings gracefully", async () => {
      const partial: Partial<VoiceSettings> = {
        enabled: false,
        sttUrl: "",
        sttKey: "",
        sttModel: "",
      };
      fetchMock.mockResolvedValue(jsonResponse(partial));

      const result = await getVoiceSettings();

      expect(result).toEqual(partial);
    });
  });

  describe("saveVoiceSettings", () => {
    it("saves settings with correct parameter mapping", async () => {
      fetchMock.mockResolvedValue(jsonResponse(undefined));

      await saveVoiceSettings(
        true,
        "https://stt.example.com/v1",
        "sk-test-key",
        "whisper-1",
        true,
        "https://tts.example.com/v1",
        "sk-tts-key",
        "tts-1",
        true
      );

      expect(fetchMock).toHaveBeenCalledWith(
        "/api/v1/voice/config",
        expect.objectContaining({
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            enabled: true,
            sttUrl: "https://stt.example.com/v1",
            sttKey: "sk-test-key",
            sttModel: "whisper-1",
            ttsEnabled: true,
            ttsUrl: "https://tts.example.com/v1",
            ttsKey: "sk-tts-key",
            ttsModel: "tts-1",
            ttsAuto: true,
          }),
        })
      );
    });

    it("throws user-friendly error when backend fails", async () => {
      fetchMock.mockRejectedValue(new Error("DB connection failed"));

      await expect(
        saveVoiceSettings(true, "https://stt.example.com/v1", "sk-test-key", "whisper-1",
          false, "", "", "tts-1", false)
      ).rejects.toThrow("Die Voice-Einstellungen konnten nicht gespeichert werden.");
    });

    it("saves disabled state correctly", async () => {
      fetchMock.mockResolvedValue(jsonResponse(undefined));

      await saveVoiceSettings(false, "", "", "", false, "", "", "tts-1", false);

      expect(fetchMock).toHaveBeenCalledWith(
        "/api/v1/voice/config",
        expect.objectContaining({
          body: JSON.stringify({
            enabled: false, sttUrl: "", sttKey: "", sttModel: "",
            ttsEnabled: false, ttsUrl: "", ttsKey: "", ttsModel: "tts-1", ttsAuto: false,
          }),
        })
      );
    });
  });

  describe("voiceTranscribe", () => {
    it("returns transcription text on success", async () => {
      const mockTranscription = "Hallo, ich möchte einen Termin vereinbaren.";
      fetchMock.mockResolvedValue(jsonResponse({ text: mockTranscription }));

      const result = await voiceTranscribe("base64-audio-data");

      expect(result).toBe(mockTranscription);
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/v1/voice/transcribe",
        expect.objectContaining({
          body: JSON.stringify({ audioBase64: "base64-audio-data" }),
        })
      );
    });

    it("returns trimmed empty string when text is missing", async () => {
      fetchMock.mockResolvedValue(jsonResponse({}));

      const result = await voiceTranscribe("base64-audio-data");

      expect(result).toBe("");
    });

    it("throws user-friendly error when transcription fails", async () => {
      fetchMock.mockRejectedValue(new Error("STT service unavailable"));

      await expect(
        voiceTranscribe("base64-audio-data")
      ).rejects.toThrow("Die Transkription konnte nicht durchgeführt werden.");
    });

    it("handles empty audio data", async () => {
      fetchMock.mockResolvedValue(jsonResponse({ text: "" }));

      const result = await voiceTranscribe("");

      expect(result).toBe("");
    });
  });

  describe("voiceSpeak (Phase D TTS proxy)", () => {
    beforeEach(() => {
      class MockAudio {
        onended: (() => void) | null = null;
        addEventListener = vi.fn();
        pause = vi.fn();
        play = vi.fn().mockResolvedValue(undefined);
        constructor(public src: string) {}
      }
      (globalThis as any).Audio = MockAudio;
      (globalThis as any).URL.createObjectURL = vi.fn(() => "blob:mock");
      (globalThis as any).URL.revokeObjectURL = vi.fn();
    });

    it("plays audio and returns the Audio element on success", async () => {
      fetchMock.mockResolvedValue({
        ok: true,
        status: 200,
        blob: async () => new Blob(["audio-bytes"], { type: "audio/mpeg" }),
      });

      const audio = await voiceSpeak("Hallo", "de");

      expect(fetchMock).toHaveBeenCalledWith(
        "/api/v1/voice/speak",
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({ text: "Hallo", lang: "de" }),
        })
      );
      expect((audio as any).play).toHaveBeenCalled();
    });

    it("throws the server error message when TTS is not configured (409)", async () => {
      fetchMock.mockResolvedValue({
        ok: false,
        status: 409,
        json: async () => ({ error: "TTS ist nicht konfiguriert" }),
      });

      await expect(voiceSpeak("Hallo")).rejects.toThrow("TTS ist nicht konfiguriert");
    });
  });
});

describe("VoiceSettings interface", () => {
  it("has correct camelCase property names", () => {
    const settings: VoiceSettings = {
      enabled: true,
      sttUrl: "https://example.com/v1",
      sttKey: "sk-test",
      sttModel: "whisper-1",
      ttsEnabled: true,
      ttsUrl: "https://tts.example.com/v1",
      ttsKey: "sk-tts",
      ttsModel: "tts-1",
      ttsAuto: true,
    };

    expect(settings).toHaveProperty("enabled");
    expect(settings).toHaveProperty("sttUrl");
    expect(settings).toHaveProperty("sttKey");
    expect(settings).toHaveProperty("sttModel");
    expect(settings).toHaveProperty("ttsEnabled");
    expect(settings).toHaveProperty("ttsUrl");
    expect(settings).toHaveProperty("ttsKey");
    expect(settings).toHaveProperty("ttsModel");
    expect(settings).toHaveProperty("ttsAuto");
  });

  it("matches backend camelCase response format", () => {
    const backendResponse = {
      enabled: true,
      sttUrl: "https://stt.example.com/v1",
      sttKey: "sk-test-key",
      sttModel: "whisper-1",
      ttsEnabled: true,
      ttsUrl: "https://tts.example.com/v1",
      ttsKey: "sk-tts-key",
      ttsModel: "tts-1",
      ttsAuto: false,
    };

    const settings: VoiceSettings = backendResponse;
    expect(settings.sttUrl).toBe("https://stt.example.com/v1");
    expect(settings.sttKey).toBe("sk-test-key");
    expect(settings.sttModel).toBe("whisper-1");
    expect(settings.ttsUrl).toBe("https://tts.example.com/v1");
    expect(settings.ttsModel).toBe("tts-1");
  });
});
