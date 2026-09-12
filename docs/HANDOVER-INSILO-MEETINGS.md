# Handover: Relay "Meetings"-Bereich mit Insilo-Anbindung

> **Zielgruppe:** Umsetzungs-Modell (qwen3.8-27b) in einer frischen Session.
> **Datum:** 2026-09-12 · **Status:** Konzept final abgestimmt, Umsetzung offen.
> **Repos:** `/home/opencode/workspace/relay-one` (Relay) und `/home/opencode/workspace/insilo` (Insilo). Beide liegen auf dem Stand, der hier dokumentiert ist — vor Beginn `git fetch` + Stand vergleichen (AGENTS.md-Regel).

---

## 0. Regeln für die Umsetzung (bitte VOR allem befolgen)

1. **Exakt EIN Tool-Call pro Antwort**, niemals Task/Subagents. Kleine Schritte. (Regelwerk `~/.config/opencode/sequential-rules.md`)
2. **Große Dateien nie komplett lesen** — nur `grep`/`sed -n`-Ausschnitte. `relay-one/web/src/routes/+page.svelte` hat 4571 Zeilen — NICCHT am Stück lesen.
3. **Dateien schrittweise schreiben** (Skeleton → Edit je Abschnitt), nie ein großes Write in einem Rutsch.
4. **Reihenfolge der Meilensteine einhalten** (M1→M5, Abschnitt 9). Jeder Meilenstein endet mit grünem Verify (Abschnitt 10).
5. Bei jedem Release: **Version an allen 5 Stellen identisch** (AGENTS.md §2.4) und **zuerst committen, dann deployen**.
6. **Nichts an `metadata.name`, `entrance.name`, `entrance.host`, K8s-Namen ändern** (AppID-/URL-Brecher, AGENTS.md §1.3).
7. Unklarheit → nicht raten, sondern im jeweiligen `docs/`-Ordner nachschlagen oder Aufgabe als "offene Frage" (Abschnitt 12) notieren und mit der nächsten Teilaufgabe weitermachen.

## 1. Mission

Relay (Mail/Contacts/Calendar/Tasks auf Olares) bekommt einen **fünften Bereich "Meetings"**:

- Insilo (Meeting-Intelligenz: Transkription + LLM-Zusammenfassung, FastAPI/Postgres/Celery) schreibt pro fertig zusammengefasstem Meeting eine **Markdown-Datei** in ein gemeinsames Verzeichnis.
- **Relay pollt** dieses Verzeichnis (Robustheit vor Latenz — Meetings sind nicht zeitkritisch), ingested die Summaries in SQLite inkl. FTS5-Volltextsuche und zeigt sie im neuen Web-Bereich.
- Phase 2: KI-Flows — Minutes-Mail an Teilnehmer (Draft+Bestätigung), To-do-Übernahme in Relay-Tasks, Wochen-Digest, Q&A über alle Meetings.

**Warum Polling statt Webhook (entschieden):**
Ein Scan heilt *immer* — verpasste Events, Relay-Neustarts, Insilo-Ausfälle. Kein Retry-Management, keine Cross-App-API, kein Auth-Problem (der Olares-Envoy-Sidecar gate-t app-zu-app-Traffic). Insilos Webhook-Infrastruktur (`meeting.ready` mit HMAC/Retries) bleibt für externe Konsumenten; für Relay ist sie nur optionaler Latenz-Booster (M6, nicht Teil dieses Handovers).

---

## 2. Architektur-Entscheidung: gemeinsames Verzeichnis = `appCommon` (WICHTIGSTE KORREKTUR)

**Die ursprüngliche Annahme "Insilo legt unter `/data/insilo` ab, Relay liest dort" funktioniert NICHT direkt:**

- Insilos Datenverzeichnis ist `.Values.userspace.appData` = `/olares/userspaces/<user>/Data/insilo` — **App-privat** (Olares hängt den App-Namen automatisch an; siehe `insilo/olares/values-olares-stub.yaml`).
- Relays `.Values.userspace.appData` resolved nach `.../Data/relay`. Auf das Insilo-Verzeichnis zu zeigen wäre ein nicht gedeckelter Hardcoded-Pfad (bricht bei User-/Installationswechsel, Market-Lint ablehnend).
- **Der vorgesehene Cross-App-Ordner ist `appCommon`** (Olares `drive/Common`, JuiceFS-backed → cross-node + gesichert, verfügbar ab Olares 1.12.6; beide Apps pinnen schon `>=1.12.6-0`). Jede App deklariert `permission.appCommon: true` im OlaresManifest und mountet denselben Ordner.

**Kontract:**
| | |
|---|---|
| Host-Pfad (Olares rendert ihn) | `{{ .Values.userspace.appCommon }}/insilo-meetings` |
| Insilo-Container-Pfad (Schreiben) | `/app/common/insilo-meetings` (Neu-Mount, Abschnitt 4) |
| Relay-Container-Pfad (Lesend) | `/data/insilo` (Mount, `readOnly: true` — erhält die Nutzer-Vorstellung "liegt unter /data/insilo") |
| Konfig-Env Relay | `RELAY_INSILO_DIR=/data/insilo` |
| Konfig-Env Insilo | `INSILO_MEETING_EXPORT_DIR=/app/common/insilo-meetings` |

Insilo schreibt **nur** in `insilo-meetings/` darunter; Relay liest **nur** und schreibt niemals dorthin (read-only-Mount erzwingt es).

## 3. Datenformat-Contract (Dateien)

Eine Datei pro Meeting, Encoding UTF-8, flaches Verzeichnis (keine Subdirs):

