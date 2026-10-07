/** Two letters from a name for the profile circle (CI HB-KONTO):
 * "Kai Böhm" → "KB", "kaivostudio" → "KA", nothing → "…". */
export function initialen(text: string): string {
  const teile = text.trim().split(/[\s._@-]+/).filter(Boolean);
  if (teile.length === 0) return "…";
  if (teile.length === 1) return teile[0].slice(0, 2).toUpperCase();
  return (teile[0][0] + teile[teile.length - 1][0]).toUpperCase();
}

/** The Olares user in an app address: "31747cb8.kaivostudio.olares.de" →
 * "kaivostudio". The last fallback for the profile circle. */
export function olaresNutzerAusHost(host: string): string {
  const m = host.toLowerCase().match(/^[^.]+\.([^.]+)\.olares(cn)?\.[a-z]+$/);
  return m ? m[1] : "";
}
