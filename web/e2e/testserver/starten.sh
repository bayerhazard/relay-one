#!/usr/bin/env bash
# Test servers for the browser tour with data: GreenMail for IMAP/SMTP and
# Radicale for CalDAV/CardDAV, both on 127.0.0.1, no Docker needed.
#
#   web/e2e/testserver/starten.sh          # start both, wait until they answer
#   python3 web/e2e/testserver/befuellen.py  # fill them and connect Relay
#
# Needs java (17+) and python3. Everything lands in $TESTSERVER_DIR
# (default web/e2e/testserver/.lauf): the GreenMail jar, a venv with
# Radicale, Radicale's storage and both logs. Ports: IMAP 3143, SMTP 3025,
# DAV 5232. The one user is erika / geheim (erika@relay.test).
set -euo pipefail

HIER="$(cd "$(dirname "$0")" && pwd)"
LAUF="${TESTSERVER_DIR:-$HIER/.lauf}"
GREENMAIL_VERSION=2.1.3
RADICALE_VERSION=3.2.3
mkdir -p "$LAUF"

# GreenMail: one jar from Maven Central, cached in $LAUF.
JAR="$LAUF/greenmail-standalone-$GREENMAIL_VERSION.jar"
if [ ! -f "$JAR" ]; then
  url="https://repo1.maven.org/maven2/com/icegreen/greenmail-standalone/$GREENMAIL_VERSION/greenmail-standalone-$GREENMAIL_VERSION.jar"
  for i in 1 2 3 4; do
    curl -sSfL -o "$JAR.tmp" "$url" && mv "$JAR.tmp" "$JAR" && break
    sleep $((i * 4))
  done
  [ -f "$JAR" ] || { echo "GreenMail konnte nicht geladen werden." >&2; exit 1; }
fi

# Radicale: in its own venv so the system Python stays untouched.
if [ ! -x "$LAUF/venv/bin/radicale" ]; then
  python3 -m venv "$LAUF/venv"
  "$LAUF/venv/bin/pip" install -q "radicale==$RADICALE_VERSION"
fi

# A fresh Radicale storage on every start, so the seed is always the same.
rm -rf "$LAUF/radicale"
mkdir -p "$LAUF/radicale/sammlung"
cat > "$LAUF/radicale/config" <<EOF
[server]
hosts = 127.0.0.1:5232
[auth]
# No login: Relay's DAV client answers Digest challenges only, and Radicale
# speaks Basic. Any user name works; the seed uses erika.
type = none
[storage]
filesystem_folder = $LAUF/radicale/sammlung
[logging]
level = warning
EOF

nohup java \
  -Dgreenmail.setup.test.smtp -Dgreenmail.setup.test.imap \
  -Dgreenmail.hostname=127.0.0.1 \
  -Dgreenmail.users=erika:geheim@relay.test \
  -Dgreenmail.users.login=local_part \
  -jar "$JAR" > "$LAUF/greenmail.log" 2>&1 &
echo $! > "$LAUF/greenmail.pid"

nohup "$LAUF/venv/bin/radicale" --config "$LAUF/radicale/config" > "$LAUF/radicale.log" 2>&1 &
echo $! > "$LAUF/radicale.pid"

# Wait until all three ports answer (30 s at most).
for port in 3143 3025 5232; do
  for _ in $(seq 60); do
    (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null && break
    sleep 0.5
  done
  (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null || {
    echo "Port $port antwortet nicht; siehe $LAUF/*.log" >&2
    exit 1
  }
done
echo "Testserver laufen: IMAP 127.0.0.1:3143, SMTP 127.0.0.1:3025, DAV http://127.0.0.1:5232/"
