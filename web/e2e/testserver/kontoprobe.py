#!/usr/bin/env python3
"""Calendar, tasks and contacts of a mail account (Kai, 9.10.2026, 26.10.18).

Against the test servers (starten.sh) and a Relay filled by befuellen.py.
The account's calendar, tasks and contacts are switched on from the DAV
server's root address only — Relay must find Erika's own collections below
the principal (as with iCloud or GMX) and sync them with the account's own
login. Then calendar off: its events leave Relay, the tasks stay; then all
off: the CalDAV account and the address book leave Relay, the server keeps
everything. Standard library only; exits 1 with the step that failed.

    python3 web/e2e/testserver/kontoprobe.py --relay http://127.0.0.1:3800
"""

import argparse
import sys
import urllib.error

from ordnerprobe import KONTO, api, warten

DAV_WURZEL = "http://127.0.0.1:5232/"


def schalten(relay, kalender, aufgaben, kontakte):
    try:
        return api(relay, "POST", f"/accounts/{KONTO}/dav", {
            "kalender": kalender, "aufgaben": aufgaben, "kontakte": kontakte,
            "caldav_url": DAV_WURZEL, "carddav_url": DAV_WURZEL,
        })
    except urllib.error.HTTPError as e:
        sys.exit(f"Kontoprobe: Schalten abgelehnt — {e.read().decode()}")


def mail_konto(relay):
    return next((a for a in api(relay, "GET", "/calendars/caldav-accounts") if a["id"] == f"mail-{KONTO}"), None)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--relay", default="http://127.0.0.1:3800")
    relay = ap.parse_args().relay

    stand = api(relay, "GET", f"/accounts/{KONTO}/dav")
    if stand["anbieter"] is not None:
        sys.exit(f"Kontoprobe: der Testserver ist kein bekannter Anbieter, gemeldet: {stand['anbieter']}")
    print("Kontoprobe: Kalender, Aufgaben und Kontakte von der Wurzel des DAV-Servers")

    r = schalten(relay, True, True, True)
    if r["kalender_gefunden"] < 2 or r["adressbuecher_gefunden"] < 1:
        sys.exit(f"Kontoprobe: unter dem Principal nicht gefunden: {r}")
    print(f"  ✓ {r['kalender_gefunden']} Kalender, {r['adressbuecher_gefunden']} Adressbuch gefunden")
    stand = api(relay, "GET", f"/accounts/{KONTO}/dav")
    if not (stand["kalender"] and stand["aufgaben"] and stand["kontakte"]):
        sys.exit(f"Kontoprobe: Schalter nicht gesetzt: {stand}")
    if stand["caldav_url"].rstrip("/") == DAV_WURZEL.rstrip("/"):
        sys.exit("Kontoprobe: die Wurzel blieb stehen, das eigene Verzeichnis wurde nicht gefunden")
    print(f"  ✓ eigenes Verzeichnis {stand['caldav_url']}")
    warten("Aufgaben abgeglichen", lambda: len(api(relay, "GET", "/todos")) > 0, 120)
    warten("Kontakte im Adressbuch", lambda: api(relay, "GET", "/contacts/zahlen")["adressbuch"] > 0, 120)

    schalten(relay, False, True, True)
    konto = mail_konto(relay)
    if not konto or konto["kalender"] or not konto["aufgaben"]:
        sys.exit(f"Kontoprobe: Kalender aus, Aufgaben an — gespeichert ist {konto}")
    warten("Aufgaben bleiben ohne Kalender", lambda: len(api(relay, "GET", "/todos")) > 0, 60)

    schalten(relay, False, False, False)
    if mail_konto(relay) is not None:
        sys.exit("Kontoprobe: das CalDAV-Konto des Mailkontos blieb nach dem Abschalten")
    stand = api(relay, "GET", f"/accounts/{KONTO}/dav")
    if stand["kalender"] or stand["aufgaben"] or stand["kontakte"]:
        sys.exit(f"Kontoprobe: nach dem Abschalten noch an: {stand}")
    if api(relay, "GET", "/contacts/zahlen")["adressbuch"] != 0:
        sys.exit("Kontoprobe: Kontakte des Adressbuchs blieben nach dem Abschalten")
    print("  ✓ abgeschaltet: Konto, Kalender und Kontakte aus Relay entfernt")
    print("Kontoprobe: alles in Ordnung")
    return 0


if __name__ == "__main__":
    sys.exit(main())
