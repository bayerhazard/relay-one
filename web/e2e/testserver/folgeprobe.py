#!/usr/bin/env python3
"""Relay follows the mail server (Kai, 9.10.2026).

Against the test servers (starten.sh) and a Relay filled by befuellen.py.
What happens elsewhere (Gmail's web view, the phone) is done here by IMAP
on the server: a mail Relay deleted is restored from the server's trash —
Relay must show it in the inbox again and not in its trash; a folder is
renamed on the server — Relay must drop the old name and show the mail
under the new one. Standard library only; exits 1 with the step that failed.

    python3 web/e2e/testserver/folgeprobe.py --relay http://127.0.0.1:3800
"""

import argparse
import sys
import time

from ordnerprobe import KONTO, api, betreffe, mit_imap, relay_ordner, warten, zeile
from email.message import EmailMessage
from email.utils import formatdate, make_msgid
import imaplib


def mail(betreff, text):
    m = EmailMessage()
    m["From"] = "Probe <probe@relay.test>"
    m["To"] = "erika@relay.test"
    m["Subject"] = betreff
    m["Date"] = formatdate(localtime=True)
    m["Message-ID"] = make_msgid(domain="relay.test")
    m.set_content(text)
    return m.as_bytes()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--relay", default="http://127.0.0.1:3800")
    relay = ap.parse_args().relay
    stempel = int(time.time())
    # ASCII: the server hands an umlaut back RFC 2047-encoded.
    zurueck = f"Folgeprobe zurueck {stempel}"
    alt, neu = f"Weg{stempel}", f"Neu{stempel}"
    im_ordner = f"Folgeprobe Ordner {stempel}"

    def anlegen(imap):
        imap.append("INBOX", "", imaplib.Time2Internaldate(time.time()), mail(zurueck, "Wird anderswo zurückgeholt."))
        imap.create(f'"{alt}"')
        imap.append(f'"{alt}"', "", imaplib.Time2Internaldate(time.time()), mail(im_ordner, "Ihr Ordner wird anderswo umbenannt."))
    mit_imap(anlegen)
    print(f"Folgeprobe {stempel}")
    warten("Relay kennt die Mail", lambda: zeile(relay, "INBOX", zurueck) is not None, 360)
    warten("Relay kennt den Ordner und seine Mail", lambda: zeile(relay, alt, im_ordner) is not None, 360)

    m = zeile(relay, "INBOX", zurueck)
    api(relay, "POST", "/messages/delete", {"account_id": KONTO, "uid": m["uid"], "source_folder": "INBOX"})
    warten("Server: im Papierkorb", lambda: zurueck in betreffe("Trash"), 300)
    warten("Relay: im Papierkorb", lambda: zeile(relay, "Trash", zurueck) is not None, 360)

    # Elsewhere: restored from the server's trash, the folder renamed.
    def anderswo(imap):
        imap.select("Trash")
        _, d = imap.uid("SEARCH", None, "SUBJECT", f'"{zurueck}"')
        for u in d[0].split():
            imap.uid("MOVE", u, "INBOX")
        imap.rename(f'"{alt}"', f'"{neu}"')
    mit_imap(anderswo)

    warten("Relay: zurück im Posteingang", lambda: zeile(relay, "INBOX", zurueck) is not None, 600)
    warten("Relay: nicht mehr im Papierkorb", lambda: zeile(relay, "Trash", zurueck) is None, 600)
    warten("Relay: der alte Ordnername ist weg", lambda: alt not in relay_ordner(relay), 600)
    warten("Relay: die Mail im umbenannten Ordner", lambda: zeile(relay, neu, im_ordner) is not None, 600)
    print("Folgeprobe bestanden: Relay folgt dem Mailserver.")


if __name__ == "__main__":
    sys.path.insert(0, __file__.rsplit("/", 1)[0])
    main()
