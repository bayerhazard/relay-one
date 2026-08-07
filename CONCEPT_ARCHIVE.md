# AImighty Relay One — Konzept: Web-App mit Local-First Mail-Archiv

> Ziel: Relay One als Olares-Web-App (1 Instanz, alle Geräte) mit lokalem Mail-Archiv,
> das den begrenzten Provider-Speicher entlastet und volle Kontrolle + Backup ermöglicht.

---

## 1. Prinzipien

1. **Lokal zuerst**: Jede Mail wird vollständig auf dem Olares-PVC gespeichert (rohes EML + Index). Der Provider ist nur Zustellpunkt.
2. **Kein Datenverlust**: Provider-Löschung nur bei expliziter Benutzeraktion und erst nach Verifikation. Lokale Kopie wird nie automatisch gelöscht.
3. **Keine automatischen Aktionen**: Nichts wird von alleine archiviert, geprunt oder gelöscht. Der Benutzer liest oder löscht Mails selbst (Entscheidung F2).
4. **Wählbarer Sync-Modus pro Account**: `mirror` (Provider = Quelle) oder `archive` (Provider wird auf Benutzeraktion entlastet).
5. **1 Instanz, 1 Konfig**: Keine Multiuser-Sessions; Web-UI mit einer Zugangs-Sperre (Olares Auth am Entrance reicht).
6. **Ein Datenstamm**: ALLE Nutzerdaten liegen unter `/data/Relay` — Mail-Archiv, Mail-DB, KI-DB, Cache, Config. Damit erfasst das normale Olares-Backup alles (Entscheidung F5).

---

## 2. Architektur (Olares-Deployment)

```
Browser (Laptop / Handy)
        │ HTTPS (Entrance, authLevel: internal)
        ▼
relay.<user>.olares.com
┌──────────────────────────── Olares Cluster ────────────────────────────┐
│ Deployment: relay-app                                                   │
│  ┌─────────────────┐   ┌─────────────────────────────────────────────┐ │
│  │ relay-web       │   │ relay-server (Rust/axum)                     │ │
│  │ SvelteKit (SSR) │──▶│  REST API /api/v1/*                          │ │
│  │ statische Assets│   │  Sync-Engine (IMAP-IDLE + 20s-Poll)          │ │
│  └─────────────────┘   │  SMTP-Client (Send)                          │ │
│                        │  AI-Client → llm.aimighty.de (OpenAI-komp.)  │ │
│                        │  Delete-Worker (nur bei Benutzeraktion)      │ │
│                        └──────────────┬──────────────────────────────┘ │
│                        PVC: relay-data → /data/Relay                   │
│                        ├── archive/<account>/YYYY/MM/<uid>-<hash>.eml  │
│                        ├── attachments/<sha256>  (Dedup, on-demand)    │
│                        ├── index.db   (Mail-DB, SQLite WAL)            │
│                        ├── ai.db      (KI-Infos: Tone-Profile, Audit)  │
│                        ├── cache/     (Attachment-/Raw-Cache)          │
│                        └── config.json                                 │
└────────────────────────────────────────────────────────────────────────┘
```

- **relay-server**: Rust-Core aus `tauri/src/` (imap/, smtp/, cache/, ai/, sync/, security/, tone/) wird zu eigenständigem axum-Binary. `tauri::command` → REST-Handler, `State` → `Arc<AppState>`.
- **relay-web**: SvelteKit-Frontend bleibt; `src/lib/services/tauri.ts` wird zum fetch-Client (gleiche Funktionsnamen → UI-Änderungen minimal).
- **Zugriff**: Entrance `relay` mit `authLevel: internal` → Olares-Auth schützt die App; kein eigenes Login nötig.
- **Datenstamm**: Einziger Pfad `/data/Relay` (PVC-Mount). Kein Datenverzeichnis außerhalb.

---

## 3. Speicher-Schema

### 3.1 Dateisystem (PVC, Mount-Punkt `/data/Relay`)

