# AImighty Relay One

Self-hosted web mail with a local-first archive. Runs as an Olares app (server + web UI) and serves every device with one config.

> Concept: see [CONCEPT_ARCHIVE.md](./CONCEPT_ARCHIVE.md)

## Positioning

- **1 instance, 1 config** — access from laptop and phone alike
- **Local-first** — mails are pulled from your provider into the Olares PVC (raw EML + SQLite index), freeing limited provider storage
- **You control your data** — backup via Olares Drive/backup, export anytime
- **Per-account sync modes** — `mirror` (provider stays source of truth) or `archive` (provider gets relieved after a verified delete pipeline)

## Stack

- `relay-server`: Rust (axum) — IMAP/SMTP, SQLite archive, AI client (OpenAI-compatible), sync engine
- `relay-web`: SvelteKit frontend (existing Relay UI)
- Deploy: Helm chart + OlaresManifest v3 for market.AImighty

## Voice (STT + TTS)

- **Dictation (STT)** — record into the assistant; Relay proxies to a configured OpenAI-compatible STT endpoint (`POST /voice/transcribe`).
- **Read-aloud (TTS)** — a speaker button on each assistant reply (plus optional auto-read) proxies to a configured OpenAI-compatible TTS endpoint (`POST /voice/speak`). Relay only forwards — there is no built-in TTS service. When TTS is not configured the endpoint returns HTTP 409 and the speaker button is hidden. Configure URL / key / model under **Settings → Voice**. An empty key sends no auth header.

## Status

Concept phase — see `CONCEPT_ARCHIVE.md` and `BACKLOG.md`.