- **Dateiname:** `<YYYY-MM-DD>T<HH>_<MM>--<insilo-id-8>.md` (z. B. `2026-09-12T14_30--a1b2c3d4.md`), UTC-Werte aus `recorded_at`. Deterministisch aus Meeting ableitbar; die 8 UUID-Zeichen machen die endgültige ID.
- **YAML-Frontmatter** (nur die unten gezeigte Teilmenge — Relay parst sie ohne YAML-Lib, Abschnitt 6.2; keine verschachtelten Strukturen!):

```markdown
---
insilo_id: "b1e4f0aa-1234-4abc-9def-0123456789ab"
title: "Kundenmeeting Müller GmbH"
recorded_at: "2026-09-12T14:30:00+00:00"
duration_min: 47
language: "de"
participants: ["Anna Weber", "Dr. Marc Böhm"]
tags: ["Mandant Müller", "Vertrag"]
template: "Standard-Notiz"
source_url: "https://insilo.kaivostudio.olares.de/meetings/b1e4f0aa-1234-4abc-9def-0123456789ab"
schema: 1
---

(Hier ab: der bestehende Insilo-Markdown-Body, unverändert —
 `render_meeting_markdown()` aus `backend/app/exports/markdown.py`,
 inkl. GFM-Checklisten für `naechste_schritte` mit `verantwortlich`/`frist`/`beschluss`.)
```

- **Atomarität (PFLICHT):** Datei als `<name>.md.tmp` schreiben, `fsync`, dann `os.rename()` auf `<name>.md`. Rename ist auf JuiceFS atomar → Relay sieht nie halbe Dateien. Zusätzlich ignoriert Relay Dateien mit `mtime < 15 s` (Backstop) und alle `.tmp`-Dateien.
- **Änderung:** Insilo überschreibt bei `meeting.updated` dieselbe Datei (gleicher Name, da ID-gleich). Relay erkennt das am SHA-256-Wechsel → Upsert.
- **Löschung:** Insilo löscht die Datei bei Meeting-Delete. Relay sieht fehlende Datei → Soft-Delete (`deleted=1`, Eintrag bleibt für Historie/Suche).
- **Stabilität:** `schema: 1` = Version des Contracts. Breaking-Changes nur mit Schema-Bump + Kompatibilitätscode in Relay.

---

## 4. Teil A — Insilo: File-Drop-Export (Repo `insilo`)

**Existierende Bausteine (BITTE WIEDERVERWENDEN, nicht neu erfinden):**

| Baustein | Datei | Zweck |
|---|---|---|
| `render_meeting_markdown(*, meeting, transcript, summary, tags, template_name, include_transcript) -> str` | `backend/app/exports/markdown.py:306` | kanonisches Meeting-Markdown (Body) |
| `_load_meeting_payload(conn, meeting_id, event)` | `backend/app/tasks/notify.py:87` | lädt meeting/transcript/summary/tags-Rows und baut Payload inkl. `markdown` (Zeile 180/208) — **Vorlage für den Row-Load** |
| `_do_summarize()` setzt Status `ready` und feuert `notify_webhook` | `backend/app/tasks/summarize.py:613` und `:623` | **Hook-Punkt** für den File-Drop |
| Celery-App / `shared_task`-Muster | `backend/app/worker.py`, alle `backend/app/tasks/*.py` | Task-Registrierung |
| Settings-Klasse (pydantic) | `backend/app/config.py` (`storage_local_path: str = "/app/data/audio"` Zeile 31 als Muster) | neue Settings |

**A1 — Neue Settings** in `backend/app/config.py` (Settings-Klasse, Defaults!):
```python
meeting_export_dir: str = ""      # "" = deaktiviert; prod: /app/common/insilo-meetings
```

**A2 — Neuer Task** `backend/app/tasks/export_file.py`:
- `@shared_task(name="export_meeting_file") def export_meeting_file(meeting_id: str) -> dict:` — Wrapper um `asyncio.run(_do_export(UUID(meeting_id)))`, Muster wie `notify.py`.
- `_do_export(conn)`:
  1. Early-return `{"status": "skipped"}`, wenn `settings.meeting_export_dir` leer.
  2. Rows laden: dieselben Queries wie `_load_meeting_payload` (meeting, transcript, summary, tags, template-Name). Kapsle den Load in eine gemeinsame Helper, wenn trivial möglich; sonst dupliziere die Queries bewusst (read-only, unkritisch).
  3. Body = `render_meeting_markdown(meeting=..., transcript=..., summary=..., tags=..., template_name=..., include_transcript=False)` — **Transcript-Teil weglassen** (Datenschutz: Relay braucht nur die Summary; Begründung als Kommentar sichern).
  4. Frontmatter-Block exakt nach Abschnitt 3 bauen (Manuell als String; `json.dumps` für die Listen-Values; `ensure_ascii=False`; alle Strings quote-safe escapen: `"` im Titel → durch `'` ersetzen oder `\"` escapen — Frontmatter-Parse in Relay ist simpel, Masking-Pflicht liegt bei Insilo).
  5. Pfad: `Path(settings.meeting_export_dir) / filename` mit `mkdir(parents=True, exist_ok=True)`; Filename aus `recorded_at` (UTC!) + `str(id)[:8]`.
  6. Atomar schreiben (Abschnitt 3): `.md.tmp` → `os.replace(tmp, final)` (os.replace = atomares Rename, POSIX).
  7. Rückgabe `{"status": "ok", "path": str(final)}`; Exceptions loggen, **nicht** in den Celery-Retry-Strudel schicken (Meeting bleibt sonst hängen) — max 3 Retries via `self.retry(..., max_retries=3)`.
- **Löschen:** eigener Task `remove_meeting_file(meeting_id: str)` — findet die Datei per UUID-Präfix-Suffix (`*--<id8>.md`) im Exportdir und löscht sie.

