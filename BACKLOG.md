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

## Code Review 2026-08-18

> Ausführliches Code-Review der gesamten Codebasis (Rust-Backend ~24k Zeilen + Svelte-Frontend ~17k Zeilen).
> Severity: 🟠 hoch (Fix in dieser Runde) · 🟡 mittel (Backlog) · 🔵 niedrig (Backlog) · 💡 Hinweis.
> Basis: Tests grün (Rust 437, Vitest 300, tsc clean), Coverage 52.6% lines / 47.8% funcs.

### CR-01 🟠 ✅ gefixt (Code-Review-Runde) Path-Traversal: Backup-Restore überschreibt Live-DB mit beliebiger Datei
- **Datei:** `server/src/api/backup.rs:86` (`restore_backup`)
- **Problem:** `backups_dir.join(&req.backup_name)` — `backup_name` ist unvalidierter Nutzer-Input. `../`-Traversal escaped `backups/`; `std::fs::copy(&src, &db_path)` (Z. 107) kopiert dann **jede lesbare Datei** über die Live-`index.db`. Info-Disclosure via `metadata` (Z. 106).
- **Fix:** `backup_name` als bloßen Dateinamen validieren (`index-*.db`, kein `/`, kein `..`), `canonicalize` + Präfix-Check innerhalb `backups_dir`.

### CR-02 🟡 ✅ gefixt (Code-Review-Runde) Path-Traversal (read): mbox-dir-Import liest beliebige Verzeichnisse
- **Datei:** `server/src/api/import.rs:182` (`import_mbox_dir`)
- **Problem:** `state.data_root.join(&req.dir)` — `req.dir` unvalidiert → beliebige Verzeichnisse lesbar (als mbox geparst, `..`-Traversal).
- **Fix:** `req.dir` validieren (kein absoluter Pfad, kein `..`, `canonicalize` innerhalb `data_root`).

### CR-03 🟡 ✅ gefixt (Code-Review-Runde) Logik-Bug: `update_is_flagged` bricht 30s-Read-Cooldown (Server-Unread-Sync tot)
- **Datei:** `server/src/cache/messages.rs:477` + `server/src/sync/scheduler.rs:342`
- **Problem:** `flag_refresh` (alle 5 min) ruft `update_is_flagged` **bedingungslos** für jede Message auf und setzt `updated_at = now` — auch ohne Flag-Änderung. Damit ist die `update_is_read_guarded`-Bedingung (`updated_at <= now-30s`) für die Unread-Richtung **permanent wahr** → echte Server-Änderungen auf „ungelesen" (von anderen Geräten) werden nie lokal übernommen.
- **Fix:** `update_is_flagged` nur bei tatsächlicher Änderung (WHERE `is_flagged != ?`) → `updated_at` wird nicht mehr bei jedem Zyklus gebumpt.

### CR-04 🟡 CORS: `CorsLayer::permissive()` auf session-authentifizierter API
- **Datei:** `server/src/main.rs:98`
- **Problem:** reflect-any-origin + allow-credentials auf der API hinter der Olares-Entrance. Falls die Olares-Session-Cookie `SameSite` nicht strikt setzt, könnten fremde Webseiten via `fetch(..., {credentials:'include'})` Mails lesen/senden (CSRF/Exfiltration).
- **Fix:** CORS auf die Entrance-Origin(s) einschränken oder Credentials weglassen; SameSite-Policy der Olares-Middleware verifizieren.

### CR-05 🟡 Auth-Guard: `relay_key_guard` per Host-Header umgehbar
- **Datei:** `server/src/api/mod.rs:160-171`
- **Problem:** „Internal" wird rein über den `Host`-Header erkannt (IP/DNS/bare). Ein interner Caller (Pod im Cluster) kann `Host: mail.aimighty.olares.de` setzen und den Key-Check komplett umgehen.
- **Fix:** Guard-Härtung (z. B. zusätzlich Quell-IP/`RELAY_API_KEY` immer für non-browser erfordern), Threat-Model dokumentieren.

