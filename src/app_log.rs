//! A log file in the data directory, so a failure that the window does not
//! show can still be found afterwards. Until this existed the `tracing`
//! calls throughout the application went nowhere.
//!
//! `EVANYA_LOG` takes a `tracing` filter such as `debug` or
//! `evanya_core=trace`; the default records warnings and the application's own
//! information messages.

use std::fs::{self, File, OpenOptions};
use std::path::Path;
use std::sync::Mutex;

use tracing_subscriber::EnvFilter;

/// A log file past this size is set aside as `evanya.log.old` at start.
const MAX_BYTES: u64 = 5 * 1024 * 1024;

const DEFAULT_FILTER: &str = "warn,evanya=info,evanya_core=info";

/// Start logging to `evanya.log` in `data_dir`. Logging is a convenience: when
/// the file cannot be opened the application carries on without it.
pub(crate) fn init(data_dir: &Path) {
    let path = data_dir.join("evanya.log");
    if fs::metadata(&path).is_ok_and(|meta| meta.len() > MAX_BYTES) {
        let _ = fs::rename(&path, data_dir.join("evanya.log.old"));
    }
    let file: File = match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("could not open the log file {}: {error}", path.display());
            return;
        }
    };
    let filter =
        EnvFilter::try_from_env("EVANYA_LOG").unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    // Another subscriber may already be installed, such as in a test.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(Mutex::new(file))
        .with_ansi(false)
        .try_init();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Evanya started");
}