```
/data/Relay/
├── archive/<account-id>/YYYY/MM/<imap-uid>-<sha1-of-msgid>.eml   # roh, unveränderlich
├── attachments/<sha256-of-content>                                # dedupliziert
├── index.db                                                       # Mail-DB (SQLite, WAL)
├── ai.db                                                          # KI-Infos (Tone-Profile, Audit-Log)
├── cache/                                                         # Attachment-/Raw-Cache
└── config.json                                                    # AppConfig (Account-Secrets referenziert)
```

- EML-Dateien = portables Format: Export, Migration, Backup, jederzeit mit mail-parser lesbar.
- **Backup-relevant: ALLES unter `/data/Relay`** — das Olares-Backup des PVC sichert Mail-DB + KI-DB + Cache in einem Rutsch (Entscheidung F5).

### 3.2 SQLite-Schema (index.db)

```sql
-- Konten: Sync-Modus pro Account
CREATE TABLE accounts (
  id            INTEGER PRIMARY KEY,
  email         TEXT NOT NULL UNIQUE,
  imap_host     TEXT NOT NULL, imap_user TEXT NOT NULL, imap_pass_ref TEXT,  -- Key im K8s-Secret
  smtp_host     TEXT, smtp_user TEXT, smtp_pass_ref TEXT,
  sync_mode     TEXT NOT NULL DEFAULT 'mirror',  -- 'mirror' | 'archive'
  created_at    TEXT DEFAULT (datetime('now'))
);

-- Ordner: Mirror-Sync aller; Archive: lokal-only Ordner (frei anlegbar, keine Vorgaben)
CREATE TABLE folders (
  id INTEGER PRIMARY KEY, account_id INTEGER NOT NULL REFERENCES accounts(id),
  name TEXT NOT NULL, imap_path TEXT,            -- NULL = lokal-only Ordner
  sync_enabled INTEGER DEFAULT 1, UNIQUE(account_id, name)
);

-- Nachrichten: Kern des Archivs
CREATE TABLE messages (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL, folder_id INTEGER NOT NULL,
  imap_uid INTEGER, msg_id TEXT, subject TEXT, from_addr TEXT, to_addrs TEXT,
  date TEXT, size INTEGER, flags TEXT,
  attachment_count INTEGER DEFAULT 0,
  raw_path TEXT NOT NULL,                        -- Pfad zur EML-Datei
  status TEXT NOT NULL DEFAULT 'mirrored',       -- 'mirrored'|'archived'|'pruned'
  synced_at TEXT DEFAULT (datetime('now')),
  UNIQUE(folder_id, imap_uid)
);

-- Lösch-Queue: NUR durch Benutzeraktion befüllt (Löschen / Verschieben nach lokal-only)
CREATE TABLE delete_queue (
  id INTEGER PRIMARY KEY, message_id INTEGER NOT NULL REFERENCES messages(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  state TEXT NOT NULL DEFAULT 'pending',         -- 'pending'|'verified'|'deleted'|'failed'
  attempts INTEGER DEFAULT 0, last_error TEXT
);

-- Sync-Status je Ordner
CREATE TABLE sync_state (
  folder_id INTEGER PRIMARY KEY, last_uid INTEGER, highest_modseq INTEGER, last_sync_at TEXT
);

-- Volltextsuche (FTS5)
CREATE VIRTUAL TABLE messages_fts USING fts5(
  subject, from_addr, to_addrs, body_text, content='messages', content_rowid='id'
);
```

### 3.3 KI-Datenbank (ai.db)

- `tone_profiles` (Kontakt → Tonalität), `audit_log` (KI-Entscheidungen) — 1:1 aus dem Desktop-Relay übernommen.
- Liegt bewusst **nicht** in index.db, damit Mail-DB und KI-DB unabhängig backupt/bearbeitet werden können.

---

## 4. Sync-Modi

### 4.1 Modus `mirror` (Default bei Erstkonfiguration)

- Voll-Sync Posteingang + Unterordner (wie heutiges Relay).
- Provider bleibt Quelle der Wahrheit; lokale DB = Cache.
- Keine Provider-Löschungen. → Sichere Startphase.

### 4.2 Modus `archive` (der neue Weg)

