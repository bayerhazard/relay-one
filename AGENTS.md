# Relay — AGENTS.md

Regeln für alle, die an Relay arbeiten: Marc und Kai, ihre Agenten (OpenCode,
Claude Code und andere). `CLAUDE.md` verweist hierher; gepflegt wird nur diese
Datei.

| | |
|---|---|
| **Produkt** | Relay — selbst gehostetes Webmail mit lokalem Archiv, für Olares |
| **Repository** | `bayerhazard/relay-one` — das eine Repo (der alte Fork `ska1walker/relay-one` wird nicht fortgeführt) |
| **Aufbau** | `server/` Rust (axum, `relay-server`) · `web/` SvelteKit 2 / Svelte 5, statisch gebaut · `chart/relay/` Helm-Chart mit `OlaresManifest.yaml` |
| **Designsystem** | AImighty-CI, `ska1walker/aimighty-ci` — Abschnitt Relay in `ABGLEICH.md` (`RL-…`) |

---

## Wie gearbeitet wird

1. **Zweige und PRs.** Jede Änderung auf einem eigenen Zweig (Claude:
   `claude/…`) mit PR nach `main`. Marc oder Kai prüfen vor
   dem Merge. Niemand schreibt direkt auf `main`.
2. **Release und Markt nur aus den Actions** (ABGLEICH RL-V2): Version,
   Tag, Release und Markteintrag kommen aus `release.yml` und `markt.yml`
   — keine Tags von Hand, kein direktes Schreiben in
   `bayerhazard/aimighty-market`. Agenten pushen nie Tags und schreiben nie
   in den Markt. Wie es geht: unten „Veröffentlichen“.
3. **Sprache:** Oberfläche und Doku auf Deutsch in der **Sie-Form**, „AI“
   statt „KI“, keine Modell- oder Bausteinnamen im Fließtext (RL-R3, RL-R4;
   `web/src/lib/__tests__/wording.test.ts` wacht). Code, Kommentare und
   Commits auf Englisch.
4. **Im Zweifel fragen** — Kai für das Designsystem, Marc für Relay.

## Designsystem: das CI ist die Quelle

**Werte, Zeichen und Bausteine kommen aus dem CI; Abweichung nur über
`ABGLEICH.md` dort.** Relay hält einen Stand `ci-YY.M.n` als Kopie unter
`web/ci/` (mit `stand.json`), geholt aus `web/` mit

```bash
node scripts/ci-holen.mjs --von <klon von aimighty-ci> --stand ci-YY.M.n
```

— **nie `main`, nie zur Bauzeit**. Nach jedem neuen Stand öffnet die Action
`stand` im CI hier einen PR „CI-Stand ci-…“ (Geheimnis `RELAY_TOKEN`, von
Marc). Dann gilt:

- **Token:** Der Block `[AM-TOKEN]` oben in `web/src/styles/global.css` ist
  wörtlich `web/ci/tokens/app.css`. Farben, Abstände, Radien, Schatten und
  Bewegung nur über `--am-*`, keine festen Hex-Werte in Komponenten.
- **Bausteine:** Jeder `AM-`/`HB-`-Abschnitt in `global.css` ist wörtlich
  `web/ci/bauteile/<KENNUNG>.css`. Komponenten nehmen ihre Klassen
  (`btn btn-primaer`, `dialog-schicht`, `feld`, `schalter`, `hinweis` …)
  statt eigener Kopien. Relays Eigenes trägt `RL-`, und jeder `<style>`-Block
  beginnt mit seiner Kennung: `/* ── Titel [RL-KENNUNG] ── */`.
- **Zeichen:** nur aus `web/ci/marke/icons/ui/`, über `<Symbol>`
  (`lib/components/Symbol.svelte`, erzeugt nach `lib/symbole.ts` mit
  `node scripts/symbole-erzeugen.mjs`). Größen 16/20/24/40, Strich 1,5 px.
  Kein Inline-`<svg>`, keine Emoji als Zeichen.
- **App-Icon:** `docs/icon/relay.svg`; alle PNG (Favicon, Apple-Touch,
  Manifest, Markt-Icon `icon.png`) erzeugt `web/scripts/app-symbole.mjs`.
- **Regeln im Kurzen:** eine Hauptaktion je Ansicht; Rot nur für Löschen mit
  Objekt im Wort („Kontakt löschen“) und höchstens einmal je Ansicht;
  Endgültiges nur mit Rückfrage (HB-DIALOG, Fokus auf „Abbrechen“), Mail in
  den Papierkorb ohne Rückfrage mit „Rückgängig“; nur Schwebendes trägt den
  Schatten; Dunkelmodus über die Klasse `dunkel` am `<html>`.

Was sich ändern soll, ändert sich **zuerst im CI** (PR dort, neuer Stand),
dann wird geholt — nie hier still angepasst. Die Wachen
`ci-stand.test.ts`, `kennungen.test.ts` und `symbole.test.ts` schlagen an,
wenn die Kopie oder Relay abweicht.

## Veröffentlichen

Ein Release ist ein PR, der die Version hebt. Nach dem Merge läuft alles
von selbst:

1. **Version `YY.M.n`** (Monat ohne führende Null, Zähler je Monat) an vier
   Stellen: `chart/relay/Chart.yaml` (`version`, `appVersion`) und
   `chart/relay/OlaresManifest.yaml` (`metadata.version`,
   `spec.versionName`). Oben in `upgradeDescription` eine Zeile
   `v<version>: …`. `values.yaml` bleibt ohne Tag — das Abbild folgt der
   Chart-Version.
2. **Notiz `chart/relay/markt/<version>.md`**: `# Titel`, darunter Englisch
   ab `v<version>: `, nach `## Deutsch` derselbe Text auf Deutsch (Sie-Form,
   „AI“). Die Beschreibung im Markt steht in
   `chart/relay/markt/beschreibung.{en,de}.md`.
3. **Prüfen**: `bash scripts/check-chart.sh`. Im PR laufen `release.yml`
   (Chart prüfen und packen) und `markt.yml` als Probe mit: Sie bauen den
   Markteintrag für die neueste Notiz gegen den aktuellen Markt und
   beweisen ihn mit wrangler, ohne etwas zu schreiben.
4. **Nach dem Merge** baut `release.yml` das Abbild
   `ghcr.io/bayerhazard/relay-one:<version>` (Geheimnis `GHCR_PAT`) und
   legt Tag und Release mit Chart und Icon an. Danach öffnet `markt.yml`
   den PR im Markt, mergt ihn, prüft `main` und wartet auf den Deploy
   (Geheimnis `MARKT_TOKEN` von Marc; ohne es meldet die Action nur).

Von Hand, unter „Run workflow“: `release` und `markt` mit „probe“ (Vorgabe)
prüfen nur. Ohne „probe“ baut `release` ein Abbild ohne Tag, `markt`
schreibt den Eintrag für eine Version, die schon ein Release hat.

## Prüfen vor jedem PR

Wie die CI des Repos (`.github/workflows/ci.yml`):

```bash
cd web && npm run check && npx vitest run && npm run build
cd server && cargo test --locked && cargo clippy --all-targets --no-deps
```

In Marcs Umgebung (4 GiB RAM) Vitest höchstens mit
`NODE_OPTIONS="--max-old-space-size=3072"` (`HANDOFF.md`).
