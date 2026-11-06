//! Pictures the user chose for their accounts. Each is kept as
//! `account-avatars/<account id>` in the data directory and shown instead of
//! the provider's picture or the account's initials.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The largest picture accepted, before it is scaled down for display.
pub(crate) const MAX_BYTES: u64 = 8 * 1024 * 1024;
/// Marks a picture source as a file chosen here rather than a provider URL.
const FILE_SOURCE: &str = "evanya-file:";

static DIRECTORY: OnceLock<PathBuf> = OnceLock::new();

/// Where chosen pictures live: in the data directory, as user data.
pub(crate) fn set_directory(data_dir: &Path) {
    let _ = DIRECTORY.set(data_dir.join("account-avatars"));
}

fn path(account_id: i64) -> Option<PathBuf> {
    Some(DIRECTORY.get()?.join(account_id.to_string()))
}

/// The picture to show for an account: one chosen here, otherwise the
/// provider's. A chosen picture's source names its modification time, so
/// choosing another one loads it anew.
pub(crate) fn source(account_id: i64, provider_url: Option<&str>) -> Option<String> {
    let chosen = path(account_id).and_then(|path| {
        let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
        let stamp = modified
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis();
        Some(format!("{FILE_SOURCE}{stamp}:{}", path.display()))
    });
    chosen.or_else(|| provider_url.map(str::to_owned))
}

/// The file a source names, when it is a chosen picture.
pub(crate) fn file_of(source: &str) -> Option<PathBuf> {
    let rest = source.strip_prefix(FILE_SOURCE)?;
    let (_, path) = rest.split_once(':')?;
    Some(PathBuf::from(path))
}

/// Whether the user chose a picture for this account.
pub(crate) fn has_chosen(account_id: i64) -> bool {
    path(account_id).is_some_and(|path| path.is_file())
}

/// Keep `bytes`, a picture, for this account.
pub(crate) fn store(account_id: i64, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err("the picture is larger than 8 MB".into());
    }
    image::load_from_memory(bytes).map_err(|error| format!("not a picture: {error}"))?;
    let path = path(account_id).ok_or("no data directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(&path, bytes).map_err(|error| error.to_string())
}

/// Forget the picture chosen for this account, if any.
pub(crate) fn remove(account_id: i64) {
    if let Some(path) = path(account_id) {
        let _ = std::fs::remove_file(path);
    }
}

/// Show the account's new picture, or its fallback, everywhere.
fn reload(
    app: &crate::AppWindow,
    state: &std::rc::Rc<std::cell::RefCell<crate::InboxState>>,
    runtime: &tokio::runtime::Runtime,
    account_id: i64,
) {
    {
        let mut state = state.borrow_mut();
        state.profile_avatar_images.remove(&account_id);
        state.profile_avatar_missing.remove(&account_id);
        state.profile_avatar_pending.remove(&account_id);
    }
    crate::refresh_connected_accounts(app, state);
    crate::schedule_profile_avatar_fetches(app, state, runtime);
}

/// Wire choosing and removing account pictures in Settings.
pub(crate) fn register(
    app: &crate::AppWindow,
    state: &std::rc::Rc<std::cell::RefCell<crate::InboxState>>,
    runtime: &std::rc::Rc<tokio::runtime::Runtime>,
) {
    use slint::ComponentHandle;
    use std::rc::Rc;

    let app_weak = app.as_weak();
    let state_for_remove = Rc::clone(state);
    let runtime_for_remove = Rc::clone(runtime);
    app.on_remove_account_avatar(move |account_id| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        remove(i64::from(account_id));
        reload(
            &app,
            &state_for_remove,
            &runtime_for_remove,
            i64::from(account_id),
        );
    });

    #[cfg(desktop)]
    {
        use crate::ui_message::{AppWindowMessages, UiMessage, translated};
        let app_weak = app.as_weak();
        let state = Rc::clone(state);
        let runtime = Rc::clone(runtime);
        let picking = Rc::new(std::cell::Cell::new(false));
        app.on_choose_account_avatar(move |account_id| {
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            if picking.replace(true) {
                return;
            }
            let title = translated(&app, &UiMessage::plain("Choose a picture for this account"))
                .to_string();
            // The portal picker needs the runtime: rfd's blocking dialog
            // panics on the UI thread, which has no Tokio context.
            let request: crate::documents::DocumentFuture<Option<Vec<u8>>> = Box::pin(async move {
                let Some(file) = rfd::AsyncFileDialog::new()
                    .set_title(title)
                    .add_filter("Pictures", &["png", "jpg", "jpeg", "gif", "webp", "ico"])
                    .pick_file()
                    .await
                else {
                    return Ok(None);
                };
                let size = std::fs::metadata(file.path())
                    .map_err(|error| error.to_string())?
                    .len();
                if size > MAX_BYTES {
                    return Err("the picture is larger than 8 MB".into());
                }
                Ok(Some(file.read().await))
            });
            let app_weak = app.as_weak();
            let state = Rc::clone(&state);
            let runtime_for_reload = Rc::clone(&runtime);
            let picking = Rc::clone(&picking);
            crate::documents::run_request(&runtime, request, move |picked| {
                picking.set(false);
                let Some(app) = app_weak.upgrade() else {
                    return;
                };
                let stored = match picked {
                    Ok(None) => return,
                    Ok(Some(bytes)) => store(i64::from(account_id), &bytes),
                    Err(error) => Err(error),
                };
                match stored {
                    Ok(()) => reload(&app, &state, &runtime_for_reload, i64::from(account_id)),
                    Err(error) => app.set_sync_status(UiMessage::detail(
                        "Could not use this picture: {}",
                        error,
                    )),
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chosen_picture_names_its_file_and_wins_over_the_provider() {
        assert_eq!(file_of("https://lh3.googleusercontent.com/a/x"), None);
        assert_eq!(
            file_of("evanya-file:123:/data/account-avatars/7"),
            Some(PathBuf::from("/data/account-avatars/7"))
        );
        // Without a chosen picture, the provider's is shown.
        assert_eq!(
            source(i64::MAX, Some("https://lh3.googleusercontent.com/a/x")).as_deref(),
            Some("https://lh3.googleusercontent.com/a/x")
        );
    }
}
