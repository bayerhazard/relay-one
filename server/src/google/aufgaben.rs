//! Google Tasks for a Gmail account (Schritt 2). Google offers no CalDAV for
//! tasks, so Relay reads and writes them through the Tasks API and files
//! them like VTODOs: every task list becomes a calendar row of the account
//! (`mail-<id>`), every task a todo whose URL is its address in the API.
//!
//! Google Tasks knows title, notes, a due date (no time), done and a parent
//! task. Priority, labels, repetition and dependencies stay in Relay.

use serde::{Deserialize, Serialize};

use super::{endpunkte, zugang};
use crate::dav::ics::IcsTodo;
use crate::dav::Calendar;

#[derive(Deserialize, Debug, Clone)]
pub struct Liste {
    pub id: String,
    #[serde(default)]
    pub title: String,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Aufgabe {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// "needsAction" or "completed".
    #[serde(default)]
    pub status: String,
    /// RFC 3339; only the date counts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing)]
    pub deleted: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Seite<T> {
    #[serde(default = "Vec::new")]
    items: Vec<T>,
    #[serde(default)]
    next_page_token: Option<String>,
}

/// The address Relay files a list under.
pub fn listen_url(liste: &str) -> String {
    format!("{}/lists/{}/tasks/", endpunkte().aufgaben, liste)
}

/// Whether a calendar or todo URL is a Google task list or task.
pub fn ist_aufgabe(url: &str) -> bool {
    url.starts_with(&format!("{}/lists/", endpunkte().aufgaben))
}

/// (list, task) from a todo URL; task is empty for a list URL.
pub fn teile(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix(&format!("{}/lists/", endpunkte().aufgaben))?;
    let (liste, aufgabe) = rest.split_once("/tasks/")?;
    Some((liste.to_string(), aufgabe.trim_end_matches('/').to_string()))
}

/// UID Relay gives a Google task.
pub fn uid(aufgabe: &str) -> String {
    format!("google-task-{aufgabe}")
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default()
}

async fn antwort<T: for<'de> Deserialize<'de>>(resp: reqwest::Response) -> Result<T, String> {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        let kurz: String = text.chars().take(300).collect();
        return Err(format!("Google Tasks antwortet {status}: {kurz}"));
    }
    serde_json::from_str(&text).map_err(|e| format!("Google Tasks: Antwort nicht lesbar: {e}"))
}

/// The account's task lists, in Google's order (the first is "Meine Aufgaben").
pub async fn listen(konto: i64) -> Result<Vec<Liste>, String> {
    let token = zugang(konto).await?;
    let mut alle = Vec::new();
    let mut seite: Option<String> = None;
    loop {
        let mut req = client()
            .get(format!("{}/users/@me/lists", endpunkte().aufgaben))
            .bearer_auth(&token)
            .query(&[("maxResults", "100")]);
        if let Some(p) = &seite {
            req = req.query(&[("pageToken", p.as_str())]);
        }
        let s: Seite<Liste> = antwort(req.send().await.map_err(|e| e.to_string())?).await?;
        alle.extend(s.items);
        match s.next_page_token {
            Some(p) if !p.is_empty() => seite = Some(p),
            _ => return Ok(alle),
        }
    }
}

/// Every task of one list, done ones included.
pub async fn aufgaben(konto: i64, liste: &str) -> Result<Vec<Aufgabe>, String> {
    let token = zugang(konto).await?;
    let mut alle = Vec::new();
    let mut seite: Option<String> = None;
    loop {
        let mut req = client()
            .get(format!("{}/lists/{}/tasks", endpunkte().aufgaben, liste))
            .bearer_auth(&token)
            .query(&[("maxResults", "100"), ("showCompleted", "true"), ("showHidden", "true")]);
        if let Some(p) = &seite {
            req = req.query(&[("pageToken", p.as_str())]);
        }
        let s: Seite<Aufgabe> = antwort(req.send().await.map_err(|e| e.to_string())?).await?;
        alle.extend(s.items.into_iter().filter(|a| !a.deleted));
        match s.next_page_token {
            Some(p) if !p.is_empty() => seite = Some(p),
            _ => return Ok(alle),
        }
    }
}

/// Google's due date (midnight UTC of the day) as Relay's date-only due:
/// the start of that day in Berlin.
pub fn faellig_nach_relay(due: &str) -> Option<String> {
    let tag = chrono::DateTime::parse_from_rfc3339(due).ok()?.date_naive();
    let lokal = chrono::TimeZone::from_local_datetime(&chrono_tz::Europe::Berlin, &tag.and_hms_opt(0, 0, 0)?)
        .earliest()?;
    Some(lokal.with_timezone(&chrono::Utc).to_rfc3339())
}

/// Relay's due (any time) as Google's: the Berlin day at midnight UTC.
pub fn faellig_nach_google(due: &str) -> Option<String> {
    let tag = chrono::DateTime::parse_from_rfc3339(due)
        .ok()?
        .with_timezone(&chrono_tz::Europe::Berlin)
        .date_naive();
    Some(format!("{}T00:00:00.000Z", tag.format("%Y-%m-%d")))
}

/// A Google task as Relay's todo.
pub fn als_todo(liste: &str, a: &Aufgabe, projekt: Option<i64>) -> IcsTodo {
    let fertig = a.status == "completed";
    IcsTodo {
        uid: uid(&a.id),
        url: format!("{}{}", listen_url(liste), a.id),
        summary: Some(a.title.clone()),
        description: a.notes.clone().filter(|n| !n.is_empty()),
        due: a.due.as_deref().and_then(faellig_nach_relay),
        completed: if fertig { a.completed.clone() } else { None },
        status: Some(if fertig { "COMPLETED" } else { "NEEDS-ACTION" }.to_string()),
        priority: None,
        rrule: None,
        labels: Vec::new(),
        parent_uid: a.parent.as_deref().map(uid),
        dependencies: Vec::new(),
        project_id: projekt,
        due_has_time: false,
        raw: String::new(),
    }
}

/// Keep what Google Tasks does not know: priority, labels, repetition,
/// dependencies, and the time of day while the date is still the same.
pub fn behalten(t: &mut IcsTodo, alt: &crate::cache::todo::TodoRow) {
    t.priority = alt.priority;
    t.labels = alt.labels.clone();
    t.rrule = alt.rrule.clone();
    t.dependencies = alt.dependencies.clone();
    if alt.due_has_time {
        let tag = |d: &str| {
            chrono::DateTime::parse_from_rfc3339(d)
                .ok()
                .map(|d| d.with_timezone(&chrono_tz::Europe::Berlin).date_naive())
        };
        if let (Some(neu), Some(vorher)) = (t.due.as_deref().and_then(tag), alt.due_at.as_deref().and_then(tag)) {
            if neu == vorher {
                t.due = alt.due_at.clone();
                t.due_has_time = true;
            }
        }
    }
}

/// What Relay writes to Google for a todo.
pub struct Felder<'a> {
    pub titel: &'a str,
    pub notizen: Option<&'a str>,
    pub faellig: Option<&'a str>,
    pub erledigt: bool,
}