**A3 — Hook** in `backend/app/tasks/summarize.py` direkt neben Zeile 623 (`notify_webhook`-Fanout), gleicher Stil:
```python
_app.send_task("export_meeting_file", args=[str(meeting_id)])
```
Zusätzlich:
- Nach `meeting.updated`-Übergängen (Status erneut `ready` nach Edit/Re-Summarize — Codestelle in `routers/meetings.py` suchen, grep `meeting.updated` / `enqueue(`) ebenfalls `export_meeting_file` senden.
- Im Meeting-Delete-Handler (`routers/meetings.py`, grep `meeting.deleted`) `remove_meeting_file` senden.
- **Backfill-Endpunkt** `POST /api/v1/meetings/export-backfill` (Router `meetings.py`, auth wie Rest): queued `export_meeting_file` für alle Meetings mit Status `ready` (max 500, newest first). Damit existierende Meetings nachholbar sind, ohne Insilo-DB-Scanner.

**A4 — Deployment** `insilo/olares/templates/deployment-backend.yaml`:
- volumes: neuer Eintrag
  ```yaml
  - name: app-common
    hostPath:
      path: {{ .Values.userspace.appCommon }}
      type: DirectoryOrCreate
  ```
- Beide Containers (backend + worker — grep `volumeMounts`, Zeilen 42/255): Mount `app-common` → `/app/common` **readonly: false** (Insilo schreibt). Env: `INSILO_MEETING_EXPORT_DIR: "/app/common/insilo-meetings"` (Env-Block ab Zeile 221).
- `insilo/olares/OlaresManifest.yaml` `permission:`-Block (Zeile 88): `appCommon: true` ergänzen (sonst Lint-Fehler "app-data cross-check", olares-chart-Lint-Regel).
- `insilo/olares/values-olares-stub.yaml`: `appCommon: /olares/userspaces/kaivostudio/Common` ergänzen (helm-lint lokal).
- Chart-Version bumpen: `insilo/olares/Chart.yaml` `version`/`appVersion` `0.1.81` → `0.1.82` + OlaresManifest `metadata.version` + `spec.versionName` identisch (Golden Rule im Insilo-Manifest-Kommentar).
- Release-Prozess Insilo: `insilo/docs/MARKET_SOURCE_PLAYBOOK.md` befolgen (Market-Source-Eintrag + frisches base64, AGENTS.md §4).

**A5 — Tests Insilo** (`backend/tests/`): Unit-Test für `_do_export` mit `tmp_path` (monkeypatch `settings.meeting_export_dir`): Frontmatter-Parsbarkeit, Filename-Determinismus, Atomarität (kein `.tmp`-Rest), idempenter Re-Export (gleiche ID → gleiche Datei, Inhalt aktualisiert). `pytest backend/tests` muss grün sein.

---

## 5. Teil B — Relay-Chart: appCommon-Mount (read-only)

**Dateien:** `relay-one/chart/relay/templates/deployment.yaml`, `chart/relay/values.yaml`, `chart/relay/OlaresManifest.yaml`. (Relay-Chart nutzt bereits `strategy: Recreate` — hostPath-kompatibel, so lassen.)

**B1 — volume + mount** in `templates/deployment.yaml`:
- `spec.template.spec.volumes:` (Zeile ~88): ergänze
  ```yaml
  - name: insilo-drop
    hostPath:
      # Cross-App-Ordner: Insilo schreibt Meeting-Summaries nach <appCommon>/insilo-meetings
      path: {{ .Values.userspace.appCommon }}/insilo-meetings
      type: DirectoryOrCreate
  ```
- Container `relay` `volumeMounts:` (Zeile ~55): ergänze
  ```yaml
  - name: insilo-drop
    mountPath: /data/insilo
    readOnly: true
  ```
- Container `env:` (ab Zeile ~38): ergänze
  ```yaml
  - name: RELAY_INSILO_DIR
    value: "/data/insilo"
  ```
- **initContainer `relay-data-chown` NICHT auf das neue Volume anwenden** (fremder Owner; read-only-Mount, chown würde fehlschlagen).
- **Wichtig:** Falls `userspace.appCommon` im Ziel-Chart der Relay-Installation nicht aufgelöst wird (Werte werden von Olares zur Installzeit injiziert; `appCommon` existiert erst ab 1.12.6 — Pin vorhanden, passt), Fallback-Diskussion → Abschnitt 12 Q1.

**B2 — `values.yaml`:** Kommentarblock `storage:` ergänzen um:
```yaml
# Read-only Cross-App-Drop für Insilo-Meeting-Summaries (Olares appCommon).
insilo:
  mountPath: /data/insilo
  scanIntervalSec: 600
  enabled: true
```
und im Deployment die Env-Werte daraus templaten: `RELAY_INSILO_ENABLED={{ .Values.insilo.enabled }}`, `RELAY_INSILO_SCAN_SECS={{ .Values.insilo.scanIntervalSec }}` (statt hartem Pfad; `RELAY_INSILO_DIR` bleibt `/data/insilo`).

**B3 — `OlaresManifest.yaml` `permission:`-Block** (Zeile 55): `appCommon: true` ergänzen.

**B4 — Version:** `Chart.yaml` `version`/`appVersion` (aktuell 26.9.149) bumpen — **pro App-Schritt hochzählen** (26.9.150, .151, …), Monatsformat `26.9.x` (September!). Alle 5 Stellen synchron (AGENTS.md §2.4): `_apps.ts`, `_lib.ts`-Key `relay-<version>.tgz`, `Chart.yaml`, OlaresManifest `metadata.version` + `spec.versionName`. Deploy-/Verify-Ablauf: AGENTS.md §4.4/§4.5 + `docs/handoff-26.9.143-145.md` (im Repo) für die relay-spezifischen Feinheiten (Image-Build via `Dockerfile`, ghcr-Tag = Chart-Version).

