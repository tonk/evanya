//! Run calendar sync with logging on a profile COPY, to see why calendars
//! stop syncing. Remove pending actions and dirty rows from the copy first if
//! nothing may be written to the provider.
//!
//! cargo run -p evanya-core --example calendar_sync_probe -- <data-copy> <cache-dir> <seconds>

use evanya_core::{Core, config::Paths};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "evanya_core=info".into()),
        )
        .init();
    let mut args = std::env::args().skip(1);
    let data = args.next().ok_or("pass the data directory copy")?;
    let cache = args.next().ok_or("pass a cache directory")?;
    let seconds: u64 = args.next().unwrap_or_else(|| "60".into()).parse()?;
    let core = Core::start_mail_ui(Paths::new(data.into(), cache.into())).await?;
    core.notify_ui_ready();
    tokio::time::sleep(Duration::from_secs(3)).await;
    core.calendar_sync_now(None).await;
    tokio::time::sleep(Duration::from_secs(seconds)).await;
    for calendar in core.list_calendars(None).await? {
        println!(
            "calendar {} {:?} enabled={} last_synced_at={:?}",
            calendar.id, calendar.display_name, calendar.enabled, calendar.last_synced_at
        );
    }
    Ok(())
}
