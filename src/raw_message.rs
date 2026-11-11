//! The source view's "full message": the whole message as it is stored,
//! headers and all, fetched when the view asks for it. Sync keeps only the
//! readable parts of mail from a server, so the whole message comes from the
//! file kept here when there is one, and otherwise from the server.

use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use slint::ComponentHandle;

use crate::{AppWindow, EmailReader, InboxState};

/// A message with attachments is mostly base64, and a text view cannot take a
/// large one whole, so no more than this many bytes are shown.
const MAX_SHOWN_BYTES: usize = 1024 * 1024;

/// The message as the view shows it, and whether the end was left out.
///
/// The bytes are shown as text, with 8-bit bytes in a header replaced rather
/// than failing, and line ends as `\n`. A message that is too long is cut at
/// the end of a line, so a base64 line or a character is never split.
pub(crate) fn shown_text(raw: &[u8]) -> (String, bool) {
    let shortened = raw.len() > MAX_SHOWN_BYTES;
    let mut end = raw.len().min(MAX_SHOWN_BYTES);
    if shortened && let Some(newline) = raw[..end].iter().rposition(|byte| *byte == b'\n') {
        end = newline + 1;
    }
    let text = String::from_utf8_lossy(&raw[..end]).replace("\r\n", "\n");
    (text, shortened)
}

/// The stored message of what the reader shows: the message that is open in a
/// conversation, and otherwise the message of the selected row.
pub(crate) fn selected_message_id(state: &InboxState) -> Option<i64> {
    let selected_id = state.selected_id?;
    let message = state
        .conversation_messages
        .get(state.conversation_selected_index)
        .filter(|_| state.conversation_owner_id == Some(selected_id))
        .or_else(|| {
            state
                .messages
                .iter()
                .find(|message| message.id == selected_id)
        })?;
    Some(message.message_id.unwrap_or_else(|| i64::from(message.id)))
}

pub(crate) fn register(
    app: &AppWindow,
    state: &Rc<RefCell<InboxState>>,
    runtime: &Rc<tokio::runtime::Runtime>,
) {
    let weak = app.as_weak();
    let state = Rc::clone(state);
    let runtime = Rc::clone(runtime);
    // Counts the requests, so an answer for a message that is no longer shown
    // never lands on the one that is.
    let generation = Arc::new(AtomicU64::new(0));
    app.global::<EmailReader>().on_load_raw(move || {
        let Some(app) = weak.upgrade() else {
            return;
        };
        let reader = app.global::<EmailReader>();
        reader.set_raw_text("".into());
        reader.set_raw_error("".into());
        reader.set_raw_shortened(false);
        let (core, message_id) = {
            let state = state.borrow();
            (state.core.clone(), selected_message_id(&state))
        };
        let (Some(core), Some(message_id)) = (core, message_id) else {
            reader.set_raw_loading(false);
            reader.set_raw_error("no message is selected".into());
            return;
        };
        let request = generation.fetch_add(1, Ordering::SeqCst) + 1;
        reader.set_raw_loading(true);
        let weak = app.as_weak();
        let generation = Arc::clone(&generation);
        runtime.spawn(async move {
            let result = core.raw_message(message_id).await;
            if let Err(error) = &result {
                tracing::warn!(message_id, %error, "the full message could not be loaded");
            }
            let _ = slint::invoke_from_event_loop(move || {
                let Some(app) = weak.upgrade() else {
                    return;
                };
                let reader = app.global::<EmailReader>();
                if generation.load(Ordering::SeqCst) != request || !reader.get_raw_mode() {
                    return;
                }
                reader.set_raw_loading(false);
                match result {
                    Ok(raw) => {
                        let (text, shortened) = shown_text(&raw);
                        reader.set_raw_text(text.into());
                        reader.set_raw_shortened(shortened);
                    }
                    Err(error) => reader.set_raw_error(error.into()),
                }
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_message_is_shown_whole_with_plain_line_ends() {
        let raw = b"From: ana@example.org\r\nSubject: Hi\r\n\r\nHello.\r\n";
        let (text, shortened) = shown_text(raw);
        assert_eq!(text, "From: ana@example.org\nSubject: Hi\n\nHello.\n");
        assert!(!shortened);
    }

    #[test]
    fn a_header_with_8_bit_bytes_is_shown_not_refused() {
        let (text, shortened) = shown_text(b"Subject: caf\xe9\r\n\r\nbody\r\n");
        assert!(text.starts_with("Subject: caf"));
        assert!(text.contains("body"));
        assert!(!shortened);
    }

    #[test]
    fn a_long_message_is_cut_at_the_end_of_a_line() {
        let line = format!("{}\r\n", "A".repeat(76));
        let raw = line.repeat(MAX_SHOWN_BYTES / line.len() + 50);
        let (text, shortened) = shown_text(raw.as_bytes());
        assert!(shortened);
        assert!(text.len() <= MAX_SHOWN_BYTES);
        // Whole lines only: nothing is split.
        assert!(text.ends_with('\n'));
        assert!(text.lines().all(|line| line.len() == 76));
    }

    #[test]
    fn a_cut_never_splits_a_character() {
        // Two-byte characters with no line end, so the cut falls inside one.
        let raw = "é".repeat(MAX_SHOWN_BYTES);
        let (text, shortened) = shown_text(raw.as_bytes());
        assert!(shortened);
        // A replacement character at most; never a panic or invalid text.
        assert!(text.chars().count() > 0);
    }

    #[test]
    fn an_empty_message_is_an_empty_text() {
        assert_eq!(shown_text(b""), (String::new(), false));
    }
}
