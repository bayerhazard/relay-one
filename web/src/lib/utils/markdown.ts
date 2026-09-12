import DOMPurify from "dompurify";

/**
 * Kleiner Markdown-Subset-Renderer für Meeting-Summaries (Insilo-Export).
 *
 * Insilo erzeugt GFM mit: Überschriften (#/##/###), **fett**, *kursiv*,
 * `code`, Listen (- / * / 1.), GFM-Checklisten (- [ ] / - [x]), Links
 * [t](https://…). Genau das Subset wird hier unterstützt — nichts mehr.
 * Der Ausgang geht IMMER durch DOMPurify (Inhalt kommt aus einer fremden
 * App und wird via {@html} gerendert).
 */

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/** Inline-Formatierung auf einer (geschützten) Zeile. */
function inline(text: string): string {
  let s = escapeHtml(text);
  // Code zuerst: der Inhalt bleibt von den übrigen Regeln unberührt.
  s = s.replace(/`([^`]+)`/g, "<code>$1</code>");
  // Fett (**…**) vor Kursiv (*…*).
  s = s.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  s = s.replace(/(^|[^*])\*([^*\n]+)\*/g, "$1<em>$2</em>");
  // Links: nur http/https, neuem Tab, ohne Referrer.
  s = s.replace(
    /\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g,
    '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>',
  );
  return s;
}

export function renderMarkdown(md: string): string {
  if (!md) return "";
  const lines = md.replace(/\r\n/g, "\n").split("\n");
  const out: string[] = [];
  let inUl = false;
  let inOl = false;
  let para: string[] = [];

  const flushPara = (): void => {
    if (para.length) {
      out.push(`<p>${inline(para.join(" "))}</p>`);
      para = [];
    }
  };
  const closeLists = (): void => {
    if (inUl) {
      out.push("</ul>");
      inUl = false;
    }
    if (inOl) {
      out.push("</ol>");
      inOl = false;
    }
  };

  for (const line of lines) {
    const heading = line.match(/^(#{1,6})\s+(.*)$/);
    if (heading) {
      flushPara();
      closeLists();
      const level = Math.min(heading[1].length, 4);
      out.push(`<h${level}>${inline(heading[2])}</h${level}>`);
      continue;
    }

    // GFM-Checkliste: - [ ] / - [x] (readonly, Insilo ist die Quelle).
    const check = line.match(/^\s*[-*]\s+\[( |x|X)\]\s+(.*)$/);
    if (check) {
      flushPara();
      if (inOl) {
        out.push("</ol>");
        inOl = false;
      }
      if (!inUl) {
        out.push('<ul class="md-checklist">');
        inUl = true;
      }
      const done = check[1].toLowerCase() === "x";
      out.push(
        `<li class="md-task${done ? " done" : ""}">` +
          `<input type="checkbox" disabled${done ? " checked" : ""}/>` +
          `<span>${inline(check[2])}</span></li>`,
      );
      continue;
    }

    const ul = line.match(/^\s*[-*]\s+(.*)$/);
    if (ul) {
      flushPara();
      if (inOl) {
        out.push("</ol>");
        inOl = false;
      }
      if (!inUl) {
        out.push("<ul>");
        inUl = true;
      }
      out.push(`<li>${inline(ul[1])}</li>`);
      continue;
    }

    const ol = line.match(/^\s*\d+[.)]\s+(.*)$/);
    if (ol) {
      flushPara();
      if (inUl) {
        out.push("</ul>");
        inUl = false;
      }
      if (!inOl) {
        out.push("<ol>");
        inOl = true;
      }
      out.push(`<li>${inline(ol[1])}</li>`);
      continue;
    }

    if (line.trim() === "") {
      flushPara();
      closeLists();
      continue;
    }

    para.push(line.trim());
  }
  flushPara();
  closeLists();

  return DOMPurify.sanitize(out.join("\n"));
}
