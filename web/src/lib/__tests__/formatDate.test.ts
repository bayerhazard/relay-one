import { describe, it, expect, vi } from "vitest";
import { formatDate, _resetNowCache } from "$lib/utils/format";

describe("formatDate", () => {
  it("returns empty string for undefined", () => {
    expect(formatDate(undefined)).toBe("");
  });

  it("returns empty string for empty string", () => {
    expect(formatDate("")).toBe("");
  });

  it("formats today as time", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-06-15T10:00:00.000Z"));
    _resetNowCache();
    const result = formatDate("2024-06-15T10:00:00.000Z");
    expect(result).toMatch(/^\d{1,2}:\d{2}(?:\s*[AP]M)?$/);
    vi.useRealTimers();
  });

  it("formats yesterday as 'Gestern'", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-06-15T12:00:00.000Z"));
    _resetNowCache();
    expect(formatDate("2024-06-14T12:00:00.000Z")).toBe("Gestern");
    vi.useRealTimers();
  });

  it("formats within 7 days as weekday", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-06-15T12:00:00.000Z"));
    _resetNowCache();
    const result = formatDate("2024-06-12T12:00:00.000Z");
    expect(result).toMatch(/^[A-ZÄÖÜ][a-zäöü]{1,2}$/);
    vi.useRealTimers();
  });

  it("formats older dates as full date", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-06-15T12:00:00.000Z"));
    _resetNowCache();
    const result = formatDate("2024-06-05T12:00:00.000Z");
    expect(result).toMatch(/^\d{2}[./]\d{2}[./]\d{4}$/);
    vi.useRealTimers();
  });

  it("handles invalid date gracefully", () => {
    expect(formatDate("not-a-date")).toBe("Invalid Date");
  });
});
