#!/usr/bin/env python3
"""Fill the test servers with dummy data and connect Relay to them.

Run after starten.sh, against a Relay server with an empty data dir:

    python3 web/e2e/testserver/befuellen.py [--relay http://127.0.0.1:3799]

Standard library only. Mail goes in by IMAP APPEND (folders, flags and
dates under control), contacts, events and to-dos by DAV PUT into
Radicale. Dates are relative to today, so the calendar and the task views
always have something this week. Then Relay gets the mail account, the
CardDAV address and the CalDAV account, and syncs once.

All names, addresses and texts are made up (example domains only).
"""

import argparse
import base64
import imaplib
import json
import sys
import time
import urllib.error
import urllib.request
import uuid
from datetime import date, datetime, timedelta, timezone
from email.message import EmailMessage
from email.utils import format_datetime, make_msgid

IMAP = ("127.0.0.1", 3143)
SMTP = ("127.0.0.1", 3025)
DAV = "http://127.0.0.1:5232"
USER, PASSWORT = "erika", "geheim"
ICH = ("Erika Muster", "erika@relay.test")

HEUTE = date.today()
JETZT = datetime.now(timezone.utc).astimezone()


# ── Mail ──────────────────────────────────────────────────────────────────

# (folder, from, subject, body, hours ago, flags, attachment)
MAILS = [
    ("INBOX", ("Jonas Weber", "jonas.weber@beispiel.de"), "Angebot Messestand Frühjahr",
     "Guten Tag Frau Muster,\n\nanbei unser Angebot für den Messestand im März. "
     "Die Preise gelten bis Ende des Monats.\n\nMit freundlichen Grüßen\nJonas Weber",
     1, [], ("angebot-messestand.pdf", "application/pdf")),
    ("INBOX", ("Lena Hoffmann", "lena@hoffmann-design.example"), "Entwürfe für die neue Broschüre",
     "Hallo Erika,\n\ndie drei Entwürfe sind fertig. Welche Variante gefällt Ihnen am besten? "
     "Ich bräuchte bis Donnerstag eine Rückmeldung.\n\nViele Grüße\nLena",
     3, ["\\Flagged"], None),
    ("INBOX", ("Buchhaltung", "buchhaltung@beispiel-gmbh.de"), "Rechnung 2026-0412",
     "Sehr geehrte Damen und Herren,\n\nim Anhang finden Sie die Rechnung 2026-0412. "
     "Zahlbar innerhalb von 14 Tagen ohne Abzug.",
     6, [], ("rechnung-2026-0412.pdf", "application/pdf")),
    ("INBOX", ("Tobias Krüger", "t.krueger@beispiel.org"), "Re: Termin nächste Woche",
     "Dienstag 10 Uhr passt mir gut. Ich bringe die Unterlagen mit.\n\nTobias",
     20, ["\\Seen"], None),
    ("INBOX", ("Newsletter Stadtwerke", "info@stadtwerke.example"), "Ihre Verbrauchsübersicht Oktober",
     "Ihre Verbrauchsübersicht steht im Kundenportal bereit.",
     26, ["\\Seen"], None),
    ("INBOX", ("Miriam Schulz", "miriam.schulz@beispiel.de"), "Protokoll Teamrunde",
     "Hallo zusammen,\n\nhier das Protokoll der Teamrunde:\n\n1. Messeplanung liegt im Zeitplan\n"
     "2. Broschüre: Entscheidung bis Donnerstag\n3. Nächste Runde in zwei Wochen\n\nMiriam",
     30, ["\\Seen"], None),
    ("INBOX", ("Paketdienst", "versand@paket.example"), "Ihre Sendung kommt heute",
     "Ihre Sendung 0034 1234 5678 wird heute zwischen 12 und 16 Uhr zugestellt.",
     50, ["\\Seen"], None),
    ("INBOX", ("Jonas Weber", "jonas.weber@beispiel.de"), "Kurze Frage zur Standgröße",
     "Reichen Ihnen 24 Quadratmeter oder planen Sie größer?",
     75, ["\\Seen", "\\Answered"], None),
    ("INBOX", ("Sarah Becker", "sarah.becker@beispiel.org"), "Einladung: Sommerfest",
     "Liebe Erika,\n\nwir feiern am Freitag in drei Wochen unser Sommerfest. "
     "Ich würde mich freuen, wenn Sie kommen!\n\nSarah",
     120, ["\\Seen"], None),
    ("INBOX", ("IT-Service", "it@beispiel-gmbh.de"), "Wartung am Wochenende",
     "Am Samstag zwischen 8 und 12 Uhr sind die Server kurz nicht erreichbar.",
     170, ["\\Seen"], None),
    ("Sent", ICH, "Re: Angebot Messestand Frühjahr",
     "Hallo Herr Weber,\n\nvielen Dank, ich melde mich bis Freitag.\n\nErika Muster",
     2, ["\\Seen"], None),
    ("Sent", ICH, "Unterlagen für Dienstag",
     "Hallo Tobias,\n\nanbei die Unterlagen für Dienstag.\n\nErika",
     22, ["\\Seen"], ("unterlagen.txt", "text/plain")),
    ("Drafts", ICH, "Rückmeldung Broschüre",
     "Hallo Lena,\n\nmir gefällt Variante",
     4, ["\\Seen", "\\Draft"], None),
    ("Archiv", ("Jonas Weber", "jonas.weber@beispiel.de"), "Vertrag Messe 2025",
     "Anbei der unterschriebene Vertrag für 2025.",
     24 * 200, ["\\Seen"], None),
    ("Projekte", ("Lena Hoffmann", "lena@hoffmann-design.example"), "Moodboard Broschüre",
     "Hier das Moodboard, wie besprochen.",
     24 * 9, ["\\Seen"], None),
]

