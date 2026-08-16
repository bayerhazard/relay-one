# Relay — Issue-Analyse (2026-08-16, aktualisiert)

Status: **Alle 6 Issues aus BACKLOG.md behandelt** — Fixes implementiert und getestet
(Stand 2026-08-16, noch NICHT deployed). Issue 1 ist Olares-seitig (App bestmöglich vorbereitet).

## Stand der Fixes (2026-08-16)

| Issue | Fix | Dateien | Tests |
|---|---|---|---|
| 3 (Flagging) | Backend: `set_imap_flag` → `mark_flag(..., Some(folder))`; Frontend: Guard bei Ordnerwechsel; Suche: `is:flagged`-Operator | messages.rs, client.rs, cache/messages.rs, +page.svelte | 437 grün, 300 grün |
| 4 (Löschen/IMAP) | **Root-Cause: Session-Race** — `fetch_all_uids()`/`fetch_recent` ohne SELECT auf geteilter IMAP-Session → `delete_messages_not_in` löschte frische INBOX-Mails (falscher Ordner). Fix: atomare `fetch_recent_in_folder` + `fetch_all_uids_in_folder` + folder-scoped Body-Fetch | scheduler.rs, client.rs | 437 grün |
| 5 (Anhänge) | Axum `DefaultBodyLimit` 64 MB (war 2 MB → 413 bei >~1.5 MB); archive-"Gesendet"-Kopie persistiert jetzt Attachment-Metadaten + Content | main.rs, send.rs | 437 grün |
| 2 (Ordner-Reihenfolge) | Order wurde nie auf die Sidebar-Quelle (foldersByAccount) angewendet → `applySavedFolderOrder()` vor jedem setAccountFolders (Cache- + Fetch-Pfad) | +page.svelte | 300 grün |
| 6 (Mobile Compose) | Editor max-height + intern scrollbar (Toolbar bleibt unten fixiert); Mobile-Toolbar-Padding 3px→8px | ComposeWindow.svelte | 300 grün |
| 1 (Einstellungen-Menü) | Olares-seitig; App vorbereitet: `?pathto=`-Auswertung beim Start (Desktop nutzt pathto als Query-Param) | +page.svelte | 300 grün |

Playwright-MCP wurde gekillt (alter Prozess mit falschem Pfad /usr/bin/chromium-headless-shell).
Neustart von opencode nötig — opencode.json ist korrekt (`--executable-path /tmp/apk-root/usr/bin/chromium-headless-shell`), das Bundle liegt unter /home/opencode/.apk-root-bundle.tar.gz, extrahiert nach /tmp/apk-root.

## Issue 3: Markieren (Flagging) — Analyse

Backend IST vorhanden und verdrahtet:
- Route: `POST /api/v1/messages/flag` (server/src/api/mod.rs:60) → `flag_message` (messages.rs:1125)
- DB-Update: `update_is_flagged_in_folder` (messages.rs:1130-1135)
- IMAP: `set_imap_flag` (messages.rs:1146) → `client.toggle_flagged` (client.rs:545) → `mark_flag(uid, "\\Flagged", flagged, None)` (client.rs:516)
- Flag-Sync: scheduler.rs:312-354 (flag_refresh liest FLAGS vom IMAP und schreibt is_read/is_flagged in DB)

**PROBLEM 1 (Backend) — GEFIXT (2026-08-16):** `set_imap_flag` ruft jetzt
`mark_flag(uid, "\\Flagged", flagged, Some(folder_name))` auf (statt toggle_flagged → folder=None).
Die IMAP-Session ist geteilt (Sync + API), daher muss das Flag auf dem Quellordner der Mail gesetzt
werden; ein ungescoptes mark_flag schreibt in den aktuell selektierten Ordner.

**PROBLEM 2 (Frontend) — GEFIXT:** `handleToggleFlag` (+page.svelte:1967) prüft jetzt
`messagesFolder !== folder` und bricht bei Ordnerwechsel ab (die Liste kann kurz dem VORHERIGEN
Ordner gehören, solange ein neuer lädt). `mailbox.updateMessage` ist bereits folder-scoped (macht
den Abgleich gegen `s.folderId`).

**PROBLEM 3 (Suche) — GEFIXT:** `search_messages` (cache/messages.rs:1091) unterstützt jetzt
`is:flagged` (und `is:flag`) als Operator: alleinstehend → alle markierten Mails (m.is_flagged = 1),
kombiniert mit Text → Text-FTS AND Flag-Filter. Die FTS-JOIN wird nur bei Text-Termen erzeugt.

Frontend-Aufruf: tauri.ts:474 `flagMessageCmd(accountId, uid, folderName, flagged)` → POST /messages/flag.

## Issue 4: Löschen → IMAP

Delete-Handler messages.rs:1418. Verzweigung:
- `move_to_trash && sync_mode == "archive"` → `delete_message_archive_trash` (1583): lokaler Trash,
  Provider-Löschung in Queue (delete_queue::enqueue, 1652-1657) — EML bleibt, Löschung später.
