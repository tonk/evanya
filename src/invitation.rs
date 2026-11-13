//! The invitation bar of the reader, after Thunderbird's: a message that
//! carries a calendar invitation says whether and how it was answered (here or
//! in another program) and in which calendar the event is, offers Accept,
//! Maybe and Decline with a choice of calendar for an event that is in none
//! yet (of any account), mails the answer to the organizer when the reader
//! asks for that, and lets a cancelled event be taken out of the calendar. An
//! attendee's answer to one of our invitations says who answered what.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::{
    AppWindow, EmailReader, InboxState,
    mail::{CoreMailSource, Invitation},
    raw_message::selected_message_id,
    ui_message::{AppWindowMessages, UiMessage},
};

type Lookup = Rc<dyn Fn(&AppWindow, bool)>;

thread_local! {
    // The calendars offered, in the order of `invite-calendars`.
    static CALENDAR_IDS: RefCell<Vec<i64>> = RefCell::default();
    // The calendar chosen last, offered first next time.
    static LAST_CHOICE: Cell<Option<i64>> = Cell::default();
    // Looks the invitation of the shown message up again.
    static LOOKUP: RefCell<Option<Lookup>> = RefCell::default();
}

/// Show the invitation of the shown message as it is stored now, without
/// clearing the bar first.
fn refresh(app: &AppWindow) {
    if let Some(lookup) = LOOKUP.with_borrow(Clone::clone) {
        lookup(app, false);
    }
}

fn clear(reader: &EmailReader<'_>) {
    reader.set_invite_event_id(0);
    reader.set_invite_title("".into());
    reader.set_invite_organizer("".into());
    reader.set_invite_response("".into());
    reader.set_invite_can_respond(false);
    reader.set_invite_busy(false);
    reader.set_invite_cancelled(false);
    reader.set_invite_calendar("".into());
    reader.set_invite_calendars(ModelRc::default());
    reader.set_invite_calendar_index(0);
    reader.set_invite_updated(false);
    reader.set_invite_answer_from("".into());
    reader.set_invite_answer("".into());
    CALENDAR_IDS.with_borrow_mut(Vec::clear);
}

/// Show an invitation. The calendar offered first is the one chosen last
/// time, else the default calendar of the account.
fn show(reader: &EmailReader<'_>, invitation: Invitation) {
    let last_choice = LAST_CHOICE.get();
    reader.set_invite_event_id(i32::try_from(invitation.event_id).unwrap_or(0));
    reader.set_invite_title(invitation.title.into());
    reader.set_invite_organizer(invitation.organizer.into());
    reader.set_invite_response(invitation.response.into());
    reader.set_invite_can_respond(invitation.can_respond);
    reader.set_invite_cancelled(invitation.cancelled);
    reader.set_invite_calendar(invitation.calendar_name.into());
    reader.set_invite_updated(invitation.updated);
    reader.set_invite_answer_from(invitation.answer_from.into());
    reader.set_invite_answer(invitation.answer.into());
    let chosen = last_choice
        .and_then(|id| {
            invitation
                .calendars
                .iter()
                .position(|(calendar, _)| *calendar == id)
        })
        .unwrap_or(0);
    reader.set_invite_calendars(ModelRc::new(VecModel::from(
        invitation
            .calendars
            .iter()
            .map(|(_, name)| SharedString::from(name.as_str()))
            .collect::<Vec<_>>(),
    )));
    reader.set_invite_calendar_index(i32::try_from(chosen).unwrap_or(0));
    CALENDAR_IDS.set(invitation.calendars.into_iter().map(|(id, _)| id).collect());
}