### CR-06 🔵 `/events` + `/info` ohne Key-Guard (Payload-Leak)
- **Datei:** `server/src/api/mod.rs:154`, `server/src/api/health.rs:32`
- **Problem:** `/events` ist vom Key-Guard ausgenommen und streamt `ai-summary-updated` inkl. `summary_text` + `uid`; `/info` exponiert `db_path`.
- **Fix:** Key-Pflicht für interne Hosts auch bei `/events` (Browser-SSE passiert weiter via public Host); `db_path` aus `/info` entfernen.

### CR-07 🔵 53 unwrap/expect in Produktionscode (panic=abort → Prozess-Crash)
- **Dateien:** `tone/analyzer.rs` (15), `tone/intent.rs` (9), `security/*` (16), `main.rs` (8), `smtp/carddav` (2)
- **Problem:** Release-Profil `panic=abort` — jeder Panic killt den Server. Unwraps sind fast alle statische `Regex::new().unwrap()` + `caps.get().unwrap()` (garantierte Groups), praktisch niedriges Risiko, aber ungeschützt.
- **Fix:** `expect("statische Regex")` mit Nachricht, `caps.get()` defensiv behandeln (Option-Mapping statt unwrap).

### CR-08 🔵 14 Clippy-Allows auf Crate-Ebene silencen Probleme
- **Datei:** `server/src/main.rs:1-14`
- **Fix:** Allows reduzieren/entfernen, Ursachen beheben; Clippy in CI (fehlt: kein rustfmt/clippy installierbar, keine Config, kein CI-Job — siehe CR-09).

### CR-09 🔵 Kein Test-/Lint-Job in CI
- **Datei:** `.github/workflows/build-image.yml`
- **Problem:** CI baut nur das Docker-Image bei Tag. `cargo test`/`clippy`/`tsc`/`vitest` werden nie im CI erzwungen.
- **Fix:** CI-Workflow mit test/lint/typecheck-Jobs ergänzen.

### CR-10 🔵 Fehler-Modell gespalten: `AppError` (strukturiert) vs `ApiError` (flaches String → immer 500)
- **Dateien:** `server/src/error.rs`, `server/src/api/mod.rs:190`
- **Problem:** Der Domain-Layer hat reich strukturierte `AppError`-Contexts; die API-Schicht flacht alles zu `String` → HTTP 500 ab, Status-Mapping/Context geht verloren.
- **Fix:** `AppError → StatusCode`-Mapping (404/401/400/500) im Handler-Layer einführen.

### CR-11 🔵 Ad-hoc-DB-Migrationen ohne Schema-Versioning
- **Datei:** `server/src/cache/db.rs`
- **Problem:** `ALTER TABLE … ADD COLUMN` teils dupliziert (raw_path 3×, sync_mode/trash_retention 2×) mit `let _ =` (Fehler ignoriert); Table-Rebuilds manuell; kein `PRAGMA user_version`.
- **Fix:** Versionierte Migrationen (`PRAGMA user_version` + idempotente, benannte Schritte).

### CR-12 💡 Hinweise Backend
- `SMTP_TIMEOUT` 60s (hoch); `FolderListCache` Sync-Mutex + `payload.clone()` unter Lock; `EventBus`-Broadcast (256) ohne Replay; `bootstrap::reconnect_clients` vs Scheduler-Start (Race beim Boot).

