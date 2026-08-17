# Relay Backlog

> Backlog wird lokal in `BACKLOG.md` geführt. Keine GitHub Issues.
> Stand: 2026-08-17 (alle Issues in Release 26.09.90 umgesetzt)

---

## Erledigt — Release 26.09.90

### Account 2: Mails verschieben schlägt fehl (IMAP-Login-Limit)
- **Status:** ✅ erledigt (26.09.90)
- **Kategorie:** Backend / IMAP
- **Priorität:** high
- **Beschreibung:** Innerhalb von Account 2 lassen sich keine Mails verschieben (Fehler: „Maximum number of connections from user+IP exceeded"). Ursache: Session-Leak in den Timeout-/Join-Fehlerpfaden von `with_session_blocking` (Sessions wurden verworfen statt sauber ausgeloggt → Provider-Login-Limit). Fix: `logout_session` in den Timeout-/Join-Fehlerpfaden (client.rs) + defensive Session-Räumung.

### Account 2: Unterverzeichnisse lassen sich nicht in der Reihenfolge verschieben
- **Status:** ✅ erledigt (26.09.90)
- **Kategorie:** UX / Frontend
- **Priorität:** medium
- **Beschreibung:** Reorder ist jetzt baum-basiert: Drag-Drop verschiebt einen Unterordner nur innerhalb seiner Geschwister (gleicher Parent, delimiter-basiert), nicht mehr linear über die flache Liste. Persistenz unverändert (`relay_folder_order_<acct>`).

### Empfänger-Anzeige: nur einer statt aller (Absender + CC)
- **Status:** ✅ erledigt (26.09.90)
- **Kategorie:** UX / Frontend
- **Priorität:** medium
- **Beschreibung:** Neue `cc_addr`-Spalte (Migration), `MailEnvelope.cc` wird beim Sync geparst, `MessageRecord`/JSON tragen `cc`, Mail-Vorschau zeigt „An:" und „CC:"-Zeilen. EML-Import liest CC ebenfalls.

### Antworten: Rückfrage „An alle oder nur an den Absender?"
- **Status:** ✅ erledigt (26.09.90)
- **Kategorie:** UX / Frontend
- **Priorität:** medium
- **Beschreibung:** Bei mehreren Empfängern (To >1 oder CC) erscheint beim Klick auf „Antworten" ein Dialog: „An alle antworten" / „Nur an Absender" / „Abbrechen" (ConfirmationDialog mit dritter Aktion).

### Ungelesen-Markierung trotz gelesen (z. B. ECommerce, store@ui.com)
- **Status:** ✅ erledigt (26.09.90)
- **Kategorie:** Backend / Sync
- **Priorität:** high
- **Beschreibung:** (1) Sent-Pipeline: Gesendet-INSERT setzt `is_read=1`; IMAP-APPEND mit `\Seen`. (2) `flag_refresh` schützt kürzlich lokal gelesene Mails (30s-Cooldown via `update_is_read_guarded`), sodass ein stale Server-Flag sie nicht zurück auf ungelesen setzt.

### Mobile Compose: Toolbar-Buttons nicht vertikal zentriert
- **Status:** ✅ erledigt (26.09.90)
- **Kategorie:** UX / Frontend
- **Priorität:** medium
- **Beschreibung:** Mobile Buttons auf fix `height:45px; padding:0 8px/16px` gesetzt (statt height auto + 15px Padding). Verifiziert per Browser-Messung (Buttons exakt 45px, Mittelpunkte auf Toolbar-Mitte, Abweichung 0.5px) und im deployed CSS (height:45px).

### Batch: Mehrere Mails gelesen/ungelesen markieren funktioniert nicht
- **Status:** ✅ erledigt (26.09.90)
- **Kategorie:** UX / Frontend
- **Priorität:** medium
- **Beschreibung:** Neue Endpunkte `POST /messages/read-batch` und `/messages/unread-batch` (eine Anfrage für alle UIDs, folder-gescoped); Frontend `markSelectedRead`/`toggleReadStatus` nutzen die Batch-API. Lokale Ordner überspringen den IMAP-Flag-Sync.

### (Neu) Ordnerwechsel sofort — Meta-only Liste + Server-Cache
- **Status:** ✅ erledigt (26.09.90)
- **Kategorie:** Performance / Backend
- **Priorität:** high
- **Beschreibung:** `fetch_inbox_meta` selektiert ohne `body_text`/`body_html` (Preview via substr), Server-Side-LRU-Cache (account, folder). Live gemessen: zweiter+ Ordnerwechsel 12ms statt ~1.5s.

### (Neu) Sidebar-Versionstext „AImighty Relay 3.0"
- **Status:** ✅ erledigt (26.09.90)
- **Kategorie:** UX / Frontend
- **Priorität:** low

### (Neu) Profilbild-Upload unterstützt SVG
- **Status:** ✅ erledigt (26.09.90)
- **Kategorie:** UX / Frontend
- **Priorität:** low
- **Hinweis:** SVG wird als `<img src="data:image/svg+xml…">` gerendert — Skripte in SVG laufen im img-Kontext nicht (kein XSS).

---

## Erledigt (frühere Releases)

### Einstellungen-Menü unter Top-Down → Relay → Einstellungen verlinken
- **Status:** ✅ erledigt
- **Kategorie:** UX / Navigation
- **Priorität:** medium

### Ordner-Neuanordnung in der Sidebar bleibt nicht persistent
- **Status:** ✅ erledigt
- **Kategorie:** UX / Frontend
- **Priorität:** medium

### Markieren von Mails funktioniert nicht
- **Status:** ✅ erledigt
- **Kategorie:** UX / Backend
- **Priorität:** high

### Löschen von Mails entfernt diese nicht vom IMAP-Server
- **Status:** ✅ erledigt
- **Kategorie:** Backend / IMAP
- **Priorität:** high

### Anhänge werden nicht versendet / nicht angezeigt
- **Status:** ✅ erledigt
- **Kategorie:** Backend / Frontend
- **Priorität:** high (kritisch)

### Mobile Compose: Buttons zu dicht an der Begrenzungslinie + Ansicht korrumpiert
- **Status:** ✅ erledigt
- **Kategorie:** UX / Frontend
- **Priorität:** medium

---

## Ausgearbeitet

*(noch leer — Historie aus relay-repo ist dort dokumentiert, nicht hier)*
