#!/usr/bin/env bash
# Proves a market entry locally with wrangler before it goes to Marc's repo
# (taken over from Rocket, CI ABGLEICH RL-V2):
#   - /api/v1/appstore/hash antwortet,
#   - /api/v1/applications/relay/chart liefert das Chart byte-gleich,
#   - /api/v2/applications/relay nennt Version und chartName.
#
#   bash scripts/markt-beweis.sh <markt-klon> <version> <sha256 des Charts>
set -euo pipefail
MARKT="$1"; V="$2"; SHA="$3"; PORT="${PORT:-8788}"
cd "$MARKT"
[ -d node_modules ] || npm ci --silent
# Eigene Prozessgruppe: wrangler startet workerd als Kind, das sonst bleibt.
setsid npx wrangler pages dev functions --port "$PORT" > wrangler.log 2>&1 &
WR=$!
trap 'kill -- -$WR 2>/dev/null || true' EXIT
for _ in $(seq 1 90); do curl -sf "localhost:$PORT/api/v1/appstore/hash" >/dev/null && break; sleep 1; done
curl -sf "localhost:$PORT/api/v1/appstore/hash" | grep -q '"hash"' || { tail -30 wrangler.log; echo "hash antwortet nicht"; exit 1; }
IST=$(curl -sf "localhost:$PORT/api/v1/applications/relay/chart" | sha256sum | cut -d' ' -f1)
[ "$IST" = "$SHA" ] || { echo "Chart weicht ab: $IST statt $SHA"; exit 1; }
curl -sf "localhost:$PORT/api/v2/applications/relay" | python3 -c "
import json, sys
s = json.dumps(json.load(sys.stdin))
for soll in ('\"chartName\": \"relay-$V.tgz\"', '\"version\": \"$V\"'):
    assert soll in s, 'fehlt: ' + soll
"
rm -f wrangler.log
echo "bewiesen: relay $V, Chart sha256 $SHA"
