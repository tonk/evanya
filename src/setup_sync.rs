//! Setup sync: keep the account setup the same on several computers through a
//! directory that a file sync tool such as Nextcloud, Syncthing or Dropbox
//! keeps in step. The directory holds one account backup, `evanya-setup.json`:
//! accounts, profiles, account colors and preferences, never credentials.
//!
//! Each run takes over what another computer changed since this one last
//! synced, then writes this computer's setup back when it differs. Accounts
//! are only ever added, as by importing a backup; preferences changed on
//! another computer replace the ones here.

use std::{cell::RefCell, path::Path, rc::Rc};

use evanya_core::models::Settings;
use slint::ComponentHandle;

use crate::{AppWindow, InboxState, data_controller};

const FILE_NAME: &str = "evanya-setup.json";
/// Look for changes from other computers this often while the app runs.
const INTERVAL: std::time::Duration = std::time::Duration::from_secs(10 * 60);
/// Wait this long after startup, so accounts and settings are loaded first.
const FIRST_RUN: std::time::Duration = std::time::Duration::from_secs(20);

/// What a run did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// No directory is chosen.
    Off,
    /// Nothing changed here or elsewhere.
    Unchanged,
    /// This computer's setup was written for the others.
    Sent,
    /// Another computer's setup was taken over, adding this many accounts.
    Received(usize),
}

/// A stable hash of what the setup is, not when it was written. FNV-1a, so it
/// is the same on every computer and Rust version.
fn content_hash(backup: &serde_json::Value) -> String {
    let content = serde_json::json!({
        "accounts": backup.get("accounts"),
        "preferences": backup.get("preferences"),
    });
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in content.to_string().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Write `bytes` to `path` whole: a sync tool must never pick up half a file.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let temporary = path.with_file_name(format!(".{FILE_NAME}.{}.tmp", std::process::id()));
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })
}

fn local_backup(state: &Rc<RefCell<InboxState>>, settings: &Settings) -> serde_json::Value {
    data_controller::account_backup_value(Some(settings), &state.borrow().account_configs)
}

/// Run setup sync once.
pub(crate) fn run(
    app: &AppWindow,
    state: &Rc<RefCell<InboxState>>,
    runtime: &tokio::runtime::Runtime,
) -> Result<Outcome, String> {
    let core = state
        .borrow()
        .core
        .clone()
        .ok_or_else(|| "local storage is not ready".to_owned())?;
    let settings = runtime.block_on(core.load_settings())?;
    let directory = settings.setup_sync_directory.trim().to_owned();
    if directory.is_empty() {
        return Ok(Outcome::Off);
    }
    let directory = Path::new(&directory);
    if !directory.is_dir() {
        return Err(format!("{} is not available", directory.display()));
    }
    let path = directory.join(FILE_NAME);
    let local_hash = content_hash(&local_backup(state, &settings));

    let shared = match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("could not read {}: {error}", path.display())),
    };
    let shared_hash = match &shared {
        Some(bytes) => {
            let value: serde_json::Value = serde_json::from_slice(bytes)
                .map_err(|error| format!("{} is not valid: {error}", path.display()))?;
            Some(content_hash(&value))
        }
        None => None,
    };

    let mut outcome = Outcome::Unchanged;
    if let (Some(bytes), Some(hash)) = (&shared, &shared_hash)
        && *hash != settings.setup_sync_hash
        && *hash != local_hash
    {
        // Another computer changed the setup since this one last synced.
        let restore = data_controller::restore_account_backup(app, bytes, &core, state, runtime)?;
        outcome = Outcome::Received(restore.imported);
    }

    // This computer's setup, now including whatever it took over.
    let settings = runtime.block_on(core.load_settings())?;
    let backup = local_backup(state, &settings);
    let hash = content_hash(&backup);
    if shared_hash.as_ref() != Some(&hash) {
        let bytes = serde_json::to_vec_pretty(&backup).map_err(|error| error.to_string())?;
        write_atomically(&path, &bytes)
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        if outcome == Outcome::Unchanged {
            outcome = Outcome::Sent;
        }
    }
    let now = chrono::Utc::now().timestamp_millis();
    runtime.block_on(core.update_settings(move |settings| {
        settings.setup_sync_hash = hash;
        settings.setup_sync_at = now;
    }))?;
    Ok(outcome)
}

