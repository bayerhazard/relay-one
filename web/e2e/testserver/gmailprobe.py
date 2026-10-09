#!/usr/bin/env python3
"""Relay against a real Gmail account (Kai, 9.10.2026).

The test server (GreenMail) has no labels and no "All Mail"; what Relay
does on Gmail is checked here, against a Gmail account kept for tests:

    GMAIL_TEST_USER=…@gmail.com GMAIL_TEST_PASSWORT=<app password> \\
      python3 web/e2e/testserver/gmailprobe.py --relay http://127.0.0.1:3801

Relay must be empty (a data folder of its own). A mail of its own goes
into the Gmail inbox; Relay archives it (inbox label off, still in "All
Mail", not in the trash), takes it back, makes a label and moves it there,
deletes it (Gmail's trash), restores it (back in the inbox — not lost on
the way), deletes it for good, and deletes the label. Gmail is asked after
each step. Without the two variables it says so and exits 0.
"""

import argparse
import imaplib
import json
import os
import re
import sys
import time
import urllib.request
from email.message import EmailMessage
from email.utils import formatdate, make_msgid

HOST = "imap.gmail.com"


def api(relay, methode, pfad, daten=None):
    body = json.dumps(daten).encode() if daten is not None else None
    req = urllib.request.Request(relay + "/api/v1" + pfad, data=body, method=methode)
    if body is not None:
        req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req, timeout=120) as antwort:
        text = antwort.read().decode() or "null"
        return json.loads(text)


class Gmail:
    def __init__(self, user, passwort):
        self.user, self.passwort = user, passwort

    def mit(self, arbeit):
        imap = imaplib.IMAP4_SSL(HOST)
        try:
            imap.login(self.user, self.passwort)
            return arbeit(imap)
        finally:
            imap.logout()

    def ordner(self):
        """name -> attributes, names as Gmail sends them (modified UTF-7)."""
        def lesen(imap):
            _, zeilen = imap.list()
            aus = {}
            for z in zeilen:
                m = re.match(rb'\((.*?)\) "(.*?)" (.*)$', z)
                if m:
                    aus[m.group(3).decode().strip('"')] = m.group(1).decode()
            return aus
        return self.mit(lesen)

    def besonders(self, attribut):
        return next(n for n, a in self.ordner().items() if attribut in a)

    def labels(self, betreff):
        """X-GM-LABELS of the mail in "All Mail", None if not there."""
        alle = self.besonders("\\All")

        def lesen(imap):
            imap.select(f'"{alle}"', readonly=True)
            _, d = imap.uid("SEARCH", None, "X-GM-RAW", f'"subject:\\"{betreff}\\""')
            uids = d[0].split()
            if not uids:
                return None
            _, f = imap.uid("FETCH", uids[-1], "(X-GM-LABELS)")
            return f[0].decode() if isinstance(f[0], bytes) else str(f[0])
        return self.mit(lesen)

    def in_ordner(self, ordner, betreff):
        def lesen(imap):
            if imap.select(f'"{ordner}"', readonly=True)[0] != "OK":
                return False
            _, d = imap.uid("SEARCH", None, "X-GM-RAW", f'"subject:\\"{betreff}\\""')
            return bool(d[0].split())
        return self.mit(lesen)


