import { describe, it, expect } from "vitest";
import { translations } from "$lib/i18n";

// Guard for the CI wording rules (aimighty-ci ABGLEICH RL-R3, RL-R4):
// German copy uses "Sie", the abbreviation is "AI" (never "KI"), and no
// model or runtime names appear in UI text. Names belong only in input
// placeholders, which live in the components, not here.

const DU = /\b(du|dich|dir|dein|deine|deinen|deinem|deiner|deines|Du|Dich|Dir|Dein|Deine|Deinen|Deinem|Deiner|Deines)\b/;
const KI = /\bKI\b|\bKI-/;
const NAMES = /\b(ollama|litellm|vllm|whisper|systran|llama|qwen|speaches|circuit[ -]?breaker)\b/i;

function offenders(lang: "de" | "en", re: RegExp): string[] {
  return Object.entries(translations[lang])
    .filter(([, text]) => re.test(text))
    .map(([key, text]) => `${key}: ${text}`);
}

describe("wording (CI RL-R3, RL-R4)", () => {
  it("German copy addresses the user as Sie", () => {
    expect(offenders("de", DU)).toEqual([]);
  });

  it('German copy says "AI", not "KI"', () => {
    expect(offenders("de", KI)).toEqual([]);
  });

  it("no model or component names in any language", () => {
    expect([...offenders("de", NAMES), ...offenders("en", NAMES)]).toEqual([]);
  });
});
