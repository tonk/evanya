//! Google contacts through the People API.
//!
//! Google's contacts are not offered over CardDAV to an OAuth client, so they
//! are read with `people.connections.list` and mapped to vCards. The cards
//! then go through the same storage as CardDAV contacts, in one address book
//! that is read only: edits would need write access and a mapping back.
//! Incremental syncs use the sync token the API hands out; an expired token
//! falls back to a full download.

use super::vcard::Card;
use crate::{
    db::{Db, repo},
    error::{CoreError, Result},
    events::{CoreEvent, EventBus},
    http_body,
};
use serde::Deserialize;
use std::collections::HashSet;

/// Marks a `carddav_config` row as a Google People connection.
pub const BASE_URL: &str = "https://people.googleapis.com/";
/// The one address book a Google connection owns.
pub const BOOK_URL: &str = "https://people.googleapis.com/v1/people/me/connections";
pub const BOOK_NAME: &str = "Google Contacts";

const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
const MAX_PAGES: usize = 500;
const PAGE_SIZE: &str = "1000";
const PERSON_FIELDS: &str = "names,nicknames,emailAddresses,phoneNumbers,addresses,organizations,birthdays,urls,biographies,metadata";

pub fn is_google(base_url: &str) -> bool {
    base_url == BASE_URL
}

fn http_client() -> Result<&'static reqwest::Client> {
    static HTTP: once_cell::sync::Lazy<std::result::Result<reqwest::Client, String>> =
        once_cell::sync::Lazy::new(|| {
            reqwest::Client::builder()
                .user_agent("flectar-mail-google-contacts/0.1")
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(60))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|error| format!("Google contacts HTTP client: {error}"))
        });
    HTTP.as_ref()
        .map_err(|message| CoreError::Network(message.clone()))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    #[serde(default)]
    connections: Vec<Person>,
    next_page_token: Option<String>,
    next_sync_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    #[serde(default)]
    pub resource_name: String,
    pub etag: Option<String>,
    #[serde(default)]
    metadata: PersonMetadata,
    #[serde(default)]
    names: Vec<Name>,
    #[serde(default)]
    nicknames: Vec<Text>,
    #[serde(default)]
    email_addresses: Vec<Typed>,
    #[serde(default)]
    phone_numbers: Vec<Typed>,
    #[serde(default)]
    addresses: Vec<Address>,
    #[serde(default)]
    organizations: Vec<Organization>,
    #[serde(default)]
    birthdays: Vec<Birthday>,
    #[serde(default)]
    urls: Vec<Typed>,
    #[serde(default)]
    biographies: Vec<Text>,
}

