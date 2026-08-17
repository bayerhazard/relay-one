# Relay Backlog

> Backlog wird lokal in `BACKLOG.md` geführt. Keine GitHub Issues.
> Stand: 2026-08-15 (6 neue Issues offen)

---

## Offen

### Account 2: Mails verschieben schlägt fehl (IMAP-Login-Limit)
- **Status:** ⬜ offen
- **Kategorie:** Backend / IMAP
- **Priorität:** high
- **Beschreibung:** Innerhalb von Account 2 lassen sich keine Mails verschieben. Fehler: „Die Nachricht konnte nicht verschoben werden: Die Nachricht konnte nicht verschoben werden. ([login] Auth: IMAP login fehlgeschlagen: No Response: [UNAVAILABLE] Maximum number of connections from user+IP exceeded (mail_max_userip_connections=25))". Ursache prüfen: zu viele offene IMAP-Verbindungen des Kontos (Connection-Pool/Leak?), Login-Limit des Providers (25 Verbindungen pro User+IP).

### Account 2: Unterverzeichnisse lassen sich nicht in der Reihenfolge verschieben
- **Status:** ⬜ offen
- **Kategorie:** UX / Frontend
- **Priorität:** medium
- **Beschreibung:** Die Unterverzeichnisse von Account 2 lassen sich nicht in der Reihenfolge verschieben (Drag-and-Drop-Reorder schlägt fehl bzw. wird nicht gespeichert). Siehe auch Issue „Ordner-Neuanordnung persistent" (erledigt) — hier speziell für Unterverzeichnisse von Account 2.

### Empfänger-Anzeige: nur einer statt aller (Absender + CC)
- **Status:** ⬜ offen
- **Kategorie:** UX / Frontend
- **Priorität:** medium
- **Beschreibung:** Bei Mails mit mehreren Empfängern wird immer nur einer bzw. der Absender angezeigt. Es sollen alle angezeigt werden (Absender und CC).

### Antworten: Rückfrage „An alle oder nur an den Absender?"
- **Status:** ⬜ offen
- **Kategorie:** UX / Frontend
- **Priorität:** medium
- **Beschreibung:** Wenn mehrere Empfänger vorhanden sind (Absender und CC), soll beim Klick auf „Antworten" eine Rückfrage kommen, ob an alle oder nur an den Absender geantwortet werden soll.

### Ungelesen-Markierung trotz gelesen (z. B. ECommerce, store@ui.com)
- **Status:** ⬜ offen
- **Kategorie:** Backend / Sync
- **Priorität:** high
- **Beschreibung:** Manche Mails haben eine Ungelesen-Markierung, obwohl sie bereits gelesen sind (Beispiel: ECommerce, Absender store@ui.com). Bitte bereinigen (DB-Repair) und analysieren, woher es kommt (Flag-Sync/Scheduler?).
- **Zusatz:** Frisch verschickte Mails erscheinen im Gesendet-Ordner immer als ungelesen → im Gesendet-Ordner können keine Mails ungelesen sein (Send-Pipeline markiert versendete Mails nicht als gelesen bzw. setzt is_read nicht korrekt).

### Mobile Compose: Toolbar-Buttons nicht vertikal zentriert
- **Status:** ⬜ offen
- **Kategorie:** UX / Frontend
- **Priorität:** medium
- **Beschreibung:** In der Mobile-Ansicht bei „Compose New Mail" sind die Buttons in der Toolbar unten falsch positioniert: Sie scheinen mit dem unteren Rand abzuschließen und sind zu hoch.
- **Gewünscht:** Die Buttons sollen zum unteren Bildschirmrand UND zum oberen Trennbereich einen exakt gleichen Abstand haben — also im Toolbereich vertikal zentriert sein.
- **Verifikation:** Screenshot erstellen und analysieren, um sicherzustellen, dass die Korrektur richtig ist.

### Batch: Mehrere Mails gelesen/ungelesen markieren funktioniert nicht
- **Status:** ⬜ offen
- **Kategorie:** UX / Frontend
- **Priorität:** medium
- **Beschreibung:** Mehrere Mails markieren (Mehrfachauswahl) und als Batch gelesen oder ungelesen markieren funktioniert nicht.

---

## Erledigt

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
