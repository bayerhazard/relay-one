# AImighty Relay — Konzept: Web-App mit Local-First Mail-Archiv

> Ziel: Relay als Olares-Web-App (1 Instanz, alle Geräte) mit lokalem Mail-Archiv,
> das den begrenzten Provider-Speicher entlastet und volle Kontrolle + Backup ermöglicht.

---

## 1. Prinzipien

1. **Lokal zuerst**: Jede Mail wird vollständig auf dem Olares-PVC gespeichert (rohes EML + Index). Der Provider ist nur Zustellpunkt.
2. **Kein Datenverlust**: Provider-Löschung passiert erst nach Verifikation und konfigurierbarer Wartezeit. Lokale Kopie wird nie automatisch gelöscht.
3. **Wählbarer Sync-Modus pro Account**: Mirror (Provider = Quelle) oder Archive (Provider wird entlastet).
4. **1 Instanz, 1 Konfig**: Keine Multiuser-Sessions; Web-UI mit einer Zugangs-Sperre (Olares Auth am Entrance reicht).
5. **Backup-fähig**: Alle Daten liegen in einem PVC — über Olares-Drive/Backup sicherbar.

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
│                        │  Archiv-Worker (Delete-Queue)                │ │
│                        └──────────────┬──────────────────────────────┘ │
│                        PVC: relay-data (/data/relay)                   │
│                        ├── archive/<account>/YYYY/MM/<uid>-<hash>.eml  │
│                        ├── attachments/<sha256>  (Dedup, on-demand)    │
│                        └── index.db (SQLite, WAL) + config.json        │
└────────────────────────────────────────────────────────────────────────┘
```

- **relay-server**: Rust-Core aus `tauri/src/` (imap/, smtp/, cache/, ai/, sync/, security/, tone/) wird zu eigenständigem axum-Binary. `tauri::command` → REST-Handler, `State` → `Arc<AppState>`.
- **relay-web**: SvelteKit-Frontend bleibt; `src/lib/services/tauri.ts` wird zum fetch-Client (gleiche Funktionsnamen → UI-Änderungen minimal).
- **Zugriff**: Entrance `relay` mit `authLevel: internal` → Olares-Auth schützt die App; kein eigenes Login nötig.

---

## 3. Speicher-Schema

### 3.1 Dateisystem (PVC)

```
/data/relay/
├── archive/<account-id>/YYYY/MM/<imap-uid>-<sha1-of-msgid>.eml   # roh, unveränderlich
├── attachments/<sha256-of-content>                                # dedupliziert
├── index.db                                                       # SQLite (WAL, synchronous=NORMAL)
└── config.json                                                    # AppConfig (Account-Secrets referenziert)
```

EML-Dateien = portables Format: Export, Migration, Backup, jederzeit mit mail-parser lesbar.

### 3.2 SQLite-Schema

```sql
-- Konten: Sync-Modus + Aufbewahrung pro Account
CREATE TABLE accounts (
  id            INTEGER PRIMARY KEY,
  email         TEXT NOT NULL UNIQUE,
  imap_host     TEXT NOT NULL, imap_user TEXT NOT NULL, imap_pass_ref TEXT,  -- Key im K8s-Secret
  smtp_host     TEXT, smtp_user TEXT, smtp_pass_ref TEXT,
  sync_mode     TEXT NOT NULL DEFAULT 'archive',   -- 'mirror' | 'archive'
  provider_retention_days INTEGER DEFAULT 30,       -- Archive: Wartezeit vor Provider-Delete
  created_at    TEXT DEFAULT (datetime('now'))
);

-- Ordner: Mirror-Sync aller; Archive: lokal-only Ordner zusätzlich möglich
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