**Pull ist automatisch, Provider-Entlastung NIE** (Entscheidung F2):

1. **Pull**: IMAP-Sync wie gehabt — neue Mails werden automatisch als EML gespeichert + indexiert. Das Archiv wächst von selbst; es wird **nie automatisch** etwas auf dem Provider gelöscht.
2. **Aktionen (nur manuell)**:
   - **Löschen (UI)**: Mail geht in den lokalen Papierkorb (30 Tage) → danach Hard-Delete von EML + Index. Provider-Löschung nur, wenn die Mail dort noch existiert (Verify-Pipeline §5).
   - **Verschieben in lokal-only Ordner**: Mail bleibt lokal (EML + Index); Provider-Kopie wird auf ausdrücklichen Wunsch gelöscht. Lokal-only Ordner legt der Benutzer **vollständig selbst** an — keine Vorgaben, kein Starter-Set (Entscheidung F3).
3. **Ungelesene Mails**: werden **nicht** anders behandelt — keine automatische Aktion, egal ob gelesen oder ungelesen (Entscheidung F2).

### 4.3 Zustellung neuer Mails

- IMAP-IDLE, Fallback 20s-Poll (bestehender Scheduler): neue UIDs → sofort lokal archivieren.
- Die Provider-Inbox wird **nicht** automatisch geleert — der Benutzer entscheidet, wann Platz geschaffen wird.

---

## 5. Lösch-Workflow (Sicherheits-Pipeline, nur benutzerinitiiert)

```
Benutzeraktion (Löschen / Verschieben nach lokal-only)
   │
   ▼
pending ──(Verify ok)──▶ verified ──(Provider-Delete ok)──▶ deleted
   │                          │
   │ (Verify-Fehler)          │ (IMAP-Fehler, max 5 Retries, Backoff)
   ▼                          ▼
 failed ──(Benutzer-Review in UI)──▶ pending (manuell erneut)
```

1. **Verify**: `raw_path` existiert, Dateigröße stimmt, Index-Zeile vollständig, `sha256(raw_path)` == bei Pull berechnetem Hash.
2. **Provider-Delete — hart, wenn Garantie gilt (Entscheidung F1)**:
   - **Primär (hart): `STORE +FLAGS \Deleted` + `EXPUNGE`** — aber nur, wenn die Verify-Garantie steht: Mail ist vollständig lokal (EML + Index + Hash ok) UND lokaler Papierkorb/Backup-Marker ist gesetzt, bevor der IMAP-Delete läuft.
   - **Fallback (weich): `MOVE` in Provider-`Trash`** — automatisch, wenn die Garantie nicht nachweisbar ist (z. B. Verify-Schritt schlägt fehl oder Sicherungs-Marker fehlt). Weich ist immer sicher.
3. **Lokal unantastbar**: `status='pruned'` nur nach erfolgreichem Provider-Delete; EML + Index bleiben immer bestehen.
4. **Fehler**: Retry mit Backoff; Mails bleiben lokal — nie lokale Daten bei Provider-Fehler löschen.
5. **Vor Aktivierung des `archive`-Modus**: Hinweis im UI, dass ein Backup eingerichtet sein sollte (Olares-Backup des PVC, §9).

---

## 6. REST-API (Mapping aus `tauri/src/ipc.rs`)

Alle bestehenden Funktionen bleiben, nur Transport ändert sich (`invoke` → `fetch`):

| Bereich | Endpoints (Auszug) |
|---|---|
| Konten | `GET/POST/DELETE /api/v1/accounts`, `POST /api/v1/accounts/{id}/test` |
| Sync | `POST /api/v1/accounts/{id}/sync`, `GET /api/v1/accounts/{id}/sync/status` |
| Ordner | `GET /api/v1/accounts/{id}/folders`, `POST /api/v1/folders` (auch lokal-only, frei), `PATCH /api/v1/folders/{id}` |
| Nachrichten | `GET /api/v1/folders/{id}/messages?since=&query=` (FTS5), `GET /api/v1/messages/{id}/raw` |
| Aktionen | `PATCH /api/v1/messages/{id}` (read/flag/status), `DELETE /api/v1/messages/{id}`, `POST /api/v1/messages/{id}/move` |
| Senden | `POST /api/v1/send` (Entwurf/Sofort), Anhänge via `POST /api/v1/attachments` (multipart) |
| AI | `/api/v1/ai/summarize|reply|draft|priority|fraud|tone` (bestehende Logik) |
| Archiv | `GET /api/v1/archive/delete-queue`, `POST /api/v1/archive/delete-queue/{id}/retry` |
| CardDAV | `/api/v1/carddav/*` (unverändert) |