---

## 6. Teil C — Relay-Server: Ingest, Suche, API, Agent-Tool

**Referenz-Muster im Code (erst ansehen, dann nachbauen):**
- Schema/Migrationen: `server/src/cache/db.rs` — `init_db()` (Zeile 8) führt `CREATE TABLE IF NOT EXISTS` aus; `user_version`-Migrationen via `PRAGMA user_version` (Tests ab Zeile 811); `init_fts()` (Zeile 756) legt FTS5 **external-content**-Tabelle + Trigger an und ist best-effort (kein FTS5-Build → App läuft ohne Suche).
- DB-Zugriff aus Handlers: `server/src/db.rs` — `with_db(state, |conn| ...)`.
- API-Handler-Vorlage: `server/src/api/todos.rs` (klein, CRUD + JSON) — Routen registrieren in `server/src/api/mod.rs` (`router()`, Bereich "/todos" Zeile ~177).
- Scheduler-Vorlage: `server/src/sync/scheduler.rs` `start_periodic_sync` (Zeile 125); Spawn in `server/src/main.rs:92`.
- Agent-Tool-Vorlage: `server/src/ai/tools/tasks.rs` (`tools(locale) -> Vec<ToolDef>`, `Tier::Read`-Handler geben `ToolOutcome::Data(json)` zurück); Registrierung folgt dem Muster der bestehenden Module in `server/src/ai/tools/mod.rs`.
- Events an Browser: `state.events.emit("name", &payload)` (`server/src/events.rs:34`), Browser hört über `/api/v1/events` SSE (Main-App Zeile 114 ff.).

**C1 — Schema** (in `cache/db.rs`, `init_db()` erweitern + `user_version` 1→2 Migration wie dort üblich):
```sql
CREATE TABLE IF NOT EXISTS meetings (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  insilo_id     TEXT NOT NULL UNIQUE,          -- UUID aus Frontmatter
  path          TEXT NOT NULL,                 -- Dateiname (nur Name, kein Pfad)
  sha256        TEXT NOT NULL,                 -- Content-Hash (Inhaltsänderungserkennung)
  title         TEXT NOT NULL,
  participants  TEXT NOT NULL DEFAULT '[]',    -- JSON-Array (Namen)
  tags          TEXT NOT NULL DEFAULT '[]',    -- JSON-Array
  meeting_date  TEXT NOT NULL,                 -- ISO8601 aus recorded_at
  duration_min  INTEGER NOT NULL DEFAULT 0,
  language      TEXT NOT NULL DEFAULT 'de',
  template      TEXT NOT NULL DEFAULT '',
  source_url    TEXT NOT NULL DEFAULT '',
  body_md       TEXT NOT NULL,                 -- Markdown ohne Frontmatter
  deleted       INTEGER NOT NULL DEFAULT 0,    -- Soft-Delete
  first_seen_at TEXT NOT NULL,
  updated_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_meetings_date ON meetings(meeting_date DESC);
```
FTS: in `init_fts()` analog `messages_fts` ergänzen: `meetings_fts` (external content, Spalten `title, body_md`, Trigger insert/update/delete) + `INSERT INTO meetings_fts(meetings_fts) VALUES('rebuild')`-Fallback wie bei messages.

**C2 — Scanner** neue Datei `server/src/sync/insilo.rs` (+ `pub mod insilo;` in `sync/mod.rs`):
```rust
pub async fn run_insilo_scan(state: &Arc<AppState>)  // einmal kompletter Scan, gibt ScanReport zurück
```
Algorithmus (bewusst simpel, keine neuen Cargo-Dependencies — `sha2`/`hex`/`regex` sind schon da):
1. Dir aus `std::env::var("RELAY_INSILO_DIR")`; fehlt/leer oder `RELAY_INSILO_ENABLED=false` → No-op. `if !dir.exists()` → Debug-Log + No-op (Insilo evtl. nicht installiert — **kein** Fehlerzustand).
2. `std::fs::read_dir` (flach), nur `*.md`, `.tmp` überspringen, `mtime` jünger als 15 s überspringen.
3. Pro Datei: lesen → SHA-256 → gegen `meetings.sha256` prüfen (per `insilo_id` oder Pfad). Unverändert → skip.
4. **Frontmatter-Parse (handgemacht, ohne YAML-Lib):** Erste Zeile exakt `---`, bis schließendes `---` sammeln; pro Zeile `key: value`; Skalar = Rest nach erstem `:` (führende Anführungszeichen abstreifen); `participants`/`tags` = JSON-Array-Literal (Insilo schreibt `json.dumps` → direkt `serde_json::from_str`). Pars-Fehler (fehlende `insilo_id`, kaputtes Array) → Datei skippen, `tracing::warn!`, Eintrag in `scan_errors`-Logzeile (nicht in DB).
5. Upsert: `INSERT INTO meetings ... ON CONFLICT(insilo_id) DO UPDATE SET ...` (body/alle Felder; `first_seen_at` behalten).
6. Lösch-Erkennung: nach dem Scan alle DB-Zeilen mit `deleted=0` deren `path` nicht mehr im Scan-Set vorkam → `deleted=1`.
7. Bei ≥1 Änderung/Löschung: `state.events.emit("meetings-changed", &json!({"count": n}))`.
8. Rückgabe `ScanReport { scanned, inserted, updated, deleted, skipped }` (für `POST /meetings/scan`-Antwort + Logs).

