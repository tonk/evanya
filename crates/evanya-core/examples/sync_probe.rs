//! Run mail sync for one account against a profile copy and log what the
//! sync engine does, to diagnose mail that never arrives. Point it at a COPY
//! of the data directory: the sync writes to it.
//!
//! cargo run -p evanya-core --example sync_probe -- <data-copy> <cache-dir> <account-id> <seconds>

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
    let account: i64 = args.next().ok_or("pass the account id")?.parse()?;
    let seconds: u64 = args.next().unwrap_or_else(|| "120".into()).parse()?;

    let core = Core::start_mail_ui(Paths::new(data.into(), cache.into())).await?;
    // The desktop app signals this once its window shows; account workers
    // start after it.
    core.notify_ui_ready();
    tokio::time::sleep(Duration::from_secs(3)).await;
    if let Err(error) = core.sync_now(Some(account)).await {
        eprintln!("sync_now: {error}");
    }
    tokio::time::sleep(Duration::from_secs(seconds)).await;
    for status in core.get_sync_status().await? {
        println!("{status:?}");
    }
    Ok(())
}
