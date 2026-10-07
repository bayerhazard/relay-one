#!/usr/bin/env bash
# Stops the test servers started by starten.sh. GreenMail keeps mail in
# memory and Radicale's storage is wiped on the next start, so a stop and
# start always gives the same empty servers for befuellen.py.
set -u
LAUF="${TESTSERVER_DIR:-$(cd "$(dirname "$0")" && pwd)/.lauf}"
for name in greenmail radicale; do
  [ -f "$LAUF/$name.pid" ] && kill "$(cat "$LAUF/$name.pid")" 2>/dev/null
  rm -f "$LAUF/$name.pid"
done
echo "Testserver gestoppt."
