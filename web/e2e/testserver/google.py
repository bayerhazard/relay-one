#!/usr/bin/env python3
"""A stand-in for Google, for the probe of the Google sign-in (Kai, 9.10.2026).

Relay talks to Google in four ways for a Gmail account: the sign-in (OAuth),
CalDAV, CardDAV and the Tasks API. Real Google cannot be reached from the CI
without a test account and a person clicking "Zulassen", so this server
answers for it on 127.0.0.1:5299. Relay is pointed at it with
RELAY_GOOGLE_TEST_BASIS=http://127.0.0.1:5299.

- /o/oauth2/v2/auth agrees at once and sends the browser back with a code.
- /token trades the code and refreshes; tokens live 61 s, so Relay has to
  refresh on every use (its margin is a minute) — the refresh is tested.
- /revoke forgets a refresh token.
- /tasks/v1/… keeps two lists in memory ("Meine Aufgaben", "Einkauf").
- Everything else is DAV: it checks the Bearer token and passes the request
  on to Radicale (starten.sh) as erika. /caldav/… and /carddav/… lead to
  Radicale's root, so Relay finds erika's collections below the principal.
- /stand shows the probe what Relay did (tasks, refreshes, revoked).

Client: relay-test.apps.googleusercontent.com / geheim-test.
Standard library only.
"""

import base64
import json
import threading
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = 5299
CLIENT_ID = "relay-test.apps.googleusercontent.com"
CLIENT_SECRET = "geheim-test"
RADICALE = "http://127.0.0.1:5232"
DAV_LOGIN = "Basic " + base64.b64encode(b"erika:geheim").decode()

sperre = threading.Lock()
codes = {}          # code -> (redirect_uri, email)
refresh = {}        # refresh token -> email
zugaenge = set()    # valid access tokens
zaehler = {"ausgegeben": 0, "erneuert": 0, "widerrufen": 0}
listen = [{"id": "L1", "title": "Meine Aufgaben"}, {"id": "L2", "title": "Einkauf"}]
aufgaben = {
    "L1": [{"id": "T1", "title": "Steuer vorbereiten", "status": "needsAction",
            "due": "2026-10-20T00:00:00.000Z", "notes": "Belege sammeln"}],
    "L2": [{"id": "T2", "title": "Milch", "status": "needsAction"}],
}
naechste = [100]


def id_token(email):
    teil = lambda d: base64.urlsafe_b64encode(json.dumps(d).encode()).decode().rstrip("=")
    return f"{teil({'alg': 'none'})}.{teil({'iss': 'stand-in', 'email': email})}.x"


def neuer_zugang():
    zaehler["ausgegeben"] += 1
    t = f"at-{zaehler['ausgegeben']}"
    zugaenge.add(t)
    return t


