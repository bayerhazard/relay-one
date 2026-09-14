# Mobile GUI-Review — 2026-09-14 (Relay, 390×844)

Gerüst. Screenshots: `docs/review-mobile-2026-09-14/` (15 Stück).


## Methode
- Playwright: Viewport 390×844 (iPhone-14-artig), deviceScaleFactor 2.
- Touch: eigener Browser-Context mit `hasTouch:true, isMobile:true` (`pointer:coarse=true`).
- Dev-Preview: `https://1f47cd9b0.aimighty.olares.de/__preview/5173/` (state: Desktop-Review 26.9.157).
- Kein Security-Scope.

## Umsetzung (26.9.158, nach Review)
| ID | Fix | Wo |
|---|---|---|
| M1 | Escape schließt mobilen Mail-Drawer (vor Menüs/Kompose/Confirm) | +page.svelte `handleKeydown` |
| M2 | Hamburger-Burger ☰ → inline SVG in Mail, Calendar, Contacts, Tasks, Meetings | jeweilige `*menu-toggle` Buttons |
| M3 | Calendar: Toolbar zwei-zeilig auf narrow (grid-template-areas), Kompakt-Paddings, Monatslabel-Ellipsis, `min-width:0` am Event-Chip | calendar/+page.svelte CSS |
| M4a | RFC2047-Decode in `parse_address` (contacts cache speichert keine „=?utf-8?B?…"-Namen mehr; Test `test_parse_address_rfc2047_name`) | server/src/cache/contacts.rs |
| M4b | `padding-bottom: 84px` gegen FAB-Overlap | .ct-list / .tk-list |
| M5 | Add-Task-Button im mobil Header | tasks/+page.svelte `.tk-mobile-new` |
| M6 | `fabHidden`-Store; AssistantFab versteckt sich bei offenem Drawer/Sheet (alle 6 Module setzen den Flag) | web/src/lib/stores/fabHidden.ts + 6 Pages + AssistantFab.svelte |
| M7 | Swipe-Labels „Ungelesen/Gelesen" → `$t("mail.markUnread"/"mail.markRead")` | MessageList.svelte:336 |

Tests: cargo 604 ✓ · vitest 448 ✓ · svelte-check 0 errors ✓ · Live-Verify 390×844 (Escape/FAB-SVG/Calendar-Toolbar/Add-Task) ✓.


## Module-Verifikation

| Modul | Screenshot | Status | Anmerkung |
|---|---|---|---|
| Mail (Liste) | mob-mail.png, mob-mail2.png | ✅ läuft | Datum „11:30 PM" deutsch-Locale EN ok |
| Mail (Drawer) | mob-mail-drawer.png | ✅ öffnet | FAB überlappt suche (M6) |
| Mail (Stuck) | mob-mail-stuck.png | 🔴 M1 | Liste tot nach Scrim-Interception |
| Mail (Swipe→Delete) | mob-mail-swipe2.png | ✅ funktioniert | Full-Swipe -320px, ConfirmationDialog korrekt, Labels hartdeutsch (M7) |
| Calendar | mob-calendar.png | 🟠 M2, M3 | View-Toggle abgeschnitten, Chips übertreten Zellgrenzen, Tofu |
| Contacts | mob-contacts.png | 🟡 M4 | MIME-Rohname „=?utf-8?B?…", FAB überlappt letzte Row |
| Tasks | mob-tasks.png | 🟡 M5 | Kein Add-Task-Affordance sichtbar |
| Meetings (leer) | mob-meetings.png, mob-meetings-list.png | 🟠 M2 | Nur Detail-Pane „Select a meeting"; Liste nur via (Tofu-)Menu-Toggle |
| Meetings (Drawer) | mob-meetings-list2.png | ✅ | Liste öffnet sauber über Drawer |
| Meetings (Detail) | mob-meetings-detail.png | ✅ | Detail-Seite sauber auf 390px |
| Settings | mob-settings.png, mob-settings-ai.png | ✅ | Nav-Liste + Section-Page i.o. |

## Befunde

### M1 🔴 Mail-Drawer: Escape schließt nicht, Scrim/Tree fängt Pointer (Stuck-Zustand)
- Repro (deterministisch, main page): Drawer via Menu-Toggle öffnen → `Escape` → Sidebar bleibt bei `transform: matrix(1,0,0,1,0,0)` (offen); `elementFromPoint(195,120)` trifft `tree-label("Spam")` statt Mail-Row → Mail-Liste komplett unbedienbar.
- Recovery: Klick auf `.sidebar-close` (mob-mail2.png → Liste wieder frei).
- Erwartet: Escape schließt Drawer (wie Desktop) **und**, solange Drawer offen, `pointer-events` auf Tree/Scrim-Panes fangen nur innerhalb des Drawer-Bereichs.
- Ort: `web/src/routes/+page.svelte` (mobile drawer / Esc-Handling / `.sidebar-pane` Layout).

### M2 🟠 Tofu-Glyphen (Mobile-Fonts rendern Unicode-Icons als Kästchen)
- Hamburger `&#9776;` (U+2630) in Mail (`.menu-toggle`) und Meetings (`.mt-menu-toggle`, aria "Menu") → ☒-Kästchen.
- Kalender-Header ebenfalls Tofu (mglw. derselbe/das Kalender-Emoji aus dem Desktop-Review).
- Fix: wie Desktop-Review begonnen → konsequent SVGIcon statt Unicode/Emoji.

### M3 🟠 Calendar 390px: Toggle abgeschnitten, All-Day-Chips überlaufen
- Header rechts: „Month | We…" am Viewport-Rand abgeschnitten (mob-calendar.png).
- Event-Chips („All day", „12:00", „08:40") ragen über Zellgrenzen in Nachbarzellen — Megafon-Effekt bei vielen Zellen (01., 11., 16., 19., 20., 23.).
- Fix: View-Toggle platzsparend — auf 390px Kurz-Labels „M/W/T/D" oder Icon-Buttons; Chips `overflow:hidden; text-overflow:ellipsis` + `min-width:0` gegen Überlauf in Nachbarzellen.

### M4 🟡 Contacts: MIME-Rohname sichtbar, FAB überlappt letzte Row
- „=?utf-8?B?…" wird 1:1 als Anzeigename gezeigt — RFC2047-Decoder fehlt (auch Desktoprelevant, neu).
- Assistant-FAB (fixed bottom-right) liegt über dem Delete-Button der letzten sichtbaren Row; Fix: `padding-bottom: calc(64px + env(safe-area-inset-bottom))` am Listcontainer (iOS-Geräte real: Black-Home-Balken).

### M5 🟡 Tasks: kein Add-Affordance auf 390px
- Nur Liste sichtbar (mob-tasks.png); kein sichtbarer „Neue Aufgabe"-Button, kein FAB. Auf Desktop via Toolbar; auf Mobile nicht erreichbar (oder via ContextMenu? nicht getestet auf Add-Pfad).
- Fix: Mobile-FAB oder Header-Plus-Button.

### M6 🟡 Assistant-FAB kollidiert mit Drawer/Sheets
- Mail-Drawer: FAB sitzt über der Suche im Drawer-Bereich (mob-mail-drawer.png).
- Meetings-Drawer: FAB überlappt Bottombar der Liste (mob-meetings-list2.png).
- Fix: FAB ausblenden, solange ein Drawer/Sheet/Bottomnav offen ist — zentral via Store-Flag `assistantFabHidden`, das die jeweilige Page setzt.

### M7 🟠 Hardcoded-Strings im Touch-Pfad (Nachfolger von T4)
- `MessageList.svelte:336` hardcoded „Ungelesen"/„Gelesen" (swipe-label). Auf `$t("mail.markUnread")`/`$t("mail.markRead")` (oder vorhandene Keys für markRead/markUnread) migrieren.
- Swipe→Delete-Dialog renderte im Touch-Context deutsch („Posteingang", „Nachricht in Papierkorb verschieben") — Sprachauflösung ist.ok. (Auto-Detect), aber Keys konsolidieren.

### M8 ✅ funktioniert korrekt (kein Handlungsbedarf)
- Mail-Swipe → gelesen/geflaggt/Papierkorb (ConfirmationDialog-Umweg korrekt).
- Long-Press 500ms → Kontextmenü-Sheet (MessageList-Path).
- Meetings: Listen-Drawer + Detail-Page sauber getrennt; Settings-Navigationsstruktur auf 390 ok.
- Kalender-Grid selbst (Wochentagskopf, KW-Zeilen, Selektion „14") i.o.; ics-drawing von Refresh-Icon (aus Desktop-Review) ok.

## Breakpoint-Konsistenz (aus Code)
| | Breakpoint(s) | Hinweis |
|---|---|---|
| Mail | `isNarrow ≤600`, `isCompact ≤900` | schmaler als Rest |
| Calendar/Contacts/Tasks/Meetings | `≤768` (Meetings: zusätzlich 718) | JS-state only |
| Settings | `≤600` | Tabs |
- Empfehlung: einheitlicher Mobile-Trigger `≤768`; Mail behält `≤900` nur für Compact-Detail-Pane.
