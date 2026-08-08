# Relay One — Handoff (2026-08-08)

Wechsle NICHT in `/home/opencode/workspace/relay-repo` (das ist die alte Tauri-App, nur als Referenz). Arbeite in `/home/opencode/workspace/relay-one`.

## Umgebungs-Limits (KRITISCH — neu gemessen)

- Host zeigt 96 GB / 24 Cores, aber der Container hat real nur **4 GiB RAM (kein Swap)** und **4 CPU-Quota** (`/sys/fs/cgroup/memory.max`, `cpu.max`).
- `nproc`=24 ist irreführend. **Vitest NIEMALS mit `--max-old-space-size` > 3072** starten (OOM-Kill bei MailboxPage-Kompilierung, riesige `+page.svelte`).
- Port **3000 ist vom opencode-Server selbst belegt** (PID 1 `opencode web --port 3000`). Rust-Server-Smoke-Tests nur mit `RELAY_BIND` auf freiem Port (z. B. 3799).
- Einzelne Tool-Calls können **>90 s** dauern (getestet: 160 s `sleep` ok). Bei langen Tests im Vordergrund: Hintergrund + `setsid nohup … &` + `disown`, Log in `web/` (überlebt Session-Resets), `pgrep -f "[M]uster"` (Klammer verhindert Selbst-Match).
- Hintergrundprozesse sterben bei Session-Interrupts; `/tmp` wird geleert — Logs in den Workspace legen.

## Kontext-/Abbruch-Problem (deshalb neue Session)

Das Modell (`deepseek-v4-flash`) hat **max. 1.048.576 Tokens Context**. Die alte Session war darüber gewachsen → Provider-Stream brach bei jedem großen Request ab ("stream error … compaction … requested > max context"). Kompaktion scheiterte zusätzlich an Rate-Limits. → **Konversationen kurz halten, keine langen Log-Dumps**, große Dateien nur per `grep`/Ausschnitt lesen.

## Stand

- `server/`: axum-Rust-Kern **fertig + gepusht** (`e88b561`). 381 Tests grün, Smoke-Test bestanden. Nur noch wartbar in `web/`.
- `web/`: SvelteKit-Frontend migriert, **UNCOMMITTET** (`?? web/`). Rust-Teil nicht mehr anfassen.

### Web-Tests: was schon grün ist
- format, formatDate, diff, accounts, settings, voice, mailbox + Dialoge/Komponenten (ConfirmationDialog … ToneControls, MessageList) — **233 Tests grün**.

### Web-Tests: was noch fehlt / zu prüfen
1. `MailboxPage.test.ts` — kompiliert die riesige `+page.svelte`, dauert ~3,5 min, braucht viel RAM. Wurde nie fertig durchgelaufen.
2. `ComposeWindow.test.ts` — noch unverifiziert.

## Nächste Schritte (in dieser Reihenfolge)

1. `MailboxPage.test.ts` isoliert:
   ```bash
   cd /home/opencode/workspace/relay-one/web
   NODE_OPTIONS="--max-old-space-size=3072" setsid nohup npx vitest run src/lib/__tests__/MailboxPage.test.ts > vitest-mb.log 2>&1 </dev/null & disown
   # dann mit kurzen Calls pollen; Log in web/ lesen (Ergebniszeilen: "Test Files/Tests")
   ```
2. `ComposeWindow.test.ts` ebenso.
3. `npm run build` (SvelteKit static → `web/build/`).
4. Wenn Tests + Build grün: Web-Commit + Push nach `relay-one`:
   ```bash
   cd /home/opencode/workspace/relay-one
   git add web/ && git commit -m "feat(web): de-tauri — REST-Client (tauri.ts), SSE, HTML-Kontextmenü, vitest singleFork, ontoggleFlag-Fix"
   git push
   ```
5. Danach Helm-Chart + `OlaresManifest.yaml` v3 für `relay-app` (Skizze in `CONCEPT_ARCHIVE.md` §8: PVC `/data/Relay`, Entrance `relay-app`, startup Probe `720/30s/60s`, `options.dependencies` olares `>=1.12.6-0`) und Deploy auf `market.AImighty` vorbereiten.

## Bekannte Web-Fixes (bereits drin)
- `src/lib/services/tauri.ts` = fetch-basierter REST-Client (ersetzt Tauri IPC; gleiche Funktionsnamen, `openEventStream` = SSE).
- `MessageList.svelte`: HTML-Kontextmenü statt Tauri-Menü; `ontoggleFlag` in `$props()`-Destrukturierung ergänzt (war "is not defined").
- `vitest.config.ts`: `pool: "forks"`, `singleFork`, `maxWorkers: 1` gegen OOM.
- `vite.config.ts`: `/api`-Proxy → `http://127.0.0.1:3000`; dev-Base `/__preview/<PORT>/`.

## Referenzen
- `CONCEPT_ARCHIVE.md` — vollständiges Konzept, Entscheidungen F1–F7, Chart-Skizze §8.
- `/home/opencode/workspace/relay-repo/tauri/src/ipc.rs` — Original-Commands (Referenz für Portierung).
