# Relay Phasen 2–4 — Umsetzungsplan (Referenzdokument)

> Gold-Standard-Playbook für die autonome Abarbeitung von Phase 2 (Mail↔Kalender),
> Phase 3 (Kontakte+Aufgaben) und Phase 4 (AI-First-Polish).
> Bei jedem Schritt auf dieses Dokument zurückgreifen.

## Status (Checkliste)

- [ ] Plan geschrieben (dieses Dokument)
- [ ] Phase 2.1 iMIP Outbound
- [ ] Phase 2.2 iMIP Inbound
- [ ] Phase 2.3 Einladungs-Queue
- [ ] Phase 2.4 Konflikte + AI-Alternativen
- [ ] Phase 2.5 Zeit-Extraktion + AI-RSVP
- [ ] **Phase 2 Verify** (tests + build + Browser-Check + Version-Bump + commit)
- [ ] Phase 3.1 Kontakte bidirektional
- [ ] Phase 3.2 Auto-Anreicherung aus Mail
- [ ] Phase 3.3 Aufgaben (VTODO)
- [ ] Phase 3.4 AI-Follow-ups
- [ ] **Phase 3 Verify**
- [ ] Phase 4.1 NL-Erstellung
- [ ] Phase 4.2 Smart Scheduling
- [ ] Phase 4.3 Meeting-Prep
- [ ] Phase 4.4 Agenda-Digest
- [ ] Phase 4.5 Globaler Assistent
- [ ] **Phase 4 Verify**
- [ ] **AI-Gate** (Experte via LiteLLM → alle AI-Features live testen)

## Entscheidungen (vom User bestätigt)

1. **iMIP = pragmatisch**: Mail + ICS-Attachment (`METHOD:REQUEST/REPLY/CANCEL`),
   RSVP-Status lokal in `invitations`/`event_attendees` tracken. Kein
   SCHEDULE-ATTENDEE/REQUEST-STATUS-Protokoll. Wie Thunderbird/Outlook/Apple Mail.
2. **AI-Test erst ganz am Ende (AI-Gate)**: AI-Features werden in ihren Phasen gebaut
   (Prompt-Builder + Handler + Frontend), aber **ohne Live-Modell** verifiziert
   (Prompt-Builder-Unit-Tests + graceful-Degradation 503). Der Live-Test aller
   AI-Features läuft erst im AI-Gate nach Phase 4 gegen `Experte` via LiteLLM.

## Verifikationsstrategie (pro Phase)

- **Kernfunktionen** (iMIP, Kontakte, Tasks, Konflikte-Query): **live-verify**
  (Radicale `https://cal.aimighty.olares.de` + Mail).
- **AI-Parts**: Prompt-Builder → Unit-Test (Input→erwarteter Prompt-String, kein Modell).
  Handler → graceful-Degradation-Test (ohne Modell → saubere 503, UI ausgegraut).
- **Pro Phase**: `cargo test` + `cargo build --release` (musl), Web-Tests (vitest),
  Browser-Check (Playwright), Version-Bump (Chart.yaml + OlaresManifest + upgradeDescription),
  commit + push (Auto-Deploy).

---

## Ausgangslage (was schon vorhanden ist)

### DB-Schema (`server/src/cache/db.rs`) — Tabellen existieren bereits

| Tabelle | Zeile | Zweck |
|---|---|---|
| `contacts` | 107 | vcard_uid, given/family/display_name, email, phone, organization, vcard_raw, source, synced_at, updated_at |
| `event_attendees` | 271 | event_id, email, name, part_stat, rsvp |
| `invitations` | 283 | event_uid, organizer, attendee_email, method (REQUEST/REPLY), status (NEEDS-ACTION/ACCEPTED/DECLINED/TENTATIVE), sequence |
| `todos` | 298 | calendar_id, uid, url, summary, description, due_at, completed_at, status, priority, ics_raw |
| `messages` | 35 | account_id, from/to/cc_addr, body_text, has_attachments, message_id, ai_summary, ai_priority |

### Infrastruktur (funktioniert schon)