- `move_to_trash` (mirror) → `delete_message_trash_mode` (1482): IMAP-move_message → Trash + DB-Update.
- sonst → `delete_message_permanent_delete` (1667): hart löschen.

**Root-Cause GEFUNDEN (2026-08-16, Live-Verifikation):** Kein Bug im Delete-Pfad selbst, sondern eine
**Session-Race im Sync-Task** (process_sync_task, scheduler.rs:958-1080):

1. `fetch_recent` (client.rs:220) macht KEIN SELECT — es verlässt sich auf die Session-Selektion.
2. `fetch_all_uids` (client.rs:364, aufgerufen scheduler.rs:1010) macht AUCH KEIN SELECT.
3. Der Sync-Task selektiert zwar `select_folder(folder_name)` davor (scheduler.rs:958), aber zwischen
   den `with_session_blocking`-Aufrufen laufen PARALLEL API-Operationen auf demselben ImapClient
   (fetch_body, move, flag, delete des Users) und schalten die Session auf einen anderen Ordner um.
4. Folge: `fetch_all_uids()` liefert die UIDs eines ANDEREN Ordners (z.B. Trash/Gelöscht) →
   `delete_messages_not_in("INBOX", [UIDs von Gelöscht])` löscht ALLE INBOX-Mails, deren UIDs dort
   nicht vorkommen — inkl. der frisch gespeicherten!

**Live-Beweis (2026-08-16 12:13-12:33):**
- JEDEN Sync-Zyklus: `Account 1: 1 neue Nachrichten in 'INBOX' synchronisiert` + unmittelbar danach
  `WARN [fetch_body_with_raw] Not found: Nachricht nicht gefunden` (die Mail wurde zwischen save und
  Body-Fetch schon wieder gelöscht).
- Live-DB: sync_state INBOX last_uid=26382 (Cursor wandert!), aber lokale INBOX = 0 Mails.
- Die Mails 26380-26382 ("Ihre eingereichten Dokumente in My AXA") liegen im Trash (folder 38) —
  vom User (oder GMX-Regel) in den Provider-Papierkorb verschoben, vom Sync als Trash-Mails gespiegelt.
- delete_queue: alle 6 Einträge state=deleted, attempts=0 (Worker lief korrekt am 10.08.).

**Warum der User "Mails bleiben auf dem Server" sieht:** Durch die Race verschwinden Mails aus der
lokalen INBOX (nie persistent gespeichert), der User löscht sie im UI → `delete_message_archive_trash`
findet keine lokale Zeile → kein delete_queue-Eintrag → Provider-Kopie bleibt. Der "Delete" wirkt nur
lokal/gar nicht, während der Server die Mail behält. Zusätzlich: Wenn er in GMX-Webmail löscht, holt
der Sync die Mail nicht zurück (Cursor-Skip) — aber die lokale Kopie wurde durch die Race schon weggeworfen.

**FIX — IMPLEMENTIERT (2026-08-16):**
1. scheduler.rs (FetchNew-Cleanup): `fetch_all_uids()` → `fetch_all_uids_in_folder(folder_name)`
   (client.rs:380, macht SELECT + UID SEARCH atomar unter dem Session-Lock) — DER kritische Fix.
2. scheduler.rs (removal_check): ebenso `fetch_all_uids_in_folder(folder_name)`.
3. client.rs: neue atomare Funktion `fetch_recent_in_folder(folder, since_uid, limit)`
   (SELECT + uid_search + uid_fetch in EINEM Lock); Scheduler nutzt sie statt select_folder + fetch_recent.
4. scheduler.rs (INBOX Body-Fetch): `fetch_body_with_raw(msg.uid)` →
   `fetch_body_with_raw_from_folder(msg.uid, Some(folder_name.to_string()))` (client.rs:417).

Verifiziert: 437 Rust-Tests grün (unverändert grün), compiliert ohne neue Warnungen.
`toggle_flagged` (client.rs:574) bleibt als pub-API für Tests bestehen, wird aber nicht mehr
im Produktionspfad verwendet.

Account: GMX (MarcBayer@gmx.de). Live-DB: /olares/rootfs/userspace/pvc-userspace-aimighty-m27rxxbszvgk1u2l/Data/relay/relay/index.db (via SSH olares@172.20.0.4, sudo sqlite3).

## Issue 5: Anhänge nicht sichtbar / nicht versendet (kritisch)

Analysiert + **2 Bugs gefixt (2026-08-16)**:

**Bug 1 (Senden):** Kein `DefaultBodyLimit` am axum-Router → Axum-Default 2 MB JSON-Body-Limit →
Base64-Anhänge > ~1.5 MB brachen beim Senden mit 413 ab. Fix: `DefaultBodyLimit::max(64 MB)` am
Router in main.rs:92. Die Sende-Pipeline selbst ist komplett verdrahtet (ComposeWindow.handleSend
→ +page.svelte handleSend → tauri sendMessage → POST /send → send.rs → SMTP-Multipart).

