#!/usr/bin/env python3
"""Deleting in Relay reaches the mail server (Kai, 7.10.2026).

Against the test servers (starten.sh) and a Relay filled by befuellen.py:
a mail of its own goes into the inbox by IMAP, Relay deletes it, and the
mail server is asked until it is gone from the inbox and lies in the trash;
Relay restores it (back in the server's inbox), deletes it again and then
for good from its trash, and the server's trash must lose it too. Standard library only; exits 1 with the step that failed.

    python3 web/e2e/testserver/loeschprobe.py --relay http://127.0.0.1:3800
"""

import argparse
import imaplib
import json
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


def betreffe(ordner):
    """Subjects in a folder of the mail server (none if it does not exist)."""
    imap = imaplib.IMAP4(*IMAP)
    try:
        imap.login(USER, PASSWORT)
        if imap.select(f'"{ordner}"')[0] != "OK":
            return []
        # ALL, not NOT DELETED: gone means expunged, not only flagged.
        _, daten = imap.search(None, "ALL")
        nummern = daten[0].split()
        if not nummern:
            return []
        # One FETCH for all; a mail expunged meanwhile simply has no entry.
        _, teile = imap.fetch(b",".join(nummern), "(BODY.PEEK[HEADER.FIELDS (SUBJECT)])")
        return [t[1].decode(errors="replace").strip().removeprefix("Subject: ")
                for t in teile if isinstance(t, tuple)]
    finally:
        imap.logout()


def warten(was, bedingung, sekunden):
    ende = time.time() + sekunden
    while time.time() < ende:
        if bedingung():
            print(f"  ✓ {was}")
            return
        time.sleep(5)
    sys.exit(f"Löschprobe: {was} — nach {sekunden} s nicht erreicht")


def uid_in(relay, ordner, betreff):
    for m in api(relay, "GET", f"/messages?account_id={KONTO}&folder={ordner}&limit=500&list_only=true"):
        if m.get("subject") == betreff:
            return m["uid"]
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--relay", default="http://127.0.0.1:3800")
    relay = ap.parse_args().relay

    betreff = f"Loeschprobe {int(time.time())}"
    m = EmailMessage()
    m["From"] = "Probe <probe@relay.test>"
    m["To"] = "erika@relay.test"
    m["Subject"] = betreff
    m["Date"] = formatdate(localtime=True)
    m["Message-ID"] = make_msgid(domain="relay.test")
    m.set_content("Diese Mail prüft, dass Löschen den Mailserver erreicht.")
    imap = imaplib.IMAP4(*IMAP)
    imap.login(USER, PASSWORT)
    imap.append("INBOX", "", imaplib.Time2Internaldate(time.time()), m.as_bytes())
    imap.logout()
    print(f"Löschprobe mit „{betreff}“")

    warten("Relay kennt die Mail", lambda: uid_in(relay, "INBOX", betreff) is not None, 240)
    uid = uid_in(relay, "INBOX", betreff)
    api(relay, "POST", "/messages/delete", {"account_id": KONTO, "uid": uid, "source_folder": "INBOX"})
    warten("Server: nicht mehr im Posteingang", lambda: betreff not in betreffe("INBOX"), 300)
    warten("Server: im Papierkorb", lambda: betreff in betreffe("Trash"), 60)

    # Restored from Relay's trash: back in the server's inbox (found in the
    # server's trash by its Message-ID, its number there is a new one).
    uid = uid_in(relay, "Trash", betreff)
    if uid is None:
        sys.exit("Löschprobe: die Mail fehlt in Relays Papierkorb")
    api(relay, "POST", "/messages/move", {"account_id": KONTO, "uid": uid, "source_folder": "Trash",
                                          "target_folder": "INBOX", "raw_source_folder": "", "raw_target_folder": ""})
    warten("Server: wiederhergestellt im Posteingang", lambda: betreff in betreffe("INBOX"), 300)
    warten("Server: nicht mehr im Papierkorb", lambda: betreff not in betreffe("Trash"), 120)

    # Deleted again, then for good.
    warten("Relay kennt die Mail im Posteingang wieder", lambda: uid_in(relay, "INBOX", betreff) is not None, 240)
    uid = uid_in(relay, "INBOX", betreff)
    api(relay, "POST", "/messages/delete", {"account_id": KONTO, "uid": uid, "source_folder": "INBOX"})
    warten("Server: wieder im Papierkorb", lambda: betreff in betreffe("Trash") and betreff not in betreffe("INBOX"), 300)
    uid = uid_in(relay, "Trash", betreff)
    if uid is None:
        sys.exit("Löschprobe: die Mail fehlt in Relays Papierkorb")
    api(relay, "POST", "/messages/delete", {"account_id": KONTO, "uid": uid, "source_folder": "Trash"})
    warten("Server: auch aus dem Papierkorb entfernt", lambda: betreff not in betreffe("Trash"), 300)
    print("Löschprobe bestanden: Löschen erreicht den Mailserver.")


if __name__ == "__main__":
    main()
