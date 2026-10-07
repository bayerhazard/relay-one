// Before the tour: the server answers. Relay keeps no sample data of its own
// — mails come from IMAP, contacts from CardDAV, tasks and events from
// CalDAV — so on a fresh box every area shows its empty state or, for the
// mail, the setup screen. That is what the tour checks; pages full of rows
// would need mail and DAV test servers in CI (open point, Etappe 7).

const BASIS = (process.env.RELAY_URL ?? "http://127.0.0.1:3799") + "/api/v1";

export default async function vorbereitung() {
  for (let i = 0; i < 30; i++) {
    try {
      const r = await fetch(`${BASIS}/accounts`);
      if (r.ok) return;
    } catch {
      // not up yet
    }
    await new Promise((fertig) => setTimeout(fertig, 1000));
  }
  throw new Error(`Relay antwortet nicht unter ${BASIS} — vorher relay-server starten.`);
}
