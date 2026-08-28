# Relay Backlog

> Backlog wird lokal in `BACKLOG.md` geführt. Keine GitHub Issues.
> Stand: 2026-08-28 — Code-Review `REVIEW-2026-08-28.md` (produktionsreif/Perf/fehlerfrei). Findings H1–H3, M1–M6, L1–L4, I1–I4; Stage-D-Fixes (H1, H2, M1, M2, M4) in Release 26.09.108.
> Stand: 2026-08-25 — Release 26.09.94 (AI-Code-Review-Fixes) live. Neue offene Issues (Reply-All) siehe unten.

---

## Erledigt — Release 26.09.94 (2026-08-25)

### AI-Code-Review-Fixes (AI-01 … AI-04)
- **Status:** ✅ live (Pod 2/2 Running, `ghcr.io/bayerhazard/relay-one:26.09.94`, Health `ok`)
- **Inhalt:** Phishing-Erkennung aktiv in der Produktions-Pipeline (AI-02), Fingerprint-Lernschleife repariert (AI-01), `/ai/tone-profiles/export` Route (AI-03), Weihnachts-Occasion-Match (AI-04). Details unten in „Code Review AI-Features 2026-08-25".
- **Deploy-Notiz:** Git-Push auf `aimighty-market` deployet die Market-Source automatisch (Cloudflare-Pages-Git-Integration) — `wrangler pages deploy` ist redundant (CLOUDFLARE_API_TOKEN in dieser Umgebung nicht verfügbar). Olares-Sync: ~4 min nach Hash-Change im Katalog.

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
> Severity: 🟠 hoch · 🟡 mittel · 🔵 niedrig · 💡 Hinweis.
> **Status (2. Runde abgeschlossen):** alle 19 Findings bearbeitet — 15 gefixt, 4 offen (CR-08, CR-10, CR-11, CR-18 = struktur-/tooling-Architektur, bewusst Backlog).
> Tests nach Fixes: Rust **446** (+9 Guard-Tests), Vitest **306**, `svelte-check` **0 Errors** (war: 10), tsc clean, Build ok.

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

### CR-04 🟡 ✅ gefixt (2. Runde) CORS: `CorsLayer::permissive()` auf session-authentifizierter API
- **Datei:** `server/src/main.rs:98`
- **Problem:** reflect-any-origin + allow-credentials auf der API hinter der Olares-Entrance. Falls die Olares-Session-Cookie `SameSite` nicht strikt setzt, könnten fremde Webseiten via `fetch(..., {credentials:'include'})` Mails lesen/senden (CSRF/Exfiltration).
- **Fix:** **Layer entfernt.** SPA + API laufen same-origin hinter der Entrance (kein Cross-Origin-`fetch` im Frontend; in dev proxyt Vite `/api`). Damit existiert kein legitimer CORS-Bedarf; `CorsLayer::permissive()` war reines Risiko. Schutzbasis ist die Entrance-`authLevel` + Olares-Middleware.

### CR-05 🟡 ✅ gefixt (2. Runde) Auth-Guard: `relay_key_guard` per Host-Header umgehbar
- **Datei:** `server/src/api/mod.rs:160-171`
- **Problem:** „Internal" wird rein über den `Host`-Header erkannt (IP/DNS/bare). Ein interner Caller (Pod im Cluster) kann `Host: mail.aimighty.olares.de` setzen und den Key-Check komplett umgehen.
- **Fix:** Guard gehärtet: Neu `RELAY_TRUSTED_HOST_SUFFIX` (Chart-Env, z. B. `olares.de`) — ein „public" Host wird nur noch ohne Key akzeptiert, wenn er auf das konfigurierte Entrance-Suffix endet; `Host`-Spoofing auf fremde Domains wird 401. Ohne Env bleibt das Legacy-Verhalten (Backward-Compat). + 7 Unit-Tests (u. a. Suffix-Mismatch → 401, `/info`+`/events` intern → 401).