- **SMTP** `server/src/smtp/client.rs`:
  `SmtpClient::send(to, cc, bcc, subject, body_text, body_html, in_reply_to, references, attachments)`.
  `EmailAttachment { filename, content (base64), content_type, size }`.
  → iMIP-Outbound: ICS-Attachment `text/calendar; method=REQUEST` + Korrelation via in_reply_to/references.
- **IMAP** `server/src/imap/client.rs`:
  `parse_message_attachments(raw) -> Vec<EmailAttachment>`, `parse_message_bodies(raw) -> (text, html)`.
  → iMIP-Inbound: ICS-Attachment in eingehender Mail erkennen.
- **CalDAV** `server/src/dav/caldav.rs`: `create_event`/`update_event`/`delete_event` (PUT/DELETE), `discover`, `sync`.
- **CardDAV** `server/src/dav/carddav.rs`: `fetch_all -> (Vec<Contact>, String)`, `sync_incremental`.
  `save_contacts_to_db` (in `dav/scheduler.rs`) upsertet Kontakte **schon** beim Sync.
- **vCard** `server/src/dav/vcard.rs`: `Contact { vcard_uid, given_name, family_name, display_name, email, phone, organization, vcard_raw }` + `parse_vcard(raw) -> Contact`.
- **AI** `server/src/ai/client.rs` + `server/src/ai/prompts.rs`:
  `AIClient::complete_user`/`complete_background`/`stream_completion`, Circuit-Breaker, Semaphore(1).
  Prompt-Builder-Muster: `fn foo(...) -> (String, String)` (system, user).
- **icalendar 0.17.13**: `Calendar::todos()`, `Todo` (new/with_uid/done/percent_complete/due/priority/summary/description),
  `METHOD` via `property_value("METHOD")`/`append_property(Property::new("METHOD","REQUEST"))`,
  `REQUEST-STATUS`, `VTIMEZONE` via `timezone(tz)`.
- **EventBus** `AppState.events`: Server→Client-Notifications (Echtzeit: neue Einladung, RSVP).

### Wichtige Patterns

- `ApiResult<T>` = `Result<Json<T>, ApiError>`. Handler: `Ok(Json(value))`.
- `with_db` liefert `&Connection`; Mutationen via `get_db(&state).as_mut()`.
- AI-Handler-Muster: Handler → Prompt-Builder (prompts.rs) → `state.ai.complete_user(...)`.
- Route-Registrierung in `server/src/api/mod.rs`. Kalender-Events unter `/calendars/events*`
  (`/events` = Health-SSE, NIE belegen).
- Frontend: SvelteKit, Routen `/` (Mail), `/calendar`, `/settings`. Pro-Seite-Navigation (kein zentrales Nav).
  Services in `web/src/lib/services/tauri.ts`. i18n `web/src/lib/i18n.ts` (de/en, `t` derived).
  Design-Tokens `web/src/styles/global.css` (--b-100..900, --gold, --color-*, --radius-s/m/l, --am-raum-*).

### Zu bauen (Übersicht)

- `server/src/imip/` (neu): `mod.rs`, `outbound.rs`, `inbound.rs`
- `server/src/cache/contacts.rs`, `cache/todos.rs` (neu, DAOs)
- `server/src/api/invitations.rs`, `api/contacts.rs`, `api/todos.rs` (neu)
- Extend: `dav/ics.rs` (METHOD/ATTENDEE-RSVP/ORGANIZER/Todo), `dav/caldav.rs` (VTODO),
  `dav/carddav.rs` (put/delete vCard), `api/calendars.rs` (invite/conflicts), `api/ai.rs` (neue Handler),
  `ai/prompts.rs` (neue Builder), `sync/scheduler.rs` (iMIP-Inbound + Auto-Anreicherung)
- Frontend: `routes/contacts/`, `routes/tasks/`, Einladungs-Queue-Panel, Konflikt-Banner,
  Zeit-Extraktions-Aktion, globaler Assistent-Drawer

---

## Phase 2 — Mail ↔ Kalender