**Bug 2 (Anzeige in "Gesendet", archive mode):** Die lokale Gesendet-Kopie (send.rs, archive-Zweig)
schrieb den messages-INSERT OHNE `has_attachments` und OHNE `message_attachments`-Zeilen → neu
gesendete Mails zeigten im lokalen "Gesendet"-Ordner keine Anhänge (mirror mode bekam sie via
Provider-Sync). Fix: nach dem INSERT werden die Anhänge aus den raw_bytes geparst
(`parse_message_attachments`), als Metadaten persistiert (`reconcile_attachments`), `has_attachments=1`
gesetzt und der Base64-Content dedupliziert auf Disk gecacht (`cache_content_dedup`).

**Randbefund (Anzeige):** `GET /messages/body` mit cached body liefert Anhänge korrekt (live
verifiziert: account 2 uid 24 → 2 PDFs, account 1 uid 71 → IMG_3549.png). EML-Fallback-Pfad
(messages.rs:410) liefert `attachments: []` — falls ein Fix dort nötig ist: EML mit
`parse_message_attachments` auswerten.

## Issue 1: Einstellungen-Menü unter Top-Down → Relay → Einstellungen

Analysiert — **Olares-seitig, app-seitig bestmöglich vorbereitet (2026-08-16):**
- Olares-Desktop (beclab/Olares, BasicWindow.vue) hat KEIN generisches "Einstellungen"-Menü für
  Apps (Fensterrahmen = Icon/Titel/Min/Max/Close). Kein `navigate-to`-Emitter, kein settings-Feld im
  Application-CR (live geprüft: `spec.settings` = clusterScoped/customDomain/policy/title/version…),
  kein pathto-Link aus dem Desktop-Menü.
- App-seitig vorhanden: `/settings`-Route (vollständig: Tabs, mobile drill-down, Backup), SSE-
  Handler `navigate-to → /settings` (+page.svelte:89-92), Sidebar-Button `account-header-btn`
  (→ goto('/settings')).
- **Neu ergänzt:** `?pathto=<route>`-Auswertung beim App-Start (onMount, +page.svelte) — wenn
  Olares die App mit `?pathto=/settings` öffnet (Desktop nutzt `pathto` als Query-Param, s.
  BasicWindow.vue), springt die App direkt in die Einstellungen.
- Verbleibendes Risiko: Wenn das Desktop-Menü "Einstellungen" gar kein pathto sendet, ist ein
  Olares-Feature/Fix nötig (kein code-seitiger Weg in der App). Mit dem User verifizieren, ob das
  Menü nach dem Deploy funktioniert.

## Issue 2: Ordner-Reihenfolge persistent — GEFIXT (2026-08-16)

Root-Cause: Drag-Reorder existierte (handleFolderMouseDown, +page.svelte:1093+, persistiert nach
`relay_folder_order_<acct>`), ABER die Order wurde beim Laden NICHT auf die Sidebar-Quelle
angewendet:
1. Cache-Pfad (initWithAccount): `folderNames = parsed` (Server-Order) → `setAccountFolders` OHNE
   Order → Sidebar (rendert aus foldersByAccount) zeigt Server-Order.
2. IMAP-Fetch-Pfad: `setAccountFolders(names)` VOR der Order-Anwendung → foldersByAccount bleibt
   unreordered, nur die separate `folderNames`-Variable wurde reordered.

Fix: `applySavedFolderOrder(accountId, names)` (neu, +page.svelte) wird VOR jedem
`setAccountFolders` angewendet (Cache-Pfad + Fetch-Pfad). Reorder schreibt weiterhin
`relay_folder_order_<acct>` (per Account) — jetzt wird sie auch beim nächsten App-Start angewendet.
Neue Ordner werden ans Ende angehängt, entfernte verworfen.

## Issue 6: Mobile Compose Buttons — GEFIXT (2026-08-16)

ComposeWindow.svelte:
1. `.editor` bekommt `max-height: min(55vh, 480px)` + `overflow-y: auto` + `overscroll-behavior:
   contain` — das Textarea scrollt intern statt die sticky Senden-Toolbar aus dem Viewport zu
   drücken (bei langem Inhalt/Reply-Chain bleibt "Senden" immer unten sichtbar).
2. Mobile-Toolbar: `padding: 3px 16px …` → `8px 16px calc(12px + safe-area)` — Buttons sitzen nicht
   mehr an der oberen Trennlinie (44px-Touch-Targets mit Luft).

## Wichtige Umgebungs-Hinweise

- Live-App: https://mail.aimighty.olares.de/ (HTTP 200)
- Server/Backend lokal: /home/opencode/workspace/relay-one/server (Rust, cargo test --quiet --lib → 437 grün)
- Frontend: /home/opencode/workspace/relay-one/web (Vitest 300 grün, tsc grün)
- Port 3000 ist von opencode-web belegt; Rust-Smoke mit RELAY_BIND anderer Port.
- Playwright nach Neustart: `[ -x /tmp/apk-root/usr/bin/chromium-headless-shell ] || tar -xzf /home/opencode/.apk-root-bundle.tar.gz -C /tmp` — sollte opencode.json automatisch machen.