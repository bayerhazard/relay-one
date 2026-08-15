import { describe, it, expect, vi, afterEach } from "vitest";
import { extractEmail, extractName, formatDate, isHtmlContent, _resetNowCache } from "$lib/utils/format";

// ---------------------------------------------------------------------------
// extractEmail
// ---------------------------------------------------------------------------
describe("extractEmail", () => {
  it("extracts email from angle brackets", () => {
    expect(extractEmail("Max Müller <max@example.com>")).toBe("max@example.com");
  });

  it("returns original string when no brackets", () => {
    expect(extractEmail("max@example.com")).toBe("max@example.com");
  });

  it("handles empty string", () => {
    expect(extractEmail("")).toBe("");
  });

  it("handles name with special chars", () => {
    expect(extractEmail("Test User (Admin) <test@domain.org>")).toBe("test@domain.org");
  });

  it("extracts first email when multiple angle brackets present", () => {
    expect(extractEmail("User <first@example.com> <second@example.com>")).toBe("first@example.com");
  });

  it("returns original string when angle brackets contain no content", () => {
    // regex requires at least one char inside <>, so no match → returns original
    expect(extractEmail("User <>")).toBe("User <>");
  });

  it("returns original string when only angle brackets given", () => {
    expect(extractEmail("<>")).toBe("<>");
  });

  it("handles email without @ symbol inside brackets", () => {
    expect(extractEmail("User <not-an-email>")).toBe("not-an-email");
  });

  it("handles nested angle brackets (returns first match)", () => {
    // regex matches first < to first >, so <<inner@test.com>> → <inner@test.com>
    expect(extractEmail("User <<inner@test.com>>")).toBe("<inner@test.com");
  });

  it("handles whitespace around email in brackets", () => {
    expect(extractEmail("User <  spaced@test.com  >")).toBe("  spaced@test.com  ");
  });

  it("handles email with plus addressing", () => {
    expect(extractEmail("User <user+tag@example.com>")).toBe("user+tag@example.com");
  });

  it("handles email with subdomain", () => {
    expect(extractEmail("User <user@sub.example.com>")).toBe("user@sub.example.com");
  });
});

// ---------------------------------------------------------------------------
// extractName
// ---------------------------------------------------------------------------
describe("extractName", () => {
  it("extracts name from standard format", () => {
    expect(extractName("Max Müller <max@example.com>")).toBe("Max Müller");
  });

  it("returns email when no name present", () => {
    expect(extractName("max@example.com")).toBe("max@example.com");
  });

  it("handles empty string", () => {
    expect(extractName("")).toBe("");
  });

  it("handles name with special characters", () => {
    expect(extractName("Test User (Admin) <test@domain.org>")).toBe("Test User (Admin)");
  });

  it("handles name with dots and hyphens", () => {
    expect(extractName("Dr. Jean-Claude <jean@example.com>")).toBe("Dr. Jean-Claude");
  });

  it("handles unicode names (Chinese)", () => {
    expect(extractName("张三 <zhang@example.com>")).toBe("张三");
  });

  it("handles unicode names (Japanese)", () => {
    expect(extractName("名前 <name@example.com>")).toBe("名前");
  });

  it("handles unicode names (Arabic)", () => {
    expect(extractName("أحمد <ahmed@example.com>")).toBe("أحمد");
  });

  it("trims whitespace around name", () => {
    expect(extractName("  John Doe  <john@example.com>")).toBe("John Doe");
  });

  it("handles name without space before angle bracket", () => {
    expect(extractName("John<john@example.com>")).toBe("John");
  });

  it("handles only angle brackets (no name)", () => {
    expect(extractName("<email@domain.com>")).toBe("<email@domain.com>");
  });

  it("handles name with apostrophe", () => {
    expect(extractName("O'Brien <obrien@example.com>")).toBe("O'Brien");
  });

  it("handles quoted name", () => {
    expect(extractName('"John Doe" <john@example.com>')).toBe('"John Doe"');
  });
});