**Gate:** iMIP + Konflikte + Zeit-Extraktion live (Radicale + Mail). AI-Parts via Unit-Test.

### 2.1 iMIP Outbound (Einladungen senden)

**Ziel:** Event mit Teilnehmern → Einladung per Mail (ICS-Attachment `METHOD:REQUEST`).

- **ICS bauen** (`dav/ics.rs` extend):
  - `IcsEvent` um `organizer: Option<String>`, `attendees: Vec<IcsAttendee>` erweitern
  - `IcsAttendee { email, name, part_stat, rsvp, role }`
  - `build_ics_with_method(event, method)` → VCALENDAR mit `METHOD`, `ORGANIZER`, `ATTENDEE`
  - `ATTENDEE;CN=<name>;ROLE=REQ-PARTICIPANT;RSVP=TRUE:mailto:<email>`
- **Modul** `server/src/imip/mod.rs` + `imip/outbound.rs`:
  - `send_invitation(state, event_id, attendees)`:
    1. Event aus DB laden, ICS `METHOD:REQUEST` bauen
    2. SMTP-Versand an jeden Teilnehmer (ICS-Attachment `text/calendar; method=REQUEST` + HTML/Text-Body mit Details)
    3. DB: `invitations` upserten (event_uid, organizer, attendee_email, method=REQUEST, status=NEEDS-ACTION, sequence)
    4. DB: `event_attendees` upserten (event_id, email, name, part_stat=NEEDS-ACTION, rsvp)
    5. Event auf CalDAV-Server aktualisieren (ATTENDEEs persistieren) via `caldav.update_event`
- **API** (`api/calendars.rs`): `POST /calendars/events/{id}/invite` body `{attendees: [{email, name}]}`
- **Tests:** ICS REQUEST bauen + zurückparsen (ORGANIZER/ATTENDEE/METHOD), SMTP-TestOverride (send-Call), DB-Rows.

### 2.2 iMIP Inbound (Einladungen empfangen)

**Ziel:** Eingehende Mail mit ICS → Event anlegen/aktualisieren + RSVP-Status.

- **Modul** `server/src/imip/inbound.rs`:
  - `process_inbound_ics(state, message, ics_bytes)`:
    1. `METHOD` parsen (`property_value("METHOD")`)
    2. **REQUEST**: Event parsen (organizer ≠ self) → Event upserten (als Einladung markieren),
       `invitations` (method=REQUEST, status=NEEDS-ACTION), `event_attendees`, in Queue
    3. **REPLY**: `ATTENDEE` mit `RSVP=TRUE` + `PARTSTAT` → `event_attendees.part_stat` + `invitations.status` updaten
    4. **CANCEL**: Event als abgesagt markieren (status)
  - `sequence`-Handling: neue Sequence überschreibt alte (Update vs. Neu)
- **IMAP-Sync-Hook** (`sync/scheduler.rs`): beim Verarbeiten eingehender Mails
  `parse_message_attachments` → ICS (content_type `text/calendar`) → `process_inbound_ics`.
- **Tests:** REQUEST/REPLY/CANCEL-Fixtures parsen, DB-Zustand, sequence-Update.

### 2.3 Einladungs-Queue (UI + API + RSVP)

**Ziel:** Ausstehende Einladungen anzeigen + accept/decline/tentative.

- **API** `server/src/api/invitations.rs` (neu):
  - `GET /invitations` → ausstehende (status=NEEDS-ACTION), mit Event-Daten
  - `POST /invitations/{uid}/rsvp` body `{decision: accept|decline|tentative}`:
    1. `METHOD:REPLY`-ICS bauen (eigener ATTENDEE mit PARTSTAT + RSVP=TRUE)
    2. SMTP an Organizer senden
    3. `invitations.status` + `event_attendees.part_stat` updaten
- **Frontend:** Einladungs-Queue-Panel im Kalender (`routes/calendar/+page.svelte` oder eigene Sektion):
  Liste ausstehender Einladungen, accept/decline/tentative Buttons, Badge mit Anzahl.
- **Tests:** RSVP-Handler (REPLY-ICS bauen + DB-Update), GET-Filter.

