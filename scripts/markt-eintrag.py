#!/usr/bin/env python3
"""Builds Relay's entry in the market (bayerhazard/aimighty-market).

Taken over from Rocket's scripts/markt-eintrag.py (CI ABGLEICH RL-V2, Kai
6.10.2026: "wie Rocket"); only the app name, the paths and the icon differ.
What OpenCode used to do by hand on the box, in one place:

- `functions/_apps.ts`: the version of the Relay entry, and in front of
  `upgradeDescription` the notes of every version the market does not know
  yet (`chart/relay/markt/<version>.md`) — a note whose entry never arrived
  travels with the next one.
- `functions/_apps.ts`: categories from `OlaresManifest.yaml`, short and
  full description from `chart/relay/markt/beschreibung.{en,de}.md`, and with
  `--icon` the icon URL (the icon of the release, not raw from main).
- `functions/_lib.ts`: the chart key `relay-<version>.tgz` with the freshly
  encoded release asset, old Relay keys removed, `CANONICAL_EPOCH_MS`
  strictly above the value on main, and with `--quelle` the `SOURCE_ID` the
  boxes registered the market under (case matters: under another id a box
  takes no update).

    python3 scripts/markt-eintrag.py --markt <clone> --chart relay-26.10.3.tgz \
        --version 26.10.3 --notizen chart/relay/markt --pr-text pr.md

Prints the PR title on stdout; exit code 3 if the market already has the
version. Stops when anything does not look as expected — no entry is
better than half an entry.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import re
import sys
from pathlib import Path


def version_tupel(v: str) -> tuple[int, ...]:
    return tuple(int(t) for t in v.split("."))


def notiz_lesen(datei: Path) -> tuple[str, str, str | None]:
    """`# Titel`, darunter Englisch ab `v<version>: `; optional nach einer Zeile
    `## Deutsch` derselbe Inhalt auf Deutsch, ebenfalls ab `v<version>: `."""
    roh = datei.read_text(encoding="utf-8").strip()
    teile = re.split(r"^## Deutsch\s*$", roh, maxsplit=1, flags=re.MULTILINE)
    zeilen = teile[0].strip().splitlines()
    if not zeilen or not zeilen[0].startswith("# "):
        raise SystemExit(f"{datei}: erste Zeile muss '# <Titel>' sein")
    titel = zeilen[0][2:].strip()
    version = datei.stem
    en = " ".join(z.strip() for z in zeilen[1:] if z.strip())
    de = " ".join(z.strip() for z in teile[1].splitlines() if z.strip()) if len(teile) > 1 else None
    for sprache, text in (("Englisch", en), ("Deutsch", de)):
        if text is None:
            continue
        if not text.startswith(f"v{version}: "):
            raise SystemExit(f"{datei}: {sprache} muss mit 'v{version}: ' beginnen")
        kein_template(text, datei)
    return titel, en, de


def kein_template(text: str, quelle: Path | str) -> None:
    if "`" in text or "${" in text:
        raise SystemExit(f"{quelle}: kein Backtick und kein '${{' — der Text steht in einem Template-String")


def texte_lesen(datei: Path) -> tuple[str, str]:
    """`# Kurz` (eine Zeile) und `# Beschreibung` (Markdown) für den Markt."""
    roh = datei.read_text(encoding="utf-8")
    m = re.match(r"\s*# Kurz\s*\n(.+?)\n\s*# Beschreibung\s*\n(.+)", roh, flags=re.DOTALL)
    if not m:
        raise SystemExit(f"{datei}: erwartet '# Kurz' und '# Beschreibung'")
    kurz, lang = m.group(1).strip(), m.group(2).strip()
    if "\n" in kurz or '"' in kurz:
        raise SystemExit(f"{datei}: Kurzbeschreibung ist eine Zeile ohne Anführungszeichen")
    kein_template(lang, datei)
    return kurz, lang


FELD = "\n      "  # Einrückung der Felder unter metadata in _apps.ts


def feld_ersetzen(block: str, name: str, wert: str) -> str:
    """Ersetzt ein metadata-Feld samt Wert, gleich in welcher Form es dasteht."""
    m = re.search(re.escape(FELD + name + ":"), block)
    if not m:
        raise SystemExit(f"_apps.ts: {name} im Relay-Eintrag nicht gefunden")
    n = re.compile(re.escape(FELD) + r"[A-Za-z]+:").search(block, m.end())
    ende = n.start() if n else len(block)
    trenner = "" if wert.startswith("\n") else " "
    return block[: m.start()] + FELD + name + ":" + trenner + wert + "," + block[ende:]


def feld_lesen(block: str, name: str) -> dict[str, str]:
    """Liest einen Text- oder Sprachen-Wert: `...` | "..." | { en: ..., de: ... }."""
    m = re.search(re.escape(FELD + name + ":"), block)
    if not m:
        raise SystemExit(f"_apps.ts: {name} im Relay-Eintrag nicht gefunden")
    n = re.compile(re.escape(FELD) + r"[A-Za-z]+:").search(block, m.end())
    roh = block[m.end() : n.start() if n else len(block)].strip().rstrip(",").strip()
    def wert(s: str) -> str:
        s = s.strip().rstrip(",").strip()
        if s[:1] == "`":
            return s[1:-1]
        return json.loads(s)
    if roh.startswith("{"):
        paare = re.findall(r'(\w+)\s*:\s*(`[^`]*`|"(?:[^"\\]|\\.)*")', roh)
        return {k: wert(v) for k, v in paare}
    return {"en": wert(roh)}


def sprachen(werte: dict[str, str], als_template: bool) -> str:
    def text(s: str) -> str:
        return f"`{s}`" if als_template else json.dumps(s, ensure_ascii=False)
    zeilen = "".join(f"{FELD}  {k}: {text(s)}," for k, s in werte.items())
    return "{" + zeilen + FELD + "}"


def liste_lesen(manifest: Path, schluessel: str) -> list[str]:
    """A list such as `categories:` or `supportArch:` from the OlaresManifest,
    without a YAML library."""
    zeilen = manifest.read_text(encoding="utf-8").splitlines()
    try:
        i = next(n for n, z in enumerate(zeilen) if z.strip() == f"{schluessel}:")
    except StopIteration:
        raise SystemExit(f"{manifest}: kein {schluessel}") from None
    werte = []
    for z in zeilen[i + 1 :]:
        s = z.strip()
        if s.startswith("#"):
            continue
        if not s.startswith("- "):
            break
        werte.append(s[2:].strip().strip("'\""))
    if not werte:
        raise SystemExit(f"{manifest}: {schluessel} ist leer")
    return werte


def kategorien_lesen(manifest: Path) -> list[str]:
    return liste_lesen(manifest, "categories")


def anmeldung_lesen(manifest: Path) -> str:
    """`authLevel` of the entrance — the market lists it with the app."""
    stufen = re.findall(r"^\s*authLevel:\s*['\"]?(\w+)", manifest.read_text(encoding="utf-8"), flags=re.MULTILINE)
    if len(stufen) != 1:
        raise SystemExit(f"{manifest}: {len(stufen)} authLevel statt einem")
    return stufen[0]


def relay_block(apps: str) -> tuple[int, int]:
    anfang = apps.index('name: "relay"')
    ende = apps.index("spec: {", anfang)
    return anfang, ende


def quelle_lesen(lib: str) -> str:
    m = re.search(r'export const SOURCE_ID = "([^"]+)";', lib)
    if not m:
        raise SystemExit("_lib.ts: SOURCE_ID nicht gefunden")
    return m.group(1)


def quelle_setzen(lib: str, quelle: str) -> str:
    if not re.fullmatch(r"[a-z0-9.]+", quelle):
        raise SystemExit(f"--quelle {quelle!r}: nur Kleinbuchstaben, Ziffern und Punkte")
    neu, n = re.subn(r'export const SOURCE_ID = "[^"]+";', f'export const SOURCE_ID = "{quelle}";', lib, count=1)
    if n != 1:
        raise SystemExit("_lib.ts: SOURCE_ID nicht gefunden")
    return neu


def epoch_heben(lib: str) -> tuple[str, int, int]:
    m = re.search(r"const CANONICAL_EPOCH_MS = (\d+);", lib)
    if not m:
        raise SystemExit("_lib.ts: CANONICAL_EPOCH_MS nicht gefunden")
    epoch_alt = int(m.group(1))
    epoch_neu = (epoch_alt // 1_000_000_000 + 1) * 1_000_000_000
    return lib[: m.start(1)] + str(epoch_neu) + lib[m.end(1):], epoch_alt, epoch_neu


def nur_quelle(arg: argparse.Namespace, lib_datei: Path, lib: str, quelle_alt: str, alt: str) -> None:
    """Der Markt kennt die Version schon, meldet sich aber unter der falschen
    Kennung — dann nur die Kennung richten und den Zeitstempel heben, damit
    die Boxen neu holen."""
    lib, epoch_alt, epoch_neu = epoch_heben(lib)
    lib = quelle_setzen(lib, arg.quelle)
    lib_datei.write_text(lib, encoding="utf-8")
    if arg.pr_text:
        arg.pr_text.write_text(
            "\n".join(
                [
                    "Kennung des Markts, gesetzt von der Action `markt.yml` im Relay-Repo.",
                    "",
                    f"- `functions/_lib.ts`: `SOURCE_ID` `{quelle_alt}` → `{arg.quelle}` — unter dieser Kennung "
                    "haben die Boxen den Markt eingetragen; bei einer anderen übernehmen sie kein Update.",
                    f"- `CANONICAL_EPOCH_MS` {epoch_alt} → {epoch_neu}, damit die Boxen neu holen.",
                    f"- Relay bleibt {alt}.",
                ]
            )
            + "\n",
            encoding="utf-8",
        )
    print(f"market: SOURCE_ID {arg.quelle} (relay {alt})")


def main() -> None:
    a = argparse.ArgumentParser()
    a.add_argument("--markt", type=Path, required=True)
    a.add_argument("--chart", type=Path, required=True)
    a.add_argument("--version", required=True)
    a.add_argument("--notizen", type=Path, required=True)
    a.add_argument("--pr-text", type=Path)
    a.add_argument("--manifest", type=Path, help="olares/OlaresManifest.yaml — Kategorien von dort")
    a.add_argument("--texte", type=Path, help="Ordner mit beschreibung.en.md und beschreibung.de.md")
    a.add_argument("--quelle", help="SOURCE_ID, unter der die Boxen den Markt kennen (z. B. market.aimighty)")
    a.add_argument("--icon", help="Adresse des Icons (aus dem Release)")
    arg = a.parse_args()

    v = arg.version
    apps_datei = arg.markt / "functions" / "_apps.ts"
    lib_datei = arg.markt / "functions" / "_lib.ts"
    apps = apps_datei.read_text(encoding="utf-8")
    lib = lib_datei.read_text(encoding="utf-8")

    # ── _apps.ts ──────────────────────────────────────────────────────────
    anfang, ende = relay_block(apps)
    block = apps[anfang:ende]
    m = re.search(r'version: "([0-9.]+)"', block)
    if not m:
        raise SystemExit("_apps.ts: keine Relay-Version gefunden")
    alt = m.group(1)
    quelle_alt = quelle_lesen(lib)
    if version_tupel(alt) >= version_tupel(v):
        if arg.quelle and quelle_alt != arg.quelle:
            nur_quelle(arg, lib_datei, lib, quelle_alt, alt)
            return
        print(f"Markt hat Relay {alt}, nichts zu tun für {v}", file=sys.stderr)
        raise SystemExit(3)

    neue = sorted(
        (d for d in arg.notizen.glob("*.md") if re.fullmatch(r"\d+\.\d+\.\d+", d.stem) and version_tupel(alt) < version_tupel(d.stem) <= version_tupel(v)),
        key=lambda d: version_tupel(d.stem),
        reverse=True,
    )
    if not neue or neue[0].stem != v:
        raise SystemExit(f"Notiz {arg.notizen}/{v}.md fehlt")
    notizen = [notiz_lesen(d) for d in neue]
    titel = notizen[0][0]
    neu_en = " ".join(en for _, en, _ in notizen)
    # Eine Version ohne deutsche Notiz steht auch im Deutschen auf Englisch.
    neu_de = " ".join(de or en for _, en, de in notizen)

    block = block.replace(f'version: "{alt}"', f'version: "{v}"', 1)
    if arg.manifest:
        kategorien = kategorien_lesen(arg.manifest)
        block, n = re.subn(
            r"categories: \[[^\]]*\]",
            "categories: [" + ", ".join(f'"{k}"' for k in kategorien) + "]",
            block,
            count=1,
        )
        if n != 1:
            raise SystemExit("_apps.ts: categories im Relay-Eintrag nicht gefunden")
        # Architectures as the chart promises them (Kai, 7.10.2026: the
        # market listed arm64 that was never built).
        arch = liste_lesen(arg.manifest, "supportArch")
        block, n = re.subn(
            r"supportArch: \[[^\]]*\]",
            "supportArch: [" + ", ".join(f'"{a}"' for a in arch) + "]",
            block,
            count=1,
        )
        if n != 1:
            raise SystemExit("_apps.ts: supportArch im Relay-Eintrag nicht gefunden")
    bisher = feld_lesen(block, "upgradeDescription")
    upgrade = {"en": neu_en + " " + bisher["en"]}
    if arg.texte:
        upgrade["de"] = neu_de + " " + bisher.get("de", bisher["en"])
    elif "de" in bisher:
        upgrade["de"] = neu_en + " " + bisher["de"]
    block = feld_ersetzen(block, "upgradeDescription", sprachen(upgrade, als_template=True) if len(upgrade) > 1 else f"{FELD}  `{upgrade['en']}`")

    if arg.icon:
        if not re.fullmatch(r"https://[\w./-]+\.png", arg.icon):
            raise SystemExit(f"--icon {arg.icon!r}: eine https-Adresse auf ein PNG")
        block, n = re.subn(r'icon: "[^"]*"', f'icon: "{arg.icon}"', block, count=1)
        if n != 1:
            raise SystemExit("_apps.ts: icon im Relay-Eintrag nicht gefunden")
    if arg.texte:
        kurz_en, lang_en = texte_lesen(arg.texte / "beschreibung.en.md")
        kurz_de, lang_de = texte_lesen(arg.texte / "beschreibung.de.md")
        block = feld_ersetzen(block, "description", sprachen({"en": kurz_en, "de": kurz_de}, als_template=False))
        block = feld_ersetzen(block, "fullDescription", sprachen({"en": lang_en, "de": lang_de}, als_template=True))
    rest = apps[ende:]
    if arg.manifest:
        # The entrance's sign-in as in the chart (Kai, 7.10.2026: the market
        # listed "public", Relay installs with "internal"). Only within the
        # Relay entry's spec, up to the next app.
        naechste = rest.find("metadata: {")
        spec = rest if naechste < 0 else rest[:naechste]
        spec, n = re.subn(r'authLevel: "[a-z]+"', f'authLevel: "{anmeldung_lesen(arg.manifest)}"', spec, count=1)
        if n != 1:
            raise SystemExit("_apps.ts: authLevel im Relay-Eintrag nicht gefunden")
        rest = spec + ("" if naechste < 0 else rest[naechste:])
    apps = apps[:anfang] + block + rest

    # ── _lib.ts ───────────────────────────────────────────────────────────
    roh = arg.chart.read_bytes()
    if roh[:2] != b"\x1f\x8b":
        raise SystemExit("Chart ist kein gzip")
    b64 = base64.b64encode(roh).decode()
    muster = r'\n\s*"relay-[0-9.]+\.tgz": "[A-Za-z0-9+/=]+",?'
    schluessel = list(re.finditer(muster, lib))
    if not schluessel:
        raise SystemExit("_lib.ts: kein Relay-Schlüssel")
    # Marc's tooling merges the market from two lineages and can bring an
    # old chart key back (10.10.2026: relay-26.10.2 beside 26.10.24). The
    # market serves one Relay chart; the surplus old keys go, from the end
    # so the earlier positions stay valid.
    for alt_k in reversed(schluessel[1:]):
        name = re.search(r'"(relay-[0-9.]+\.tgz)"', alt_k.group(0)).group(1)
        print(f"_lib.ts: überzähliger Relay-Schlüssel {name} entfernt", file=sys.stderr)
        lib = lib[: alt_k.start()] + lib[alt_k.end():]
    k = re.search(muster, lib)
    komma = "," if k.group(0).endswith(",") else ""
    einrueck = re.match(r"\n(\s*)", k.group(0)).group(1)
    lib = lib[: k.start()] + f'\n{einrueck}"relay-{v}.tgz": "{b64}"{komma}' + lib[k.end():]

    lib, epoch_alt, epoch_neu = epoch_heben(lib)
    if arg.quelle:
        lib = quelle_setzen(lib, arg.quelle)

    apps_datei.write_text(apps, encoding="utf-8")
    lib_datei.write_text(lib, encoding="utf-8")

    sha = hashlib.sha256(roh).hexdigest()
    if arg.pr_text:
        arg.pr_text.write_text(
            "\n".join(
                [
                    f"Relay {alt} → {v}, gebaut von der Action `markt.yml` im Relay-Repo nach dem Release.",
                    "",
                    f"- `functions/_apps.ts`: Version {alt} → {v}, Notiz{'en' if len(neue) > 1 else ''} "
                    + ", ".join(d.stem for d in neue)
                    + " vorn in `upgradeDescription`.",
                    (
                        f"- `functions/_lib.ts`: `relay-{v}.tgz` (Release-Anhang, sha256 `{sha}`), "
                        f"`CANONICAL_EPOCH_MS` {epoch_alt} → {epoch_neu}."
                    ),
                    *(
                        [f"- `functions/_lib.ts`: `SOURCE_ID` `{quelle_alt}` → `{arg.quelle}` (Kennung der Boxen)."]
                        if arg.quelle and quelle_alt != arg.quelle
                        else []
                    ),
                    "",
                    "Vor dem PR lokal mit wrangler bewiesen: Hash, Chart byte-gleich, Detail mit Version und chartName.",
                ]
            )
            + "\n",
            encoding="utf-8",
        )
    print(f"relay {v}: {titel}")


if __name__ == "__main__":
    main()