# Newsletters offer "Abo beenden" (RFC 2369/8058): one click to an address
# that does not resolve here, so Relay falls back to the unsubscribe mail —
# which GreenMail accepts, so the whole way can be tried locally.
ABMELDEN = {
    "Ihre Verbrauchsübersicht Oktober": {
        "List-Unsubscribe": "<https://stadtwerke.example/abmelden?id=4711>, <mailto:abmelden@stadtwerke.example?subject=Abmelden>",
        "List-Unsubscribe-Post": "List-Unsubscribe=One-Click",
    },
}

# Who the mails in Sent and Drafts went to.
EMPFAENGER = {
    "Re: Angebot Messestand Frühjahr": ("Jonas Weber", "jonas.weber@beispiel.de"),
    "Unterlagen für Dienstag": ("Tobias Krüger", "t.krueger@beispiel.org"),
    "Rückmeldung Broschüre": ("Lena Hoffmann", "lena@hoffmann-design.example"),
}


def mail(absender, betreff, text, stunden, anhang):
    m = EmailMessage()
    m["From"] = f"{absender[0]} <{absender[1]}>"
    an = EMPFAENGER.get(betreff, ICH)
    m["To"] = f"{an[0]} <{an[1]}>"
    m["Subject"] = betreff
    m["Date"] = format_datetime(JETZT - timedelta(hours=stunden))
    m["Message-ID"] = make_msgid(domain="relay.test")
    for k, v in ABMELDEN.get(betreff, {}).items():
        m[k] = v
    m.set_content(text)
    if anhang:
        name, art = anhang
        haupt, neben = art.split("/")
        inhalt = b"%PDF-1.4\n% Testdatei\n" if neben == "pdf" else "Testanhang\n".encode()
        m.add_attachment(inhalt, maintype=haupt, subtype=neben, filename=name)
    return m