### 2.4 Konflikte + AI-Alternativen

**Ziel:** Überlappende Events erkennen + AI-Vorschläge.

- **Overlap-Query** (`api/calendars.rs`): `GET /calendars/conflicts?start=&end=` →
  Events mit `[start < end_param AND end > start_param]` (alle Kalender).
- **AI** (`ai/prompts.rs` + `api/ai.rs`): `POST /ai/conflict-alternatives`
  body `{event: {title, start, end, duration_min}, conflicts: [...]}` → JSON `{suggestions: [{start, end}]}`.
  Prompt-Builder `conflict_alternatives(...) -> (String, String)`.
- **Frontend:** Konflikt-Banner im Event-Editor (wenn Overlap) + "AI-Vorschläge"-Button.
- **Tests:** Overlap-Query (Unit, diverse Fälle), Prompt-Builder-String.

### 2.5 Zeit-Extraktion + AI-RSVP

**Ziel:** Aus Mail-Text Termin-Daten extrahieren + AI-RSVP-Entwurf.

- **AI** (`ai/prompts.rs` + `api/ai.rs`):
  - `POST /ai/extract-time` body `{text}` → JSON `{start, end, title, attendees?}`.
    Prompt-Builder `extract_time(...)`.
  - `POST /ai/rsvp-draft` body `{event, decision, note?}` → `{reply_text}`.
    Prompt-Builder `rsvp_draft(...)`.
- **Frontend:** Mail → "Zu Termin machen"-Aktion (extract-time → Event-Editor vorbefüllen);
  RSVP-Entwurf in Einladungs-Queue.
- **Tests:** Prompt-Builder-Strings (beide), JSON-Parsing der AI-Antwort (Mock).

### Phase 2 Verify

- `cargo test` (alle neu + bestehend grün), `cargo build --release --target x86_64-unknown-linux-musl`
- Web-Tests (vitest) für neue Komponenten
- Browser-Check: Einladungs-Queue, Konflikt-Banner, Zeit-Extraktion
- **Version-Bump 26.09.95 → 26.09.96** (Chart.yaml version+appVersion, OlaresManifest version+versionName, upgradeDescription neue Zeile)
- commit `relay 26.09.96: Mail↔Kalender (iMIP, Konflikte, Zeit-Extraktion) — Phase 2` + push

---

## Phase 3 — Kontakte + Aufgaben

**Gate:** Kontakte bidirektional + Auto-Anreicherung, VTODO + AI-Follow-ups live. AI-Parts via Unit-Test.

### 3.1 Kontakte-Modul (bidirektional)

**Ziel:** Kontakte anzeigen/suchen/erstellen/bearbeiten/löschen, Roundtrip zu CardDAV.

- **DAO** `server/src/cache/contacts.rs` (neu):
  - `ContactRow` struct, `list_contacts(conn, q) -> Vec<ContactRow>` (Suche über name/email/org),
    `get_contact(conn, vcard_uid)`, `upsert_contact(conn, &Contact, source)`, `delete_contact(conn, vcard_uid)`
- **API** `server/src/api/contacts.rs` (neu):
  - `GET /contacts?q=` (Liste+Suche), `GET /contacts/{uid}`,
    `POST /contacts` (neu anlegen → vCard bauen → CardDAV-PUT),
    `PUT /contacts/{uid}` (bearbeiten → CardDAV-PUT), `DELETE /contacts/{uid}` (→ CardDAV-DELETE)
- **CardDAV bidirektional** (`dav/carddav.rs` extend): `put_vcard(client, vcard_raw)`, `delete_vcard(client, uid)`
- **vCard-Builder** (`dav/vcard.rs` extend): `build_vcard(&Contact) -> String` (parse_vcard schon da)
- **Frontend** `web/src/routes/contacts/+page.svelte` (neu): Liste + Suche + Detail + Edit-Formular
- **Tests:** DAO CRUD, vCard Parse/Build Roundtrip, CardDAV PUT/DELETE (Mock), API-Handler.

### 3.2 Auto-Anreicherung aus Mail