**C3 — Takt** in `main.rs` (analog Zeile 92, eigener Spawn, entkoppelt vom Mail-Backoff):
```rust
// Intervall aus RELAY_INSILO_SCAN_SECS (Default 600); 1x Scan direkt beim Start.
tokio::spawn(sync::insilo::spawn_loop(insilo_state, insilo_shutdown_rx));
```
`spawn_loop` = `while recv_timeout(interval) { run_insilo_scan(); }` — Shutdown via `sync_shutdown_tx`-Muster oder eigener Channel (Haupt-Shutdown respektieren!).

**C4 — REST-API** neue Datei `server/src/api/meetings.rs` (+ `pub mod meetings;` und Routen in `api/mod.rs`):
- `GET /meetings?limit=50&offset=0&query=<fts>&tag=&participant=&from=&to=` → Listeneinträge **ohne** `body_md` (nur Metadaten + `snippet(meetings_fts,1,'','…','<b>',20)` wenn `query`), newest first, `deleted=0`.
- `GET /meetings/:id` → voller Eintrag inkl. `body_md` (`:id` = numerische PK).
- `POST /meetings/scan` → sofortiger Scan (Button "Jetzt aktualisieren" + manueller Trigger für Tests), antwortet `ScanReport`.
- Fehler-Semantik wie in `todos.rs` (`ApiError`/Status-Codes aus `error.rs`).
- Alle Routen laufen automatisch unter `relay_key_guard` (Mod-Datei regelt das zentral — nichts tun).

**C5 — Agent-Tool** neue Datei `server/src/ai/tools/meetings.rs` nach `tasks.rs`-Muster, nur Read-Tier:
- `meetings_list {open_scope?: string, limit?: integer}` → letzte N Meetings (Metadaten).
- `meetings_get {id: integer}` → voller Markdown-Body.
- `meetings_search {query: string}` → FTS-Treffer mit Snippets.
Registrierung + `de`/`en`-Beschreibungen wie bestehend. Damit kann der Assistent bereits "Was haben wir in Meeting X beschlossen?" beantworten, bevor Phase-2-Flows kommen.

---

## 7. Teil D — Relay-Web: Bereich "Meetings"

**Referenz-Muster:**
- Seiten-Vorlage: `web/src/routes/tasks/+page.svelte` (Liste+Detail in einer Route, Sidebar + `ModuleIcons` + `AssistantFab module="tasks"`, `useSidebarResize`, `dataVersion`-Invalidation aus `$lib/stores/invalidation`).
- Service-Funktionen: `web/src/lib/services/tauri.ts` — Helpers `get<T>()`/`post<T>()`/`apiCall<T>()` (Muster Zeile 1372 ff. `listTodos`).
- Modul-Umschalter: `web/src/lib/components/ModuleIcons.svelte` (Buttons `goto("/calendar"|"/contacts"|"/tasks")`).
- I18n: `web/src/lib/i18n.ts` — **beide** Blöcke `de:` und `en:` pflegen, Keys `meetings.*`.
- Markdown: **noch kein Renderer vorhanden.** `dompurify` ist Dependency. → kleinen Subset-Renderer neu schreiben.

**D1 — Service** (`tauri.ts` ergänzen): `MeetingInfo` (Liste: id, title, meetingDate, participants[], tags[], durationMin, snippet?), `MeetingDetail` (+ bodyMd, sourceUrl, template, language), `listMeetings(params)`, `getMeeting(id)`, `searchMeetings(query)`, `triggerMeetingScan()`; SSE-Event `"meetings-changed"` → `dataVersion` bumpen (bestehender Listener-Muster folgen).

**D2 — Markdown-Renderer** `web/src/lib/utils/markdown.ts` (+ Vitest): Subset `#`/`##`/`###`, `**bold**`, `*italic*`, `` `code` ``, `- `/`* ` Listen, **GFM-Checklisten** `- [ ]`/`- [x]` (als readonly Checkboxen), Links `[t](u)` (nur http/https), Absätze. Ausgang **immer** durch `DOMPurify.sanitize()`. Keine Tabellen nötig (Insilo-Renderer erzeugt welche → `<table>` als Fallback Plain-Zeile ist ok, aber nicht Pflicht; wenn Zeit: einfache Pipe-Tables).

**D3 — Route `web/src/routes/meetings/+page.svelte`** (nach `tasks/+page.svelte`):
- Kopf: Titelleiste "Meetings" + `ModuleIcons active="meetings"` + Suchfeld (debounced → `searchMeetings`) + Refresh-Button (`triggerMeetingScan`, Spinner).
- Liste (links/Hauptspalte): Datum + Titel + Teilnehmer-Chips + Tags; Klick → Detail.
- Detail: renderter Markdown (D2), Meta-Zeile (Dauer, Template, Sprache), Link `sourceUrl` → Insilo ("In Insilo öffnen"), Action-Buttons (Phase 2, erst Platzhalter-kommentiert lassen).
- Empty-State: "Noch keine Meetings — erscheint automatisch, sobald Insilo Zusammenfassungen exportiert" + Test-Button "Jetzt scannen".
- Leerer Zustand bei fehlendem Volume darf keinen Fehler zeigen (Scanner liefert einfach 0).
- Filter-Chips: Teilnehmer/Tag aus aggregierten Werten (`GET /meetings` darf dafür `?facet=1` erweitern, optional).

**D4 — Navigation:** `ModuleIcons.svelte` vierten Button "Meetings" ergänzen (Icon: Sprechblasen/Kamera-Kreis, Stroke-Stil der bestehenden Icons exakt nachbauen: `width=16 stroke-width=1.5`), `goto("/meetings")`. Sidebar-Einstieg in `+page.svelte` (Mail-App) **nur** über das dortige `ModuleIcons`-Vorkommen — grep `ModuleIcons` in `routes/+page.svelte` und identisch erweitern, keine neue Sidebar-Bauweise.

