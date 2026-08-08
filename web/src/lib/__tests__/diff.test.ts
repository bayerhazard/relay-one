import { describe, it, expect } from "vitest";
import { computeDiff } from "$lib/utils/diff";

describe("computeDiff", () => {
  it("returns unchanged for identical text", () => {
    const result = computeDiff("hello\nworld", "hello\nworld");
    expect(result).toEqual([
      { type: "unchanged", content: "hello" },
      { type: "unchanged", content: "world" },
    ]);
  });

  it("detects added lines", () => {
    const result = computeDiff("hello", "hello\nworld");
    expect(result).toEqual([
      { type: "unchanged", content: "hello" },
      { type: "added", content: "world" },
    ]);
  });

  it("detects removed lines", () => {
    const result = computeDiff("hello\nworld", "hello");
    expect(result).toEqual([
      { type: "unchanged", content: "hello" },
      { type: "removed", content: "world" },
    ]);
  });

  it("detects modified lines", () => {
    const result = computeDiff("hello", "world");
    expect(result).toEqual([
      { type: "removed", content: "hello" },
      { type: "added", content: "world" },
    ]);
  });

  it("handles empty strings", () => {
    const result = computeDiff("", "");
    // "".split("\n") = [""], so both sides have one empty line
    expect(result).toEqual([{ type: "unchanged", content: "" }]);
  });

  it("handles empty to non-empty", () => {
    const result = computeDiff("", "hello");
    expect(result).toEqual([
      { type: "removed", content: "" },
      { type: "added", content: "hello" },
    ]);
  });

  it("handles non-empty to empty", () => {
    const result = computeDiff("hello", "");
    expect(result).toEqual([
      { type: "removed", content: "hello" },
      { type: "added", content: "" },
    ]);
  });
});