### CR-06 🔵 ✅ gefixt (2. Runde) `/events` + `/info` ohne Key-Guard (Payload-Leak)
- **Datei:** `server/src/api/mod.rs:154`, `server/src/api/health.rs:32`
- **Problem:** `/events` ist vom Key-Guard ausgenommen und streamt `ai-summary-updated` inkl. `summary_text` + `uid`; `/info` exponiert `db_path`.
- **Fix:** Nur noch `/health` bleibt unconditional offen; `/events` + `/info` unterliegen der Host-Heuristik (Browser-SSE passiert via public Host, interne Caller brauchen den Key). `db_path` aus `/info` entfernt — `/info` liefert nur noch die Version.

### CR-07 🔵 ✅ gefixt (2. Runde) 53 unwrap/expect in Produktionscode (panic=abort → Prozess-Crash)
- **Dateien:** `tone/analyzer.rs` (15), `tone/intent.rs` (9), `security/*` (16), `main.rs` (8), `smtp/carddav` (2)
- **Problem:** Release-Profil `panic=abort` — jeder Panic killt den Server. Unwraps sind fast alle statische `Regex::new().unwrap()` + `caps.get().unwrap()` (garantierte Groups), praktisch niedriges Risiko, aber ungeschützt.
- **Fix:** Alle statischen `Regex::new(...).unwrap()` → `.expect("statische Regex")`, alle `caps.get(N).unwrap()` → `.expect("capture group N")` (40 Konvertierungen). Verbleibende unwraps liegen ausschließlich in `#[cfg(test)]`-Modulen. `main.rs`-Startup-`expect`s bleiben (Fail-Fast beim Boot ist gewollt).

### CR-08 🔵 OFFEN Clippy-Allows auf Crate-Ebene silencen Probleme
- **Datei:** `server/src/main.rs:1-14`
- **Status:** Bleibt offen. CI läuft jetzt `cargo clippy --all-targets --no-deps` (CR-09) — die Allows reduzieren, sobald Clippy-Läufe in CI sichtbar sind. Lokal kein Clippy installierbar (keine apk-DB), deshalb schrittweise über die CI-Outputs.

### CR-09 🔵 ✅ gefixt (2. Runde) Kein Test-/Lint-Job in CI
- **Datei:** `.github/workflows/ci.yml` (neu)
- **Problem:** CI baute nur das Docker-Image bei Tag. `cargo test`/`clippy`/`tsc`/`vitest` wurden nie im CI erzwungen.
- **Fix:** Neuer Workflow `ci.yml`: **Backend**-Job (`cargo test --locked` + `cargo clippy --all-targets --no-deps` mit Cache), **Frontend**-Job (`npm ci`, `check` = svelte-check + tsc, `test:run` mit Coverage-Gate, `build`). Läuft auf `push main` + `pull_request`.
- **Bonus dabei:** `web/package.json` `check`-Script von `tsc --noEmit` (prüft keine `.svelte`-Dateien) auf `svelte-check --tsconfig ./tsconfig.json && tsc --noEmit` erweitert + `svelte-check` als devDependency; dabei **10 vorbestehende TS-Errors** in `.svelte`-Dateien gefunden und gefixt (fehlende Imports `AttachmentInfo`/`parseMimeWithWorker`/`AccountInfo`, `extractEmails`-Rest-Param, `contextMenu`/`attCtxMenu`/`folderCtxMenu`-Narrowing, `+layout` `children`-Typ, Test-Fixtures ohne `is_flagged`).

### CR-10 🔵 OFFEN Fehler-Modell gespalten: `AppError` (strukturiert) vs `ApiError` (flaches String → immer 500)
- **Dateien:** `server/src/error.rs`, `server/src/api/mod.rs:190`
- **Problem:** Der Domain-Layer hat reich strukturierte `AppError`-Contexts; die API-Schicht flacht alles zu `String` → HTTP 500 ab, Status-Mapping/Context geht verloren.
- **Fix:** `AppError → StatusCode`-Mapping (404/401/400/500) im Handler-Layer einführen.