// ---------------------------------------------------------------------------
// isHtmlContent
// ---------------------------------------------------------------------------
describe("isHtmlContent", () => {
  it("detects real HTML markup", () => {
    expect(isHtmlContent("<!doctype html><html><body><p>Hallo</p></body></html>")).toBe(true);
    expect(isHtmlContent("<div>Zeile</div>")).toBe(true);
    expect(isHtmlContent("<table><tr><td>x</td></tr></table>")).toBe(true);
    expect(isHtmlContent('<a href="https://x.de">Link</a>')).toBe(true);
    expect(isHtmlContent("<br/>")).toBe(true);
  });

  it("rejects plain text (even multi-line)", () => {
    expect(isHtmlContent("Diese Email wurde maschinell erstellt.\n\nHallo Marc Bayer,\n\nwir haben Ihren Änderungsantrag erhalten.")).toBe(false);
    expect(isHtmlContent("Guten Morgen Herr Bayer,\r\n\r\nwir haben Ihre Maschine per UPS erhalten.")).toBe(false);
    expect(isHtmlContent("Rechnung Nr. 1234")).toBe(false);
  });

  it("rejects empty / null", () => {
    expect(isHtmlContent("")).toBe(false);
    expect(isHtmlContent(null)).toBe(false);
    expect(isHtmlContent(undefined)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// formatDate
// ---------------------------------------------------------------------------
describe("formatDate", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('returns time string for today', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-06-15T12:00:00.000Z"));
    const result = formatDate("2024-06-15T10:00:00.000Z");
    // jsdom may return "10:00" or "10:00 AM" depending on locale support
    expect(result).toMatch(/^\d{1,2}:\d{2}(?:\s*[AP]M)?$/);
  });

  it('returns "Gestern" for yesterday', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-06-15T12:00:00.000Z"));
    expect(formatDate("2024-06-14T10:00:00.000Z")).toBe("Gestern");
  });

  it('returns weekday short for this week (2-6 days ago)', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-06-15T12:00:00.000Z"));
    const result = formatDate("2024-06-12T10:00:00.000Z");
    // Should be a short weekday like "Mi." or "Wed" depending on locale
    expect(result).toBeTruthy();
    expect(result.length).toBeLessThanOrEqual(5);
  });

  it('returns formatted date for 7+ days ago', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-06-15T12:00:00.000Z"));
    const result = formatDate("2024-06-07T10:00:00.000Z");
    // jsdom may return "07.06.2024" (de-DE) or "06/07/2024" (en-US fallback)
    expect(result).toMatch(/^\d{2}[./]\d{2}[./]\d{4}$/);
  });

  it('returns empty string for null', () => {
    expect(formatDate(null as unknown as string)).toBe("");
  });

  it('returns empty string for undefined', () => {
    expect(formatDate(undefined)).toBe("");
  });

  it('returns "Invalid Date" for unparseable string', () => {
    // new Date("not-a-date") creates an Invalid Date without throwing,
    // and toLocaleString on Invalid Date returns "Invalid Date"
    expect(formatDate("not-a-date")).toBe("Invalid Date");
  });

  it('returns "Invalid Date" for garbage input', () => {
    expect(formatDate("garbage!!!")).toBe("Invalid Date");
  });

  it('handles exactly 1 day difference as yesterday', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-06-15T10:00:00.000Z"));
    _resetNowCache();
    expect(formatDate("2024-06-14T10:00:00.000Z")).toBe("Gestern");
    vi.useRealTimers();
  });

  it('handles exactly 7 days difference as older (not this week)', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-06-15T10:00:00.000Z"));
    _resetNowCache();
    const result = formatDate("2024-06-08T10:00:00.000Z");
    expect(result).toMatch(/^\d{2}[./]\d{2}[./]\d{4}$/);
    vi.useRealTimers();
  });

  it('handles future dates (negative diff)', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-06-15T12:00:00.000Z"));
    _resetNowCache();
    const result = formatDate("2024-06-20T10:00:00.000Z");
    expect(result).toBeTruthy();
    expect(result.length).toBeLessThanOrEqual(5);
    vi.useRealTimers();
  });

  it('handles ISO date string without time (within this week)', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-06-15T12:00:00.000Z"));
    _resetNowCache();
    const result = formatDate("2024-06-10");
    expect(result).toBeTruthy();
    expect(result.length).toBeLessThanOrEqual(5);
    vi.useRealTimers();
  });
});