---

## 8. Teil E — KI-Flows (Phase 2, erst nach M1–M4 grün)

**Grundregel Sicherheit (entschieden):** Alles **hinausgehende** (Mail-Versand, Task für Dritte, Kalender-Termin) läuft über das vorhandene Karten-/Plan-Interface (`Tier::Write`/`External`, `ai_action_plans` + `POST /ai/plans/:id/confirm` in `api/mod.rs:118-121`) — nie autark ohne Bestätigung. Rein lesende/lokale Schritte (Extraktion, Suche, Digest-Entwurf) dürfen vollautomatisch laufen.

**E0 — Parser für Insilo-Checklisten** (deterministisch, Unit-Test-pflichtig). Insilo rendert To-dos exakt so (`insilo/backend/app/exports/markdown.py:140`):
```
- [ ] <beschreibung> — <verantwortlich>, fällig <frist>
```
Parse-Regeln: Zeile matcht `^- \[( |x)\] `; Split auf ` — ` (Leerzeichen-Em-Dash-Leerzeichen); Suffix-Split auf `, `; Segment mit Präfix `fällig ` → Due-Datum (ISO oder `DD.MM.YYYY` → normalisieren, unparsebar → leer lassen); übrige Suffix-Segmente → Verantwortlich. Kein LLM nötig; LLM nur als Fallback für Freiform-Absätze ("Offene Fragen" etc.) über `POST /ai/extract-time`-Muster (`api/ai.rs`).

**E1 — "To-dos übernehmen"** (Detail-Ansicht Button):
1. `POST /meetings/:id/extract-todos` (neu, `api/meetings.rs`): E0-Parser über `body_md`, Abschnitt "Offene Aufgaben"/`naechste_schritte` bevorzugen (Heading-Match, sonst alle Checklisten).
2. Antwort: Vorschlagsliste `{summary, due, assignee, sourceLine}`; UI zeigt Checkbox-Karte; Bestätigung → n× `createTodo()` (bestehender Endpunkt). Im Task-Feld `notes`/Beschreibung Rücklink: Meeting-Titel + `source_url`.
3. Duplikatsschutz: Task-Notiz enthält `insilo-meeting:<id>`; vor Anlage prüfen (LIKE-Query), bereits vorhandene grau markieren.

**E2 — "Minutes mailen"** (Detail-Ansicht Button):
1. `POST /meetings/:id/minutes-draft` (neu): Teilnehmer → Kontaktauflösung (bestehende Contacts-Tabelle, Match über Name + Alias-Felder; Trefferquote-Log). Unbekannte Teilnehmer als Warnung listen.
2. Mail-Entwurf bauen: Betreff `Meeting-Protokoll: <Titel> (<Datum>)`, Anrede + gekürzte Summary (LLM-Aufbereitung via bestehendem `ai/summarize`-Client, Length-Cap ~300 Wörter) + vollständiges Protokoll als **Textteil** (kein Anhang — EML-Größe), Footer "Erstellt mit Insilo + Relay".
3. Entwurf im existierenden Compose-Flow öffnen (`send::save_draft` / Draft-Route `api/mod.rs:97`) — Versand manuell. **Kein Auto-Send** (opt-in später, dann über `Tier::External`-Karte).

**E3 — Wochen-Digest** (Settings-Schalter, Default AUS): Scheduler-Zyklus Fr < configurable Zeit: alle Meetings der Woche → `meetings_fts`-Inhalt + To-do-Status → LLM-Digest (max 400 Wörter) → **Entwurf** an eigenes Postfach (Adresse aus Settings/Profil). Nur Entwürfe, nie Direktversand.

**E4 — Überfällige To-dos:** bestehende Tasks mit `insilo-meeting:`-Notiz und überfälliger Due → Reminder-Entwurf-Empfehlung als Badge auf der Meeting-Detailseite (rein lokal, kein Versand).

**E5 — Kontext-Seitenhinweis (nice-to-have):** In `ComposeWindow` beim Tipen eines Empfängers: letztes Meeting mit dieser Person (Contacts-Match → `meetings?participant=`) als einzeilige Info-Zeile. Nur wenn M1–M4 stabil sind.

---

## 9. Meilensteine & Abnahmekriterien

| # | Meilenstein | Inhalt | Abnahme (alle Punkte prüfbar!) |
|---|---|---|---|
| **M1** | Insilo-Export | A1–A5 | `pytest backend/tests` grün; lokal via docker-compose: Meeting fertig zusammenfassen → `.md` mit korrektem Frontmatter taucht im Export-Dir auf; Re-Summarize überschreibt; Delete entfernt; Backfill-Endpoint liefert alle `ready`-Meetings |
| **M2** | Relay-Ingest | B1–B3 (Chart), C1–C4 (Server) | `cargo test` grün (neue Tests: Frontmatter-Parser inkl. Umlaute/Quotes/ kaputte Datei, Upsert-Idempotenz, Lösch-Erkennung, FTS-Snippet); lokal: `RELAY_INSILO_DIR=/tmp/insilo-test` mit 3 Musterdateien → `GET /api/v1/meetings` listet sie; `POST /meetings/scan` Report korrekt; `helm lint`/`helm template` beider Charts grün (Stub-Values) |
| **M3** | Relay-Web | D1–D4 | Vitest grün (markdown.ts, Service-Funktionen, MeetingsPage — RAM-Limit beachten, Abschnitt 11); `npm run build` grün; Browser-Screenshot: Liste + Detail + Suche + leeren Zustand; `svelte-check` ohne neue Fehler |
| **M4** | Release beider Apps | B4 + Insilo-Release | Beide Apps live im market.AImighty; Olares-Katalog zeigt neue Versionen; Relay zeigt echte Insilo-Meetings der Box; AppID/Routen unverändert (`expert`/`mail` etc. funktionieren) |
| **M5** | Phase-2-Flow E0–E2 | Extraktion + Minutes-Draft | Tests für E0-Parser (Formatfälle ohne/mit Frist, `DD.MM.` + ISO); manuell: 1 Meeting → Tasks mit Frist/Notiz; 1 Minutes-Entwurf korrekt adressiert |
| M6 (opt.) | Webhook-Booster | Insilo-Webhook → Relay-Scan-Trigger | Nur nach M5 und nur wenn Latenz stört; Relay-Endpoint dafür braucht API-Key-Auth (Envoy-Thema, AGENTS.md §6.5) — vorher Abschnitt 12 Q2 klären |