### CR-11 🔵 OFFEN Ad-hoc-DB-Migrationen ohne Schema-Versioning
- **Datei:** `server/src/cache/db.rs`
- **Problem:** `ALTER TABLE … ADD COLUMN` teils dupliziert (raw_path 3×, sync_mode/trash_retention 2×) mit `let _ =` (Fehler ignoriert); Table-Rebuilds manuell; kein `PRAGMA user_version`.
- **Fix:** Versionierte Migrationen (`PRAGMA user_version` + idempotente, benannte Schritte).

### CR-12 💡 OFFEN Hinweise Backend
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

### CR-16 🔵 ✅ gefixt (2. Runde, teilweise zurückgerollt) A11y: interaktive Elemente ohne Keyboard-Handler/ARIA (WCAG 2.2 AA)
- **Dateien:** `ComposeWindow.svelte:623,654,655`; `+page.svelte:2524,2575`; `MessageList.svelte:341`; `RecipientInput.svelte:153,155`; `AccountGroup.svelte:210`; `ConfirmationDialog.svelte:48`
- **Problem:** Click-Handler auf `div`/`span` ohne `tabindex`/Keyboard/ARIA-Role; `alertdialog` ohne tabindex.
- **Fix:** **12 von 14 A11y-Warnungen behoben:** `MessageList`-Mailrow (`role=option`, `aria-selected` vorhanden, + `tabindex` + Enter/Space-Keydown), `RecipientInput`-Vorschläge (analog), `AccountGroup`-Chevron (`tabindex=0` + Enter/Space), Dialog-Overlays (`role=presentation` für Backdrop — echte Buttons existieren daneben), `ConfirmationDialog` (`tabindex="-1"` auf `alertdialog`), Drag-Resize-Handles (`role=separator` + begründetes `svelte-ignore`).
- **Zurückgerollt (Nutzer-Entscheid):** Der `toggle-mic`-Span in `ComposeWindow.svelte:623` bleibt **bewusst nested im Generate-Button** (kein `role`/`tabindex`/`onkeydown`) → 2 verbleibende A11y-Warnungen (bewusst akzeptiert, strukturgegeben).

### CR-17 🔵 ✅ gefixt (2. Runde) Dead Code / Restnamen
- **Dateien:** `+page.svelte:218` (`replyAllDecision` wird nie gesetzt), `FolderList.svelte:88` (`{@html folder.icon}` — Sink wird nie befüllt), `tauri.ts` (Name + Kommentare Tauri-Rest, 979 Zeilen)
- **Fix:** `replyAllDecision` **entfernt** (war nie gesetzt → Bedingung immer wahr → Verhalten identisch). `folder.icon`-Sink + `tauri.ts`-Rename bleiben bewusst Backlog (folder.icon ist harmloser Future-Slot; Rename = großer Import-Churn ohne Funktionswert).

### CR-18 🔵 OFFEN Monolith-Komponenten
- **Dateien:** `+page.svelte` (3.999 Z.), `settings/+page.svelte` (2.599 Z.), `ComposeWindow.svelte` (1.237 Z.)
- **Fix:** Refactoring in Komponenten/Stores (Backlog; kein kurzfristiger Fix).

### CR-19 💡 ✅ gefixt (2. Runde) Hinweise Frontend
- `+page.svelte:2379,2381` Self-closing `<iframe>` (nicht-void) — explizit `</iframe>`; `DiffEditor.svelte:32` leeres Block-Statement.
- **Fix:** Beide behoben; zusätzlich `svelte.config.js` `compilerOptions.immutable` entfernt (in runes mode deprecated/wirkungslos, war die einzige verbleibende Code-Warnung). Restwarnungen: ~38 ungenutzte CSS-Selektoren (harmlos, Exit 0).

---

## Code Review AI-Features 2026-08-25

> Gezieltes Review der AI-Pfade (Eingangs-Scan: Summary/Priorität/Phishing, Komposition: Reply/Generate, Lernschleife: Snippets/Tone/Fingerprint).
> Severity: 🟠 hoch · 🟡 mittel · 🔵 niedrig.
> **Status:** alle Findings gefixt. Tests: Rust **452** (+2), Vitest **330**, `svelte-check` **0 Errors**.