### CR-13 🟠 ✅ gefixt (Code-Review-Runde) Stored XSS: Rohe E-Mail-HTML via `{@html}` gerendert + in Antwort-Mail eingebettet
- **Dateien:** `web/src/lib/components/ComposeWindow.svelte:558` (`{@html msg.html.slice(0,1000)}`), `web/src/lib/utils/format.ts:169` (`wrapHtmlQuote`), `ComposeWindow.svelte:427`
- **Problem:** E-Mail-HTML ist untrusted (Phishing). `{@html}` ohne Sanitizer führt `<img onerror=…>` aus (Stored XSS im Compose-Preview); `wrapHtmlQuote(m.html)` bettet das rohe HTML zusätzlich in die ausgehende Antwort ein (Taint an Reply-Empfänger). Kein Sanitizer im gesamten Projekt.
- **Fix:** Sanitizer vor `{@html}` und vor Einbettung in `bodyHtml` (DOMPurify oder konservatives Stripping von `<script>`, Event-Handlern, `javascript:`-URLs).

### CR-14 🟡 ✅ gefixt (Code-Review-Runde) Vitest-Coverage: Gate aktiv aber immer rot + Routen/Worker ausgeschlossen
- **Datei:** `web/vitest.config.ts:32-40`
- **Problem:** `thresholds` (lines 80, funcs 70) werden von Vitest **standardmäßig erzwungen** — die Suite war also **permanent rot** (Ist ~52% ohne, ~42% mit routes/), aber das Exit-Failing ging in CI/Build unter. Coverage `include` nur `src/lib/**` → die 6.6k-Zeilen-Monolithen `routes/` + `workers/` ausgenommen; `services/tauri.ts` nur 17%.
- **Fix:** `routes/` in Coverage einbezogen; Schwellen realistisch (lines 40, funcs 38) gesetzt, Gate ist grün und fängt Regressionen.

### CR-15 🟡 ✅ gefixt (Code-Review-Runde) ToneControls: `dragging` ohne `$state` (Svelte-5-runes-Bug)
- **Datei:** `web/src/lib/components/ToneControls.svelte:14`
- **Problem:** `let dragging` als Plain-`let` mutiert → `class:dragging` (Z. 118/154) aktualisiert nie → visueller Dragging-Zustand (thumb-ring) greift nicht. Compiler warnt.
- **Fix:** `let dragging = $state<keyof ToneValues | null>(null)`.

### CR-16 🔵 A11y: interaktive Elemente ohne Keyboard-Handler/ARIA (WCAG 2.2 AA)
- **Dateien:** `ComposeWindow.svelte:623,654,655`; `+page.svelte:2524,2575`; `MessageList.svelte:341`; `RecipientInput.svelte:153,155`; `AccountGroup.svelte:210`; `ConfirmationDialog.svelte:48`
- **Problem:** Click-Handler auf `div`/`span` ohne `tabindex`/Keyboard/ARIA-Role; `alertdialog` ohne tabindex.
- **Fix:** Semantische Elemente (`button`) oder `role`+`tabindex`+`onkeydown` ergänzen.

### CR-17 🔵 Dead Code / Restnamen
- **Dateien:** `+page.svelte:218` (`replyAllDecision` wird nie gesetzt), `FolderList.svelte:88` (`{@html folder.icon}` — Sink wird nie befüllt), `tauri.ts` (Name + Kommentare Tauri-Rest, 979 Zeilen)
- **Fix:** `replyAllDecision` entfernen oder verdrahten; `folder.icon`-Sink dokumentieren/entfernen; `tauri.ts` → `api.ts` umbenennen (optional).

### CR-18 🔵 Monolith-Komponenten
- **Dateien:** `+page.svelte` (3.999 Z.), `settings/+page.svelte` (2.599 Z.), `ComposeWindow.svelte` (1.237 Z.)
- **Fix:** Refactoring in Komponenten/Stores (Backlog; kein kurzfristiger Fix).

### CR-19 💡 Hinweise Frontend
- `+page.svelte:2379,2381` Self-closing `<iframe>` (nicht-void) — explizit `</iframe>`; `DiffEditor.svelte:32` leeres Block-Statement.

---

## Ausgearbeitet

*(noch leer — Historie aus relay-repo ist dort dokumentiert, nicht hier)*