def mails_einlegen():
    imap = imaplib.IMAP4(*IMAP)
    imap.login(USER, PASSWORT)
    for ordner in sorted({m[0] for m in MAILS} - {"INBOX"}):
        imap.create(ordner)
    for ordner, absender, betreff, text, stunden, flags, anhang in MAILS:
        m = mail(absender, betreff, text, stunden, anhang)
        zeit = imaplib.Time2Internaldate((JETZT - timedelta(hours=stunden)).timestamp())
        typ, _ = imap.append(ordner, "(" + " ".join(flags) + ")", zeit, m.as_bytes())
        if typ != "OK":
            raise SystemExit(f"IMAP APPEND in {ordner} fehlgeschlagen")
    imap.logout()
    print(f"Mail: {len(MAILS)} Nachrichten in {len({m[0] for m in MAILS})} Ordnern")


# ── DAV ───────────────────────────────────────────────────────────────────

def dav(methode, pfad, inhalt=None, art="application/xml; charset=utf-8", erlaubt=(200, 201, 204, 207)):
    req = urllib.request.Request(DAV + pfad, data=inhalt.encode() if inhalt else None, method=methode)
    req.add_header("Authorization", "Basic " + base64.b64encode(f"{USER}:{PASSWORT}".encode()).decode())
    if inhalt:
        req.add_header("Content-Type", art)
    try:
        with urllib.request.urlopen(req) as r:
            return r.status
    except urllib.error.HTTPError as e:
        if e.code in erlaubt:
            return e.code
        raise SystemExit(f"DAV {methode} {pfad}: {e.code} {e.read()[:300]!r}")


def sammlung(pfad, name, art):
    """MKCOL an address book or a calendar for VEVENT or VTODO."""
    if art == "kontakte":
        typ = "<D:collection/><C:addressbook/>"
        ns = 'xmlns:C="urn:ietf:params:xml:ns:carddav"'
        extra = ""
    else:
        typ = "<D:collection/><C:calendar/>"
        ns = 'xmlns:C="urn:ietf:params:xml:ns:caldav"'
        extra = f'<C:supported-calendar-component-set><C:comp name="{art}"/></C:supported-calendar-component-set>'
    body = (f'<?xml version="1.0" encoding="utf-8"?><D:mkcol xmlns:D="DAV:" {ns}><D:set><D:prop>'
            f"<D:resourcetype>{typ}</D:resourcetype><D:displayname>{name}</D:displayname>{extra}"
            "</D:prop></D:set></D:mkcol>")
    dav("MKCOL", pfad, body)


KONTAKTE = [
    ("Jonas", "Weber", "jonas.weber@beispiel.de", "+49 30 1234567", "Messebau Weber GmbH"),
    ("Lena", "Hoffmann", "lena@hoffmann-design.example", "+49 40 7654321", "Hoffmann Design"),
    ("Tobias", "Krüger", "t.krueger@beispiel.org", "+49 89 5550100", ""),
    ("Miriam", "Schulz", "miriam.schulz@beispiel.de", "+49 221 998877", "Beispiel GmbH"),
    ("Sarah", "Becker", "sarah.becker@beispiel.org", "", ""),
    ("Ahmet", "Yılmaz", "ahmet.yilmaz@beispiel.de", "+49 711 334455", "Beispiel GmbH"),
    ("Clara", "Neumann", "clara.neumann@beispiel.org", "+49 351 112233", "Stadtbibliothek"),
    ("David", "Fischer", "d.fischer@beispiel.de", "", "Fischer & Partner"),
]


def kontakte_einlegen():
    sammlung(f"/{USER}/kontakte/", "Kontakte", "kontakte")
    for vor, nach, mail_, tel, firma in KONTAKTE:
        uid = str(uuid.uuid5(uuid.NAMESPACE_URL, mail_))
        zeilen = ["BEGIN:VCARD", "VERSION:3.0", f"UID:{uid}", f"N:{nach};{vor};;;", f"FN:{vor} {nach}",
                  f"EMAIL;TYPE=WORK:{mail_}"]
        if tel:
            zeilen.append(f"TEL;TYPE=WORK:{tel}")
        if firma:
            zeilen.append(f"ORG:{firma}")
        zeilen.append("END:VCARD")
        dav("PUT", f"/{USER}/kontakte/{uid}.vcf", "\r\n".join(zeilen) + "\r\n", "text/vcard; charset=utf-8")
    print(f"Kontakte: {len(KONTAKTE)}")