**Reihenfolge ist Pflicht:** M2 braucht M1 nur für den Live-Test, nicht für Unit-Tests (Fixture-Dateien anlegen). M4 **vor** M5 (stabile Basis ausliefern).

## 10. Verify-Befehle (Kopiervorlagen)

```bash
# ── Insilo ──────────────────────────────────────────────
cd /home/opencode/workspace/insilo && pytest backend/tests -q
helm lint olares/ -f olares/values-olares-stub.yaml --set userspace.appCommon=/tmp/common
# Release: Prozess in insilo/docs/MARKET_SOURCE_PLAYBOOK.md

# ── Relay Server ────────────────────────────────────────
cd /home/opencode/workspace/relay-one/server
cargo test -q                                   # volle Suite (381+ Tests) dürfen nicht brechen
RELAY_DATA_DIR=/tmp/relay-m2 RELAY_BIND=127.0.0.1:3799 RELAY_INSILO_DIR=/tmp/insilo-test \
  cargo run --release 2>&1 | tee /tmp/relay-m2.log &     # Port 3000 ist belegt!
curl -s localhost:3799/api/v1/meetings | python3 -m json.tool
curl -s -X POST localhost:3799/api/v1/meetings/scan
# ── Relay Web ───────────────────────────────────────────
cd /home/opencode/workspace/relay-one/web
NODE_OPTIONS="--max-old-space-size=3072" npx vitest run src/lib/__tests__/<neu> -q
npm run build
# ── Chart + Deploy (AGENTS.md §4.4/4.5) ─────────────────
helm lint chart/relay -f chart/relay/values.yaml   # + ggf. Stub-Values für userspace.appCommon
cd "/home/opencode/workspace/aimighty-market"      # Market-Source-Repo (bayerhazard/aimighty-market)
git add functions/ && git commit -m "relay <ver>: meetings area" && git push
export CLOUDFLARE_API_TOKEN="$(cat /home/opencode/.config/opencode/cloudflare-token)"
./node_modules/.bin/wrangler pages deploy functions/ --project-name=aimighty-market
# Olares-Sync 5 min abwarten:
olares-cli market get relay -s market.AImighty
olares-cli market upgrade relay --watch
# Route-Mail prüfen: olares-cli settings apps domain set relay relay --third-level mail  (nur falls gelöscht wurde)
```

Chart-Packaging für Market: `helm package chart/relay/` → `relay-<version>.tgz` → **frisches** `base64 -i … | tr -d '\n'` in `_lib.ts` (AGENTS.md §4.2/§8.6 — nie alten Base64 wiederverwenden), `_apps.ts`-Version + `chartName`-Key synchron.

---

## 11. Fallen & Umwelt-Limits (aus AGENTS.md + relay-one/HANDOFF.md destilliert)

**Umgebung (Container!):**
- Nur **4 GiB RAM / 4 CPU** real (Host zeigt 96/24 vor). `nproc` ignorieren. Vitest nie > 3072 MB heap.
- **Port 3000** gehört dem opencode-Server → Rust-Smoke-Tests immer via `RELAY_BIND` auf freiem Port (3799).
- Lange Kommandos: `setsid nohup … & disown`, Logs in Workspace-Dateien (nicht `/tmp`, das leert sich), Pollen mit kurzen Calls.