class Google(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass

    def antwort(self, status, daten=None, kopf=None, roh=None, art="application/json"):
        body = roh if roh is not None else (json.dumps(daten).encode() if daten is not None else b"")
        self.send_response(status)
        for k, v in (kopf or {}).items():
            self.send_header(k, v)
        if body or status not in (204, 304):
            self.send_header("Content-Type", art)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def koerper(self):
        n = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(n) if n else b""

    def angemeldet(self):
        a = self.headers.get("Authorization", "")
        return a.startswith("Bearer ") and a[7:] in zugaenge

    # ── OAuth ──────────────────────────────────────────────────────────
    def anmelden(self, q):
        if q.get("client_id") != CLIENT_ID:
            return self.antwort(400, {"error": "invalid_client"})
        for bereich in ("auth/calendar", "auth/carddav", "auth/tasks"):
            if bereich not in q.get("scope", ""):
                return self.antwort(400, {"error": "invalid_scope", "fehlt": bereich})
        code = f"code-{len(codes) + 1}"
        codes[code] = (q["redirect_uri"], q.get("login_hint") or "erika.relay@gmail.com")
        ziel = q["redirect_uri"] + "?" + urllib.parse.urlencode({"code": code, "state": q.get("state", "")})
        self.antwort(302, kopf={"Location": ziel})

    def token(self):
        f = dict(urllib.parse.parse_qsl(self.koerper().decode()))
        if f.get("client_id") != CLIENT_ID or f.get("client_secret") != CLIENT_SECRET:
            return self.antwort(401, {"error": "invalid_client"})
        if f.get("grant_type") == "authorization_code":
            c = codes.pop(f.get("code", ""), None)
            if not c or c[0] != f.get("redirect_uri"):
                return self.antwort(400, {"error": "invalid_grant"})
            rt = f"rt-{len(refresh) + 1}"
            refresh[rt] = c[1]
            return self.antwort(200, {"access_token": neuer_zugang(), "expires_in": 61, "token_type": "Bearer",
                                      "refresh_token": rt, "id_token": id_token(c[1])})
        if f.get("grant_type") == "refresh_token":
            if f.get("refresh_token") not in refresh:
                return self.antwort(400, {"error": "invalid_grant"})
            zaehler["erneuert"] += 1
            return self.antwort(200, {"access_token": neuer_zugang(), "expires_in": 61, "token_type": "Bearer"})
        self.antwort(400, {"error": "unsupported_grant_type"})

    def widerruf(self):
        f = dict(urllib.parse.parse_qsl(self.koerper().decode()))
        if refresh.pop(f.get("token", ""), None) is not None:
            zaehler["widerrufen"] += 1
        self.antwort(200, {})

    # ── Tasks API ──────────────────────────────────────────────────────
    def tasks(self, methode, teile):
        if not self.angemeldet():
            return self.antwort(401, {"error": {"code": 401}})
        # users/@me/lists
        if teile[:2] == ["users", "@me"] and teile[2:] == ["lists"] and methode == "GET":
            return self.antwort(200, {"kind": "tasks#taskLists", "items": listen})
        if len(teile) >= 3 and teile[0] == "lists" and teile[2] == "tasks" and teile[1] in aufgaben:
            liste = aufgaben[teile[1]]
            if len(teile) == 3 and methode == "GET":
                return self.antwort(200, {"kind": "tasks#tasks", "items": liste})
            if len(teile) == 3 and methode == "POST":
                d = json.loads(self.koerper() or b"{}")
                naechste[0] += 1
                neu = {"id": f"T{naechste[0]}", "title": d.get("title", ""), "notes": d.get("notes", ""),
                       "status": d.get("status", "needsAction"), "due": d.get("due")}
                liste.append(neu)
                return self.antwort(200, neu)
            if len(teile) == 4:
                treffer = [a for a in liste if a["id"] == teile[3]]
                if not treffer:
                    return self.antwort(404, {"error": {"code": 404}})
                a = treffer[0]
                if methode == "PATCH":
                    d = json.loads(self.koerper() or b"{}")
                    for k in ("title", "notes", "status", "due"):
                        if k in d:
                            a[k] = d[k]
                    if a.get("status") == "completed":
                        a.setdefault("completed", "2026-10-09T12:00:00.000Z")
                    return self.antwort(200, a)
                if methode == "DELETE":
                    liste.remove(a)
                    return self.antwort(204)
        self.antwort(404, {"error": {"code": 404}})

    # ── DAV, passed on to Radicale ─────────────────────────────────────
    def dav(self, methode, pfad):
        if not self.angemeldet():
            return self.antwort(401, roh=b"", kopf={"WWW-Authenticate": 'Bearer realm="stand-in"'})
        if pfad.startswith("/caldav/") or pfad.startswith("/carddav/"):
            pfad = "/"
        body = self.koerper()
        req = urllib.request.Request(RADICALE + pfad, data=body or None, method=methode)
        for k in ("Depth", "Content-Type", "If-Match", "If-None-Match"):
            if self.headers.get(k):
                req.add_header(k, self.headers[k])
        req.add_header("Authorization", DAV_LOGIN)
        try:
            with urllib.request.urlopen(req, timeout=30) as r:
                status, kopf, roh = r.status, r.headers, r.read()
        except urllib.error.HTTPError as e:
            status, kopf, roh = e.code, e.headers, e.read()
        weiter = {k: kopf[k] for k in ("ETag", "Location") if kopf.get(k)}
        self.antwort(status, roh=roh, kopf=weiter, art=kopf.get("Content-Type", "application/xml"))

    def verteilen(self, methode):
        u = urllib.parse.urlsplit(self.path)
        q = dict(urllib.parse.parse_qsl(u.query))
        with sperre:
            if u.path == "/o/oauth2/v2/auth" and methode == "GET":
                return self.anmelden(q)
            if u.path == "/token" and methode == "POST":
                return self.token()
            if u.path == "/revoke" and methode == "POST":
                return self.widerruf()
            if u.path == "/stand" and methode == "GET":
                return self.antwort(200, {"zaehler": zaehler, "aufgaben": aufgaben, "angemeldet": list(refresh.values())})
            if u.path.startswith("/tasks/v1/"):
                return self.tasks(methode, [urllib.parse.unquote(t) for t in u.path[len("/tasks/v1/"):].split("/") if t])
        self.dav(methode, u.path)

    def do_GET(self):
        self.verteilen("GET")

    def do_POST(self):
        self.verteilen("POST")

    def do_PATCH(self):
        self.verteilen("PATCH")

    def do_DELETE(self):
        self.verteilen("DELETE")

    def do_PUT(self):
        self.verteilen("PUT")

    def do_PROPFIND(self):
        self.verteilen("PROPFIND")

    def do_REPORT(self):
        self.verteilen("REPORT")


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", PORT), Google).serve_forever()