-- Lösch-Queue (Archive-Modus)
CREATE TABLE delete_queue (
  id INTEGER PRIMARY KEY, message_id INTEGER NOT NULL REFERENCES messages(id),
  scheduled_at TEXT NOT NULL,                    -- provider_retention_days nach synced_at
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

---

## 4. Sync-Modi

### 4.1 Modus `mirror` (Default bei Erstkonfiguration)

- Voll-Sync Posteingang + Unterordner (wie heutiges Relay).
- Provider bleibt Quelle der Wahrheit; lokale DB = Cache.
- Keine Provider-Löschungen. → Sichere Startphase.

### 4.2 Modus `archive` (der neue Weg)

1. **Pull**: IMAP-Sync wie gehabt — neue Mails werden als EML gespeichert + indexiert.
2. **Prune-Planung**: Nach `synced_at + provider_retention_days` (Default 30) wird die Mail in die `delete_queue` eingereiht (Status `pending`).
3. **Provider-Delete** (siehe §5): Mail wird auf dem Provider permanent gelöscht, **lokal bleibt alles** (`status='pruned'`).
4. **Lokal-only Ordner**: Der Benutzer kann Ordner anlegen, die es nur lokal gibt (z. B. "Archiv/Steuer", "Archiv/Projekte") — dort verschobene Mails werden sofort aus der Delete-Queue genommen und dauerhaft lokal gehalten (wichtig: nicht mehr mit Provider synchronisierbar).

**Verhalten bei "verschieben in lokal-only Ordner"**: Mail bleibt lokal (EML + Index), Provider-Kopie wird vorgezogen gelöscht (Wartezeit kann übersprungen werden, wenn Benutzer bestätigt).

**Verhalten bei Mail-Löschung durch Benutzer (UI)**: Lokaler Papierkorb (30 Tage) → danach Hard-Delete von EML + Index. Provider-Löschung folgt demselben Flow, wenn die Mail dort noch existiert.

### 4.3 Zustellung neuer Mails

- IMAP-IDLE, Fallback 20s-Poll (bestehender Scheduler): neue UIDs → sofort lokal archivieren.
- Provider-Inbox wird damit stets nahezu leer gehalten (nur Mails jünger als Retention-Fenster + ungelesene).

---

## 5. Lösch-Workflow (Sicherheits-Pipeline)

```
pending ──(Fälligkeit erreicht)──▶ verified ──(IMAP-Delete ok)──▶ deleted
   │                                  │
   │ (Verify-Fehler)                  │ (IMAP-Fehler, max 5 Retries, Backoff)
   ▼                                  ▼
 failed ──(Benutzer-Review in UI)──▶ pending
```

1. **Verify**: `raw_path` existiert, Dateigröße stimmt, Index-Zeile vollständig, `sha256(raw_path)` == bei Pull berechnetem Hash.
2. **Provider-Delete (konfigurierbar)**: `STORE +FLAGS \Deleted` + `EXPUNGE` (hart) **oder** `MOVE` in Provider-`Trash` (weich, empfohlen als Default).
3. **Lokal unantastbar**: `status='pruned'` nur nach erfolgreichem Provider-Delete; EML + Index bleiben immer bestehen.
4. **Fehler**: Retry mit Backoff; Mails bleiben lokal — nie lokale Daten bei Provider-Fehler löschen.
5. **Bevor Archive-Modus aktiv wird**: Hinweis im UI, dass Backup eingerichtet sein sollte (Drive/PVC-Backup).

---

## 6. REST-API (Mapping aus `tauri/src/ipc.rs`)

Alle bestehenden Funktionen bleiben, nur Transport ändert sich (`invoke` → `fetch`):

| Bereich | Endpoints (Auszug) |
|---|---|
| Konten | `GET/POST/DELETE /api/v1/accounts`, `POST /api/v1/accounts/{id}/test` |
| Sync | `POST /api/v1/accounts/{id}/sync`, `GET /api/v1/accounts/{id}/sync/status` |
| Ordner | `GET /api/v1/accounts/{id}/folders`, `POST /api/v1/folders` (auch lokal-only), `PATCH /api/v1/folders/{id}` |
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
- Neu in den Einstellungen: Sync-Modus je Account (`mirror`/`archive`), `provider_retention_days`, Ordner-Typ (Provider/lokal-only), Delete-Queue-Review-Liste.
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
  title: AImighty Relay
  categories: [Utilities]
  version: 26.08.1
entrances:
  - name: relay-app
    port: 3000
    host: relay-app
    title: AImighty Relay
    icon: relay-icon.png
    openMethod: window
    authLevel: internal
spec:
  versionName: 26.08.1
  fullDescription: "**Model** / **Inference Engine** / **Key Features** / **API** / **Resource Usage** ..."
  developer: bayerhazard
  website: https://github.com/bayerhazard/relay
  sourceCode: https://github.com/bayerhazard/relay
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
  size: 100Gi
image:
  repository: ghcr.io/bayerhazard/relay
  tag: 26.08.1          # ohne v-Präfix
```

### Wichtige Olares-Fallen (aus Playbook)

- `apiVersion: 'v3'` + `olaresManifest.version: '0.12.0'` + `options.dependencies`-Pin → Validierung 1+2.
- Deployment-Replicas via `.Values.workloads.relay-app.replicaCount` (Validierung 3).
- Probes: startup `720/30s/60s`, liveness `5/30s/600s` (langes Booten bei Erst-Sync).
- Secret für IMAP/SMTP-Passwörter: als K8s-Secret, `pass_ref` referenziert daraus; app-service verlangt keine `OLARES_USER_*`-Namen → einfach halten.
- Upgrade-Falle: Values-Änderungen → `uninstall + install`, nie `upgrade`.

---

## 9. Backup & Wiederherstellung

| Was | Wie |
|---|---|
| EML-Archiv + Index | PVC-Inhalt (`/data/relay/`) — Olares-Backup oder Kopie nach Drive (Seafile-Sync) |
| Wiederherstellung | PVC-Restore, Container startet → `sync_state`-Vergleich: fehlende lokale Mails aus Provider nachziehen (falls noch vorhanden) |
| Konsistenz | SQLite WAL + `synchronous=NORMAL`; Backup-Fenster: `VACUUM INTO`-Kopie oder Datei-Snapshot |
| Export | Komplette MBox-/EML-Export-Funktion in den Einstellungen (Provider-unabhängig) |

**Migrations-Hinweis**: Bestehende Desktop-SQLite-DB kann importiert werden (EML-Rekonstruktion aus Cache-Content).

---

## 10. Roadmap

| Phase | Inhalt | Ergebnis |
|---|---|---|
| **P1 — Web-Migration** | Rust-Core → axum-Binary; `tauri.ts` → fetch-Client; Chart + Deploy auf market.AImighty | Relay läuft im Browser, Mirror-Modus, Zugriff von allen Geräten |
| **P2 — Archiv** | EML-Speicher + SQLite-Index (Schema §3), Sync-Modus `archive`, Delete-Queue + Verify-Pipeline (§5), lokal-only Ordner | Provider-Speicher wird entlastet, Backup-fähig |
| **P3 — Komfort** | FTS5-Suche im UI, Attachment-Dedup, mbox-Export, Backup-Integration (Drive), IMAP-IDLE | Produktionsreif |

**Empfohlene Reihenfolge**: P1 zuerst (schneller Nutzen, Risiko-mitigation), P2 danach — Archive-Modus erst, wenn Backup-Routine steht (§5.5).

---

## 11. Offene Fragen

1. Provider-Delete hart (`EXPUNGE`) oder weich (`MOVE` → Provider-Trash)? Default-Vorschlag: weich, hart konfigurierbar.
2. Ungelesene Mails: auch archivieren/prunen, oder erst lesen? Vorschlag: unabhängig vom Lesestatus nach Retention.
3. Sollen lokal-only Ordner per App-übergreifender Struktur (z. B. "Archiv") vorgegeben werden oder frei sein? Vorschlag: frei.
4. Konto-Migration von der Desktop-App: Cache-Import in P2 oder P3?