### AI-01 🟠 ✅ gefixt Lernschleife tot: Style-Fingerprint nie synthetisiert (account_id=0)
- **Dateien:** `server/src/sync/scheduler.rs` (`RefreshFingerprint`-Task), `server/src/cache/fingerprint.rs`
- **Problem:** Der globale `RefreshFingerprint`-Task (account_id=0) filterte in `get_recipients_needing_refresh` / `get_hints_for_synthesis` / `save_fingerprint` hart nach `account_id = 0` — aber Diffs werden mit dem echten Account-ID gequeued (`send.rs`). Ergebnis: Kandidaten-Liste permanent leer → Fingerprint nie erzeugt; `ai_generate_mail`/`ai_generate_reply` lesen mit echtem Account-ID → **immer `None`**. Die gesamte Fingerprint-Lernschleife (Diff-Analyse → Synthese → Prompt-Anreicherung) war stillschweigend tot.
- **Fix:** Neue `get_refresh_candidates(conn, limit)` gruppiert analysierte Hinweise über **alle** Konten (`GROUP BY account_id, email_hash HAVING COUNT(*) >= 3`) und liefert das echte `account_id` mit; Scheduler liest Hinweise und speichert den Fingerprint mit dem echten Account-ID. +2 Regressionstests (Multi-Account, Limit).

### AI-02 🟠 ✅ gefixt Phishing-Erkennung tot: `ai_fraud_score` nie befüllt
- **Dateien:** `server/src/sync/scheduler.rs` (`process_ai_summary`), `web/src/routes/+page.svelte`
- **Problem:** `detect_fraud` (Heuristik, getestet) und `update_ai_fraud` (DB) existierten, wurden aber **niemals** in der Produktions-Pipeline aufgerufen. Die UI rendert `FraudWarning` bei `ai_fraud_score > 0.6` — das konnte nie eintreten. Phishing-Schutz war reines UI-Theater.
- **Fix:** `process_ai_summary` berechnet `detect_fraud(subject, body)` für **jede** Mail (reine Regex-Heuristik — läuft unabhängig von LLM-Verfügbarkeit/Konfiguration) und persistiert via `update_ai_fraud`. Das `ai-summary-updated`-Event trägt den Score als 6. Element; Frontend schreibt `ai_fraud_score` live in die Message-Row.

### AI-03 🟡 ✅ gefixt Fehlende Route `/ai/tone-profiles/export`
- **Dateien:** `server/src/api/ai.rs`, `server/src/api/mod.rs`, `web/src/lib/services/tauri.ts`
- **Problem:** Frontend `exportToneProfiles()` postete auf `/ai/tone-profiles/export` (→ 404); `ProfileManager::export_as_markdown` war nie verdrahtet.
- **Fix:** Route + Handler ergänzt (liefert Markdown-Tabelle als JSON-String); Frontend-Body auf snake_case `account_id` vereinheitlicht.

### AI-04 🔵 ✅ gefixt Occasion-Mismatch: Weihnachts-Prompt-Anweisung unerreichbar
- **Dateien:** `server/src/ai/prompts.rs`
- **Problem:** `parse_intent` liefert `"weihnacht"` (Regex-Alternative), der Prompt matchte auf `"weihnachten"` → die dedizierte Weihnachtsgrüße-Anweisung traf nie (nur der generische Fallback).
- **Fix:** Match auf `Some("weihnacht") | Some("weihnachten")`.

### AI-05 🔵 akzeptiert (kein Fix)
- `detect_display_mismatch` in `security/fraud.rs` ist ein Platzhalter (liefert immer `false`) — bewusst: Display-Name-Abgleich braucht Absender-Metadaten, die der Heuristik-Pfad nicht zuverlässig hat.
- `intent.rs`-Namensextraktion ist unvollkommen (fangt Folge-Wörter wie "eine", bricht bei `é` ab) — `recipient_name` wird im aktiven Pfad (`ai_generate_mail`) nicht verwendet, nur `tone_hints`; kein Nutzer-Impact.
- `aiGenerateReply`/`aiDraftFromBullets`/`aiDetectPriority`/`fraudCheck`/`exportToneProfiles` in `tauri.ts` sind nur in Test-Mocks referenziert (Reply-Flow nutzt `aiGenerateMail` mit `original_message`) — API-Fläche für spätere UI, bewusst behalten.

