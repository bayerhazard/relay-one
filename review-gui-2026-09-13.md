# Relay GUI- & Funktionalitäts-Review — 2026-09-13

> Ziel: **einheitliche Darstellung, durchgängige Bedienlogik, vollständiger KI-Assistent.**
> Basis: Release 26.9.155 (live auf der Box). Ausdrücklich **ohne Security** — reine
> GUI- und Funktionsprüfung auf interne Konsistenz.
> Methodik: statischer Audit aller 6 Module + Live-Durchlauf jedes Moduls via
> Playwright (Dev-Server mit `RELAY_API_URL=https://mail.aimighty.olares.de`),
> Screenshots unter `docs/review-gui-2026-09-13/`.
> Severity: 🔴 kritisch · 🟠 hoch · 🟡 mittel · 🔵 niedrig · 💡 strukturell.

## Baseline (Stage A)

| Check | Ergebnis |
|---|---|
| `cargo test --locked` | 602 passed · 0 failed · 7 ignored |
| `npm run test:run` (vitest) | 447 passed (39 Dateien) |
| `svelte-check` | 0 errors · 92 warnings |
| Live `/api/v1/health` | 200 `{"service":"relay-one","status":"ok"}` in 14 ms |

**Fazit Baseline:** grün, stabil. Der Fokus dieses Reviews liegt bewusst nicht auf
Test-Grünheit, sondern auf Querschnittskonsistenz — und genau dort liegen die Funde.

## Findings (Übersicht)