pub(crate) fn register(
    app: &AppWindow,
    state: &Rc<RefCell<InboxState>>,
    runtime: &Rc<tokio::runtime::Runtime>,
) {
    // Counts the lookups, so the answer for a message that is no longer shown
    // never lands on the one that is.
    let generation = Arc::new(AtomicU64::new(0));

    // Look the invitation of the shown message up and show it; `fresh` clears
    // the bar first, for another message.
    let lookup: Lookup = {
        let state = Rc::clone(state);
        let runtime = Rc::clone(runtime);
        let generation = Arc::clone(&generation);
        Rc::new(move |app: &AppWindow, fresh: bool| {
            if fresh {
                clear(&app.global::<EmailReader>());
            }
            let (core, message_id) = {
                let state = state.borrow();
                (state.core.clone(), selected_message_id(&state))
            };
            let (Some(core), Some(message_id)) = (core, message_id) else {
                clear(&app.global::<EmailReader>());
                return;
            };
            let request = generation.fetch_add(1, Ordering::SeqCst) + 1;
            let weak = app.as_weak();
            let generation = Arc::clone(&generation);
            let task = fetch(core, message_id);
            runtime.spawn(async move {
                let result = task.await;
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(app) = weak.upgrade() else {
                        return;
                    };
                    if generation.load(Ordering::SeqCst) != request {
                        return;
                    }
                    let reader = app.global::<EmailReader>();
                    match result {
                        Some(invitation) => show(&reader, invitation),
                        None => clear(&reader),
                    }
                });
            });
        })
    };

    LOOKUP.set(Some(Rc::clone(&lookup)));
    let weak = app.as_weak();
    app.global::<EmailReader>().on_load_invite(move || {
        if let Some(app) = weak.upgrade() {
            lookup(&app, true);
        }
    });

    let weak = app.as_weak();
    let respond_state = Rc::clone(state);
    let respond_runtime = Rc::clone(runtime);
    app.global::<EmailReader>()
        .on_respond_invite(move |response, send_reply, calendar_index| {
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
            // Only an event in no calendar yet is placed; one in a calendar
            // stays where it is.
            let calendar_id = (reader.get_invite_calendar().is_empty())
                .then(|| {
                    usize::try_from(calendar_index)
                        .ok()
                        .and_then(|index| CALENDAR_IDS.with_borrow(|ids| ids.get(index).copied()))
                })
                .flatten();
            if calendar_id.is_some() {
                LAST_CHOICE.set(calendar_id);
            }
            reader.set_invite_busy(true);
            let weak = app.as_weak();
            let task = async move {
                core.rsvp_event(event_id, response.as_str(), send_reply, calendar_id)
                    .await
            };
            respond_runtime.spawn(async move {
                let result = task.await;
                if let Err(error) = &result {
                    tracing::warn!(event_id, %error, "the answer to the invitation failed");
                }
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(app) = weak.upgrade() else {
                        return;
                    };
                    let reader = app.global::<EmailReader>();
                    let still_shown = i64::from(reader.get_invite_event_id()) == event_id;
                    if still_shown {
                        reader.set_invite_busy(false);
                    }
                    match result {
                        Ok(()) => {
                            // Show the answer and the calendar as stored.
                            if still_shown {
                                refresh(&app);
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

    let weak = app.as_weak();
    let remove_state = Rc::clone(state);
    let remove_runtime = Rc::clone(runtime);
    app.global::<EmailReader>().on_remove_invite(move || {
        let Some(app) = weak.upgrade() else {
            return;
        };
        let reader = app.global::<EmailReader>();
        let event_id = i64::from(reader.get_invite_event_id());
        let Some(core) = remove_state.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain("Calendar storage is unavailable."));
            return;
        };
        if event_id == 0 || reader.get_invite_busy() {
            return;
        }
        reader.set_invite_busy(true);
        let weak = app.as_weak();
        remove_runtime.spawn(async move {
            let result = core.delete_event(event_id).await;
            let _ = slint::invoke_from_event_loop(move || {
                let Some(app) = weak.upgrade() else {
                    return;
                };
                let reader = app.global::<EmailReader>();
                if i64::from(reader.get_invite_event_id()) == event_id {
                    reader.set_invite_busy(false);
                    refresh(&app);
                }
                match result {
                    Ok(()) => {
                        app.set_sync_status(UiMessage::plain("Event removed from the calendar."));
                    }
                    Err(error) => app.set_sync_status(UiMessage::detail(
                        "Could not remove the event: {}",
                        error,
                    )),
                }
            });
        });
    });
}

async fn fetch(core: CoreMailSource, message_id: i64) -> Option<Invitation> {
    match core.invitation_of_message(message_id).await {
        Ok(invitation) => invitation,
        Err(error) => {
            tracing::warn!(message_id, %error, "the invitation of the message could not be loaded");
            None
        }
    }
}
