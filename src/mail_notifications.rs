//! Desktop notifications for new mail, and the new-mail sound.
//!
//! Sync queues one notification per fresh thread in the core's outbox, with
//! sender and subject. This dispatcher delivers them as system notifications,
//! applying Settings → Notifications: off, important mail only, or all mail.
//! The sound follows the core's `MailNew` event, which only fires for fresh
//! mail from people.

use std::{
    cell::Cell,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use evanya_core::{Core, db::repo::notifications::RoutedTab, models::now_ms};
use slint::ComponentHandle;

use crate::{AppWindow, InboxState};

/// How often the outbox is checked.
const INTERVAL: Duration = Duration::from_secs(4);
/// Mail that arrived this long before the app started is not announced: the
/// catch-up after a restart should not flood the screen.
const STARTUP_GRACE_MS: i64 = 2 * 60 * 1000;
/// More new threads than this at once become one summary notification.
const MAX_SINGLE: usize = 3;

/// One notification to show: who it is from and what it is about.
struct Shown {
    sender: String,
    subject: String,
}

/// Deliver what is due. Returns the notifications to show.
async fn collect(core: &Core, enabled: bool, important_only: bool, started_ms: i64) -> Vec<Shown> {
    let Ok(due) = core.due_notifications(50).await else {
        return Vec::new();
    };
    let mut shown = Vec::new();
    for item in due {
        if !enabled {
            let _ = core
                .suppress_notification_delivery(item.id, "notifications are off")
                .await;
            continue;
        }
        if item.created_at < started_ms - STARTUP_GRACE_MS {
            let _ = core
                .suppress_notification_delivery(item.id, "arrived before the app started")
                .await;
            continue;
        }
        if important_only && let Some(thread_id) = item.thread_id {
            match core.notification_thread_tab(thread_id).await {
                // Classification still runs: look again shortly.
                Ok(Some(RoutedTab::Pending)) => {
                    let _ = core
                        .defer_notification_delivery(item.id, now_ms() + 10_000)
                        .await;
                    continue;
                }
                Ok(Some(RoutedTab::Resolved(tab))) if tab != "important" => {
                    let _ = core
                        .suppress_notification_delivery(item.id, "not important")
                        .await;
                    continue;
                }
                _ => {}
            }
        }
        if !core
            .claim_notification_delivery(item.id)
            .await
            .unwrap_or(false)
        {
            continue;
        }
        let _ = core.mark_notification_delivered(item.id).await;
        shown.push(Shown {
            sender: item
                .sender_name
                .filter(|name| !name.trim().is_empty())
                .or(item.sender_addr)
                .unwrap_or_default(),
            subject: item.subject,
        });
    }
    shown
}

/// Show the notifications; clicking one brings Evanya forward.
fn show(app: slint::Weak<AppWindow>, shown: Vec<Shown>, summary_title: String) {
    let notifications: Vec<(String, String)> = if shown.len() > MAX_SINGLE {
        let senders = shown
            .iter()
            .map(|item| item.sender.as_str())
            .take(4)
            .collect::<Vec<_>>()
            .join(", ");
        vec![(summary_title, senders)]
    } else {
        shown
            .into_iter()
            .map(|item| (item.sender, item.subject))
            .collect()
    };
    for (summary, body) in notifications {
        let app = app.clone();
        // Showing and waiting for a click block, so each gets its own thread.
        std::thread::spawn(move || {
            let mut notification = notify_rust::Notification::new();
            notification
                .appname("Evanya")
                .summary(&summary)
                .body(&body)
                .icon("com.evanya.mail");
            #[cfg(all(unix, not(target_os = "macos")))]
            notification.action("default", "Open");
            let Ok(handle) = notification.show() else {
                return;
            };
            #[cfg(all(unix, not(target_os = "macos")))]
            handle.wait_for_action(|action| {
                if action == "default" {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(app) = app.upgrade() {
                            crate::window_controller::activate(&app, None);
                        }
                    });
                }
            });
            #[cfg(not(all(unix, not(target_os = "macos"))))]
            let _ = (handle, app);
        });
    }
}

/// Play the new-mail sound, through the desktop's sound player.
pub(crate) fn play_new_mail_sound() {
    thread_local! {
        static LAST: Cell<Option<Instant>> = const { Cell::new(None) };
    }
    // A burst of arrivals sounds once.
    let now = Instant::now();
    if LAST.with(|last| {
        last.get()
            .is_some_and(|at| now.duration_since(at) < Duration::from_secs(3))
    }) {
        return;
    }
    LAST.with(|last| last.set(Some(now)));
    std::thread::spawn(|| {
        #[cfg(target_os = "macos")]
        let players: &[(&str, &[&str])] = &[("afplay", &["/System/Library/Sounds/Glass.aiff"])];
        #[cfg(not(target_os = "macos"))]
        let players: &[(&str, &[&str])] = &[
            ("canberra-gtk-play", &["--id", "message-new-instant"]),
            (
                "pw-play",
                &["/usr/share/sounds/freedesktop/stereo/message-new-instant.oga"],
            ),
            (
                "paplay",
                &["/usr/share/sounds/freedesktop/stereo/message-new-instant.oga"],
            ),
        ];
        for (player, args) in players {
            let played = std::process::Command::new(player)
                .args(*args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success());
            if played {
                break;
            }
        }
    });
}

/// Check the outbox periodically and deliver what is due.
pub(crate) fn start(
    app: &AppWindow,
    state: &Rc<std::cell::RefCell<InboxState>>,
    runtime: &Rc<tokio::runtime::Runtime>,
) {
    let started_ms = now_ms();
    let busy = Rc::new(Cell::new(false));
    let app_weak = app.as_weak();
    let state = Rc::clone(state);
    let runtime = Rc::clone(runtime);
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, INTERVAL, move || {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        if busy.get() {
            return;
        }
        let Some(core) = state
            .borrow()
            .core
            .as_ref()
            .map(|core| core.notification_core())
        else {
            return;
        };
        let enabled = app.get_notifications_enabled();
        let important_only = app.get_notification_scope() != "all";
        let summary_title =
            crate::ui_message::translated(&app, &crate::ui_message::UiMessage::plain("New mail"))
                .to_string();
        busy.set(true);
        let busy_done = Rc::clone(&busy);
        let app_for_show = app.as_weak();
        let task = runtime.spawn(async move {
            let shown = collect(&Arc::clone(&core), enabled, important_only, started_ms).await;
            if !shown.is_empty() {
                show(app_for_show, shown, summary_title);
            }
        });
        // Clear the busy flag on the UI thread once the round is over.
        slint::spawn_local(async move {
            let _ = task.await;
            busy_done.set(false);
        })
        .ok();
    });
    // The timer lives as long as the window.
    std::mem::forget(timer);
}
