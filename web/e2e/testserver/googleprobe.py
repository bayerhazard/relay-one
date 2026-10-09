#!/usr/bin/env python3
"""Calendar, contacts and tasks of a Gmail account through Google's sign-in
(Kai, 9.10.2026, Schritt 2).

Against the test servers (starten.sh, with the stand-in for Google on
127.0.0.1:5299) and a Relay filled by befuellen.py and started with
RELAY_GOOGLE_TEST_BASIS=http://127.0.0.1:5299. A second mail account with a
gmail.com address is added. Before the box has a Google project, and before
the account signed in, switching on is refused. Then the project is entered,
the sign-in runs as the browser would (Google's page, back to Relay), and
calendar, contacts and tasks are switched on: Relay finds the calendars and
the address book below Google's addresses with the token, and the two task
lists. A task made in Relay lands in Google's first list, done and deleted
there too; a Google task keeps Relay's priority. Calendar off: Google's
calendars leave, the tasks stay. Signing out revokes the token and takes
everything along. Standard library only; exits 1 with the step that failed.

    python3 web/e2e/testserver/googleprobe.py --relay http://127.0.0.1:3800
"""

import argparse
import json
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

GOOGLE = "http://127.0.0.1:5299"
EMAIL = "erika.relay@gmail.com"


def api(relay, methode, pfad, daten=None):
    body = json.dumps(daten).encode() if daten is not None else None
    req = urllib.request.Request(relay + "/api/v1" + pfad, data=body, method=methode)
    if body is not None:
        req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req, timeout=60) as antwort:
        text = antwort.read().decode() or "null"
        return json.loads(text)


def abgelehnt(relay, methode, pfad, daten):
    """The error text of a call that must fail."""
    try:
        api(relay, methode, pfad, daten)
    except urllib.error.HTTPError as e:
        return e.read().decode()
    sys.exit(f"Googleprobe: {methode} {pfad} hätte abgelehnt werden müssen")


def google_stand():
    with urllib.request.urlopen(GOOGLE + "/stand", timeout=10) as r:
        return json.loads(r.read())


class OhneWeiterleitung(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *a, **k):
        return None


def warten(was, bedingung, sekunden):
    ende = time.time() + sekunden
    while time.time() < ende:
        if bedingung():
            print(f"  ✓ {was}")
            return
        time.sleep(2)
    sys.exit(f"Googleprobe: {was} — nach {sekunden} s nicht erreicht")


def pruefen(bedingung, was):
    if not bedingung:
        sys.exit(f"Googleprobe: {was}")
    print(f"  ✓ {was}")


def schalten(relay, konto, kalender, aufgaben, kontakte):
    try:
        return api(relay, "POST", f"/accounts/{konto}/dav",
                   {"kalender": kalender, "aufgaben": aufgaben, "kontakte": kontakte})
    except urllib.error.HTTPError as e:
        sys.exit(f"Googleprobe: Schalten abgelehnt — {e.read().decode()}")