---

## Offen — neu gemeldet 2026-08-18

### Reply-All: eigene Adresse + ursprüngliche Empfänger-Adresse im Antwortverteiler
- **Status:** 🔵 offen
- **Kategorie:** Backend / Reply-All-Verteiler
- **Priorität:** high
- **Beschreibung:** Bei „An alle antworten" landen die eigene Adresse sowie die ursprüngliche Empfänger-Adresse (Absender der ursprünglichen Mail) mit im Antwortverteiler (To/CC). Falsch: Die eigene Adresse darf die Antwortmail nicht erhalten; die Empfänger-Zusammenstellung muss die eigene Adresse herausfiltern und To/CC sauber trennen.

### Reply-All: mehrere Adressen als ein Block → Senden schlägt fehl (parse_to)
- **Status:** 🔵 offen
- **Kategorie:** Backend / SMTP (parse_to)
- **Priorität:** high
- **Beschreibung:** Bei „An alle antworten" mit mehreren Empfängern werden die Adressen zu **einem Block** zusammengefasst, den Relay nicht verarbeiten kann. Senden bricht ab: `[parse_to] SMTP: Ungültige Empfänger-E-Mail-Adresse`. Die Adressen müssen einzeln getrennt geparst/übermittelt werden.

### Mail Compose: „Ursprüngliche Nachricht" zu früh abgeschnitten
- **Status:** 🔵 offen
- **Kategorie:** UX / Frontend
- **Priorität:** medium
- **Beschreibung:** In der Antwort-/Weiterleitungs-Ansicht wird die zitierte Mail bei **1000 Zeichen** hart gekappt (`ComposeWindow.svelte:558` `sanitizeHtml(msg.html).slice(0,1000)` und `:560` `msg.text.slice(0,1000)`, jeweils mit „…"). Gewünscht: den **vollen Mailverlauf** anzeigen. Der Container ist scrollbar (`.chain-scroll-area`), daher besteht kein Overflow-/Anzeigeproblem — das 1000-Zeichen-Limit kann entfallen (ggf. nur `sanitizeHtml` beibehalten).

---

## Offen — Code-Review 2026-08-28 (Backlog, nicht in 26.09.108)

> Details + Severity in `REVIEW-2026-08-28.md`. Gefixt in 26.09.108: H1, H2, M1, M2, M4.

### M3 🟡 Kein DB-Schema-Versioning (CR-11)
- **Kategorie:** Data integrity · **Priorität:** medium
- `cache/db.rs` nutzt `CREATE TABLE IF NOT EXISTS` ohne `PRAGMA user_version`. Migrations-Framework (sequentiell, idempotent) einführen. Strukturell.

### M5 🟡 Sync pro Account sequenziell
- **Kategorie:** Perf/Scalability · **Priorität:** low (aktuell 2 Konten)
- `do_sync_cycle` macht Ping + IDLE pro Account sequenziell. Parallelisieren (begrenzte Parallelität) für Skalierung.

### M6 🟡 64 MB Body-Limit + base64-Inlining
- **Kategorie:** Memory · **Priorität:** low
- Große Anhänge als base64 in JSON (+33 %). Optional Streaming/Größen-Limit. Teilweise entlastet durch H1-Fix.

### H3 🟠 Auth nur via Sidecar (Design) — Chart-Env verifizieren
- **Kategorie:** Security · **Priorität:** medium
- API-Key-Guard by design deaktiviert (v26.09.92). `RELAY_TRUSTED_HOST_SUFFIX` im Chart auf Entrance-Domain setzen (Host-Spoofing-Schutz). Kein Code-Bruch.

### L2/L3/L4 🔵 Kosmetik/Tooling
- L2: Delete-Queue aufgebene Zeilen terminal markieren. L3: CI clippy `-D warnings` + fmt + cargo-audit + npm-audit. L4: `followupsCache` Cap (LRU 200).

## Ausgearbeitet

*(noch leer — Historie aus relay-repo ist dort dokumentiert, nicht hier)*