def tag(versatz):
    return HEUTE + timedelta(days=versatz)


def zeit(versatz, stunde, minute=0):
    return datetime.combine(tag(versatz), datetime.min.time()).replace(hour=stunde, minute=minute)


def ics(*teile):
    return "\r\n".join(["BEGIN:VCALENDAR", "VERSION:2.0", "PRODID:-//Relay//Testserver//DE", *teile,
                        "END:VCALENDAR"]) + "\r\n"


def termin(kal, titel, beginn, ende=None, ort="", ganztag=False):
    uid = str(uuid.uuid5(uuid.NAMESPACE_URL, f"{kal}/{titel}"))
    stempel = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    if ganztag:
        zeilen = [f"DTSTART;VALUE=DATE:{beginn:%Y%m%d}", f"DTEND;VALUE=DATE:{(ende or beginn + timedelta(days=1)):%Y%m%d}"]
    else:
        zeilen = [f"DTSTART:{beginn:%Y%m%dT%H%M%S}", f"DTEND:{ende:%Y%m%dT%H%M%S}"]
    if ort:
        zeilen.append(f"LOCATION:{ort}")
    dav("PUT", f"/{USER}/{kal}/{uid}.ics",
        ics("BEGIN:VEVENT", f"UID:{uid}", f"DTSTAMP:{stempel}", f"SUMMARY:{titel}", *zeilen, "END:VEVENT"),
        "text/calendar; charset=utf-8")


def aufgabe(titel, faellig=None, prio=0, erledigt=False, notiz=""):
    uid = str(uuid.uuid5(uuid.NAMESPACE_URL, f"aufgaben/{titel}"))
    stempel = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    zeilen = [f"DUE;VALUE=DATE:{faellig:%Y%m%d}"] if faellig else []
    if prio:
        zeilen.append(f"PRIORITY:{prio}")
    if notiz:
        zeilen.append(f"DESCRIPTION:{notiz}")
    zeilen += ["STATUS:COMPLETED", f"COMPLETED:{stempel}"] if erledigt else ["STATUS:NEEDS-ACTION"]
    dav("PUT", f"/{USER}/aufgaben/{uid}.ics",
        ics("BEGIN:VTODO", f"UID:{uid}", f"DTSTAMP:{stempel}", f"SUMMARY:{titel}", *zeilen, "END:VTODO"),
        "text/calendar; charset=utf-8")


def kalender_einlegen():
    sammlung(f"/{USER}/arbeit/", "Arbeit", "VEVENT")
    sammlung(f"/{USER}/privat/", "Privat", "VEVENT")
    sammlung(f"/{USER}/aufgaben/", "Aufgaben", "VTODO")
    montag = -HEUTE.weekday()
    termine = [
        ("arbeit", "Teamrunde", zeit(montag, 9), zeit(montag, 10), "Besprechungsraum 2"),
        ("arbeit", "Abstimmung Messestand", zeit(montag + 1, 10), zeit(montag + 1, 11, 30), "Videokonferenz"),
        ("arbeit", "Mittagessen mit Tobias", zeit(0, 12, 30), zeit(0, 13, 30), "Café am Markt"),
        ("arbeit", "Freigabe Broschüre", zeit(montag + 3, 14), zeit(montag + 3, 15), ""),
        ("arbeit", "Quartalsplanung", zeit(montag + 8, 9), zeit(montag + 8, 12), "Besprechungsraum 1"),
        ("privat", "Zahnarzt", zeit(montag + 2, 16), zeit(montag + 2, 17), "Praxis Dr. Beispiel"),
        ("privat", "Laufgruppe", zeit(montag + 4, 18), zeit(montag + 4, 19), "Stadtpark"),
    ]
    for kal, titel, b, e, ort in termine:
        termin(kal, titel, b, e, ort)
    termin("arbeit", "Messe Frühjahr", tag(montag + 21), tag(montag + 24), "Messegelände", ganztag=True)
    termin("privat", "Sommerfest", tag(montag + 25), ganztag=True)
    print(f"Termine: {len(termine) + 2} in 2 Kalendern")

    aufgaben = [
        ("Angebot Messestand prüfen", tag(0), 1, False, "Preise mit dem Vorjahr vergleichen"),
        ("Rückmeldung zur Broschüre an Lena", tag(1), 5, False, ""),
        ("Rechnung 2026-0412 freigeben", tag(-1), 1, False, ""),
        ("Reisekosten abrechnen", tag(5), 0, False, ""),
        ("Unterlagen für Dienstag vorbereiten", tag(-2), 0, True, ""),
        ("Ideen für das Sommerfest sammeln", None, 0, False, ""),
    ]
    for a in aufgaben:
        aufgabe(*a)
    print(f"Aufgaben: {len(aufgaben)}")


