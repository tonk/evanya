//! The text cursor that the application draws itself, in the message editor
//! and in caret browsing: a thin line or a block, blinking or steady.

use evanya_core::models::Settings;
use slint::ComponentHandle;

use crate::{AppWindow, CursorStyle};

pub(crate) fn apply(app: &AppWindow, settings: &Settings) {
    let style = app.global::<CursorStyle>();
    style.set_block(settings.cursor_block);
    style.set_blink(settings.cursor_blink);
}