#[derive(Debug, Default, Deserialize)]
struct PersonMetadata {
    #[serde(default)]
    deleted: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Name {
    display_name: Option<String>,
    family_name: Option<String>,
    given_name: Option<String>,
    middle_name: Option<String>,
    honorific_prefix: Option<String>,
    honorific_suffix: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Text {
    value: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Typed {
    value: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    metadata: FieldMetadata,
}

#[derive(Debug, Default, Deserialize)]
struct FieldMetadata {
    #[serde(default)]
    primary: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Address {
    street_address: Option<String>,
    extended_address: Option<String>,
    city: Option<String>,
    region: Option<String>,
    postal_code: Option<String>,
    country: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Organization {
    name: Option<String>,
    title: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Birthday {
    date: Option<Date>,
}

#[derive(Debug, Default, Deserialize)]
struct Date {
    year: Option<i32>,
    month: Option<u32>,
    day: Option<u32>,
}

/// What one run learned from Google.
struct Changes {
    people: Vec<Person>,
    sync_token: Option<String>,
}

async fn fetch(token: &str, sync_token: Option<&str>) -> Result<Changes> {
    let client = http_client()?;
    let mut people = Vec::new();
    let mut page_token: Option<String> = None;
    let mut seen_tokens = HashSet::new();
    for _ in 0..MAX_PAGES {
        let mut url = url::Url::parse("https://people.googleapis.com/v1/people/me/connections")
            .map_err(|error| CoreError::Other(format!("invalid People API URL: {error}")))?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("personFields", PERSON_FIELDS);
            query.append_pair("pageSize", PAGE_SIZE);
            query.append_pair("requestSyncToken", "true");
            if let Some(sync) = sync_token {
                query.append_pair("syncToken", sync);
            }
            if let Some(page) = page_token.as_deref() {
                query.append_pair("pageToken", page);
            }
        }
        let response = client
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| CoreError::Network(format!("Google contacts: {error}")))?;
        let status = response.status();
        let body = http_body::text(response, MAX_BODY_BYTES, "Google contacts response").await?;
        if status == reqwest::StatusCode::UNAUTHORIZED
            || (status == reqwest::StatusCode::FORBIDDEN
                && body.contains("insufficient")
                && body.contains("scope"))
        {
            tracing::warn!(%status, body = %body.chars().take(500).collect::<String>(), "Google contacts need a new sign-in");
            return Err(CoreError::NeedsReauth);
        }
        if !status.is_success() {
            if status == reqwest::StatusCode::BAD_REQUEST && body.contains("EXPIRED_SYNC_TOKEN") {
                return Err(CoreError::NotFound(
                    "expired Google contacts sync token".into(),
                ));
            }
            let detail = http_body::single_line_excerpt(&body, 300);
            return Err(super::err(format!(
                "Google contacts returned {}: {detail}",
                status.as_u16()
            )));
        }
        let page: Page = serde_json::from_str(&body)
            .map_err(|error| super::err(format!("Google contacts response: {error}")))?;
        people.extend(page.connections);
        match page.next_page_token.filter(|token| !token.is_empty()) {
            Some(next) => {
                if !seen_tokens.insert(next.clone()) {
                    return Err(super::err("Google contacts repeated a page token"));
                }
                page_token = Some(next);
            }
            None => {
                return Ok(Changes {
                    people,
                    sync_token: page.next_sync_token,
                });
            }
        }
    }
    Err(super::err("Google contacts exceeded the page limit"))
}

/// Download Google's changes and store them in the Google Contacts address
/// book of `account_id`. Returns whether anything changed.
pub async fn sync_account(
    db: &Db,
    bus: &EventBus,
    account_id: i64,
    access_token: &str,
) -> Result<bool> {
    let book_id = db
        .write(move |conn| {
            let id = repo::carddav::upsert_addressbook(
                conn,
                account_id,
                BOOK_URL,
                Some(BOOK_NAME),
                true,
            )?;
            repo::carddav::ensure_default(conn, account_id, id)?;
            Ok(id)
        })
        .await?;
    let (_, stored_token) = db
        .read(move |conn| repo::carddav::sync_state(conn, book_id))
        .await?;
    let (changes, full) = match fetch(access_token, stored_token.as_deref()).await {
        Ok(changes) => (changes, stored_token.is_none()),
        Err(CoreError::NotFound(_)) if stored_token.is_some() => {
            (fetch(access_token, None).await?, true)
        }
        Err(error) => return Err(error),
    };
    let Changes { people, sync_token } = changes;
    let changed = !people.is_empty() || full;
    db.write(move |conn| {
        let tx = conn.transaction()?;
        let mut seen = HashSet::new();
        for person in &people {
            if person.resource_name.is_empty() {
                continue;
            }
            if person.metadata.deleted {
                repo::carddav::remove_remote(&tx, book_id, &person.resource_name)?;
                continue;
            }
            seen.insert(person.resource_name.clone());
            match Card::parse(person_to_vcard(person)) {
                Ok(card) => repo::carddav::apply_remote(
                    &tx,
                    book_id,
                    &person.resource_name,
                    person.etag.as_deref(),
                    &card,
                )?,
                Err(error) => repo::carddav::record_resource_error(
                    &tx,
                    book_id,
                    &person.resource_name,
                    person.etag.as_deref(),
                    &error.to_string(),
                )?,
            }
        }
        if full {
            // A full download lists every contact, so anything else is gone.
            for (href, _) in repo::carddav::object_etags(&tx, book_id)? {
                if !seen.contains(&href) {
                    repo::carddav::remove_remote(&tx, book_id, &href)?;
                }
            }
        }
        repo::carddav::set_sync_state(
            &tx,
            book_id,
            None,
            sync_token.as_deref(),
            crate::models::now_ms(),
        )?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    if changed {
        bus.emit(CoreEvent::ContactsUpdated { account_id });
    }
    Ok(changed)
}

fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            other => out.push(other),
        }
    }
    out
}

fn clean(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

fn type_param(kind: &Option<String>, allowed: &[&str]) -> String {
    let kind = kind.as_deref().unwrap_or_default().to_ascii_lowercase();
    match allowed.contains(&kind.as_str()) {
        true => format!(";TYPE={}", kind.to_ascii_uppercase()),
        false => String::new(),
    }
}

/// Map a People API person to a vCard 3.0 document.
pub fn person_to_vcard(person: &Person) -> String {
    let mut lines = vec!["BEGIN:VCARD".to_owned(), "VERSION:3.0".to_owned()];
    lines.push(format!("UID:google:{}", escape(&person.resource_name)));
    let name = person.names.first();
    let mut display = name
        .and_then(|name| clean(&name.display_name))
        .map(str::to_owned)
        .unwrap_or_default();
    if display.is_empty() {
        display = person
            .organizations
            .iter()
            .find_map(|org| clean(&org.name))
            .or_else(|| person.email_addresses.iter().find_map(|e| clean(&e.value)))
            .unwrap_or("Unnamed contact")
            .to_owned();
    }
    lines.push(format!("FN:{}", escape(&display)));
    if let Some(name) = name {
        let part = |value: &Option<String>| escape(clean(value).unwrap_or_default());
        lines.push(format!(
            "N:{};{};{};{};{}",
            part(&name.family_name),
            part(&name.given_name),
            part(&name.middle_name),
            part(&name.honorific_prefix),
            part(&name.honorific_suffix)
        ));
    }
    if let Some(nick) = person.nicknames.iter().find_map(|n| clean(&n.value)) {
        lines.push(format!("NICKNAME:{}", escape(nick)));
    }
    // The primary value first, so it is the preferred one.
    fn ordered(values: &[Typed]) -> Vec<&Typed> {
        let mut values: Vec<&Typed> = values
            .iter()
            .filter(|v| clean(&v.value).is_some())
            .collect();
        values.sort_by_key(|value| !value.metadata.primary);
        values
    }
    for email in ordered(&person.email_addresses) {
        lines.push(format!(
            "EMAIL{}:{}",
            type_param(&email.kind, &["home", "work", "other"]),
            escape(clean(&email.value).unwrap_or_default())
        ));
    }
    for phone in ordered(&person.phone_numbers) {
        let kind = match phone.kind.as_deref().unwrap_or_default() {
            "mobile" => ";TYPE=CELL".to_owned(),
            other => type_param(&Some(other.to_owned()), &["home", "work", "fax"]),
        };
        lines.push(format!(
            "TEL{kind}:{}",
            escape(clean(&phone.value).unwrap_or_default())
        ));
    }
    for address in &person.addresses {
        let part = |value: &Option<String>| escape(clean(value).unwrap_or_default());
        let fields = [
            part(&address.extended_address),
            part(&address.street_address),
            part(&address.city),
            part(&address.region),
            part(&address.postal_code),
            part(&address.country),
        ];
        if fields.iter().all(String::is_empty) {
            continue;
        }
        lines.push(format!(
            "ADR{}:;{}",
            type_param(&address.kind, &["home", "work"]),
            fields.join(";")
        ));
    }
    if let Some(org) = person
        .organizations
        .iter()
        .find(|org| clean(&org.name).is_some())
    {
        lines.push(format!(
            "ORG:{}",
            escape(clean(&org.name).unwrap_or_default())
        ));
    }
    if let Some(title) = person
        .organizations
        .iter()
        .find_map(|org| clean(&org.title))
    {
        lines.push(format!("TITLE:{}", escape(title)));
    }
    for url in person.urls.iter().filter_map(|url| clean(&url.value)) {
        lines.push(format!("URL:{}", escape(url)));
    }
    if let Some(date) = person.birthdays.iter().find_map(|b| b.date.as_ref())
        && let (Some(month), Some(day)) = (date.month, date.day)
    {
        match date.year {
            Some(year) => lines.push(format!("BDAY:{year:04}-{month:02}-{day:02}")),
            // vCard 3.0 has no year-less date; the 4.0 form is widely read.
            None => lines.push(format!("BDAY:--{month:02}{day:02}")),
        }
    }
    if let Some(note) = person.biographies.iter().find_map(|b| clean(&b.value)) {
        lines.push(format!("NOTE:{}", escape(note)));
    }
    lines.push("END:VCARD".to_owned());
    lines.join("\r\n") + "\r\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(json: &str) -> Person {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_person_becomes_a_parseable_card() {
        let person = person(
            r#"{"resourceName":"people/c1","etag":"e1",
                "names":[{"displayName":"Ada Lovelace","familyName":"Lovelace","givenName":"Ada"}],
                "emailAddresses":[{"value":"old@example.com","type":"home"},
                                  {"value":"ada@example.com","type":"work","metadata":{"primary":true}}],
                "phoneNumbers":[{"value":"+31 6 1234","type":"mobile"}],
                "organizations":[{"name":"Analytical; Engines, Ltd","title":"Countess"}],
                "birthdays":[{"date":{"month":12,"day":10}}],
                "biographies":[{"value":"line one\nline two"}]}"#,
        );
        let card = Card::parse(person_to_vcard(&person)).expect("valid vCard");
        let record = card.record();
        assert_eq!(record.name, "Ada Lovelace");
        assert_eq!(record.email, "ada@example.com");
        assert_eq!(record.company, "Analytical; Engines, Ltd");
        assert_eq!(record.job_title, "Countess");
        assert!(record.notes.contains("line two"));
    }

    #[test]
    fn a_person_without_a_name_uses_the_email_address() {
        let person =
            person(r#"{"resourceName":"people/c2","emailAddresses":[{"value":"x@example.com"}]}"#);
        let card = Card::parse(person_to_vcard(&person)).expect("valid vCard");
        assert_eq!(card.record().name, "x@example.com");
    }

    #[test]
    fn deleted_people_are_flagged() {
        let person = person(r#"{"resourceName":"people/c3","metadata":{"deleted":true}}"#);
        assert!(person.metadata.deleted);
    }
}
