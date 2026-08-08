import { describe, it, expect, beforeEach, vi } from "vitest";
import { getVoiceSettings, saveVoiceSettings, voiceTranscribe } from "$lib/services/tauri";
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
        "whisper-1"
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
          }),
        })
      );
    });

    it("throws user-friendly error when backend fails", async () => {
      fetchMock.mockRejectedValue(new Error("DB connection failed"));

      await expect(
        saveVoiceSettings(true, "https://stt.example.com/v1", "sk-test-key", "whisper-1")
      ).rejects.toThrow("Die Voice-Einstellungen konnten nicht gespeichert werden.");
    });

    it("saves disabled state correctly", async () => {
      fetchMock.mockResolvedValue(jsonResponse(undefined));

      await saveVoiceSettings(false, "", "", "");

      expect(fetchMock).toHaveBeenCalledWith(
        "/api/v1/voice/config",
        expect.objectContaining({
          body: JSON.stringify({ enabled: false, sttUrl: "", sttKey: "", sttModel: "" }),
        })
      );
    });
  });

  describe("voiceTranscribe", () => {
    it("returns transcription text on success", async () => {
      const mockTranscription = "Hallo, ich möchte einen Termin vereinbaren.";
      fetchMock.mockResolvedValue(jsonResponse(mockTranscription));

      const result = await voiceTranscribe("base64-audio-data");

      expect(result).toBe(mockTranscription);
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/v1/voice/transcribe",
        expect.objectContaining({
          body: JSON.stringify({ audioBase64: "base64-audio-data" }),
        })
      );
    });

    it("throws user-friendly error when transcription fails", async () => {
      fetchMock.mockRejectedValue(new Error("STT service unavailable"));

      await expect(
        voiceTranscribe("base64-audio-data")
      ).rejects.toThrow("Die Transkription konnte nicht durchgeführt werden.");
    });

    it("handles empty audio data", async () => {
      fetchMock.mockResolvedValue(jsonResponse(""));

      const result = await voiceTranscribe("");

      expect(result).toBe("");
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
    };

    expect(settings).toHaveProperty("enabled");
    expect(settings).toHaveProperty("sttUrl");
    expect(settings).toHaveProperty("sttKey");
    expect(settings).toHaveProperty("sttModel");
  });

  it("matches backend camelCase response format", () => {
    const backendResponse = {
      enabled: true,
      sttUrl: "https://stt.example.com/v1",
      sttKey: "sk-test-key",
      sttModel: "whisper-1",
    };

    const settings: VoiceSettings = backendResponse;
    expect(settings.sttUrl).toBe("https://stt.example.com/v1");
    expect(settings.sttKey).toBe("sk-test-key");
    expect(settings.sttModel).toBe("whisper-1");
  });
});
