import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { compile } from "svelte/compiler";

// Dynamic contenteditable children (ul/li/h3 from execCommand, pasted code/a)
// carry no Svelte scope class. Plain `.editor ul` selectors are dropped by the
// compiler as css_unused_selector — the rules must use :global() so bullets
// keep their indent instead of being drawn into the editor padding.
describe("ComposeWindow editor CSS scoping", () => {
  const compiled = compile(readFileSync("src/lib/components/ComposeWindow.svelte", "utf8"), {
    filename: "ComposeWindow.svelte",
    generate: "client",
  });
  const cssCode = compiled.css ? compiled.css.code : "";

  it("has no unused-selector warnings for .editor rules", () => {
    const unused = compiled.warnings.filter(
      (w) => w.code === "css_unused_selector" && /\.editor/.test(w.message || "")
    );
    expect(unused.map((w) => w.message)).toEqual([]);
  });

  it("ships compiled rules for lists, headings, code and links", () => {
    for (const sel of ["ul", "ol", "li", "h3", "code", "a"]) {
      expect(cssCode).toMatch(new RegExp(`\\.editor\\.svelte-[a-z0-9]+\\s+${sel}\\b`));
    }
    const ulRule = cssCode.match(/\.editor\.svelte-[a-z0-9]+ ul[^}]*}/s)?.[0] ?? "";
    expect(ulRule).toContain("padding-left: 1em");
  });
});