# ── Relay ─────────────────────────────────────────────────────────────────

def relay(basis, methode, pfad, daten=None, zeitlimit=120):
    req = urllib.request.Request(basis + "/api/v1" + pfad, method=methode,
                                 data=json.dumps(daten).encode() if daten is not None else None)
    req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=zeitlimit) as r:
            text = r.read().decode()
            return json.loads(text) if text else None
    except urllib.error.HTTPError as e:
        raise SystemExit(f"Relay {methode} {pfad}: {e.code} {e.read()[:300]!r}")


def relay_verbinden(basis):
    if relay(basis, "GET", "/accounts"):
        raise SystemExit("Relay hat schon ein Konto; bitte mit leerem Datenordner starten.")
    relay(basis, "POST", "/accounts", {
        "name": "Erika (Test)",
        "imap_host": IMAP[0], "imap_port": IMAP[1], "imap_ssl": False,
        "smtp_host": SMTP[0], "smtp_port": SMTP[1], "smtp_tls": False,
        "imap_username": USER, "imap_password": PASSWORT,
        "smtp_username": USER, "smtp_password": PASSWORT,
        "sender_name": ICH[0], "sender_email": ICH[1],
    })
    relay(basis, "POST", "/carddav/settings", {"url": f"{DAV}/{USER}/", "username": USER, "password": PASSWORT})
    relay(basis, "POST", "/calendars/caldav-accounts",
          {"name": "Testserver", "url": f"{DAV}/{USER}/", "username": USER, "password": PASSWORT})
    relay(basis, "POST", "/carddav/sync")
    relay(basis, "POST", "/calendars/sync")
    relay(basis, "POST", "/todos/sync")

    # The mail sync runs in the background; wait until the inbox is there.
    konto = relay(basis, "GET", "/accounts")[0]["id"]
    erwartet = sum(1 for m in MAILS if m[0] == "INBOX")
    # The first cycle after start backs off to 40 s, so allow three minutes.
    for _ in range(180):
        da = relay(basis, "GET", f"/messages?account_id={konto}&folder=INBOX&list_only=1")
        if len(da) >= erwartet:
            break
        time.sleep(1)
    else:
        raise SystemExit(f"Relay hat nach 3 min erst {len(da)} von {erwartet} Mails im Posteingang.")
    print(f"Relay: Konto {konto} verbunden, {len(da)} Mails im Posteingang, Kontakte, Kalender und Aufgaben abgeglichen")


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--relay", default="http://127.0.0.1:3799", help="Adresse des Relay-Servers")
    p.add_argument("--ohne-relay", action="store_true", help="nur die Testserver füllen")
    a = p.parse_args()
    mails_einlegen()
    kontakte_einlegen()
    kalender_einlegen()
    if not a.ohne_relay:
        relay_verbinden(a.relay.rstrip("/"))


if __name__ == "__main__":
    sys.exit(main())
