#!/usr/bin/env bash
# Checks the Olares chart before every release PR and in release.yml
# (CI ABGLEICH RL-V2, after Rocket's scripts/check-chart.sh). Every rule
# stands for a mistake that cost a release on some box.
#
#   bash scripts/check-chart.sh
set -euo pipefail
cd "$(dirname "$0")/.."

CHART=chart/relay
MANIFEST=$CHART/OlaresManifest.yaml
FEHLER=0
melde() { echo "  FEHLER: $*"; FEHLER=1; }
wert() { grep -E "^[[:space:]]*$1:" "$2" | head -1 | sed -E "s/^[^:]+:[[:space:]]*//; s/[\"']//g; s/[[:space:]]+$//"; }

echo "→ Versionen im Gleichschritt"
# The market reads the version from four places; one left behind and the box
# shows the old version or refuses the upgrade (RL-V1: once five by hand).
V=$(wert version $CHART/Chart.yaml)
for paar in "appVersion:$CHART/Chart.yaml" "version:$MANIFEST" "versionName:$MANIFEST"; do
  feld=${paar%%:*}; datei=${paar#*:}
  [ "$(wert "$feld" "$datei")" = "$V" ] || melde "$datei $feld ist $(wert "$feld" "$datei"), Chart.yaml version ist $V"
done
# YY.M.n, month without a leading zero: 26.09.1 is no valid SemVer and
# `market upgrade` gets stuck on it.
echo "$V" | grep -Eq '^[0-9]{2}\.([1-9]|1[0-2])\.[1-9][0-9]*$' || melde "Version $V ist nicht YY.M.n (Monat ohne führende Null)"

echo "→ Namen identisch (Ordner, Chart, metadata.name, appid)"
for n in "$(basename $CHART)" "$(wert name $CHART/Chart.yaml)" "$(wert name $MANIFEST)" "$(wert appid $MANIFEST)"; do
  [ "$n" = relay ] || melde "Name '$n' statt 'relay'"
done

echo "→ Kein .Files.Get, keine Helm-Hooks"
! grep -rn "Files.Get" $CHART/templates || melde ".Files.Get — der Markt-Linter lehnt es ab"
! grep -rn "helm.sh/hook" $CHART/templates || melde "Helm-Hook — läuft vor dem ns-owner-Label und kommt nie durch"

echo "→ Kein NodePort, LoadBalancer oder hostNetwork"
! grep -rnE "NodePort|LoadBalancer|hostNetwork" $CHART/templates || melde "nur ClusterIP und der Entrance"

echo "→ Abbild folgt der Chart-Version, values.yaml pinnt nichts"
# Olares plays back the values of the installation on an upgrade; a tag in
# values.yaml would freeze the image at the first installed version.
grep -q '{{ .Chart.Version }}\|{{ .Chart.AppVersion }}' $CHART/templates/deployment.yaml || melde "deployment.yaml nimmt den Tag nicht aus dem Chart"
T=$(wert tag $CHART/values.yaml)
[ -z "$T" ] || melde "values.yaml image.tag ist '$T' — leer lassen"

echo "→ Markt-Notiz und Upgrade-Text zur Version"
# markt.yml writes the note into the market after the release; without it
# the action stops — the PR should notice first.
NOTIZ=$CHART/markt/$V.md
if [ ! -f "$NOTIZ" ]; then
  melde "$NOTIZ fehlt (# Titel, darunter v$V: … auf Englisch, nach '## Deutsch' auf Deutsch)"
else
  head -1 "$NOTIZ" | grep -q '^# ' || melde "$NOTIZ: erste Zeile '# Titel'"
  grep -q "^v$V: " "$NOTIZ" || melde "$NOTIZ: Text beginnt mit 'v$V: '"
fi
grep -q "^    v$V: " $MANIFEST || melde "upgradeDescription in $MANIFEST beginnt nicht mit 'v$V: '"
for t in beschreibung.en.md beschreibung.de.md; do
  [ -f $CHART/markt/$t ] || melde "$CHART/markt/$t fehlt"
done

echo "→ helm lint und helm template"
if command -v helm >/dev/null; then
  helm lint $CHART -f scripts/olares-werte.yaml >/dev/null || melde "helm lint"
  helm template relay $CHART -f scripts/olares-werte.yaml >/dev/null || melde "helm template"
else
  echo "  (kein helm hier — prüft release.yml)"
fi

if [ $FEHLER -eq 0 ]; then
  echo "Alles in Ordnung (relay $V)."
else
  echo "Prüfung fehlgeschlagen."
  exit 1
fi