**Ziel:** Kontakte automatisch aus Mail-Verkehr anlegen/anreichern.

- **Sync-Hook** (`sync/scheduler.rs` oder IMAP-Sync): beim Verarbeiten von Mails
  (Gesendet + Eingehend) → from/to/cc (Name+Email) → `contacts` upserten (source='mail').
  - Bestandskontakte: neue E-Mail/Telefon ergänzen (nicht überschreiben vorhandene).
  - Nur plausible Adressen (kein no-reply, postmaster, etc. filtern).
- **DAO**: `upsert_contact_from_email(conn, name, email, source)`
- **Tests:** Mail → contacts upsert (Unit), Filter-Logik.

### 3.3 Aufgaben (VTODO)

**Ziel:** Aufgaben aus CalDAV syncen + anlegen/erledigen.

- **CalDAV VTODO** (`dav/caldav.rs` extend): VTODO im Sync miterfassen (PROPFIND + ICS).
- **ICS Todo** (`dav/ics.rs` extend): `Todo` parsen/bauen (summary, due_at, completed, priority, description).
- **DAO** `server/src/cache/todos.rs` (neu): `TodoRow`, `list_todos(conn, filter)`, `get_todo`,
  `upsert_todo`, `delete_todo`, `toggle_complete`
- **API** `server/src/api/todos.rs` (neu): `GET /todos` (filter: today/open/done),
  `POST /todos`, `PUT /todos/{id}`, `DELETE /todos/{id}`, `POST /todos/{id}/complete`
- **Frontend** `web/src/routes/tasks/+page.svelte` (neu): Heute/Offen/Erledigt, Task anlegen, togglen, fällig
- **Tests:** VTODO Parse/Build, DAO CRUD, toggle, CalDAV VTODO-Sync (Mock).

### 3.4 AI-Follow-ups

**Ziel:** Aus Mail Follow-up-Aktionen extrahieren → Tasks.

- **AI** (`ai/prompts.rs` + `api/ai.rs`): `POST /ai/extract-followups` body `{text}` →
  JSON `{followups: [{task, due?, who?}]}`. Prompt-Builder `extract_followups(...)`.
- **Frontend:** Mail → "Follow-ups extrahieren" → Tasks anlegen (bestätigen).
- **Tests:** Prompt-Builder-String, JSON-Parsing (Mock).

### Phase 3 Verify

- `cargo test` + `cargo build --release --target x86_64-unknown-linux-musl`
- Web-Tests (vitest) für Contacts + Tasks
- Browser-Check: Kontakte-UI, Tasks-UI
- **Version-Bump 26.09.96 → 26.09.97** (alle 3 Stellen + upgradeDescription)
- commit `relay 26.09.97: Kontakte + Aufgaben (VTODO) — Phase 3` + push

---

## Phase 4 — AI-First-Polish

**Gate:** NL-Erstellung, Smart Scheduling, Meeting-Prep, Agenda-Digest, globaler Assistent.
**Centerpiece = 4.5 (Assistent).** AI-Parts via Unit-Test (Live erst im AI-Gate).

### 4.1 NL-Erstellung

- **AI** (`ai/prompts.rs` + `api/ai.rs`): `POST /ai/nl-create` body `{text, context?}` →
  JSON `{type: event|task, title, start?, end?, attendees?, description?}`. Intent-Erkennung.
  Prompt-Builder `nl_create(...)`.
- **Frontend:** Composer-Box in Kalender + Tasks ("Morgen 14 Uhr Kaffee mit Anna" → anlegen).
- **Tests:** Prompt-Builder, Intent-Parsing (Mock-Antworten).

### 4.2 Smart Scheduling

- **AI**: `POST /ai/schedule` body `{request, participants?, free_slots?, constraints?}` →
  JSON `{suggestions: [{start, end, confidence, reason}]}`. Nutzt 2.4-Konflikt-Detection.
  Prompt-Builder `smart_schedule(...)`.
- **Tests:** Prompt-Builder, Vorschlags-Logik (Mock).

### 4.3 Meeting-Prep