def warten(was, bedingung, sekunden):
    ende = time.time() + sekunden
    while time.time() < ende:
        if bedingung():
            print(f"  ✓ {was}")
            return
        time.sleep(10)
    sys.exit(f"Gmail-Probe: {was} — nach {sekunden} s nicht erreicht")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--relay", default="http://127.0.0.1:3801")
    relay = ap.parse_args().relay
    user, passwort = os.environ.get("GMAIL_TEST_USER"), os.environ.get("GMAIL_TEST_PASSWORT")
    if not user or not passwort:
        print("Gmail-Probe übersprungen: GMAIL_TEST_USER / GMAIL_TEST_PASSWORT fehlen.")
        return
    g = Gmail(user, passwort)
    trash = g.besonders("\\Trash")

    if api(relay, "GET", "/accounts"):
        sys.exit("Gmail-Probe: Relay hat schon ein Konto; bitte mit leerem Datenordner starten.")
    api(relay, "POST", "/accounts", {
        "name": "Gmail (Probe)",
        "imap_host": HOST, "imap_port": 993, "imap_ssl": True,
        "smtp_host": "smtp.gmail.com", "smtp_port": 587, "smtp_tls": True,
        "imap_username": user, "imap_password": passwort,
        "smtp_username": user, "smtp_password": passwort,
        "sender_name": "Probe", "sender_email": user,
    })
    konto = api(relay, "GET", "/accounts")[0]["id"]

    stempel = int(time.time())
    betreff = f"Gmailprobe {stempel}"
    m = EmailMessage()
    m["From"] = f"Probe <{user}>"
    m["To"] = user
    m["Subject"] = betreff
    m["Date"] = formatdate(localtime=True)
    m["Message-ID"] = make_msgid(domain="relay.test")
    m.set_content("Diese Mail prüft Relay mit Gmail. Sie wird am Ende endgültig gelöscht.")
    g.mit(lambda imap: imap.append("INBOX", "", imaplib.Time2Internaldate(time.time()), m.as_bytes()))
    print(f"Gmail-Probe mit „{betreff}“")

    def zeile(ordner):
        for r in api(relay, "GET", f"/messages?account_id={konto}&folder={urllib.request.quote(ordner)}&limit=500&list_only=true"):
            if r.get("subject") == betreff:
                return r
        return None

    def verschieben(quelle, ziel):
        r = zeile(quelle)
        if r is None:
            sys.exit(f"Gmail-Probe: die Mail fehlt in Relays {quelle}")
        api(relay, "POST", "/messages/move", {"account_id": konto, "uid": r["uid"], "source_folder": quelle,
                                              "target_folder": ziel, "raw_source_folder": "", "raw_target_folder": ""})

    def loeschen(ordner):
        r = zeile(ordner)
        if r is None:
            sys.exit(f"Gmail-Probe: die Mail fehlt in Relays {ordner}")
        api(relay, "POST", "/messages/delete", {"account_id": konto, "uid": r["uid"], "source_folder": ordner})

    label = f"RelayProbe{stempel}"
    try:
        warten("Relay kennt die Mail", lambda: zeile("INBOX") is not None, 600)

        verschieben("INBOX", "Archive")
        warten("Gmail: archiviert (ohne Posteingang, in Alle Nachrichten)",
               lambda: (l := g.labels(betreff)) is not None and "\\\\Inbox" not in l, 400)
        if g.in_ordner(trash, betreff):
            sys.exit("Gmail-Probe: die archivierte Mail liegt im Papierkorb")
        warten("Relay zeigt sie im Archiv", lambda: zeile("Archive") is not None, 600)

        verschieben("Archive", "INBOX")
        warten("Gmail: zurück im Posteingang", lambda: "\\\\Inbox" in (g.labels(betreff) or ""), 400)
        warten("Relay kennt sie im Posteingang wieder", lambda: zeile("INBOX") is not None, 600)

        api(relay, "POST", "/folders", {"account_id": konto, "name": label})
        warten("Gmail: Label angelegt", lambda: label in g.ordner(), 60)
        verschieben("INBOX", label)
        warten("Gmail: im Label, nicht mehr im Posteingang",
               lambda: label in (l := g.labels(betreff) or "") and "\\\\Inbox" not in l, 400)

        warten("Relay kennt sie im Label", lambda: zeile(label) is not None, 600)
        loeschen(label)
        warten("Gmail: im Papierkorb", lambda: g.in_ordner(trash, betreff), 400)

        warten("Relay kennt sie im Papierkorb", lambda: zeile("Trash") is not None, 120)
        verschieben("Trash", "INBOX")
        warten("Gmail: wiederhergestellt im Posteingang (nicht verloren)",
               lambda: "\\\\Inbox" in (g.labels(betreff) or "") and not g.in_ordner(trash, betreff), 400)

        warten("Relay kennt sie im Posteingang wieder", lambda: zeile("INBOX") is not None, 600)
        loeschen("INBOX")
        warten("Gmail: wieder im Papierkorb", lambda: g.in_ordner(trash, betreff), 400)
        warten("Relay kennt sie im Papierkorb", lambda: zeile("Trash") is not None, 600)
        loeschen("Trash")
        warten("Gmail: endgültig gelöscht",
               lambda: not g.in_ordner(trash, betreff) and g.labels(betreff) is None, 400)

        api(relay, "POST", "/folders/delete", {"account_id": konto, "name": label})
        warten("Gmail: Label gelöscht", lambda: label not in g.ordner(), 60)
    finally:
        # Leave the test account tidy whatever happened.
        def aufraeumen(imap):
            if label in g.ordner():
                imap.delete(f'"{label}"')
        try:
            g.mit(aufraeumen)
        except Exception:
            pass
    print("Gmail-Probe bestanden: Archivieren, Labels und Löschen stimmen bei Gmail.")


if __name__ == "__main__":
    main()
