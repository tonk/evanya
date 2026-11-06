//! Try to create an event in every calendar of a profile COPY, without
//! syncing, and report what happens. Diagnoses events that cannot be made.
//!
//! cargo run -p evanya-core --example calendar_probe -- <data-copy> <cache-dir>

use evanya_core::{Core, config::Paths, models::CreateEventArgs};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let data = args.next().ok_or("pass the data directory copy")?;
    let cache = args.next().ok_or("pass a cache directory")?;
    let core = Core::start_mail_ui(Paths::new(data.into(), cache.into())).await?;
    let start = chrono::Utc::now().timestamp_millis() + 86_400_000;
    let mut targets = vec![(0, None, "this device".to_owned())];
    for calendar in core.list_calendars(None).await? {
        println!(
            "calendar {} account {} {:?} read_only={} enabled={}",
            calendar.id,
            calendar.account_id,
            calendar.display_name,
            calendar.read_only,
            calendar.enabled
        );
        targets.push((
            calendar.account_id,
            Some(calendar.id),
            calendar.display_name.clone().unwrap_or_default(),
        ));
    }
    for (account_id, calendar_id, name) in targets {
        let result = core
            .create_event(CreateEventArgs {
                account_id,
                calendar_id,
                summary: format!("probe in {name}"),
                description: None,
                location: None,
                join_url: None,
                starts_at: start,
                ends_at: start + 3_600_000,
                all_day: false,
                attendees: Vec::new(),
            })
            .await;
        match result {
            Ok(event) => {
                let listed = core
                    .list_events(start - 1, start + 3_600_001)
                    .await?
                    .iter()
                    .any(|listed| listed.id == event.id);
                println!("  created in {name}: id {} listed={listed}", event.id);
            }
            Err(error) => println!("  FAILED in {name}: {error}"),
        }
    }
    Ok(())
}