**Integration:**
1. **`appCommon` existiert erst ab Olares 1.12.6** — Manifeste pinnen `>=1.12.6-0` ✓. Wenn `.Values.userspace.appCommon` beim `helm template` mit Stub-Values fehlt → Stub ergänzen (wurde in B4/A4 dokumentiert), nicht den Template-Pfad erfinden.
2. **hostPath + RollingUpdate = HTTP 400 beim Market-Upload** → beide Deployments bleiben bei `strategy: Recreate` (so vorhanden, nicht ändern).
3. **Relay-Pod ist per `nodeSelector` an Node "olares" gepinnt** (values.yaml-Kommentar: JuiceFS/DB nur dort konsistent). appCommon ist JuiceFS → cross-node ok, aber Pin bleibt.
4. **`meetings`-Tabelle heißt wie das K8s-Wort, ist aber rein SQLite-intern** — kein Konflikt mit `events`-Tabelle; trotzdem SQL-Queries kopieren statt raten (Spaltennamen exakt aus C1).
5. **Frontmatter ist ein Mini-Dialekt** (Abschnitt 3): keine mehrzeiligen Werte, keine Anker, Listen als JSON-Arrays. Wenn Insilo-Titel Sonderzeichen (`"`, `:`, Zeilenumbrüche) enthält: Insilo masked (`"`→`'`, `\n`→` `), Relay ist streng (Parse-Fehler → Skip + Warn-Log, nie Panic).
6. **Unicode-Dateinamen vermeiden** → Dateiname nur ASCII (Datum + UUID-Hex), Titel lebt in Frontmatter.
7. **Leeres/fehlendes Drop-Verzeichnis ist Normalzustand** (Insilo nicht installiert/deaktiviert) — Relay muss das als "0 Meetings, kein Fehler" behandeln; Startup darf nie davon abhängen.
8. **FTS5 best-effort:** `meetings_fts`-Setup in `init_fts()` in dessen try-Pfad integrieren; ohne FTS5-Support Suche auf `LIKE`-Fallback der API begrenzen oder 503 + Hinweis (Muster: bestehender messages-Fallback-Code in `api/messages.rs::search_messages` ansehen).
9. **Versions-Drift:** Relay liegt aktuell bei 26.9.149 (Chart.yaml). Vor jedem Bump `git fetch` + Live-Chart gegen `_lib.ts` prüfen (AGENTS.md: Repos lagen schon hinter Live).
10. **`market upgrade`-Values-Freeze:** falls Chart-Änderung (neues Volume) nach Upgrade nicht greift → AGENTS.md §6.4: erst Upgrade, im Fehlerfall einmalig uninstall+install (Route-ID `mail` danach neu setzen!).
11. **Insilo-Env-Variablen-Prefix:** Insilo liest Settings via pydantic (`app/config.py`) — Env-Name muss zum Settings-Feld passen (`INSILO_`-Prefix? In `config.py` `env_prefix` prüfen, bevor deployment-Env gesetzt wird!).
12. **Nach jeder Relay-Installation Probes prüfen** (AGENTS.md §6.3 gilt v. a. für vLLM-Apps — Relay hat lange Timeouts, unverändert lassen).

## 12. Offene Fragen (mit Marc klären, nicht selbst entscheiden)

- **Q1:** Darf Insilo `appCommon` schreiben (Volume readonly: false im Backend+Worker)? Alternativ: Relay liest Insilos appData über hardgecodeten Pfad `/olares/userspaces/<user>/Data/insilo/exports` — unsauber, nur Fallback.
- **Q2:** Webhook-Booster (M6): Relay-Endpoint würde den Envoy-Sidecar brauchen (X-Bfl-User-Problem, siehe Insilo-Manifest-Kommentar Zeile 93 ff.) — vermutlich nicht machbar ohne `relay_key_guard`-Ausnahme. Low priority.
- **Q3:** Mehrsprachigkeit: Insilo rendert deutsche Section-Titel ("Offene Aufgaben"); Relay-UI ist de/en. E0-Parser deliberately auf "alle Checkliste-Zeilen" statt "Abschnitt X" auslegen.
- **Q4:** Sollen Meeting-Mails (E2) im gesendeten Ordner eines bestimmten Kontos abgelegt werden (Standard-Account vs. Auswahl)?
- **Q5:** Aufbewahrung: Löscht Insilo die Datei nach N Tagen, oder behält Relay sie für die Ewigkeit (aktuell: Relay behält, Insilo ist Source-of-Truth für den Export-Ordner)?

## 13. Datei-Landkarte (was wo angefasst wird)

```
insilo/
├── backend/app/config.py                     [A1]  +meeting_export_dir
├── backend/app/tasks/export_file.py          [A2]  NEU
├── backend/app/tasks/summarize.py            [A3]  +1 send_task Zeile (~623)
├── backend/app/routers/meetings.py           [A3]  updated/deleted-Hooks + Backfill-Route
├── backend/tests/…                           [A5]  NEU
├── olares/templates/deployment-backend.yaml  [A4]  app-common Volume+Mount+Env
├── olares/OlaresManifest.yaml                [A4]  permission.appCommon + Version
├── olares/values-olares-stub.yaml            [A4]  +appCommon
└── olares/Chart.yaml                         [A4]  0.1.81 → 0.1.82

relay-one/
├── chart/relay/templates/deployment.yaml     [B1/B2] insilo-drop Volume+Mount+Env
├── chart/relay/values.yaml                   [B2]   insilo: {enabled, mountPath, scanIntervalSec}
├── chart/relay/OlaresManifest.yaml           [B3/B4] permission.appCommon + Version (5 Stellen!)
├── chart/relay/Chart.yaml                    [B4]   26.9.149 → 26.9.15x
├── server/src/cache/db.rs                    [C1]   meetings-Tabelle + FTS + user_version 2
├── server/src/sync/insilo.rs                 [C2]   NEU: Scanner
├── server/src/sync/mod.rs                    [C2]   +pub mod insilo;
├── server/src/main.rs                        [C3]   +Spawn Scan-Loop
├── server/src/api/meetings.rs                [C4]   NEU: REST
├── server/src/api/mod.rs                     [C4]   +mod + Routen
├── server/src/ai/tools/meetings.rs           [C5]   NEU: 3 Read-Tools
├── server/src/ai/tools/mod.rs                [C5]   +mod/Registrierung
├── web/src/lib/services/tauri.ts             [D1]   +Meeting-Services + SSE-Event
├── web/src/lib/utils/markdown.ts             [D2]   NEU + Test
├── web/src/routes/meetings/+page.svelte      [D3]   NEU
├── web/src/lib/components/ModuleIcons.svelte [D4]   +Button
├── web/src/lib/i18n.ts                       [D3]   +meetings.* (de UND en)
└── server/src/api/{meetings.rs,ai.rs}        [E1–E3] extract-todos + minutes-draft + digest
```

**Kurzfassung für den ersten Arbeitsschritt:** Mit M1 (Insilo `export_file.py`) beginnen — kleinste unabhängige Scheibe, nichts blockiert, Tests lokal ausführbar.
