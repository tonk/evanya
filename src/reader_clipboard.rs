//! Optional desktop formats; the Slint text clipboard remains the fallback.
#[cfg(desktop)]
thread_local! {
    static CLIPBOARD: std::cell::RefCell<Option<arboard::Clipboard>> = const { std::cell::RefCell::new(None) };
}

pub fn set_html(html: &str, text: &str) -> bool {
    #[cfg(desktop)]
    return CLIPBOARD.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = arboard::Clipboard::new().ok();
        }
        slot.as_mut()
            .is_some_and(|clipboard| clipboard.set_html(html, Some(text)).is_ok())
    });
    #[cfg(mobile)]
    {
        let _ = (html, text);
        false
    }
}

pub fn set_primary(text: &str) {
    #[cfg(linux_desktop)]
    if !text.is_empty() {
        use arboard::SetExtLinux;
        CLIPBOARD.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_none() {
                *slot = arboard::Clipboard::new().ok();
            }
            if let Some(clipboard) = slot.as_mut() {
                let _ = clipboard
                    .set()
                    .clipboard(arboard::LinuxClipboardKind::Primary)
                    .text(text);
            }
        });
    }
    #[cfg(not(linux_desktop))]
    let _ = text;
}

/// The image on the clipboard as straight-alpha RGBA (width, height, pixels),
/// unless it also holds text, which a paste then inserts instead: office
/// programs offer a picture of copied cells beside their text.
pub fn image() -> Option<(u32, u32, Vec<u8>)> {
    #[cfg(desktop)]
    return CLIPBOARD.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = arboard::Clipboard::new().ok();
        }
        let clipboard = slot.as_mut()?;
        if clipboard
            .get_text()
            .is_ok_and(|text| !text.trim().is_empty())
        {
            return None;
        }
        let image = clipboard.get_image().ok()?;
        Some((
            u32::try_from(image.width).ok()?,
            u32::try_from(image.height).ok()?,
            image.bytes.into_owned(),
        ))
    });
    #[cfg(mobile)]
    None
}