fn koerper(f: &Felder) -> serde_json::Value {
    serde_json::json!({
        "title": f.titel,
        "notes": f.notizen.unwrap_or(""),
        // null clears the date on PATCH.
        "due": f.faellig.and_then(faellig_nach_google),
        "status": if f.erledigt { "completed" } else { "needsAction" },
    })
}

/// Create a task in a list; returns its todo URL and UID.
pub async fn anlegen(konto: i64, listen_url_: &str, f: &Felder<'_>) -> Result<(String, String), String> {
    let (liste, _) = teile(listen_url_).ok_or("Keine Google-Aufgabenliste")?;
    let token = zugang(konto).await?;
    let neu: Aufgabe = antwort(
        client()
            .post(format!("{}/lists/{}/tasks", endpunkte().aufgaben, liste))
            .bearer_auth(&token)
            .json(&koerper(f))
            .send()
            .await
            .map_err(|e| e.to_string())?,
    )
    .await?;
    Ok((format!("{}{}", listen_url(&liste), neu.id), uid(&neu.id)))
}

/// Write title, notes, due and done back to a task.
pub async fn aendern(konto: i64, url: &str, f: &Felder<'_>) -> Result<(), String> {
    let (liste, aufgabe) = teile(url).filter(|(_, a)| !a.is_empty()).ok_or("Keine Google-Aufgabe")?;
    let token = zugang(konto).await?;
    let _: serde_json::Value = antwort(
        client()
            .patch(format!("{}/lists/{}/tasks/{}", endpunkte().aufgaben, liste, aufgabe))
            .bearer_auth(&token)
            .json(&koerper(f))
            .send()
            .await
            .map_err(|e| e.to_string())?,
    )
    .await?;
    Ok(())
}