/// Run setup sync and show how it went in Settings.
fn run_and_show(
    app: &AppWindow,
    state: &Rc<RefCell<InboxState>>,
    runtime: &tokio::runtime::Runtime,
) {
    if app.get_setup_sync_directory().is_empty() {
        return;
    }
    app.set_setup_sync_busy(true);
    let result = run(app, state, runtime);
    app.set_setup_sync_busy(false);
    match result {
        Ok(Outcome::Off) => {}
        Ok(outcome) => {
            app.set_setup_sync_error("".into());
            app.set_setup_sync_received(match outcome {
                Outcome::Received(accounts) => accounts as i32,
                _ => -1,
            });
            app.set_setup_sync_time(chrono::Local::now().format("%H:%M").to_string().into());
        }
        Err(error) => {
            tracing::warn!(%error, "setup sync failed");
            app.set_setup_sync_error(error.into());
        }
    }
}

/// Show the saved directory.
pub(crate) fn apply(app: &AppWindow, settings: &Settings) {
    app.set_setup_sync_directory(settings.setup_sync_directory.clone().into());
}

/// Wire Settings to setup sync and run it after startup and periodically.
pub(crate) fn register(
    app: &AppWindow,
    state: &Rc<RefCell<InboxState>>,
    runtime: &Rc<tokio::runtime::Runtime>,
) {
    let app_weak = app.as_weak();
    let state_for_now = Rc::clone(state);
    let runtime_for_now = Rc::clone(runtime);
    app.on_sync_setup_now(move || {
        if let Some(app) = app_weak.upgrade() {
            run_and_show(&app, &state_for_now, &runtime_for_now);
        }
    });

    let app_weak = app.as_weak();
    let state_for_stop = Rc::clone(state);
    let runtime_for_stop = Rc::clone(runtime);
    app.on_stop_setup_sync(move || {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let core = state_for_stop.borrow().core.clone();
        if let Some(core) = core {
            let _ = runtime_for_stop.block_on(core.update_settings(|settings| {
                settings.setup_sync_directory.clear();
                settings.setup_sync_hash.clear();
            }));
        }
        app.set_setup_sync_directory("".into());
        app.set_setup_sync_error("".into());
        app.set_setup_sync_time("".into());
    });

    #[cfg(desktop)]
    register_directory_choice(app, state, runtime);

    // The first run waits for startup; later runs repeat. The timer lives as
    // long as the window.
    let app_weak = app.as_weak();
    let state_for_timer = Rc::clone(state);
    let runtime_for_timer = Rc::clone(runtime);
    let timer = Rc::new(slint::Timer::default());
    let repeat = Rc::clone(&timer);
    slint::Timer::single_shot(FIRST_RUN, move || {
        if let Some(app) = app_weak.upgrade() {
            run_and_show(&app, &state_for_timer, &runtime_for_timer);
        }
        let app_weak = app_weak.clone();
        let state = Rc::clone(&state_for_timer);
        let runtime = Rc::clone(&runtime_for_timer);
        repeat.start(slint::TimerMode::Repeated, INTERVAL, move || {
            if let Some(app) = app_weak.upgrade() {
                run_and_show(&app, &state, &runtime);
            }
        });
    });
    std::mem::forget(timer);
}