**Auth**: Intern über Olares-Entrance (authLevel internal) — die REST-API ist nur im Cluster-Netz erreichbar.

---

## 7. Frontend-Änderungen (minimal)

- `src/lib/services/tauri.ts` → `relayApi.ts`: identische Funktionensignaturen, Implementierung `fetch('/api/v1/...')`.
- Neu in den Einstellungen: Sync-Modus je Account (`mirror`/`archive`), Ordner-Typ (Provider/lokal-only), Delete-Queue-Review-Liste. **Kein `provider_retention_days`-Feld** (gibt es nicht mehr, Entscheidung F2).
- Mailbox-UI: Lokal-only Ordner mit eigenem Icon (kein Provider-Sync-Icon).
- Anhänge öffnen: `<input type=file>` statt `open_file_picker`; Downloads via `window.open('/api/v1/messages/{id}/attachment/...')`.

---

## 8. Olares-Chart-Skizze (relay-app)

```
relay-app/
├── Chart.yaml                  # name: relay-app, version: 26.08.1, apiVersion: v2
├── OlaresManifest.yaml         # v3 (v0.12.0), siehe unten
├── values.yaml
├── templates/
│   ├── deployment.yaml         # relay-server + relay-web (nginx/static), PVC-Mount, Probes
│   ├── service.yaml
│   └── pvc.yaml                # relay-data, size aus values (Default 100Gi)
```

### OlaresManifest.yaml (Kern)

```yaml
olaresManifest.version: '0.12.0'
olaresManifest.type: app
apiVersion: 'v3'
workloadReplicas:
  relay-app: 1
metadata:
  name: relay-app
  appid: relay
  icon: relay-icon.png
  title: AImighty Relay One
  categories: [Utilities]
  version: 26.08.1
entrances:
  - name: relay-app
    port: 3000
    host: relay-app
    title: AImighty Relay One
    icon: relay-icon.png
    openMethod: window
    authLevel: internal
spec:
  versionName: 26.08.1
  fullDescription: "**Model** / **Inference Engine** / **Key Features** / **API** / **Resource Usage** ..."
  developer: bayerhazard
  website: https://github.com/bayerhazard/relay-one
  sourceCode: https://github.com/bayerhazard/relay-one
  locale: en
  supportArch: [amd64, arm64]
  requiredCpu: 0.5
  limitedCpu: 4
  requiredMemory: 1Gi
  limitedMemory: 8Gi
  requiredDisk: 10Gi
  limitedDisk: 200Gi
permission:
envs: []
options:
  apiTimeout: 0
  dependencies:
    - name: olares
      version: '>=1.12.6-0'
```

### values.yaml

```yaml
workloads:
  relay-app:
    replicaCount: 1
storage:
  mountPath: /data/Relay      # Datenstamm — ALLE Daten hier (Entscheidung F5)
  size: 100Gi
image:
  repository: ghcr.io/bayerhazard/relay-one
  tag: 26.08.1                # ohne v-Präfix
```

### Wichtige Olares-Fallen (aus Playbook)

- `apiVersion: 'v3'` + `olaresManifest.version: '0.12.0'` + `options.dependencies`-Pin → Validierung 1+2.
- Deployment-Replicas via `.Values.workloads.relay-app.replicaCount` (Validierung 3).
- Probes: startup `720/30s/60s`, liveness `5/30s/600s` (langes Booten bei Erst-Sync).
- Secret für IMAP/SMTP-Passwörter: als K8s-Secret, `pass_ref` referenziert daraus; app-service verlangt keine `OLARES_USER_*`-Namen → einfach halten.
- Upgrade-Falle: Values-Änderungen → `uninstall + install`, nie `upgrade`.
- PVC-Mount-Pfad = `/data/Relay` (Großschreibung!), sonst liegt Daten außerhalb des erwarteten Backup-Pfads.