/// Delete a task at Google.
pub async fn loeschen(konto: i64, url: &str) -> Result<(), String> {
    let (liste, aufgabe) = teile(url).filter(|(_, a)| !a.is_empty()).ok_or("Keine Google-Aufgabe")?;
    let token = zugang(konto).await?;
    let resp = client()
        .delete(format!("{}/lists/{}/tasks/{}", endpunkte().aufgaben, liste, aufgabe))
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() || resp.status() == reqwest::StatusCode::NOT_FOUND {
        Ok(())
    } else {
        Err(format!("Google Tasks antwortet {}", resp.status()))
    }
}

/// Fetch every list and task and file them under the account `konto_id`
/// (`mail-<id>`). The first list is Relay's inbox, the others are projects.
pub async fn abgleichen(state: &crate::AppState, konto: i64, konto_id: &str) -> Result<usize, String> {
    let listen = listen(konto).await?;
    let mut gelesen: Vec<(Liste, Vec<Aufgabe>)> = Vec::new();
    for l in listen {
        let a = aufgaben(konto, &l.id).await?;
        gelesen.push((l, a));
    }
    let mut geschrieben = 0usize;
    let mut guard = crate::db::get_db(state).map_err(|e| e.to_string())?;
    let conn = guard.as_mut().ok_or("Datenbank nicht verfügbar")?;
    for (i, (l, aufgaben)) in gelesen.iter().enumerate() {
        let cal = Calendar { href: l.id.clone(), display_name: Some(l.title.clone()), url: listen_url(&l.id) };
        let cal_id = crate::cache::cal::upsert_calendar(conn, &cal, konto_id).map_err(|e| e.to_string())?;
        let projekt = if i == 0 { None } else { Some(cal_id) };
        for a in aufgaben {
            let mut t = als_todo(&l.id, a, projekt);
            if let Ok(Some(alt)) = crate::cache::todo::find_todo(conn, &t.uid) {
                behalten(&mut t, &alt);
            }
            match crate::cache::todo::upsert_todo(conn, cal_id, &t) {
                Ok(()) => geschrieben += 1,
                Err(e) => tracing::warn!("Google Tasks: '{}' nicht gespeichert: {}", a.title, e),
            }
        }
    }
    Ok(geschrieben)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faelligkeit_hin_und_zurueck() {
        // Google: the day at midnight UTC; Relay: the day's start in Berlin.
        let relay = faellig_nach_relay("2026-10-12T00:00:00.000Z").unwrap();
        assert_eq!(relay, "2026-10-11T22:00:00+00:00");
        assert_eq!(faellig_nach_google(&relay).as_deref(), Some("2026-10-12T00:00:00.000Z"));
        // Winter time.
        assert_eq!(faellig_nach_relay("2026-12-01T00:00:00Z").unwrap(), "2026-11-30T23:00:00+00:00");
    }

    #[test]
    fn aufgabe_wird_todo() {
        std::env::remove_var("RELAY_GOOGLE_TEST_BASIS");
        let a = Aufgabe {
            id: "abc".into(),
            title: "Angebot schicken".into(),
            notes: Some("an Weber".into()),
            status: "completed".into(),
            due: Some("2026-10-12T00:00:00.000Z".into()),
            completed: Some("2026-10-09T10:00:00.000Z".into()),
            parent: Some("eltern".into()),
            deleted: false,
        };
        let t = als_todo("L1", &a, Some(4));
        assert_eq!(t.uid, "google-task-abc");
        assert_eq!(t.url, "https://tasks.googleapis.com/tasks/v1/lists/L1/tasks/abc");
        assert_eq!(t.status.as_deref(), Some("COMPLETED"));
        assert_eq!(t.parent_uid.as_deref(), Some("google-task-eltern"));
        assert_eq!(t.project_id, Some(4));
        assert!(!t.due_has_time);
        assert!(ist_aufgabe(&t.url));
        assert_eq!(teile(&t.url), Some(("L1".into(), "abc".into())));
        assert_eq!(teile(&listen_url("L1")), Some(("L1".into(), String::new())));
        assert!(!ist_aufgabe("https://dav.example.org/erika/tasks/x.ics"));
    }

    #[test]
    fn koerper_leert_datum_mit_null() {
        let k = koerper(&Felder { titel: "x", notizen: None, faellig: None, erledigt: true });
        assert!(k["due"].is_null());
        assert_eq!(k["status"], "completed");
    }
}
