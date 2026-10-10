# Testserver mit Beispieldaten

Für den Rundgang und zum Ausprobieren: ein Mailserver (GreenMail, IMAP und
SMTP) und ein Kalender- und Kontaktserver (Radicale, CalDAV und CardDAV),
beide lokal auf `127.0.0.1`, ohne Docker. Dazu ein Skript, das beide mit
erfundenen Daten füllt und Relay mit ihnen verbindet.

## Benutzen

Sie brauchen Java (ab 17) und Python 3. Aus dem Wurzelverzeichnis:

```bash
web/e2e/testserver/starten.sh                 # beide Server starten
# Relay mit leerem Datenordner starten, z. B. auf 127.0.0.1:3799
python3 web/e2e/testserver/befuellen.py       # füllen und Relay verbinden
web/e2e/testserver/stoppen.sh                 # beide Server stoppen
```

Ein anderes Relay geben Sie mit `--relay http://…` an; mit `--ohne-relay`
füllt das Skript nur die Testserver.

| Dienst | Adresse | Anmeldung |
|---|---|---|
| IMAP | `127.0.0.1:3143`, ohne TLS | `erika` / `geheim` |
| SMTP | `127.0.0.1:3025`, ohne TLS | `erika` / `geheim` |
| CalDAV/CardDAV | `http://127.0.0.1:5232/erika/` | `erika` / `geheim` |
| Google (Ersatz) | `http://127.0.0.1:5299/` | Client `relay-test.apps.googleusercontent.com` / `geheim-test` |

Der Ersatz für Google (`google.py`) spielt Anmeldung, Google Tasks und
Kalender und Kontakte (weitergereicht an Radicale) nach. Relay spricht ihn
an, wenn es mit `RELAY_GOOGLE_TEST_BASIS=http://127.0.0.1:5299` startet;
`googleprobe.py` prüft damit die Google-Anmeldung eines Gmail-Kontos.

## Was drin ist

Alle Daten sind erfunden, die Termine liegen relativ zu heute.

- **Mail:** 15 Nachrichten in Posteingang, Gesendet, Entwürfe, Archiv und
  Projekte; ungelesen, markiert, beantwortet, mit Anhängen.
- **Kontakte:** 8 Personen mit Adresse, Telefon und Firma.
- **Kalender:** „Arbeit“ und „Privat“ mit Terminen in dieser und der
  nächsten Woche, dazu zwei ganztägige.
- **Aufgaben:** 6 Aufgaben, eine überfällig, eine erledigt, mit Prioritäten.

Jeder Neustart beginnt leer: GreenMail hält die Mails nur im Speicher,
Radicales Ablage wird beim Start gelöscht. Downloads (GreenMail-Jar,
Radicale) und Protokolle liegen in `.lauf/`.