def todos(relay):
    return api(relay, "GET", "/todos")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--relay", default="http://127.0.0.1:3800")
    relay = ap.parse_args().relay
    print("Googleprobe: Gmail mit Google-Anmeldung")

    # A Gmail account: the mail comes from GreenMail, the address makes it Gmail.
    vorher = {a["id"] for a in api(relay, "GET", "/accounts")}
    api(relay, "POST", "/accounts", {
        "name": "Gmail (Test)",
        "imap_host": "127.0.0.1", "imap_port": 3143, "imap_ssl": False,
        "smtp_host": "127.0.0.1", "smtp_port": 3025, "smtp_tls": False,
        "imap_username": "erika", "imap_password": "geheim",
        "smtp_username": "erika", "smtp_password": "geheim",
        "sender_name": "Erika Relay", "sender_email": EMAIL,
    })
    konto = next(a["id"] for a in api(relay, "GET", "/accounts") if a["id"] not in vorher)

    stand = api(relay, "GET", f"/accounts/{konto}/dav")
    pruefen(stand["anbieter"] and stand["anbieter"]["nur_google_anmeldung"], "als Gmail erkannt")
    pruefen(stand["google"] == {"eingerichtet": False, "angemeldet": None}, "ohne Google-Projekt nicht eingerichtet")
    fehler = abgelehnt(relay, "POST", f"/accounts/{konto}/dav", {"kalender": True, "aufgaben": True, "kontakte": True})
    pruefen("zuerst mit Google an" in fehler, "Einschalten ohne Anmeldung abgelehnt")
    fehler = abgelehnt(relay, "POST", f"/accounts/{konto}/google/start", {"origin": relay})
    pruefen("noch nicht eingerichtet" in fehler, "Anmeldung ohne Google-Projekt abgelehnt")

    # The box owner enters the project; the secret never comes back.
    app = api(relay, "POST", "/google/app",
              {"client_id": "relay-test.apps.googleusercontent.com", "client_secret": "geheim-test"})
    pruefen(app["eingerichtet"] and "geheim" not in json.dumps(app), "Google-Projekt eingetragen, Schlüssel bleibt auf der Box")
    pruefen(app["rueckweg"] == "/api/v1/google/rueckweg", "Rückweg genannt")

    # The sign-in as the browser runs it.
    url = api(relay, "POST", f"/accounts/{konto}/google/start", {"origin": relay})["url"]
    q = dict(urllib.parse.parse_qsl(urllib.parse.urlsplit(url).query))
    pruefen(url.startswith(GOOGLE + "/o/oauth2/v2/auth") and q["access_type"] == "offline"
            and q["login_hint"] == EMAIL and q["redirect_uri"] == relay + "/api/v1/google/rueckweg",
            "Anmeldeseite mit Rückweg und Konto")
    oeffner = urllib.request.build_opener(OhneWeiterleitung)
    try:
        oeffner.open(url, timeout=10)
        sys.exit("Googleprobe: Google hat nicht zurückgeleitet")
    except urllib.error.HTTPError as e:
        zurueck = e.headers["Location"]
    with urllib.request.urlopen(zurueck, timeout=30) as r:
        seite = r.read().decode()
    pruefen("Mit Google angemeldet" in seite and "postMessage" in seite, "zurück in Relay angemeldet")
    stand = api(relay, "GET", f"/accounts/{konto}/dav")
    pruefen(stand["google"]["angemeldet"] == EMAIL, f"angemeldet als {EMAIL}")
    with urllib.request.urlopen(zurueck, timeout=30) as r:
        pruefen("abgelaufen" in r.read().decode(), "derselbe Rückweg ein zweites Mal abgelehnt")

    # Everything on: found below Google's addresses with the token.
    r = schalten(relay, konto, True, True, True)
    pruefen(r["kalender_gefunden"] >= 2 and r["adressbuecher_gefunden"] >= 1 and r["aufgabenlisten_gefunden"] == 2,
            f"{r['kalender_gefunden']} Kalender, {r['adressbuecher_gefunden']} Adressbuch, 2 Aufgabenlisten")
    caldav = next(a for a in api(relay, "GET", "/calendars/caldav-accounts") if a["id"] == f"mail-{konto}")
    pruefen(caldav["url"].startswith(GOOGLE) and caldav["kalender"] and caldav["aufgaben"],
            f"Kalender über Google: {caldav['url']}")
    warten("Google-Aufgaben in Relay",
           lambda: {"Steuer vorbereiten", "Milch"} <= {t.get("summary") for t in todos(relay)}, 90)
    steuer = next(t for t in todos(relay) if t.get("summary") == "Steuer vorbereiten")
    pruefen(steuer["uid"] == "google-task-T1" and steuer["description"] == "Belege sammeln"
            and steuer["due_at"].startswith("2026-10-19T22:00"), "Notiz und Fälligkeit übernommen")
    milch = next(t for t in todos(relay) if t.get("summary") == "Milch")
    pruefen(milch["project_id"] is not None and steuer["project_id"] is None,
            "erste Liste ist der Eingang, „Einkauf“ ein Projekt")
    warten("Google-Kalender in Relay",
           lambda: any(c["url"].startswith(GOOGLE) for c in api(relay, "GET", "/calendars")), 60)
    warten("Kontakte über Google", lambda: api(relay, "GET", "/contacts/zahlen")["adressbuch"] > 0, 90)

    # Relay's own fields survive a sync; done, new and deleted reach Google.
    api(relay, "PATCH", "/todos/google-task-T1", {"priority": 1})
    steuer = next(t for t in todos(relay) if t.get("summary") == "Steuer vorbereiten")
    # Into "Meine Aufgaben" (the inbox is the first account's with tasks,
    # here the test server's).
    neu = api(relay, "POST", "/todos",
              {"summary": "Rückruf Weber", "due": "2026-10-22", "project_id": steuer["calendar_id"]})
    pruefen(neu["uid"].startswith("google-task-"), "neue Aufgabe als Google-Aufgabe angelegt")
    eingang = google_stand()["aufgaben"]["L1"]
    pruefen(any(a["title"] == "Rückruf Weber" and a["due"] == "2026-10-22T00:00:00.000Z" for a in eingang),
            "bei Google in „Meine Aufgaben“ mit Datum")
    api(relay, "PATCH", f"/todos/{neu['uid']}", {"completed": True})
    pruefen(any(a["title"] == "Rückruf Weber" and a["status"] == "completed" for a in google_stand()["aufgaben"]["L1"]),
            "erledigt bei Google")
    api(relay, "POST", "/calendars/sync")
    steuer = next(t for t in todos(relay) if t.get("summary") == "Steuer vorbereiten")
    pruefen(steuer["priority"] == 1, "Priorität bleibt nach dem Abgleich")
    api(relay, "DELETE", f"/todos/{neu['uid']}")
    pruefen(not any(a["title"] == "Rückruf Weber" for a in google_stand()["aufgaben"]["L1"]), "gelöscht bei Google")
    pruefen(google_stand()["zaehler"]["erneuert"] > 0, "Zugang erneuert, als er ablief")

    # Calendar off: Google's calendars leave Relay, the tasks stay.
    schalten(relay, konto, False, True, True)
    pruefen(not any(c["url"].startswith(GOOGLE + "/erika") for c in api(relay, "GET", "/calendars")),
            "Google-Kalender entfernt")
    pruefen(any(t.get("summary") == "Milch" for t in todos(relay)), "Aufgaben bleiben")

    # Signing out: token revoked, everything gone from Relay, kept at Google.
    api(relay, "POST", f"/accounts/{konto}/google/trennen")
    stand = api(relay, "GET", f"/accounts/{konto}/dav")
    pruefen(stand["google"]["angemeldet"] is None and not (stand["kalender"] or stand["aufgaben"] or stand["kontakte"]),
            "abgemeldet, alles aus")
    pruefen(google_stand()["zaehler"]["widerrufen"] == 1, "Zugang bei Google widerrufen")
    pruefen(not any(t["uid"].startswith("google-task-") for t in todos(relay)), "Google-Aufgaben aus Relay entfernt")
    pruefen(any(a["title"] == "Milch" for a in google_stand()["aufgaben"]["L2"]), "bei Google bleibt alles")

    # Signed in again, then the mail account deleted: the sign-in goes with it.
    url = api(relay, "POST", f"/accounts/{konto}/google/start", {"origin": relay})["url"]
    try:
        oeffner.open(url, timeout=10)
    except urllib.error.HTTPError as e:
        urllib.request.urlopen(e.headers["Location"], timeout=30).read()
    schalten(relay, konto, True, True, True)
    api(relay, "POST", "/accounts/delete", {"account_id": konto})
    pruefen(google_stand()["zaehler"]["widerrufen"] == 2, "Konto gelöscht: Google-Zugang widerrufen")
    pruefen(not any(a["id"] == f"mail-{konto}" for a in api(relay, "GET", "/calendars/caldav-accounts")),
            "Konto gelöscht: Kalender-Zugang entfernt")
    pruefen(not any(t["uid"].startswith("google-task-") for t in todos(relay)), "Konto gelöscht: Google-Aufgaben entfernt")
    print("Googleprobe: alles in Ordnung")
    return 0


if __name__ == "__main__":
    sys.exit(main())