---

## 9. Backup & Wiederherstellung

| Was | Wie |
|---|---|
| **Alles** (EML, index.db, ai.db, cache, config) | Ein einziger PVC-Stamm `/data/Relay` — normales Olares-Backup des PVC erfasst automatisch Mail-DB + KI-DB + Cache (Entscheidung F5) |
| Wiederherstellung | PVC-Restore, Container startet → `sync_state`-Vergleich: fehlende lokale Mails aus Provider nachziehen (falls noch vorhanden) |
| Konsistenz | SQLite WAL + `synchronous=NORMAL`; Backup-Fenster: `VACUUM INTO`-Kopie oder Datei-Snapshot |
| Export | Komplette MBox-/EML-Export-Funktion in den Einstellungen (Provider-unabhängig) |

**Migrations-Hinweis**: Desktop-SQLite-Import (Mail- + KI-DB) in P3, falls überhaupt nötig (Entscheidung F4).

---

## 10. Roadmap

| Phase | Inhalt | Ergebnis |
|---|---|---|
| **P1 — Web-Migration** | Rust-Core → axum-Binary; `tauri.ts` → fetch-Client; Chart + Deploy auf market.AImighty; `/data/Relay`-Struktur | Relay One läuft im Browser, Mirror-Modus, Zugriff von allen Geräten |
| **P2 — Archiv** | EML-Speicher + SQLite-Index (Schema §3), Sync-Modus `archive`, lokale Ordner (frei), Verify-Pipeline + Delete-Queue (nur manuell, §5) | Provider wird auf Wunsch entlastet, Backup-fähig |
| **P3 — Komfort** | FTS5-Suche im UI, Attachment-Dedup, mbox-Export, Backup-Integration (Drive), IMAP-IDLE; optional Desktop-DB-Import | Produktionsreif |

**Empfohlene Reihenfolge**: P1 zuerst (schneller Nutzen, Risiko-Mitigation), P2 danach — `archive`-Modus erst, wenn Backup-Routine steht (§5.5).

---

## 11. Entscheidungen (dokumentiert)

| # | Frage | Entscheidung |
|---|---|---|
| F1 | Provider-Delete hart oder weich? | **Hart (`EXPUNGE`)** — sofern die Garantie steht, dass Mails beim Verschieben nicht verloren gehen (Verify-Pipeline + lokaler Sicherungs-Marker vor dem Delete). Kann die Garantie nicht nachgewiesen werden → automatisch **weich** (`MOVE` → Provider-Trash) |
| F2 | Ungelesene Mails automatisch archivieren? | **Nie automatisch.** Der Benutzer liest alle ungelesenen Mails oder löscht sie selbst. Keine automatische Aktion (weder Archiv noch Prune) |
| F3 | Lokal-only Ordner vorgeben? | **Nein** — der Benutzer legt lokale Ordner vollständig selbst an, keine Vorgaben/Starter-Sets |
| F4 | Desktop-DB-Import wann? | **P3, optional** — wird möglicherweise gar nicht benötigt |
| F5 | Datenpfad | **ALLES unter `/data/Relay`** (Mail-DB index.db, KI-DB ai.db, Cache, EML-Archiv, Config) — ein Stamm für das normale Olares-Backup |

---

## 12. Noch offen

1. **API-Sperre innerhalb Olares**: REST-API ist nur im Cluster-Netz erreichbar (authLevel internal), aber andere Apps im Netz könnten theoretisch zugreifen. Vorschlag: `X-Relay-Key`-Header (aus K8s-Secret) für Server-zu-Server-Aufrufe.
2. Papierkorb-Aufbewahrung lokal: 30 Tage fest oder konfigurierbar?
