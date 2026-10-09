#!/usr/bin/env python3
"""Moving in Relay keeps the mail on the mail server (Kai, 9.10.2026).

Against the test servers (starten.sh) and a Relay filled by befuellen.py.
Before 26.10.11 a move into a folder Relay knew no mail in (an "Archive"
the server does not have, an empty folder) made that folder "only in
Relay", and from the second move on the mail went off the server: on
Gmail into its trash. This probe archives two mails, moves two into an
empty folder of the server, stars a mail right after its move and drags
one into the trash, and asks the mail server after each step.
Standard library only; exits 1 with the step that failed.

    python3 web/e2e/testserver/ordnerprobe.py --relay http://127.0.0.1:3800
"""

import argparse
import imaplib
import json
import re
import sys
import time
import urllib.request
from email.message import EmailMessage
from email.utils import formatdate, make_msgid

IMAP = ("127.0.0.1", 3143)
USER, PASSWORT = "erika", "geheim"
KONTO = 1


def api(relay, methode, pfad, daten=None):
    body = json.dumps(daten).encode() if daten is not None else None
    req = urllib.request.Request(relay + "/api/v1" + pfad, data=body, method=methode)
    if body is not None:
        req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req, timeout=30) as antwort:
        text = antwort.read().decode() or "null"
        return json.loads(text)


def mit_imap(arbeit):
    imap = imaplib.IMAP4(*IMAP)
    try:
        imap.login(USER, PASSWORT)
        return arbeit(imap)
    finally:
        imap.logout()


def betreffe(ordner, nur_markiert=False):
    """Subjects in a folder of the mail server (none if it does not exist)."""
    def lesen(imap):
        if imap.select(f'"{ordner}"')[0] != "OK":
            return []
        # ALL, not NOT DELETED: gone means expunged, not only flagged.
        _, daten = imap.search(None, "FLAGGED" if nur_markiert else "ALL")
        nummern = daten[0].split()
        if not nummern:
            return []
        _, teile = imap.fetch(b",".join(nummern), "(BODY.PEEK[HEADER.FIELDS (SUBJECT)])")
        return [t[1].decode(errors="replace").strip().removeprefix("Subject: ")
                for t in teile if isinstance(t, tuple)]
    return mit_imap(lesen)


def warten(was, bedingung, sekunden):
    ende = time.time() + sekunden
    while time.time() < ende:
        if bedingung():
            print(f"  ✓ {was}")
            return
        time.sleep(5)
    sys.exit(f"Ordnerprobe: {was} — nach {sekunden} s nicht erreicht")


def zeile(relay, ordner, betreff):
    for m in api(relay, "GET", f"/messages?account_id={KONTO}&folder={urllib.request.quote(ordner)}&limit=500&list_only=true"):
        if m.get("subject") == betreff:
            return m
    return None


def verschieben(relay, ordner, betreff, ziel):
    m = zeile(relay, ordner, betreff)
    if m is None:
        sys.exit(f"Ordnerprobe: „{betreff}“ fehlt in Relays {ordner}")
    api(relay, "POST", "/messages/move", {"account_id": KONTO, "uid": m["uid"], "source_folder": ordner,
                                          "target_folder": ziel, "raw_source_folder": "", "raw_target_folder": ""})


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--relay", default="http://127.0.0.1:3800")
    relay = ap.parse_args().relay

    stempel = int(time.time())
    leer = f"Leer{stempel}"
    namen = {k: f"Ordnerprobe {k} {stempel}" for k in "ABCDE"}

    def anlegen(imap):
        # An empty folder of the server: Relay knows no mail in it.
        imap.create(f'"{leer}"')
        for betreff in namen.values():
            m = EmailMessage()
            m["From"] = "Probe <probe@relay.test>"
            m["To"] = "erika@relay.test"
            m["Subject"] = betreff
            m["Date"] = formatdate(localtime=True)
            m["Message-ID"] = make_msgid(domain="relay.test")
            m.set_content("Diese Mail prüft, dass Verschieben den Mailserver erreicht.")
            imap.append("INBOX", "", imaplib.Time2Internaldate(time.time()), m.as_bytes())
    mit_imap(anlegen)
    print(f"Ordnerprobe {stempel}")
    warten("Relay kennt die Mails", lambda: all(zeile(relay, "INBOX", b) for b in namen.values()), 300)

    # Archive twice: the server has no "Archive", Relay makes it there; the
    # second mail must land there too, not in the trash.
    verschieben(relay, "INBOX", namen["A"], "Archive")
    verschieben(relay, "INBOX", namen["B"], "Archive")
    warten("Server: beide archiviert",
           lambda: {namen["A"], namen["B"]} <= set(betreffe("Archive")), 300)
    warten("Server: nicht mehr im Posteingang, nicht im Papierkorb",
           lambda: not ({namen["A"], namen["B"]} & (set(betreffe("INBOX")) | set(betreffe("Trash")))), 120)

    # Twice into a folder of the server Relay knew empty.
    markiert_vorher = sorted(betreffe("INBOX", nur_markiert=True))
    verschieben(relay, "INBOX", namen["C"], leer)
    verschieben(relay, "INBOX", namen["D"], leer)
    # A star right after the move: the row still has the inbox's number.
    c = zeile(relay, leer, namen["C"])
    if c is None:
        sys.exit("Ordnerprobe: C fehlt in Relays leerem Ordner")
    api(relay, "POST", "/messages/flag", {"account_id": KONTO, "uid": c["uid"], "folder_name": leer, "flagged": True})
    warten(f"Server: beide in „{leer}“", lambda: {namen["C"], namen["D"]} <= set(betreffe(leer)), 300)
    warten("Server: nicht im Papierkorb", lambda: not ({namen["C"], namen["D"]} & set(betreffe("Trash"))), 60)
    warten("Server: der Stern sitzt an C, an keiner anderen", lambda: betreffe(leer, nur_markiert=True) == [namen["C"]], 300)
    if sorted(betreffe("INBOX", nur_markiert=True)) != markiert_vorher:
        sys.exit("Ordnerprobe: im Posteingang trägt eine andere Mail den Stern")

    # Dragged into Relay's trash: into the server's trash, not deleted.
    verschieben(relay, "INBOX", namen["E"], "Trash")
    warten("Server: E im Papierkorb", lambda: namen["E"] in betreffe("Trash"), 300)

    # Out of the archive back into the inbox.
    warten("Relay kennt A im Archiv", lambda: zeile(relay, "Archive", namen["A"]) is not None, 60)
    verschieben(relay, "Archive", namen["A"], "INBOX")
    warten("Server: A wieder im Posteingang", lambda: namen["A"] in betreffe("INBOX"), 300)
    print("Ordnerprobe bestanden: Verschieben behält die Mails auf dem Mailserver.")


if __name__ == "__main__":
    main()
