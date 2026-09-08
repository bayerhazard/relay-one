# HANDOFF — Relay Release-Kette 26.9.143 → 145

> Stand: 2026-09-08. Repo `bayerhazard/relay-one`. Live: **26.9.142**.
> Phase D (Voice-out) lokal committet (`40079d0`), undeployed.

## Kontext & Gates

- Versionsschema `26.9.X`.
- Gates je Release vor Commit (alle grün):
  - `cd server && cargo test`
  - `cd web && npx svelte-check --tsconfig ./tsconfig.json`
  - `cd web && npx vitest run`
- Release-Mechanik (pro Version): Bump an 5 Stellen → commit → `git tag v26.9.X` → Docker-Build (GH Actions) → `wrangler pages deploy` → `olares-cli market upgrade relay -s market.AImighty --watch`.

## Kette

| Release | Inhalt | Risiko |
|---|---|---|
| **26.9.143** | Phase D (Voice-out) + Reply-All/Compose-Fixes (#1,#2,#3) + #5 löschen | niedrig |
| **26.9.144** | DB-Migrations-Framework (#4) | mittel (isoliert) |
| **26.9.145** | KI-Analyse-Timing (Pre-Gen zuverlässig + Sofort-Footer) | niedrig |

---

## RELEASE 26.9.143 — Compose/Reply-Cluster (+ Phase D)

### #2 + #1 — Reply-All korrekt (Standard-Semantik)

Semantik: `to` = Original-Absender + Original-To minus eigene Adresse; `cc` = Original-CC minus eigene Adresse und minus bereits in `to` enthaltene.

1. `web/src/lib/utils/format.ts:69` `replyAllRecipients` → Rückgabe `{ to: string[]; cc: string[] }`:
   - `to` = Original-Absender + Original-To, minus eigene Adresse (`sender_email`, case-insensitive)
   - `cc` = Original-CC, minus eigene Adresse, minus bereits in `to` enthaltene; dedupliziert
2. `web/src/lib/components/ComposeWindow.svelte`:
   - Prop `replyCc?: string` ergänzen (`:25-52`)
   - `$effect` (`:290-314`): `to = replyTo ? replyTo.split(",").map(s => s.trim()).filter(Boolean) : []` (statt `[replyTo]`); `cc = replyCc ? replyCc.split(",").map(s => s.trim()).filter(Boolean) : []`; Bedingung + `lastReplyCc`-Tracker erweitern
3. `web/src/routes/+page.svelte`:
   - `doHandleReply` (`:1896-1897`): `const r = replyAllRecipients(...); replyTo = replyAll ? r.to.join(", ") : extractEmail(msg.from); replyCc = replyAll ? r.cc.join(", ") : ""` (neuer `$state replyCc`)
   - ComposeWindow-Aufruf (`:2602`): `replyCc={replyCc}` durchreichen; `replyCc` in allen Reset-Pfaden (`:1790`, `:1849`, `:1935`) leeren
4. **Backend-Härtung** `server/src/api/send.rs`:
   - Helper `fn split_addr_list(items: &[String]) -> Vec<String>` (splittet jedes Element an `,`, trimmt, verwirft leere)
   - Anwenden auf `req.to/cc/bcc` vor Tuple-Mapping (`:237`, `:240`, `:243`) **und** im `save_draft`-Pfad
   - `server/src/smtp/client.rs:193` bleibt (erwartet saubere Einzeladressen)

### #3 — voller Mailverlauf

- `ComposeWindow.svelte:621` `{@html sanitizeHtml(msg.html)}` (kein `.slice`/`...`); `:623` `<pre>{msg.text}</pre>` (kein `.slice`/`...`). `sanitizeHtml` bleibt. Container `.chain-scroll-area` scrollt.

### #5 — löschen

- `BACKLOG.md`: H3-Eintrag (`:284-286`) + Stand-Erwähnung entfernen. Chart-`trustedHostSuffix`-Logik bleibt unangetastet.

### Tests 143

- `format.test.ts`: `{to,cc}`-Form + CC-Trennung + Own-Filter + Dedup
- `ComposeWindow.test.ts`: Multi-Adressen-`replyTo` → mehrere Chips; `replyCc` → cc-Prefill; >1000-Zeichen-Original voll gerendert
- Rust `send.rs`: `split_addr_list(["a@x.com, b@y.com"]) → ["a@x.com","b@y.com"]`; `build_message_from_config` mit gesplitteten Recipients → 2 To-Header

---

## RELEASE 26.9.144 — DB-Migrations-Framework (#4)

`server/src/cache/db.rs:3 init_db`:

1. `PRAGMA user_version` lesen (i64).
2. **Baseline v1** = bestehende `CREATE TABLE` + **deduplizierte** `ALTER`-Liste (Duplikate bereinigen: `raw_path` 3×, `raw_sha256` 2×, `sync_mode` 2×, `trash_retention_days` 2×, `local_only` 2× → je 1×; Phase-D-TTS-Spalten folded in). Läuft nur wenn `user_version < 1`, dann `PRAGMA user_version = 1`. Baseline bleibt tolerant (`let _ =`) — Legacy-Catch-up auf 26.9.142-DBs.
3. **Vorwärts-Muster:** neue Änderungen als nummerierte Schritte:
   ```
   if user_version < 2 { <migration 2>; PRAGMA user_version = 2; }  // strikt, Fehler loggen
   ```
4. `migrate_messages_uid_constraint` / `migrate_provider_trash_folders` in die geordnete Kette einbetten (idempotent halten).

### Tests 144

- fresh DB → `user_version == 1`; 2× `init_db` = No-op (zweiter Aufruf ändert nichts)
- Legacy-Sim: Tabellen vorhanden, `user_version=0` → upgraded sauber, TTS-Spalten vorhanden
- keine doppelten `ADD COLUMN`-Statements mehr (statischer Test / grep)

---

## RELEASE 26.9.145 — KI-Analyse-Timing (nur Timing, Summary bleibt v1, INBOX-only)

### Phase 1 — Sofort-Aktions-Fußzeile

1. `server/src/cache/messages.rs`: `ai_followups: Option<String>` zur `MessageRecord`-Struktur (`:20`-Bereich) ergänzen; in `fetch_inbox_meta`-SELECT (`~:600`) + `row.get(...)`-Indices aufnehmen.
2. `server/src/api/messages.rs:70 message_to_json_meta`: `"ai_followups": m.ai_followups` (Roh-JSON-String) hinzufügen.
   - **Größen-Guard:** `ai_followups` nur für die neuesten ~200 Zeilen serialisieren (restliche `null` → On-Demand-Fallback), da Prepared-Cards mehrere KB/Mail belegen können; `folder_cache` puffert serverseitig.
3. `web/src/lib/services/tauri.ts`: `Message`-Typ `ai_followups?: string | null`.
4. `web/src/routes/+page.svelte` Followup-`$effect` (`:131-175`):
   - Vor Roundtrip: `if (msg.ai_followups)` → `JSON.parse` → `followups` setzen + `followupsCache.set(uid, actions)` → **ohne** `loadingBodyUid`-Spinner returnen.
   - Sonst heutiger On-Demand-Pfad (`getFollowups`).
   - Footer-Bedingung (`:2923`) bleibt; `followups.length > 0` greift sofort.
5. Fußzeile entkoppelt vom Body-Load: gecachte Aktionen zeigen unabhängig von `loadingBodyUid`.

### Phase 2 — zuverlässige Ankunfts-Pre-Gen (INBOX-only)

6. `server/src/sync/scheduler.rs:1441` Enqueue-Gate erweitern: `needs_summary = ai_summary.is_none() || (is_inbox && ai_followups.is_none())` (beide Spalten im Vor-Check lesen).
7. `server/src/api/ai.rs:316` Catch-up-Query verbreitern: `WHERE ... AND body_text IS NOT NULL AND (ai_summary IS NULL OR ai_followups IS NULL)`; auf INBOX beschränken (`folder_id` = INBOX).
8. **Auto-Catch-up bei Startup + IDLE-Zyklus** (statt nur Ordner-Öffnen): in `bootstrap`/Scheduler-Start einen spawned, low-priority Task, der `trigger_folder_summaries`-Logik für INBOX einmalig ausführt (LIMIT ~200). Wirkt zugleich als Retry für bei Ankunft LLM-busy übersprungene Mails.
9. `process_ai_summary` folgt weiter `is_inbox`-Bedingung (`:2331`) — andere Ordner bleiben on-demand.

### Tests 145

- Rust: `message_to_json_meta` enthält `ai_followups` (≤200-Zeilen-Guard); Gate-Logik `needs_summary` bei vorhandenem Summary + fehlenden Followups → true
- `+page.svelte`/Store: Auswahl mit `ai_followups` → `followups` sofort gesetzt, kein `getFollowups`-Aufruf (Mock-Spy 0×); ohne → 1×
- Startup-Catch-up: INBOX-Mails mit `ai_summary` aber `ai_followups IS NULL` werden eingequeueht

---

## Persistenz

1. `docs/handoff-26.9.143-145.md` anlegen (dieses Dokument).
2. `BACKLOG.md`: #1/#2/#3 → „geplant 26.9.143", #4 → „geplant 26.9.144", #5 **löschen**; neue Zeile „KI-Analyse-Timing → 26.9.145"; Stand-Zeile ergänzen.
3. Bestehender Commit `40079d0` (Phase D) + `BACKLOG.md` (uncommitted) bleiben; 143-Versionierung wird erst bei Release-Ausführung gesetzt.

## Ausführungsreihenfolge

1. **143-Code** (#2/#1/#3 + #5-Löschen) → Gates → Commit → Version-Bump → Deploy → `market upgrade`
2. **144** (Migration) → Gates → Commit → Bump → Deploy
3. **145** (Timing) → Gates → Commit → Bump → Deploy
4. `BACKLOG.md` je Release auf „live" ziehen.