- **AI**: `POST /ai/meeting-prep` body `{event}` →
  JSON `{attendees: [...], related_mails: [...], agenda: [...], prep_notes}`.
  Zieht Kontakte (3.1) + relevante Mails. Prompt-Builder `meeting_prep(...)`.
- **Frontend:** Event-Detail → "Meeting-Prep"-Panel.
- **Tests:** Prompt-Builder, Datenzusammenführung (Kontakte+Mails).

### 4.4 Agenda-Digest

- **AI**: `POST /ai/agenda-digest` body `{date, horizon?}` →
  JSON `{digest, priorities: [...], followups: [...]}`. Prompt-Builder `agenda_digest(...)`.
- **Frontend:** Morgen-Digest (Tagesansicht).
- **Tests:** Prompt-Builder.

### 4.5 Globaler Assistent (Centerpiece)

- **AI**: `POST /ai/assistant` body `{message, context?}` →
  JSON `{reply, actions: [{type, payload}]}`. Actions rufen andere APIs auf
  (event_create, task_create, find_mail, schedule, ...). Prompt-Builder `assistant(...)`.
- **Frontend:** globaler Assistent-Drawer (von überall erreichbar), Chat-UI, Action-Vorschau.
- **Tests:** Prompt-Builder, Action-Routing (Mock-AI).

### Phase 4 Verify

- `cargo test` + `cargo build --release --target x86_64-unknown-linux-musl`
- Web-Tests (vitest) für Assistent-Drawer + Composer
- Browser-Check: Assistent-UI, Composer, Prep/Digest-Panels
- **Version-Bump 26.09.97 → 26.09.98** (alle 3 Stellen + upgradeDescription)
- commit `relay 26.09.98: AI-First (NL, Scheduling, Prep, Digest, Assistent) — Phase 4` + push

---

## AI-Gate (nach Phase 4 — ganz am Ende)

**Erst dann live AI testen.** Ablauf:

1. `Experte` via LiteLLM in Relay-Settings konfigurieren (AI-Settings-Tab).
2. Alle AI-Features end-to-end live testen:
   - 2.4 `POST /ai/conflict-alternatives`
   - 2.5 `POST /ai/extract-time`, `POST /ai/rsvp-draft`
   - 3.4 `POST /ai/extract-followups`
   - 4.1 `POST /ai/nl-create`
   - 4.2 `POST /ai/schedule`
   - 4.3 `POST /ai/meeting-prep`
   - 4.4 `POST /ai/agenda-digest`
   - 4.5 `POST /ai/assistant`
3. Prompt-Qualität prüfen (Output-Sinnvollkeit, JSON-Validität, Latenz).
4. Ggf. Prompt-Feinschliff + finaler Version-Bump + commit.
5. **An User melden: bereit für AI-Gate.**

---

## Querschnitt & Reihenfolge

- **AI-Abhängigkeit:** Alle AI-Features brauchen ein konfiguriertes Modell. Ohne Modell →
  graceful 503 (Feature ausgegraut, Kern ohne AI nutzbar). Live erst im AI-Gate.
- **Reihenfolge (Phase-Gates):**
  1. Phase 2 (2.1→2.5) → live-Verify
  2. Phase 3 (3.1→3.4) → live-Verify
  3. Phase 4 (4.1→4.5, Assistent zuletzt) → live-Verify
  4. AI-Gate (Experte via LiteLLM → alle AI-Features live)
- **Pro Phase:** Version-Bump (Chart.yaml + OlaresManifest + upgradeDescription) → commit →
  push (Auto-Deploy) → Browser-Check → live-Verify.
- **Konventionen:** Titel `AIM ...` nicht relevant (Relay ist 1 App). Version `YY.MM.<n>` pro App.
  `name`/`appid`/K8s-Ressourcen NIE ändern. Routen unter `/calendars/*`, `/contacts`, `/todos`,
  `/invitations`, `/ai/*` (`/events` = Health-SSE, nicht belegen).
- **Deploys:** Git push → Auto-Deploy (market-src). 4 GiB RAM, 4 CPU, musl-Target, rustc 1.91.1.

