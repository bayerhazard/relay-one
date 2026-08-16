# Relay Backlog

> Backlog wird lokal in `BACKLOG.md` geführt. Keine GitHub Issues.
> Stand: 2026-08-15

---

## Offen

### Einstellungen-Menü unter Top-Down → Relay → Einstellungen verlinken
- **Status:** ⬜ offen
- **Kategorie:** UX / Navigation
- **Priorität:** medium
- **Beschreibung:** Unter dem obersten Top-Down-Menü → `relay` → `einstellungen` soll das Einstellungen-Menü von Relay verlinkt werden (direkter Zugriff auf die Relay-Einstellungen aus dem Menü).

### Ordner-Neuanordnung in der Sidebar bleibt nicht persistent
- **Status:** ⬜ offen
- **Kategorie:** UX / Frontend
- **Priorität:** medium
- **Beschreibung:** Neuanordnungen der Verzeichnisse in der linken Sidebar überleben den App-Neustart nicht. Die Reihenfolge soll persistent gespeichert werden (z. B. localStorage/Settings), damit sie nach einem Restart erhalten bleibt.

### Markieren von Mails funktioniert nicht
- **Status:** ⬜ offen
- **Kategorie:** UX / Backend
- **Priorität:** high
- **Beschreibung:** Das Markieren von Mails funktioniert derzeit nicht. Erwartetes Systemverhalten:
  - Markierungen sind an der Mail sichtbar (Indikator).
  - Über die rechte Maustaste / Kontextmenü kann eine Markierung gelöscht werden.
  - Über die Suche lassen sich markierte Mails finden (Suchbegriff „Markierung" bzw. entsprechendes Flag).
- **Hinweis:** In der alten `relay-repo` (Tauri-App) existiert bereits eine Markierungs-Funktion (is_flagged, toggle_flagged, Kontextmenü) — als Referenz prüfen. In relay-one ist das Flagging offenbar defekt/unvollständig.

### Löschen von Mails entfernt diese nicht vom IMAP-Server
- **Status:** ⬜ offen
- **Kategorie:** Backend / IMAP
- **Priorität:** high
- **Beschreibung:** Werden Mails aus dem Posteingang in Relay gelöscht, verbleiben diese dennoch auf dem IMAP-Server. Es muss sichergestellt werden, dass Löschen wirklich Löschen bedeutet (d. h. die Mails werden auch auf dem IMAP-Server entfernt bzw. entsprechend markiert/archiviert).

### Anhänge werden nicht versendet / nicht angezeigt
- **Status:** ⬜ offen
- **Kategorie:** Backend / Frontend
- **Priorität:** high (kritisch)
- **Beschreibung:** Es werden keine Anhänge an den Mailausgängen angezeigt. Vermutlich werden die Mails sogar ohne Anhänge versendet. Bitte ausführlich prüfen — kritischer Fehler. Zu untersuchen: Attachment-Anzeige in der MessageList/Detailansicht, Sende-Pipeline (Compose → Server → SMTP), Attachment-Upload/Encoding.

### Mobile Compose: Buttons zu dicht an der Begrenzungslinie + Ansicht korrumpiert
- **Status:** ⬜ offen
- **Kategorie:** UX / Frontend
- **Priorität:** medium
- **Beschreibung:** In der Mobile-Ansicht der Mail-Komposition (sowohl „Neue Mail" als auch „Antworten") sitzen die drei Buttons am unteren Ende zu dicht an der Begrenzungslinie darüber. Es soll ein kleines Padding (3–5 px) eingefügt werden.
- **Zusatz (Antwort-Mail):** Nach unten ist kaum noch Platz. Vermutlich rücken die drei Buttons durch die Elemente darüber weiter nach unten und korrumpieren dadurch die Gesamtansicht.
- **Lösungsvorschlag:** Den unteren Teilbereich mit den drei Buttons in der Höhe fixieren. Andere Elemente (z. B. das Eingabefeld für die Mail) verschwinden dahinter, wenn sie nicht passen, und können durch Scrollen angeschaut werden. Dies stellt sicher, dass die Mail-Compose-Ansicht auch bei kleineren Bildschirmen sauber ist.

---

## Ausgearbeitet

*(noch leer — Historie aus relay-repo ist dort dokumentiert, nicht hier)*
