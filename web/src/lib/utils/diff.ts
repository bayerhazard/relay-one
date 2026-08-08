export function computeDiff(
  original: string,
  modified: string
): { type: "unchanged" | "added" | "removed"; content: string }[] {
  const origLines = original.split("\n");
  const modLines = modified.split("\n");
  const result: { type: "unchanged" | "added" | "removed"; content: string }[] = [];
  const maxLen = Math.max(origLines.length, modLines.length);
  let origIdx = 0;
  let modIdx = 0;

  while (origIdx < maxLen || modIdx < maxLen) {
    const ol = origLines[origIdx];
    const ml = modLines[modIdx];

    if (ol === ml) {
      if (ol !== undefined) result.push({ type: "unchanged", content: ol });
      origIdx++;
      modIdx++;
    } else if (ol === undefined) {
      result.push({ type: "added", content: ml ?? "" });
      modIdx++;
    } else if (ml === undefined) {
      result.push({ type: "removed", content: ol });
      origIdx++;
    } else {
      if (origIdx + 1 < origLines.length && origLines[origIdx + 1] === ml) {
        result.push({ type: "removed", content: ol });
        origIdx++;
      } else if (modIdx + 1 < modLines.length && modLines[modIdx + 1] === ol) {
        result.push({ type: "added", content: ml });
        modIdx++;
      } else {
        result.push({ type: "removed", content: ol });
        result.push({ type: "added", content: ml });
        origIdx++;
        modIdx++;
      }
    }
  }

  return result;
}
