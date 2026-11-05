//! Evanya's interface, generated from the `.slint` files under `ui/`. The
//! application re-exports everything here, so it uses these types as its own.

slint::include_modules!();

/// The tray icon, compiled apart from the main window (see build.rs).
pub mod tray_ui {
    include!(concat!(env!("OUT_DIR"), "/tray.rs"));
}
