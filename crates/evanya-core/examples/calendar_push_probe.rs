//! Push a test event to each given calendar from a profile COPY, report what
//! the provider answers, then delete the test events again. Diagnoses events
//! that never reach Google or Microsoft. This writes to the provider.
//!
//! cargo run -p evanya-core --example calendar_push_probe -- <data-copy> <cache-dir> <calendar-id>...

use evanya_core::{Core, config::Paths, models::CreateEventArgs};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "evanya_core=warn".into()),
        )
        .init();
    let mut args = std::env::args().skip(1);
    let data = args.next().ok_or("pass the data directory copy")?;
    let cache = args.next().ok_or("pass a cache directory")?;
    let calendar_ids: Vec<i64> = args.map(|id| id.parse()).collect::<Result<_, _>>()?;
    let core = Core::start_mail_ui(Paths::new(data.into(), cache.into())).await?;
    core.notify_ui_ready();
    let calendars = core.list_calendars(None).await?;
    let start = chrono::Utc::now().timestamp_millis() + 7 * 86_400_000;
    let mut created = Vec::new();
    for id in calendar_ids {
        let calendar = calendars
            .iter()
            .find(|calendar| calendar.id == id)
            .ok_or("no such calendar")?;
        let event = core
            .create_event(CreateEventArgs {
                account_id: calendar.account_id,
                calendar_id: Some(id),
                summary: "Evanya test – delete me".into(),
                description: None,
                location: None,
                join_url: None,
                starts_at: start,
                ends_at: start + 1_800_000,
                all_day: false,
                attendees: Vec::new(),
                rrule: None,
            })
            .await?;
        println!(
            "created event {} in calendar {id} ({:?})",
            event.id, calendar.display_name
        );
        created.push(event.id);
    }
    core.calendar_sync_now(None).await;
    tokio::time::sleep(Duration::from_secs(45)).await;
    println!("--- pushed; now deleting the test events");
    for id in &created {
        if let Err(error) = core.delete_event(*id, false).await {
            println!("delete {id}: {error}");
        }
    }
    core.calendar_sync_now(None).await;
    tokio::time::sleep(Duration::from_secs(45)).await;
    println!("--- done");
    Ok(())
}
