//! The invitation bar of the reader: a message that carries a calendar
//! invitation offers Accept, Maybe and Decline, and mails the answer to the
//! organizer when the reader asks for that.

use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use slint::ComponentHandle;

use crate::{
    AppWindow, EmailReader, InboxState,
    raw_message::selected_message_id,
    ui_message::{AppWindowMessages, UiMessage},
};

fn clear(reader: &EmailReader<'_>) {
    reader.set_invite_event_id(0);
    reader.set_invite_title("".into());
    reader.set_invite_organizer("".into());
    reader.set_invite_response("".into());
    reader.set_invite_can_respond(false);
    reader.set_invite_busy(false);
}

pub(crate) fn register(
    app: &AppWindow,
    state: &Rc<RefCell<InboxState>>,
    runtime: &Rc<tokio::runtime::Runtime>,
) {
    // Counts the lookups, so the answer for a message that is no longer shown
    // never lands on the one that is.
    let generation = Arc::new(AtomicU64::new(0));

    let weak = app.as_weak();
    let load_state = Rc::clone(state);
    let load_runtime = Rc::clone(runtime);
    let load_generation = Arc::clone(&generation);
    app.global::<EmailReader>().on_load_invite(move || {
        let Some(app) = weak.upgrade() else {
            return;
        };
        clear(&app.global::<EmailReader>());
        let (core, message_id) = {
            let state = load_state.borrow();
            (state.core.clone(), selected_message_id(&state))
        };
        let (Some(core), Some(message_id)) = (core, message_id) else {
            return;
        };
        let request = load_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let weak = app.as_weak();
        let generation = Arc::clone(&load_generation);
        load_runtime.spawn(async move {
            let result = core.invitation_of_message(message_id).await;
            if let Err(error) = &result {
                tracing::warn!(message_id, %error, "the invitation of the message could not be loaded");
            }
            let _ = slint::invoke_from_event_loop(move || {
                let Some(app) = weak.upgrade() else {
                    return;
                };
                if generation.load(Ordering::SeqCst) != request {
                    return;
                }
                let Ok(Some(invitation)) = result else {
                    return;
                };
                let reader = app.global::<EmailReader>();
                reader.set_invite_event_id(i32::try_from(invitation.event_id).unwrap_or(0));
                reader.set_invite_title(invitation.title.into());
                reader.set_invite_organizer(invitation.organizer.into());
                reader.set_invite_response(invitation.response.into());
                reader.set_invite_can_respond(invitation.can_respond);
            });
        });
    });

    let weak = app.as_weak();
    let respond_state = Rc::clone(state);
    let respond_runtime = Rc::clone(runtime);
    app.global::<EmailReader>()
        .on_respond_invite(move |response, send_reply| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let reader = app.global::<EmailReader>();
            let event_id = i64::from(reader.get_invite_event_id());
            let Some(core) = respond_state.borrow().core.clone() else {
                app.set_sync_status(UiMessage::plain("Calendar storage is unavailable."));
                return;
            };
            if event_id == 0 || reader.get_invite_busy() {
                return;
            }
            reader.set_invite_busy(true);
            let weak = app.as_weak();
            respond_runtime.spawn(async move {
                let result = core
                    .rsvp_event(event_id, response.as_str(), send_reply)
                    .await;
                if let Err(error) = &result {
                    tracing::warn!(event_id, %error, "the answer to the invitation failed");
                }
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(app) = weak.upgrade() else {
                        return;
                    };
                    let reader = app.global::<EmailReader>();
                    // Another message may be shown by now.
                    if i64::from(reader.get_invite_event_id()) == event_id {
                        reader.set_invite_busy(false);
                    }
                    match result {
                        Ok(()) => {
                            if i64::from(reader.get_invite_event_id()) == event_id {
                                reader.set_invite_response(
                                    match response.as_str() {
                                        "accepted" => "ACCEPTED",
                                        "tentative" => "TENTATIVE",
                                        _ => "DECLINED",
                                    }
                                    .into(),
                                );
                            }
                            if send_reply {
                                app.set_sync_status(UiMessage::plain("Response sent."));
                            } else {
                                app.set_sync_status(UiMessage::plain(
                                    "Response noted in the calendar.",
                                ));
                            }
                        }
                        Err(error) => app.set_sync_status(UiMessage::detail(
                            "Could not send your response: {}",
                            error,
                        )),
                    }
                });
            });
        });
}