| ID | Sev | Bereich | Kurz |
|---|---|---|---|
| T1 | 🔴 | Theming | Dark-Mode zerfällt modulübergreifend (nur Mail+Settings setzen die Theme-Klasse) |
| T2 | 🔴 | Meetings | Detail-View rendert YAML-Frontmatter als Klartext |
| T3 | 🟠 | Rechtsklick | Kontextmenüs nur im Mail-Modul; andere Module ohne Rechtsklick/Long-Press |
| T4 | 🟠 | i18n | Hart kodierte deutsche Strings neben übersetzten UI-Teilen |
| T5 | 🟠 | Assistent | Assistant-Chat rendert kein Markdown (zeigt `**fett**` als Text) |
| T6 | 🟠 | Assistent | Meeting lässt sich per Assistent nicht öffnen (kein `meetings.open`) |
| T7 | 🟠 | Layout | Drei verschiedene Grundlayouts + inkonsistente Sync-Affordance |
| T8 | 🟡 | Dialoge | Escape schließt Kontakt-/Task-Modal nicht (Backdrop-Fokus nötig) |
| T9 | 🟡 | Empty-States | Drei verschiedene Empty-State-Ausprägungen |
| T10 | 🟡 | Kalender | `each_key_duplicate` — doppelte Event-Keys im Render |
| T11 | 🟡 | Meetings | Listen-Metazeile bricht („58\nmin"), Tags überlappen |
| T12 | 🟡 | Kontakte | RFC2047-kodierte Namen unverarbeitet angezeigt (`=?utf-8?B?…?=`) |
| T13 | 🟡 | Meetings | „Open in Insilo" toter Link (`source_url = "''"`) |
| T14 | 🟡 | Konsistenz | Zähler-Strings verhalten sich modulabhängig („239 contacts" / „3 tasks" / kein Zähler) |
| T15 | 🟡 | Datum | Vier verschiedene Datum/Zeit-Formate nebeneinander |
| T16 | 🔵 | Navigation | ModuleIcons ohne Mail/Settings; Assistant-Prompt erwähnt Meetings nicht |
| T17 | 🔵 | Kalender | Kopfzeilen-Buttons „Back/Forward" kollidieren semantisch mit Mail-„Forward" |
| S1 | 💡 | Assistent | `module`-Feld im Chat-Request wird serverseitig nie gelesen |
| S2 | 💡 | Struktur | CSS/Buttons/States pro Modul dupliziert (`mt-`/`tk-`/`ct-`/`cal-`) |
| S3 | 💡 | Struktur | `EntityChip.svelte` ist toter Code |
| S4 | 💡 | Dialoge | Native `confirm()` vs. `ConfirmationDialog`-Komponente mischweise im Einsatz |

## Details

### T1 🔴 Dark-Mode zerfällt modulübergreifend
- **Dateien:** `routes/+page.svelte:376,935-939` (Mail) · `routes/settings/+page.svelte:59-68` — beides je ein
  eigenes `$effect`, das `theme-dark` auf `documentElement` setzt. Die anderen vier Module
  (`calendar`, `contacts`, `tasks`, `meetings`) wenden das Theme **nicht** an.
- **Problem:** Nach Deep-Link/Reload auf eines dieser Module fehlt `theme-dark` komplett.
- **Live-Beweis:** `localStorage.relay_theme = "dark"`, aber auf `/meetings` ist
  `documentElement.className === ""` und der Body weiß. Vergleiche
  `review-meetings-dark.png` (hell) vs. `review-mail-dark.png` / `review-settings-dark.png` (korrekt dunkel).
- **Repro:** Settings → Dark wählen → direkt `/meetings` laden (F5) → helles UI; Refresh zurück auf `/` → dunkel.
- **Fix:** Theme-Init einmalig in `+layout.svelte` aus `localStorage` anwenden (dort existiert
  bereits der globale Effekt-Router); die zwei Duplikat-Effekte in Mail/Settings entfernen.
  Plus SSR-freundlich: Initial-Apply per Inline-Script in `app.html` gegen FOUC.
- **Test:** Vitest-Setup prüft `documentElement.classList` nach Theme-Wechsel; Live-Check jedes Moduls im Dark-Mode.
- **Effort:** klein (~30 min). Größter optischer Konsistenz-Wert.

### T2 🔴 Meetings-Detail zeigt YAML-Frontmatter im Klartext
- **Datei:** `routes/meetings/+page.svelte:132` (`detailHtml`), Server: `sync/insilo.rs` (Upsert speichert Body inkl. Header).
- **Problem:** `body_md` beginnt mit der Frontmatter (`--- source: insilo … ---`). Der
  Detail-View rendert sie als Markdown-Klartext (inkl. gerenderter Metadaten-Listen).
  Nur der Mail-Versand strippt die Frontmatter (`stripFrontmatter`, von 26.9.154) —
  die **Anzeige nicht**.
- **Live-Beweis:** `review-meetings-detail.png` — der View beginnt mit
  `--- source: insilo meeting_id: "2ea1673e-…`.
- **Fix:** Dieselbe `stripFrontmatter`-Logik auf `detail.body_md` anwenden (entweder im
  `detailHtml`-Derived oder serverseitig beim GET von `body_md` trennen und eine
  `metadata`-Struktur mitschicken). Serverseitig wäre robuster (Assistent
  `meetings_get` liefert denselben Roh-Body).
- **Test:** Vitest für das Derived/Server-Stripping; Live-Check.
- **Effort:** klein (~20 min).

### T3 🟠 Kontextmenü nur im Mail-Modul
- **Datei:** `lib/components/MessageList.svelte` (`oncontextmenu` + Long-Press), `AccountGroup.svelte:101-236`.
  Kalender (`calendar/+page.svelte`), Kontakte, Aufgaben, Meetings: kein `contextmenu`-Handler.
- **Problem:** Rechtsklick ist die etablierte Aktionsschnittstelle des Mail-Moduls
  (Antworten/Weiterleiten/Lesen-Status/Markieren/Dringlich/Verschieben/Löschen,
  Ordner-Menü, Anhang-Menü, Link-Menü). In allen übrigen Modulen fehlt sie. Erschwerend:
  Rechtsklick löst dort **zufällige** Standardaktionen aus — im Kalender selektiert
  Rechtsklick das Event (= Linksklick-Ersatz) und öffnet das Default-Browsermenü sonst.
- **Live-Beweis:** Rechtsklick auf Mail → Menü (`review-mail-ctx.png`); Rechtsklick auf
  Kalender-Event → Event wird geöffnet, kein Menü.
- **Fix:** Ein gemeinsames Kontextmenü-Pattern (die Mail-Implementierung `ctx-menu-*`
  ist der Goldstandard, Touch-Sheet inklusive) auf die anderen Module heben:
  Kalender-Event (Bearbeiten/Löschen/RSVP), Kontakt (Bearbeiten/Löschen/mailen),
  Task (Umschalten/Löschen), Meeting (Löschen/Mailen/Open in Insilo).
- **Test:** Komponenten-Tests für Menü-Öffnen/Aktionen; Live-Klick je Modul.
- **Effort:** mittel (je Modul ~1 h, inkl. i18n-Keys).

### T4 🟠 Hart kodierte deutsche Strings neben übersetzter UI
- **Stellen (Kompilat):**
  - `MessageList.svelte:436-438` — Kontextmenü „Als gelesen markieren"/„Markieren"/
    „Dringlich löschen"/„Dringlich" (Rest des Menüs ist übersetzt).
  - `MessageList.svelte:345` — Swipe-Label „Entmarkieren"/„Markieren".
  - `ModuleIcons.svelte:7-15` — alle Tooltips/Aria-Labels deutsch („Kalender", „Kontakte",
    „Aufgaben", „Meetings") — in jeder Module-Sidebar sichtbar.
  - `ComposeWindow.svelte:203,216,225,354,374,388` — Status „Kein Audio aufgezeichnet.",
    „Transkribiere…", „Generiere Text…", „Ermittle Adresse…" etc.
  - `ComposeWindow.svelte:644-687` — „Deine Nachricht:", Format-Button-Tooltips
    („Fett", „Kursiv", „Überschrift", „Liste", „Link", „Code", „Entfernen").
  - `routes/calendar/+page.svelte:577` — Import-Fehler „Keine Termine in der Datei gefunden."
  - `routes/+layout.svelte:59` + Offline-Banner (Zeile 47) — „Fehler beim Start"/„Offline — gelesene …".
  - `AccountGroup.svelte:256` — „Unterordner einblenden/ausblenden".
- **Live-Beweis:** `review-mail-ctx.png` (gemischtsprachiges Kontextmenü), Kalender-
  Sidebar-Tooltips deutsch bei englischem UI.
- **Fix:** Alle Stellen auf `$t()`/`translate()` umstellen + Keys in `i18n.ts` (de/en);
  Der vorhandene Key-Parity-Test (i18n.test.ts) fängt fehlende Übersetzungen automatisch.
- **Test:** i18n-Parity-Tests laufen automatisch; manuelle EN-Durchsicht je Modul.
- **Effort:** klein-mittel (~1 h Gesamttour).

### T5 🟠 Assistent rendert kein Markdown
- **Datei:** `lib/components/AssistantDrawer.svelte` (Antwort-Bubble).
- **Problem:** Tool-Antworten und Modelltext enthalten Markdown (Fett, Listen) — die
  Bubble zeigt es als Klartext mit Sternchen. Die übrige App rendert Markdown
  (Mail-Body, Meetings-Detail via `renderMarkdown`).
- **Live-Beweis:** `review-assistant-meetings.png` — `**AImighty — Roadmap-Abstimmung**` sichtbar.
- **Fix:** Antwort-Bubble über `renderMarkdown` (DOMPurify-gesichert, wie Mail) leiten;
  Whitelist der erlaubten Elemente beschränken.
- **Test:** Komponenten-Test: `**x**` rendert als `<strong>`.
- **Effort:** klein (~20 min).

### T6 🟠 Assistent kann Meetings nicht öffnen
- **Dateien:** `server/src/ai/tools/ui.rs:49` (`ui_open_item` erlaubt nur calendar/tasks/contacts),
  `web/src/lib/stores/effects.ts` (kein `meetings.open`-Effekt).
- **Problem:** Der Assistent liest Meetings (`meetings_list/get/search`), aber „Öffne das
  Meeting …" führt nicht zur Detail-View — er antwortet nur mit Text. In Mail/Kalender/
  Kontakten/Aufgaben ist das Öffnen eines konkreten Eintrags möglich
  (`mail.open`/`calendar.open_event`/`contacts.open`/`tasks.open`).
- **Live-Beweis:** Frage „Öffne das Meeting Kunden-Onboarding" → nur Text, keine Navigation.
- **Fix:** (1) `meetings.open`-Effekt in `effects.ts` + `meetings/+page.svelte` Abonniert
  einen Selection-Store wie tasks/contacts. (2) `ui_open_item` um `meetings` erweitern
  (ID-Abstammung über `known_ids` abprüfen). (3) ggf. Tool-Beschreibung von `meetings_get` um den Hinweis ergänzen.
- **Test:** Server-Tool-Test (Nachfrage→Nav) + `stores.test.ts`-Fall; Live-Frage.
- **Effort:** klein (~45 min, 3 Dateien).

### T7 🟠 Drei Grundlayouts + Sync-Affordance-Zoo
- **Beobachtung (Live + Code):**
  - 3-Pane: Mail (Konten/Liste/Reader), Kalender (Sidebar/Grid/Detail-Aside), Meetings.
  - 2-Pane Vollbreite-Liste: Kontakte, Aufgaben (Sidebar + full-width Cards, kein Detail-Pane;
    Editing über Modals).
  - Eigenbau: Einstellungen (Section-Sidebar, Karten-Content, **ohne ModuleIcons-Footer**).
  - Sync/Refresh: Mail = Icon-Button im Listen-Header; Aufgaben = Ghost-Button in der
    Sidebar („Load from CalDAV"); Meetings = Full-width-Ghost („Refresh", von 26.9.155
    an Aufgaben angeglichen); Kalender = keiner sichtbar; Kontakte = keiner.
- **Problem:** Nutzern ist nicht erkennbar, welches Refresh-Pattern verbindlich und wo
  Sync überhaupt möglich ist. Kalender/Kontakte syncen implizit ohne Anlaufpunkt.
- **Fix-Vorschlag (konservativ):** Kalender bekommt einen Refresh-Button analog Mail
  (Header), Kontakte ebenso (CardDAV-Sync triggern). 2-Pane-Layouts bewusst lassen
  (Listen-Ziel passt), aber Empty-State rechts an 3-Pane angleichen (T9).
  ModuleIcons-Footer auch in Settings einsetzen.
- **Effort:** mittel (~2 h).

### T8 🟡 Escape schließt Kontakt-/Task-Modal nicht
- **Dateien:** `contacts/+page.svelte:230-238`, `tasks/+page.svelte` (gleiches Pattern).
- **Problem:** Der Escape-Handler hängt mit `tabindex="0"` am **Backdrop-Element**. Der
  Fokus liegt nach Öffnen auf dem ersten Feld — Escape erreicht den Handler nicht.
- **Live-Beweis:** Modal geöffnet, Escape → Backdrop-Klasse weiterhin sichtbar (getestet
  in beiden Modulen). Mail/Kalender fassen Escape global (`svelte:window onkeydown`).
- **Fix:** `svelte:window onkeydown` wie in Mail/Kalender; im Handler nur schließen,
  wenn kein Input fokussiert ist (oder immer, außer während Busy).
- **Test:** Komponenten-Test mit `dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }))`.
- **Effort:** klein (~15 min je Modul).

### T9 🟡 Drei Empty-State-Ausprägungen
- **Dateien:** `EmptyState.svelte` (nur Mail, mit Icon + Titel + Untertitel) ·
  `meetings/+page.svelte` („Select a meeting" — Textzeile oben ohne Icon) ·
  `cal-detail-empty` (Kalender — nur Text) · Kontakte/Aufgaben (eigene Inline-Divs).
- **Live-Beweis:** Mail-Empty (`review-mail.png`, zentriert mit Icon) vs. Meetings
  (`review-meetings.png`, nackter Text oben).
- **Fix:** `EmptyState` in Meetings (Detail-Empty) und Kalender (Detail-Aside), mit
  passenden Icons; Inline-Divs in Kontakte/Aufgaben belassen, aber maß-/typografisch
  an EmptyState angleichen.
- **Effort:** klein (~30 min).

### T10 🟡 Kalender: `each_key_duplicate` im Console-Log
- **Datei:** `calendar/+page.svelte`(z. B. `upcoming`-Block, Zeile 973: `{#each upcoming as ev (ev.id)}`).
- **Problem:** Beim Modul-Load wirft Svelte `each_key_duplicate` — die ID
  `relay-1788385857532660684` tauchte doppelt auf, d. h. `listEvents` liefert
  dasselbe Event zweimal (vermutlich Doppel-Eintrag in den Rohdaten — z. B. ICS-Import
  doppelt — oder Recurring-Expansion).
- **Auswirkung:** Warnung + potenzielle Render-Artefakte (Svelte kann bei doppelten
  Keys Listen verwürfeln).
- **Fix:** Kurzfristig Serverseite deduplizieren (listEvents distinct on id) — langfristig
  Doppel-Datenpflege im Import prüfen (ICS-Import doppelt?).
- **Test:** Regression: Fix einspielen → Console clean; Server-Test auf Distinktheit.
- **Effort:** klein (~30 min) — Datenursache abhängig.

### T11 🟡 Meetings-Listen-Meta bricht/überlappt
- **Datei:** `meetings/+page.svelte` (`.mt-item-meta`, `gap: 8px`, `flex`).
- **Problem:** „58 min" bricht innerhalb der Zeile („58\nmin", sichtbar in
  `review-meetings.png`), Tags laufen bis an den Rand. `white-space: nowrap` fehlt
  auf der Dauer-Span.
- **Fix:** `.mt-item-meta` → `flex-wrap: nowrap` + Dauer-Span `white-space: nowrap`;
  Tags bleiben mit Ellipsis.
- **Effort:** klein (~10 min).

### T12 🟡 RFC2047-kodierte Kontaktnamen unverarbeitet
- **Beobachtung (Live):** Kontakt mit `=?utf-8?B?…?=`-Codierung als
  Anzeigenamen (`review-contacts.png`, zweiter Eintrag). CardDAV-Sync speichert den
  codierten Header-Wert unverarbeitet.
- **Fix:** Beim Sync/Anzeigen dekodieren (RFC2047, `rust`-seitig z. B. via
  `mail-parser`-oder eigene Charset-Decoder-Helfer — im IMAP-Pfad existiert das
  Dekodieren bereits, hier fehlt es im CardDAV-Zweig).
- **Effort:** mittel (~1 h, inkl. Test mit Base64-Namen).

### T13 🟡 „Open in Insilo" toter Link bei Meetings
- **Server:** `sync/insilo.rs:62-64` — `unquote()` trimmt nur `"`; Insilo schreibt
  fehlende URL als `source_url: ''` (einfache Anführungszeichen) → der String
  `"''"` landet wörtlich in der DB.
- **Client:** `meetings/+page.svelte` — `{#if detail.source_url}` ist truthy für `"''"`
  → Link mit `href="''"` wird gerendert (Live: `/url: "''"` im Snapshot).
- **Fix:** (a) `unquote()` um `'` erweitern und Empty-YAML nach Empty-String mappen;
  (b) Client-Fallback `source_url.startsWith("http")` als Render-Bedingung.
  Bestehende DB-Zeilen per Migration/Re-Import heilen (Version-Bump des Exports).
- **Effort:** klein (~20 min).

### T14 🟡 Zähler-Strings inkonsistent
- **Beobachtung:** Kontakte „239 contacts", Aufgaben „3 tasks", Mail ohne sichtbaren
  Zähler, Kalender ohne, Meetings: Zähler wurde auf Nutzer-Wunsch entfernt (26.9.155).
- **Problem:** Nicht die Frage „Zähler ja/nein" ist der Befund, sondern dass die
  Entscheidung modulabhängig fällt. Mail zeigt keine Ordner-Anzahl im Footer,
  Kalender zeigt keine Kalender-Anzahl.
- **Fix-Vorschlag:** Zielbild festlegen (Empfehlung: Zähler überall dort, wo eine
  Liste das Primär-Objekt ist: Mail-Ordnerbadge existiert — Kontakte/Tasks lassen,
  Meetings gelassen wie entschieden) und dokumentieren; technisch kein Zwang.
- **Effort:** 0 — Entscheidung + Dokumentation.

### T15 🟡 Vier Datum/Zeit-Formate nebeneinander
- **Stellen:**
  - Mail: `formatDate()` — hardcoded `de-DE`, inkl. hartem „Gestern" (`format.ts:109-127`),
    nur im Mail-Modul genutzt.
  - Aufgaben: `tasks/+page.svelte:171` — hardcoded `de-DE`.
  - Kalender: mischt `undefined` (Browser-Locale) und `$lang === "de"? "de-DE" : "en-US"`
    (`calendar/+page.svelte:321`).
  - Meetings: `undefined` (Browser-Locale) — `09/12/2026` bei en-Browser.
- **Live-Beweis:** Tasks „12.09.2026"-Stil vs. Meetings „09/12/2026"-Stil
  (`review-tasks.png` vs. `review-meetings.png`, gleiches Browser-Profil).
- **Fix:** Ein zentrales Format-Module (`format.ts`): `fmtDate`, `fmtTime`, `fmtDateTime`,
  `fmtRelative` — parametrisiert von `$lang` (de ↔ en, analog `calendar:321`), alle
  Module schalten auf dieselbe Funktion um; Assistent-Antwortformate folgen der App-Locale.
- **Effort:** mittel (~1,5 h, viele Call-Sites + Tests).

### T16 🔵 ModuleIcons ohne Mail/Settings · Prompt-Text veraltet
- **Dateien:** `ModuleIcons.svelte` (nur Kalender/Kontakte/Aufgaben/Meetings),
  `AssistantDrawer.svelte` („Ask me about events, tasks, or emails.").
- **Problem:** Mail ist nur über das AIM-Logo erreichbar (semantisch ok, aber als einziger
  Return-Pfad unbefriedigend), Settings gar nicht per Icon. Der Assistant-Begrüßungstext
  listet die Module nicht, die er seit 26.9.150 tatsächlich beherrscht.
- **Fix:** Entweder Mail-Icon ergänzen (Logo bleibt Home) oder bewusst dokumentieren;
  Prompt-Text auf „… events, tasks, emails, and meetings." erweitern (i18n).
- **Effort:** klein (~20 min).

### T17 🔵 Kalender-„Forward" semantische Kollision
- **Datei:** `calendar/+page.svelte` (Kopfzeile „‹ Back"/„Today"/„Forward ›").
- **Problem:** „Forward" heißt in Mail „Weiterleiten". Hier heißt es „nächster Monat".
  Dieselbe Wortbedeutung für verschiedene Aktionen.
- **Fix:** `calendar.next`/`calendar.prev` Keys benutzen, Label „Next"/„Previous" (de:
  „Weiter"/„Zurück"). Accessibility-Labels bleiben, Sichttexte angleichen.
- **Effort:** klein (~15 min).

### S1 💡 Assistent: `module`-Feld tote Daten
- **Dateien:** `server/src/ai/agent.rs:36` (Feld existiert) — Lesestellen: keine.
  Frontend sendet es (`AssistantDrawer.svelte:281`).
- **Bewertung:** Heute harmlos (Kontext ist bewusst modulübergreifend, `api/ai.rs:1685`).
  Aber die Schnittstelle suggeriert Modul-Scoping, das nicht existiert. Empfehlung:
  Feld dokumentieren + später für Modul-Fokus nutzen (z. B. Priorisierung der Tools/
  Kontexte), oder entfernen.
- **Effort:** 0 (dokumentieren) bis klein (falls zum Feature gemacht).

### S2 💡 CSS-Prefix-Duplication pro Modul
- **Beobachtung:** `mt-`/`tk-`/`ct-`/`cal-` Präfixe mit je eigener Kopie von Buttons,
  States, Footer. Buttons: Mail nutzt `icon-btn`/`selection-btn`/`action-btn-pill`
  (gänzlich anderes System) vs. `xx-btn-primary/ghost/danger` der übrigen Module.
- **Auswirkung:** Jede Stilkorrektur x6, Drift ist bereits sichtbar (T7, T9).
- **Fix-Vorschlag (mittel/langfristig):** Shared-Bibliothek anfangen: eine
  `app-shell.css` (Sidebar/Footer/ModuleIcons/States) + `Button.svelte` mit
  `variant`-Prop. Migration modulweise, kosmetisch neutral.
- **Effort:** groß — als Backlog-Item, kein Sofort-Fix.

### S3 💡 `EntityChip.svelte` toter Code
- **Beobachtung:** wird nirgends importiert. Entfernen oder als Baustein für T6 nutzen.
- **Effort:** klein.

### S4 💡 Dialog-Systematik gemischweise
- **Beobachtung:** `ConfirmationDialog`/`PromptDialog` existieren (Settings/Mail/Kalender
  nutzen sie), Kontakte/Aufgaben/Meetings nutzen natives `confirm()`. Visuell und
  in-app ein Bruch (natives Dialogfenster, kein Dark-Theme, kein App-Look).
- **Live-Beweis:** Meetings-Delete-Bestätigung = natives Fenster.
- **Fix:** Kontakte/Tasks/Meetings auf `ConfirmationDialog` umstellen (Aufrufstellen austauschbar:
  Titel, Text, Callback).
- **Effort:** klein (~45 min).

## Konsistenz-Matrix

Legende: ✅ konsistent/gut · 🟡 teils · ❌ fehlt/inkonsistent.

| Dimension | Mail | Kalender | Kontakte | Aufgaben | Meetings | Einstellungen |
|---|---|---|---|---|---|---|
| Layout 3-Pane Standard | ✅ | ✅ | ❌ (2-Pane) | ❌ (2-Pane) | ✅ | ❌ (Eigenbau) |
| ModuleIcons-Footer | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ |
| Refresh/Sync-Affordance | ✅ Header-Icon | ❌ | ❌ | ✅ Sidebar | ✅ Sidebar full-width | n/a |
| Rechtsklick | ✅ (+Long-Press) | ❌ | ❌ | ❌ | ❌ | ❌ |
| Escape handhabt Dialoge | ✅ | ✅ | ❌ | ❌ | n/a (nativ confirm) | ✅ |
| EmptyState-Komponente | ✅ | 🟡 (nur Text) | 🟡 | 🟡 | 🟡 (nur Text) | n/a |
| i18n vollständig | 🟡 (Kontextmenü) | 🟡 (1 Stelle) | ✅ | ✅ | ✅ | 🟡 (de/en-Toggle-Titel ok) |
| Datum einheitlich zur App-Locale | ❌ (de-DE hard) | 🟡 (mixt) | n/a | ❌ (de-DE hard) | 🟡 (Browser-Loc.) | n/a |
| Dark-Theme korrekt | ✅ (setzt selbst) | ❌ (nicht angewendet) | ❌ | ❌ | ❌ | ✅ (setzt selbst) |
| Assistent: Modul lesbar | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ (indirekt) |
| Assistent: Element öffnen | ✅ | ✅ | ✅ | ✅ | ❌ | — |
| Assistent: Modul schreibbar | ✅ | ✅ | ✅ (create) | ✅ | — (by design read-only) | — |

## Live-Sitzung (Phase 3) — Kernbeobachtungen

1. Mail, Kalender, Kontakte, Aufgaben, Meetings, Einstellungen jeweils komplett durchlaufen:
   Laden, Kernflows, Rechtsklick, Escape, Assistant-Frage.
2. Assistent + Meetings: Fragen zu Meetings werden korrekt mit Read-Tools beantwortet
   (Titel/Datum/Teilnehmer, ~Live-Daten). Das Öffnen im UI fehlt (T6).
3. Dark-Mode-Szenario beweist T1 sauber reproduzierbar.
4. Screenshots: `docs/review-gui-2026-09-13/` (12 Stück, Überblick je Modul/Metrik — siehe Konsistenz-Matrix oben).

## Empfehlungen (Priorisierung)

1. **Sofort-Cluster (Release 26.9.156):** T1 (Theme-Init ins Layout), T2
   (Frontmatter-Stripping serverseitig), T5 (Markdown-Bubble), T13 (unquote + Link-Guard),
   T11 (Meta-Wrap), T8 (Escape global).
2. **Next-Cluster (26.9.157):** T4 (i18n-Tour), T9 (EmptyStates), T16/T17 (Texte/Tooltips),
   S4 (Dialoge vereinheitlichen).
3. **Funktions-Cluster:** T3 (Kontextmenüs in die Module), T6 (meetings.open),
   T7 (Refresh in Kalender/Kontakte).
4. **Backlog:** T10 (Datenursache doppelte Events), T12 (RFC2047-Kontakte),
   S2 (Shared-Components-Bibliothek), S1/S3 aufräumen.
5. **Dokumentierte Entscheidungen:** T14 (Zähler-Zielbild), Layout-Zielbild (2-Pane
   bewusst lassen oder angleichen) — als GUI-Standard in ein `docs/gui-standard.md`
   ableiten, damit künftige Änderungen nicht erneut drift-fähig sind.

## Positiv bemerkenswert

- Baseline komplett grün (602/447 Tests, svelte-check 0 errors) trotz des engen Release-Rhythmus.
- Assistent-Meetings-Roundtrip live solide (Antwort in Sekunden, korrekte Daten).
- Mail-Kontextmenü ist ein ausgereiftes, Touch-fähiges Pattern — der Goldstandard für T3.
- Dark-/Light-Vorschau in Settings ist vorbildlich gelöst (Preview-Karten).
- Geist/Geist-Mono-Selbstladung + `--am-*`: durchgehend saubere Typo-Basis.


---

## Umsetzung (26.9.156 → 26.9.157) — Nachtrag

Alle findings aus dieser Review sind umgesetzt und live:

| Finding | Umsetzung | Verifikation |
|---|---|---|
| T1 | Theme-Init in `+layout.svelte` aus `localStorage`; Mail/Settings-Effekte unangetastet | Live: Deep-Link auf `/meetings` mit `relay_theme=dark` → dunkel |
| T2 | `stripFrontmatter` auf `detailHtml`, live weg | Live-Screen: fix-meetings-detail.png |
| T3 | Shared `ContextMenu.svelte` in Kalender/Kontakte/Tasks/Meetings (+ Escape-Global-Handler) | Live: Menüs in Meetings/Kalender/Kontakte; Escape schließt |
| T4 | i18n-Tour: MessageList, ModuleIcons, ComposeWindow, Layout, AccountGroup, Kalender-Import, SplashScreen/Settings-Placeholder | Live: englische Tooltips, i18n-Parity-Tests grün |
| T5 | Assistant-Bubble rendert Markdown via `renderMarkdown` | Live: fix-assistant-md.png (fette Titel) |
| T6 | `meetings.open`-Effekt + `selection.setMeeting` + `ui_open_item` erlaubt Meetings (numerische id, known_ids-Pflicht) | Unit-Test stores.test.ts; Agent-Wahl bleibt Modellfrage |
| T7 | Refresh-Buttons: Kalender-Header (bindet das bestehende, tote `handleSync`), Contacts (`syncCardDav` + reload) | Live: Snapshot refresh-Button beide |
| T8 | Escape global in Kontakte/Tasks (foldet Modals) | Live: Escape schließt Editor/Delete-Konfirmation |
| T8+ | Kontextmenü-Komponente schließt selbst auf Escape | Live: fix-meetings-ctx.png, Escape → scrim weg |
| T9 | `EmptyState` in Meetings (Detail-Empty) + Kalender (Details-Aside), neuer `calendar`-Icon-Typ (SVG) | Live: fix-meetings-dark.png / fix-calendar2.png |
| T10 | Ursache lokalisiert: **Invitations** liefert doppelte `event_uid`; Events kehren die id je Occurrence doppelt (recurring). Fix: Dedupe der INVITATIONS + Composite-Key `evKey` (id@occurrence) statt id | Live: Console clean |
| T11 | `.mt-item-meta nowrap` | Live: „58 min" bricht nicht |
| T13 | Server `unquote()` stript auch `''`-YAML-Empty + client `startsWith("http")`-Guard | Live: API liefert korrekt; Unit-Test 603 grün |
| T16 | Prompt-Text „…emails, or meetings." | Live: fix-assistant-md.png (Prompt) |
| T17 | `calendar.prevPeriod`/`nextPeriod`-Keys statt `back/forward` | Live: „Previous“/„Next“ |
| S3 | `EntityChip.svelte` entfernt | - |
| S4 | `ConfirmationDialog` statt natives `confirm()`: Kontakte, Tasks, Meetings, Settings-Restore | Live: fix-contacts-confirm.png |

Nicht umgesetzt (dokumentiert im Body): T12 (RFC2047-Kontakte — Backlog), T14 (Zähler-Entscheidung — nur Dokumentation), S1 (Doku), S2 (Backlog-Refactor).

Zusätzlich existiert ein **Infra-Befund aus dem Release-Prozess**: Docker-GHA-Cache lief ein STALES `COPY web/`-Layer → 26.9.156 enthielt den alten Web-Build (`AImighty Relay 3.0`). Fix: `no-cache: true` im
build-image-Workflow (26.9.157), danach frisches Frontend auf der Box; parallel
veraltete die dritte Ebene (Route-Registrierung) — `settings apps domain set relay relay --third-level mail` nach JEDEM Upgrade refreshen, sonst bleibt das öffentliche Domain-
Mapping am alten Backend hängen.