#[cfg(desktop)]
fn register_directory_choice(
    app: &AppWindow,
    state: &Rc<RefCell<InboxState>>,
    runtime: &Rc<tokio::runtime::Runtime>,
) {
    use crate::ui_message::{UiMessage, translated};
    let app_weak = app.as_weak();
    let state = Rc::clone(state);
    let runtime = Rc::clone(runtime);
    let picking = Rc::new(std::cell::Cell::new(false));
    app.on_choose_setup_sync_directory(move || {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        if picking.replace(true) {
            return;
        }
        let title = translated(
            &app,
            &UiMessage::plain("Choose the directory to sync through"),
        )
        .to_string();
        let current = app.get_setup_sync_directory().to_string();
        // The portal picker needs the runtime: rfd's blocking dialog panics
        // on the UI thread, which has no Tokio context.
        let request: crate::documents::DocumentFuture<Option<std::path::PathBuf>> =
            Box::pin(async move {
                let dialog = rfd::AsyncFileDialog::new().set_title(title);
                let dialog = if current.is_empty() {
                    dialog
                } else {
                    dialog.set_directory(current)
                };
                Ok(dialog
                    .pick_folder()
                    .await
                    .map(|folder| folder.path().to_owned()))
            });
        let app_weak = app.as_weak();
        let state = Rc::clone(&state);
        let runtime_for_save = Rc::clone(&runtime);
        let picking = Rc::clone(&picking);
        crate::documents::run_request(&runtime, request, move |picked| {
            picking.set(false);
            let (Some(app), Ok(Some(directory))) = (app_weak.upgrade(), picked) else {
                return;
            };
            let directory = directory.to_string_lossy().into_owned();
            let core = state.borrow().core.clone();
            if let Some(core) = core {
                let saved = directory.clone();
                let _ = runtime_for_save.block_on(core.update_settings(move |settings| {
                    settings.setup_sync_directory = saved;
                    // A new directory starts fresh: take over what is there.
                    settings.setup_sync_hash.clear();
                }));
            }
            app.set_setup_sync_directory(directory.into());
            run_and_show(&app, &state, &runtime_for_save);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CoreMailSource, UiSender, UiWake, WarmStartCacheWriter, bounded_ui_channel};
    use evanya_core::accounts::credentials::DevelopmentFileCredentialStore;
    use slint::platform::{
        Platform, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    };
    use std::sync::Arc;

    struct Headless(Rc<MinimalSoftwareWindow>);

    impl Platform for Headless {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
            Ok(self.0.clone())
        }
    }

    /// One computer: its own database, sharing `shared` for setup sync.
    fn computer(
        app: &AppWindow,
        runtime: &tokio::runtime::Runtime,
        data: &Path,
        shared: &Path,
    ) -> Rc<RefCell<InboxState>> {
        let core = runtime
            .block_on(CoreMailSource::start(
                evanya_core::config::Paths::for_tests(data),
                Arc::new(DevelopmentFileCredentialStore::new(
                    data.join("credentials.json"),
                )),
                Arc::new(evanya_core::oauth::redirect::LoopbackRedirectBroker::default()),
            ))
            .unwrap();
        let shared = shared.to_string_lossy().into_owned();
        runtime
            .block_on(core.update_settings(move |settings| settings.setup_sync_directory = shared))
            .unwrap();
        let (favicons, _) = bounded_ui_channel();
        let (avatars, _) = bounded_ui_channel();
        let mut state = InboxState::empty(
            None,
            UiSender::new(favicons, UiWake::new(app.as_weak(), |_| {})),
            None,
            UiSender::new(avatars, UiWake::new(app.as_weak(), |_| {})),
            false,
            false,
            WarmStartCacheWriter::spawn(runtime, data.join("warm.json")),
        );
        state.core = Some(core);
        state.using_core = true;
        Rc::new(RefCell::new(state))
    }

    fn theme(state: &Rc<RefCell<InboxState>>, runtime: &tokio::runtime::Runtime) -> String {
        let core = state.borrow().core.clone().unwrap();
        runtime.block_on(core.load_settings()).unwrap().theme
    }

    fn set_theme(state: &Rc<RefCell<InboxState>>, runtime: &tokio::runtime::Runtime, theme: &str) {
        let core = state.borrow().core.clone().unwrap();
        let theme = theme.to_owned();
        runtime
            .block_on(core.update_settings(move |settings| settings.theme = theme))
            .unwrap();
    }

    #[test]
    fn two_computers_take_over_each_others_changes() {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(Headless(window))).unwrap();
        let app = AppWindow::new().unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (one, two, shared) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        let a = computer(&app, &runtime, one.path(), shared.path());
        let b = computer(&app, &runtime, two.path(), shared.path());

        set_theme(&a, &runtime, "dark");
        assert_eq!(run(&app, &a, &runtime), Ok(Outcome::Sent));
        assert!(shared.path().join(FILE_NAME).is_file());
        assert_eq!(run(&app, &b, &runtime), Ok(Outcome::Received(0)));
        assert_eq!(theme(&b, &runtime), "dark");

        // A change on the second computer comes back to the first.
        set_theme(&b, &runtime, "light");
        assert_eq!(run(&app, &b, &runtime), Ok(Outcome::Sent));
        assert_eq!(run(&app, &a, &runtime), Ok(Outcome::Received(0)));
        assert_eq!(theme(&a, &runtime), "light");

        // Once both agree, nothing moves.
        assert_eq!(run(&app, &a, &runtime), Ok(Outcome::Unchanged));
        assert_eq!(run(&app, &b, &runtime), Ok(Outcome::Unchanged));

        // A change here is not undone by the older shared setup.
        set_theme(&a, &runtime, "dark");
        assert_eq!(run(&app, &a, &runtime), Ok(Outcome::Sent));
        assert_eq!(theme(&a, &runtime), "dark");
    }

    #[test]
    fn the_hash_ignores_when_and_where_a_setup_was_written() {
        let one = serde_json::json!({
            "format": "evanya-account-backup", "createdAt": "2026-11-06T10:00:00Z",
            "accounts": [{"email": "a@example.com"}], "preferences": {"theme": "dark"},
        });
        let mut later = one.clone();
        later["createdAt"] = "2026-11-06T11:00:00Z".into();
        assert_eq!(content_hash(&one), content_hash(&later));
        let mut changed = one.clone();
        changed["preferences"]["theme"] = "light".into();
        assert_ne!(content_hash(&one), content_hash(&changed));
    }

    #[test]
    fn a_whole_file_replaces_the_old_one() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(FILE_NAME);
        std::fs::write(&path, b"old").unwrap();
        write_atomically(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        // No temporary file is left behind.
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
