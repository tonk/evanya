//! Settings preference callback wiring.
//!
//! These callbacks share one lifecycle and persistence boundary: update the
//! session immediately where needed, then write through the core when it is
//! available. Keeping them together prevents the application composition root
//! from accumulating one closure per setting.

use super::*;
use flectar_mail_core::models::CustomTheme;

/// The parts of the workspace with a font of their own, in Settings order.
const FONT_AREAS: [&str; 7] = [
    "sidebar", "list", "reader", "calendar", "contacts", "files", "composer",
];
/// The size scales offered, as the Fonts settings list them.
const FONT_SCALES: [f32; 8] = [0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0];

/// The installed font families, sorted, read once.
fn installed_font_families() -> Vec<String> {
    thread_local! {
        static FAMILIES: std::cell::OnceCell<Vec<String>> = const { std::cell::OnceCell::new() };
    }
    FAMILIES.with(|families| {
        families
            .get_or_init(|| {
                let mut collection = slint::fontique_011::shared_collection();
                let mut names = collection
                    .family_names()
                    .filter(|name| !name.trim().is_empty() && !name.starts_with('.'))
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                names.sort_by_key(|name| name.to_lowercase());
                names.dedup();
                names
            })
            .clone()
    })
}

/// Show each part of the workspace in its chosen font and size.
pub(crate) fn apply_area_fonts(app: &AppWindow, settings: &flectar_mail_core::models::Settings) {
    let families = installed_font_families();
    let fonts = app.global::<AreaFonts>();
    let mut rows = Vec::new();
    for area in FONT_AREAS {
        let chosen = settings.area_fonts.get(area);
        let family = chosen
            .map(|font| font.family.trim())
            .filter(|family| !family.is_empty() && families.iter().any(|known| known == family));
        let scale = chosen.map_or(1.0, |font| font.scale).clamp(0.5, 3.0);
        let shown = slint::SharedString::from(family.unwrap_or(compose_editor::UI_FONT_FAMILY));
        match area {
            "sidebar" => {
                fonts.set_sidebar_family(shown);
                fonts.set_sidebar_scale(scale);
            }
            "list" => {
                fonts.set_list_family(shown);
                fonts.set_list_scale(scale);
            }
            "reader" => {
                fonts.set_reader_family(shown);
                fonts.set_reader_scale(scale);
            }
            "calendar" => {
                fonts.set_calendar_family(shown);
                fonts.set_calendar_scale(scale);
            }
            "contacts" => {
                fonts.set_contacts_family(shown);
                fonts.set_contacts_scale(scale);
            }
            "files" => {
                fonts.set_files_family(shown);
                fonts.set_files_scale(scale);
            }
            _ => {
                fonts.set_composer_family(shown);
                fonts.set_composer_scale(scale);
                compose_editor::set_body_font(family.unwrap_or_default(), scale);
            }
        }
        let family_index = family
            .and_then(|family| families.iter().position(|known| known == family))
            .map_or(0, |index| index + 1);
        let scale_index = FONT_SCALES
            .iter()
            .position(|known| (known - scale).abs() < 0.01)
            .unwrap_or(2);
        rows.push(AreaFontRow {
            key: area.into(),
            family_index: i32::try_from(family_index).unwrap_or_default(),
            scale_index: i32::try_from(scale_index).unwrap_or(2),
        });
    }
    let mut names = vec![translated(
        app,
        &UiMessage::detail("App font ({})", compose_editor::UI_FONT_FAMILY),
    )];
    names.extend(families.into_iter().map(slint::SharedString::from));
    app.set_font_family_names(ModelRc::new(VecModel::from(names)));
    app.set_area_font_rows(ModelRc::new(VecModel::from(rows)));
}

/// Load the spelling dictionaries these settings ask for and show the
/// choice in Settings.
pub(crate) fn apply_spellcheck(app: &AppWindow, settings: &flectar_mail_core::models::Settings) {
    let available = crate::spellcheck::available_languages();
    let interface_language = if settings.language == "system" {
        sys_locale::get_locale().unwrap_or_default()
    } else {
        settings.language.clone()
    };
    let languages = crate::spellcheck::effective_languages(
        settings.spellcheck_enabled,
        &settings.spellcheck_languages,
        &interface_language,
        &available,
    );
    crate::spellcheck::configure(&languages, &settings.spellcheck_words);
    let row = |code: &String| SpellLanguageRow {
        code: code.clone().into(),
        name: crate::spellcheck::language_name(code).into(),
    };
    app.set_spellcheck_enabled(settings.spellcheck_enabled);
    app.set_spellcheck_automatic(settings.spellcheck_languages.is_empty());
    app.set_spellcheck_languages(ModelRc::new(VecModel::from(
        languages.iter().map(row).collect::<Vec<_>>(),
    )));
    let choices = available
        .iter()
        .filter(|code| !languages.contains(code))
        .map(row)
        .collect::<Vec<_>>();
    // The list opens on a prompt, so choosing any language is a change.
    let mut names = vec![translated(app, &UiMessage::plain("Add a language…"))];
    names.extend(choices.iter().map(|choice| choice.name.clone()));
    app.set_spellcheck_choice_names(ModelRc::new(VecModel::from(names)));
    app.set_spellcheck_choices(ModelRc::new(VecModel::from(choices)));
    app.set_spellcheck_dictionaries_found(!available.is_empty());
}

fn apply_account_presentation_settings(
    app: &AppWindow,
    state: &Rc<RefCell<InboxState>>,
    runtime: &tokio::runtime::Runtime,
    settings: &flectar_mail_core::models::Settings,
) {
    state.borrow_mut().account_presentation = AccountPresentationSettings::from_settings(settings);
    refresh_connected_accounts(app, state);
    refresh_rows_only(app, state, runtime);
    // A profile change can change which accounts the mail view shows.
    app.invoke_mail_profile_changed();
}

/// Persists the download location and only then reflects it in the UI, so a
/// failed write never leaves Settings showing a directory that was not saved.
#[cfg(desktop)]
fn save_download_directory(
    app: &AppWindow,
    state: &Rc<RefCell<InboxState>>,
    runtime: &tokio::runtime::Runtime,
    directory: String,
) {
    let Some(core) = state.borrow().core.clone() else {
        app.set_sync_status(UiMessage::plain(
            "Download location is saved once an account is set up.",
        ));
        return;
    };
    match runtime.block_on(core.set_download_directory(&directory)) {
        Ok(()) => {
            app.set_download_directory(
                crate::documents::download_directory_label(&directory).into(),
            );
            app.set_sync_status(UiMessage::plain("Download location saved."));
        }
        Err(error) => app.set_sync_status(UiMessage::detail(
            "Could not save the download location: {}",
            error,
        )),
    }
}

pub(super) fn register_settings_preference_callbacks(
    app: &AppWindow,
    state: &Rc<RefCell<InboxState>>,
    runtime: &Rc<tokio::runtime::Runtime>,
) {
    let app_weak = app.as_weak();
    let state_for_notifications = Rc::clone(state);
    let runtime_for_notifications = Rc::clone(runtime);
    app.on_save_notification_settings(move |enabled, sound, scope| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_notifications.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain(
                "Notification preferences are active for this session.",
            ));
            return;
        };
        match runtime_for_notifications.block_on(core.set_notification_settings(
            enabled,
            sound,
            scope.as_str(),
        )) {
            Ok(()) => app.set_sync_status(UiMessage::plain("Notification preferences saved.")),
            Err(error) => {
                app.set_sync_status(UiMessage::detail("Could not save notifications: {}", error))
            }
        }
    });

    let app_weak = app.as_weak();
    let state_for_sync_interval = Rc::clone(state);
    let runtime_for_sync_interval = Rc::clone(runtime);
    app.on_save_sync_interval(move |minutes| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_sync_interval.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain(
                "Sync interval is active for this session.",
            ));
            return;
        };
        match runtime_for_sync_interval.block_on(core.set_sync_interval_minutes(i64::from(minutes)))
        {
            Ok(()) => app.set_sync_status(UiMessage::detail(
                "Sync interval set to {} minutes.",
                minutes,
            )),
            Err(error) => {
                app.set_sync_status(UiMessage::detail("Could not save sync interval: {}", error))
            }
        }
    });

    let app_weak = app.as_weak();
    let state_for_read_on_open = Rc::clone(state);
    let runtime_for_read_on_open = Rc::clone(runtime);
    app.on_save_mark_read_on_open(move |enabled| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        state_for_read_on_open.borrow_mut().mark_read_on_open = enabled;
        let Some(core) = state_for_read_on_open.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain(
                "Message-opening preference updated for this session.",
            ));
            return;
        };
        match runtime_for_read_on_open.block_on(core.set_mark_read_on_open(enabled)) {
            Ok(()) => app.set_sync_status(UiMessage::plain("Message-opening preference saved.")),
            Err(error) => app.set_sync_status(UiMessage::detail(
                "Could not save message-opening preference: {}",
                error,
            )),
        }
    });

    let app_weak = app.as_weak();
    let state_for_contacts = Rc::clone(state);
    let runtime_for_contacts = Rc::clone(runtime);
    app.on_save_contact_discovery_settings(move |outgoing, incoming, all_accounts, suggest| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_contacts.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain(
                "Contact suggestion preferences are active for this session.",
            ));
            return;
        };
        match runtime_for_contacts.block_on(core.set_contact_discovery_settings(
            outgoing,
            incoming,
            all_accounts,
            suggest,
        )) {
            Ok(()) => {
                app.set_sync_status(UiMessage::plain("Contact suggestion preferences saved."))
            }
            Err(error) => {
                if let Ok(settings) = runtime_for_contacts.block_on(core.load_settings()) {
                    app.set_collect_outgoing_contacts(settings.collect_outgoing_contacts);
                    app.set_collect_incoming_contacts(settings.collect_incoming_contacts);
                    app.set_contact_suggest_all_accounts(settings.contact_suggest_all_accounts);
                    app.set_suggest_learned_contacts(settings.suggest_learned_contacts);
                }
                app.set_sync_status(UiMessage::detail(
                    "Could not save contact suggestion preferences: {}",
                    error,
                ));
            }
        }
    });

    let app_weak = app.as_weak();
    let state_for_clear_contacts = Rc::clone(state);
    let runtime_for_clear_contacts = Rc::clone(runtime);
    app.on_clear_contact_suggestions(move || {
        let Some(app) = app_weak.upgrade() else {
            return false;
        };
        let Some(core) = state_for_clear_contacts.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain(
                "Contact storage is still starting. Try again shortly.",
            ));
            return false;
        };
        match runtime_for_clear_contacts.block_on(core.clear_contact_suggestions()) {
            Ok(removed) => {
                app.set_sync_status(UiMessage::detail("Cleared {} suggested people.", removed));
                app.invoke_search_contacts(app.get_contact_search_query());
                true
            }
            Err(error) => {
                app.set_sync_status(UiMessage::detail(
                    "Could not clear suggested people: {}",
                    error,
                ));
                false
            }
        }
    });

    let app_weak = app.as_weak();
    let state_for_theme = Rc::clone(state);
    let runtime_for_theme = Rc::clone(runtime);
    app.on_save_theme(move |theme| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_theme.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain("Theme updated for this session."));
            return;
        };
        match runtime_for_theme.block_on(core.set_theme(&theme)) {
            Ok(()) => app.set_sync_status(UiMessage::plain("Theme preference saved.")),
            Err(error) => app.set_sync_status(UiMessage::detail("Could not save theme: {}", error)),
        }
    });

    let app_weak = app.as_weak();
    // Only desktop hosts pick a filesystem directory; mobile hosts export
    // through their own document provider and hide the setting entirely.
    #[cfg(desktop)]
    {
        let state_for_download = Rc::clone(state);
        let runtime_for_download = Rc::clone(runtime);
        let app_weak_for_download = app_weak.clone();
        let picking_download = Rc::new(Cell::new(false));
        app.on_choose_download_directory(move || {
            let Some(app) = app_weak_for_download.upgrade() else {
                return;
            };
            if picking_download.replace(true) {
                return;
            }
            let title = translated(&app, &UiMessage::plain("Choose download location")).to_string();
            let core = state_for_download.borrow().core.clone();
            // The portal picker needs the runtime: rfd's blocking dialog
            // panics on the UI thread, which has no Tokio context.
            let request: crate::documents::DocumentFuture<Option<std::path::PathBuf>> =
                Box::pin(async move {
                    // Starts in the saved folder as stored, not as Settings
                    // shows it: inside Flatpak the shown host path may be
                    // outside the sandbox.
                    let current = match core {
                        Some(core) => core.load_settings().await.ok().and_then(|settings| {
                            crate::documents::resolve_download_directory(
                                &settings.download_directory,
                            )
                        }),
                        None => None,
                    };
                    let dialog = rfd::AsyncFileDialog::new().set_title(title);
                    let dialog = match current {
                        Some(directory) => {
                            dialog.set_directory(crate::documents::picker_directory(&directory))
                        }
                        None => dialog,
                    };
                    Ok(dialog
                        .pick_folder()
                        .await
                        .map(|folder| folder.path().to_owned()))
                });
            let state = Rc::clone(&state_for_download);
            let runtime = Rc::clone(&runtime_for_download);
            let picking = Rc::clone(&picking_download);
            let app_weak = app.as_weak();
            crate::documents::run_request(&runtime_for_download, request, move |picked| {
                picking.set(false);
                let Some(app) = app_weak.upgrade() else {
                    return;
                };
                match picked {
                    Ok(Some(directory)) => save_download_directory(
                        &app,
                        &state,
                        &runtime,
                        directory.to_string_lossy().into_owned(),
                    ),
                    Ok(None) => {}
                    Err(error) => app.set_sync_status(UiMessage::detail(
                        "Could not save the download location: {}",
                        error,
                    )),
                }
            });
        });
        let state_for_clear = Rc::clone(state);
        let runtime_for_clear = Rc::clone(runtime);
        let app_weak_for_clear = app_weak.clone();
        app.on_clear_download_directory(move || {
            let Some(app) = app_weak_for_clear.upgrade() else {
                return;
            };
            save_download_directory(&app, &state_for_clear, &runtime_for_clear, String::new());
        });
    }
    let state_for_theme_preset = Rc::clone(state);
    let runtime_for_theme_preset = Rc::clone(runtime);
    app.on_save_theme_preset(move |preset| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_theme_preset.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain("Theme palette updated for this session."));
            return;
        };
        match runtime_for_theme_preset.block_on(core.set_theme_preset(preset.as_str())) {
            Ok(()) => app.set_sync_status(UiMessage::plain("Theme palette saved.")),
            Err(error) => {
                app.set_sync_status(UiMessage::detail("Could not save theme palette: {}", error))
            }
        }
    });

    let app_weak = app.as_weak();
    let state_for_appearance = Rc::clone(state);
    let runtime_for_appearance = Rc::clone(runtime);
    app.global::<MessageColors>().on_choose(move |preference| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        if flectar_mail_core::models::MessageAppearance::parse(&preference).is_none() {
            return;
        }
        let colors = app.global::<MessageColors>();
        let previous = colors.get_preference();
        colors.set_preference(preference.clone());
        let Some(core) = state_for_appearance.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain(
                "Message appearance updated for this session.",
            ));
            return;
        };
        match runtime_for_appearance.block_on(core.set_message_appearance(&preference)) {
            Ok(()) => app.set_sync_status(UiMessage::plain("Message appearance preference saved.")),
            Err(error) => {
                colors.set_preference(previous);
                app.set_sync_status(UiMessage::detail(
                    "Could not save message appearance: {}",
                    error,
                ));
            }
        }
    });

    let app_weak = app.as_weak();
    let state_for_custom_theme = Rc::clone(state);
    let runtime_for_custom_theme = Rc::clone(runtime);
    app.on_save_custom_theme(
        move |light_primary,
              light_page,
              light_surface,
              light_text,
              light_border,
              dark_primary,
              dark_page,
              dark_surface,
              dark_text,
              dark_border| {
            let Some(app) = app_weak.upgrade() else {
                return false;
            };
            let custom_theme = CustomTheme {
                light_primary: theme::color_to_hex(light_primary),
                light_page_background: theme::color_to_hex(light_page),
                light_surface: theme::color_to_hex(light_surface),
                light_text: theme::color_to_hex(light_text),
                light_border: theme::color_to_hex(light_border),
                dark_primary: theme::color_to_hex(dark_primary),
                dark_page_background: theme::color_to_hex(dark_page),
                dark_surface: theme::color_to_hex(dark_surface),
                dark_text: theme::color_to_hex(dark_text),
                dark_border: theme::color_to_hex(dark_border),
            };
            let Some(core) = state_for_custom_theme.borrow().core.clone() else {
                app.set_sync_status(UiMessage::plain("Custom theme updated for this session."));
                return false;
            };
            match runtime_for_custom_theme.block_on(core.set_custom_theme(custom_theme)) {
                Ok(()) => {
                    app.set_sync_status(UiMessage::plain("Custom theme saved."));
                    true
                }
                Err(error) => {
                    app.set_sync_status(UiMessage::detail(
                        "Could not save custom theme: {}",
                        error,
                    ));
                    false
                }
            }
        },
    );

    let app_weak = app.as_weak();
    let state_for_language = Rc::clone(state);
    let runtime_for_language = Rc::clone(runtime);
    app.on_save_language(move |language| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        apply_language(&app, language.as_str());
        let Some(core) = state_for_language.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain("Language updated for this session."));
            return;
        };
        match runtime_for_language.block_on(core.set_language(language.as_str())) {
            Ok(()) => app.set_sync_status(UiMessage::plain("Language preference saved.")),
            Err(error) => {
                app.set_sync_status(UiMessage::detail("Could not save language: {}", error))
            }
        }
    });

    let app_weak = app.as_weak();
    let state_for_time_format = Rc::clone(state);
    let runtime_for_time_format = Rc::clone(runtime);
    app.on_save_time_format(move |value| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(format) = flectar_mail_core::models::TimeFormat::parse(&value) else {
            return;
        };
        date_format::reload(&app);
        let Some(core) = state_for_time_format.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain("Time format updated for this session."));
            return;
        };
        match runtime_for_time_format.block_on(core.set_time_format(format)) {
            Ok(()) => app.set_sync_status(UiMessage::plain("Time format preference saved.")),
            Err(error) => {
                app.set_sync_status(UiMessage::detail("Could not save time format: {}", error))
            }
        }
    });

    let app_weak = app.as_weak();
    let state_for_app_icon = Rc::clone(state);
    let runtime_for_app_icon = Rc::clone(runtime);
    app.global::<AppIcon>().on_choose(move |variant| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let previous = crate::app_icon::choice(app.global::<AppIcon>().get_variant());
        let icon = crate::app_icon::choice(variant);
        crate::app_icon::apply(&app, icon);
        let Some(core) = state_for_app_icon.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain("App icon updated for this session."));
            return;
        };
        match runtime_for_app_icon.block_on(core.set_app_icon(icon)) {
            Ok(()) => app.set_sync_status(UiMessage::plain("App icon saved.")),
            Err(error) => {
                crate::app_icon::apply(&app, previous);
                app.set_sync_status(UiMessage::detail("Could not save app icon: {}", error));
            }
        }
    });

    let app_weak = app.as_weak();
    let state_for_sidebar_icons = Rc::clone(state);
    let runtime_for_sidebar_icons = Rc::clone(runtime);
    app.on_save_sidebar_icon_style(move |monochrome| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_sidebar_icons.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain(
                "Sidebar icon style updated for this session.",
            ));
            return;
        };
        match runtime_for_sidebar_icons.block_on(core.set_monochrome_sidebar_icons(monochrome)) {
            Ok(()) => app.set_sync_status(UiMessage::plain("Sidebar icon preference saved.")),
            Err(error) => app.set_sync_status(UiMessage::detail(
                "Could not save sidebar icon preference: {}",
                error,
            )),
        }
    });

    let app_weak = app.as_weak();
    let state_for_avatars = Rc::clone(state);
    let runtime_for_avatars = Rc::clone(runtime);
    app.on_save_avatar_visibility(move |enabled| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_avatars.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain(
                "Avatar preference updated for this session.",
            ));
            return;
        };
        match runtime_for_avatars.block_on(core.set_show_avatars(enabled)) {
            Ok(()) => app.set_sync_status(UiMessage::plain("Avatar preference saved.")),
            Err(error) => app.set_sync_status(UiMessage::detail(
                "Could not save avatar preference: {}",
                error,
            )),
        }
    });

    let app_weak = app.as_weak();
    let state_for_tabs = Rc::clone(state);
    let runtime_for_tabs = Rc::clone(runtime);
    app.on_save_show_tabs(move |enabled| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_tabs.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain(
                "Tabs preference updated for this session.",
            ));
            return;
        };
        match runtime_for_tabs.block_on(core.set_show_tabs(enabled)) {
            Ok(()) => app.set_sync_status(UiMessage::plain("Tabs preference saved.")),
            Err(error) => app.set_sync_status(UiMessage::detail(
                "Could not save tabs preference: {}",
                error,
            )),
        }
    });

    let app_weak = app.as_weak();
    let state_for_grouping = Rc::clone(state);
    let runtime_for_grouping = Rc::clone(runtime);
    app.on_save_conversation_view(move |enabled| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let grouping = if enabled {
            MailGrouping::Conversations
        } else {
            MailGrouping::Messages
        };
        apply_mail_grouping(&app, &state_for_grouping, &runtime_for_grouping, grouping);
        let Some(core) = state_for_grouping.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain(
                "Conversation view updated for this session.",
            ));
            return;
        };
        match runtime_for_grouping.block_on(core.set_conversation_view(enabled)) {
            Ok(()) => app.set_sync_status(UiMessage::plain("Conversation view preference saved.")),
            Err(error) => {
                let previous = if enabled {
                    MailGrouping::Messages
                } else {
                    MailGrouping::Conversations
                };
                apply_mail_grouping(&app, &state_for_grouping, &runtime_for_grouping, previous);
                app.set_sync_status(UiMessage::detail(
                    "Could not save conversation view preference: {}",
                    error,
                ));
            }
        }
    });

    let app_weak = app.as_weak();
    let state_for_date_groups = Rc::clone(state);
    let runtime_for_date_groups = Rc::clone(runtime);
    app.on_save_group_mail_by_date(move |enabled| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        state_for_date_groups.borrow_mut().mail_groups.clear();
        refresh_rows_only(&app, &state_for_date_groups, &runtime_for_date_groups);
        let Some(core) = state_for_date_groups.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain(
                "Mail grouping is active for this session.",
            ));
            return;
        };
        match runtime_for_date_groups.block_on(core.set_group_mail_by_date(enabled)) {
            Ok(()) => app.set_sync_status(UiMessage::plain("Mail grouping preference saved.")),
            Err(error) => {
                app.set_group_mail_by_date(!enabled);
                refresh_rows_only(&app, &state_for_date_groups, &runtime_for_date_groups);
                app.set_sync_status(UiMessage::detail(
                    "Could not save mail grouping preference: {}",
                    error,
                ));
            }
        }
    });

    let app_weak = app.as_weak();
    let state_for_profiles = Rc::clone(state);
    let runtime_for_profiles = Rc::clone(runtime);
    app.on_save_mail_profile(move |profile_id, name, color| {
        let Some(app) = app_weak.upgrade() else {
            return Default::default();
        };
        let Some(core) = state_for_profiles.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain("Profile changes require local mail data."));
            return Default::default();
        };
        let profile_id = (!profile_id.trim().is_empty()).then(|| profile_id.to_string());
        let (mut settings, profile) = match runtime_for_profiles.block_on(core.save_mail_profile(
            profile_id,
            name.to_string(),
            theme::color_to_hex(color),
        )) {
            Ok(saved) => saved,
            Err(error) => {
                app.set_sync_status(UiMessage::detail("Could not save profile: {}", error));
                return Default::default();
            }
        };
        // The editor's checked accounts become the profile's accounts.
        let members = app.get_profile_members();
        let mut failure = None;
        for member in members.iter() {
            let account_id = i64::from(member.id);
            let in_profile = profile.account_ids.contains(&account_id);
            let assignment = match (member.checked, in_profile) {
                (true, false) => Some(profile.id.clone()),
                (false, true) => None,
                _ => continue,
            };
            match runtime_for_profiles.block_on(core.assign_account_profile(account_id, assignment))
            {
                Ok(updated) => settings = updated,
                Err(error) => failure = Some(error),
            }
        }
        apply_account_presentation_settings(
            &app,
            &state_for_profiles,
            &runtime_for_profiles,
            &settings,
        );
        match failure {
            Some(error) => app.set_sync_status(UiMessage::detail(
                "Could not update account profile: {}",
                error,
            )),
            None => app.set_sync_status(UiMessage::plain("Profile saved.")),
        }
        profile.id.into()
    });

    let app_weak = app.as_weak();
    app.on_prepare_profile_members(move |profile_id, checked_account| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let rows = connected_account_rows(&app)
            .into_iter()
            .map(|account| {
                let member = !profile_id.is_empty() && account.profile_id == profile_id;
                ProfileMemberRow {
                    id: account.id,
                    name: account.name,
                    email: account.email,
                    initials: account.initials,
                    avatar: account.avatar_small,
                    has_avatar: account.has_avatar,
                    other_profile: if member {
                        Default::default()
                    } else {
                        account.profile_name
                    },
                    checked: member || account.id == checked_account,
                }
            })
            .collect::<Vec<_>>();
        app.set_profile_members(ModelRc::new(VecModel::from(rows)));
    });

    let app_weak = app.as_weak();
    let state_for_profile_assignment = Rc::clone(state);
    let runtime_for_profile_assignment = Rc::clone(runtime);
    app.on_assign_account_profile(move |account_id, profile_id| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_profile_assignment.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain("Profile changes require local mail data."));
            return;
        };
        let profile_id = (!profile_id.trim().is_empty()).then(|| profile_id.to_string());
        match runtime_for_profile_assignment
            .block_on(core.assign_account_profile(i64::from(account_id), profile_id))
        {
            Ok(settings) => {
                apply_account_presentation_settings(
                    &app,
                    &state_for_profile_assignment,
                    &runtime_for_profile_assignment,
                    &settings,
                );
                app.set_sync_status(UiMessage::plain("Account profile updated."));
            }
            Err(error) => app.set_sync_status(UiMessage::detail(
                "Could not update account profile: {}",
                error,
            )),
        }
    });

    let app_weak = app.as_weak();
    let state_for_account_color = Rc::clone(state);
    let runtime_for_account_color = Rc::clone(runtime);
    app.on_save_account_color(move |account_id, color, generated| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_account_color.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain(
                "Account color changes require local mail data.",
            ));
            return;
        };
        let color = (!generated).then(|| theme::color_to_hex(color));
        match runtime_for_account_color
            .block_on(core.set_account_color(i64::from(account_id), color))
        {
            Ok(settings) => {
                apply_account_presentation_settings(
                    &app,
                    &state_for_account_color,
                    &runtime_for_account_color,
                    &settings,
                );
                app.set_sync_status(UiMessage::plain("Account color updated."));
            }
            Err(error) => app.set_sync_status(UiMessage::detail(
                "Could not update account color: {}",
                error,
            )),
        }
    });

    let app_weak = app.as_weak();
    let state_for_active_profile = Rc::clone(state);
    let runtime_for_active_profile = Rc::clone(runtime);
    app.on_select_mail_profile(move |profile_id| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_active_profile.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain("Profile changes require local mail data."));
            return;
        };
        let profile_id = (!profile_id.trim().is_empty()).then(|| profile_id.to_string());
        match runtime_for_active_profile.block_on(core.set_active_mail_profile(profile_id)) {
            Ok(settings) => apply_account_presentation_settings(
                &app,
                &state_for_active_profile,
                &runtime_for_active_profile,
                &settings,
            ),
            Err(error) => {
                app.set_sync_status(UiMessage::detail("Could not switch profile: {}", error))
            }
        }
    });

    let app_weak = app.as_weak();
    let state_for_profile_delete = Rc::clone(state);
    let runtime_for_profile_delete = Rc::clone(runtime);
    app.on_delete_mail_profile(move |profile_id| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_profile_delete.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain("Profile changes require local mail data."));
            return;
        };
        match runtime_for_profile_delete.block_on(core.delete_mail_profile(profile_id.to_string()))
        {
            Ok(settings) => {
                apply_account_presentation_settings(
                    &app,
                    &state_for_profile_delete,
                    &runtime_for_profile_delete,
                    &settings,
                );
                app.set_sync_status(UiMessage::plain("Profile deleted."));
            }
            Err(error) => {
                app.set_sync_status(UiMessage::detail("Could not delete profile: {}", error))
            }
        }
    });

    let app_weak = app.as_weak();
    let state_for_rename = Rc::clone(state);
    let runtime_for_rename = Rc::clone(runtime);
    app.on_rename_account(move |account_id, name| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_rename.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain("Account names require local mail data."));
            return;
        };
        match runtime_for_rename
            .block_on(core.set_account_short_name(i64::from(account_id), name.as_str()))
        {
            Ok(settings) => {
                apply_account_presentation_settings(
                    &app,
                    &state_for_rename,
                    &runtime_for_rename,
                    &settings,
                );
                // The sidebar's account headings come with the mailbox list.
                let _ =
                    refresh_from_source(&app, &state_for_rename, &runtime_for_rename, true, &[]);
                app.set_sync_status(UiMessage::plain("Account name saved."));
            }
            Err(error) => {
                app.set_sync_status(UiMessage::detail("Could not save account name: {}", error))
            }
        }
    });

    let app_weak = app.as_weak();
    let state_for_subscriptions = Rc::clone(state);
    app.on_open_folder_subscriptions(move |account_id| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let rows = crate::sidebar_model::subscription_rows(
            &state_for_subscriptions.borrow(),
            i64::from(account_id),
        );
        app.set_folder_subscription_rows(ModelRc::new(VecModel::from(rows)));
        app.set_folder_subscription_account(account_id);
    });

    let app_weak = app.as_weak();
    let state_for_subscribe = Rc::clone(state);
    let runtime_for_subscribe = Rc::clone(runtime);
    app.on_set_mailbox_subscribed(move |kind, id, subscribed| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_subscribe.borrow().core.clone() else {
            return;
        };
        match runtime_for_subscribe.block_on(core.set_mailbox_subscribed(
            kind.as_str(),
            i64::from(id),
            subscribed,
        )) {
            Ok(settings) => {
                apply_account_presentation_settings(
                    &app,
                    &state_for_subscribe,
                    &runtime_for_subscribe,
                    &settings,
                );
                crate::sidebar_model::refresh_sidebar(&state_for_subscribe);
                let account_id = app.get_folder_subscription_account();
                if account_id >= 0 {
                    let rows = crate::sidebar_model::subscription_rows(
                        &state_for_subscribe.borrow(),
                        i64::from(account_id),
                    );
                    app.set_folder_subscription_rows(ModelRc::new(VecModel::from(rows)));
                }
            }
            Err(error) => app.set_sync_status(UiMessage::detail(
                "Could not save the folder choice: {}",
                error,
            )),
        }
    });

    let app_weak = app.as_weak();
    let state_for_hidden = Rc::clone(state);
    let runtime_for_hidden = Rc::clone(runtime);
    app.on_set_account_shown_in_mail(move |account_id, shown| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_hidden.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain("Account names require local mail data."));
            return;
        };
        match runtime_for_hidden
            .block_on(core.set_mail_account_hidden(i64::from(account_id), !shown))
        {
            Ok(settings) => {
                apply_account_presentation_settings(
                    &app,
                    &state_for_hidden,
                    &runtime_for_hidden,
                    &settings,
                );
                {
                    let mut state = state_for_hidden.borrow_mut();
                    state.page = 1;
                    state.next_cursor = None;
                }
                let _ =
                    refresh_from_source(&app, &state_for_hidden, &runtime_for_hidden, false, &[]);
                app.set_sync_status(if shown {
                    UiMessage::plain("The account's mail is shown again.")
                } else {
                    UiMessage::plain("The account's mail is hidden; it keeps synchronizing.")
                });
            }
            Err(error) => app.set_sync_status(UiMessage::detail(
                "Could not save the account's mail visibility: {}",
                error,
            )),
        }
    });

    let app_weak = app.as_weak();
    let state_for_marker_visibility = Rc::clone(state);
    let runtime_for_marker_visibility = Rc::clone(runtime);
    app.on_save_account_markers_visibility(move |enabled| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_marker_visibility.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain(
                "Marker preference is active for this session.",
            ));
            state_for_marker_visibility
                .borrow_mut()
                .account_presentation
                .show_markers = enabled;
            refresh_rows_only(
                &app,
                &state_for_marker_visibility,
                &runtime_for_marker_visibility,
            );
            return;
        };
        match runtime_for_marker_visibility.block_on(core.set_show_account_badges(enabled)) {
            Ok(settings) => {
                apply_account_presentation_settings(
                    &app,
                    &state_for_marker_visibility,
                    &runtime_for_marker_visibility,
                    &settings,
                );
                app.set_sync_status(UiMessage::plain("Mail-list marker preference saved."));
            }
            Err(error) => app.set_sync_status(UiMessage::detail(
                "Could not save mail-list marker preference: {}",
                error,
            )),
        }
    });

    let app_weak = app.as_weak();
    let state_for_unified = Rc::clone(state);
    let runtime_for_unified = Rc::clone(runtime);
    app.on_save_unified_mailbox(move |enabled| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let core = state_for_unified.borrow().core.clone();
        match core {
            Some(core) => match runtime_for_unified.block_on(core.set_unified_mailbox(enabled)) {
                Ok(()) => {
                    app.set_sync_status(UiMessage::plain("Unified mailbox preference saved."))
                }
                Err(error) => app.set_sync_status(UiMessage::detail(
                    "Could not save unified mailbox preference: {}",
                    error,
                )),
            },
            None => {}
        }
        crate::sidebar_model::set_unified_mailbox_enabled(enabled);
        crate::sidebar_model::refresh_sidebar(&state_for_unified);
        // Leave a unified scope the sidebar no longer offers.
        if !enabled {
            let next = {
                let state = state_for_unified.borrow();
                let next = state.scope.visible_for(&state.mail_accounts());
                (next != state.scope).then_some(next)
            };
            if let Some(scope) = next {
                app.invoke_select_scope(scope.to_string().into());
            }
        }
    });

    // A font or size for one part of the workspace.
    let app_weak = app.as_weak();
    let state_for_fonts = Rc::clone(state);
    let runtime_for_fonts = Rc::clone(runtime);
    app.on_save_area_font(move |area, family, scale| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_fonts.borrow().core.clone() else {
            return;
        };
        let area = area.to_string();
        let family = family.trim().to_owned();
        let result = runtime_for_fonts.block_on(core.update_settings(|settings| {
            if family.is_empty() && (scale - 1.0).abs() < 0.01 {
                settings.area_fonts.remove(&area);
            } else {
                settings
                    .area_fonts
                    .insert(area, flectar_mail_core::models::AreaFont { family, scale });
            }
        }));
        match result.and(runtime_for_fonts.block_on(core.load_settings())) {
            Ok(settings) => apply_area_fonts(&app, &settings),
            Err(error) => app.set_sync_status(UiMessage::detail(
                "Could not save the font settings: {}",
                error,
            )),
        }
    });

    // Spelling: switch it on or off, and choose the languages.
    let app_weak = app.as_weak();
    let state_for_spelling = Rc::clone(state);
    let runtime_for_spelling = Rc::clone(runtime);
    app.on_save_spellcheck(move |action, code| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_spelling.borrow().core.clone() else {
            return;
        };
        // The languages shown are the ones in effect; a first change turns
        // the automatic choice into an explicit list.
        let shown = (0..app.get_spellcheck_languages().row_count())
            .filter_map(|index| app.get_spellcheck_languages().row_data(index))
            .map(|row| row.code.to_string())
            .collect::<Vec<_>>();
        let action = action.to_string();
        let code = code.to_string();
        let result =
            runtime_for_spelling.block_on(core.update_settings(|settings| match action.as_str() {
                "on" => settings.spellcheck_enabled = true,
                "off" => settings.spellcheck_enabled = false,
                "add" => {
                    let mut languages = shown.clone();
                    if !languages.contains(&code) {
                        languages.push(code.clone());
                    }
                    settings.spellcheck_languages = languages;
                }
                "remove" => {
                    settings.spellcheck_languages = shown
                        .iter()
                        .filter(|known| **known != code)
                        .cloned()
                        .collect();
                }
                "automatic" => settings.spellcheck_languages.clear(),
                _ => {}
            }));
        match result.and(runtime_for_spelling.block_on(core.load_settings())) {
            Ok(settings) => apply_spellcheck(&app, &settings),
            Err(error) => app.set_sync_status(UiMessage::detail(
                "Could not save the spelling settings: {}",
                error,
            )),
        }
    });

    let state_for_layout = Rc::clone(state);
    let runtime_for_layout = Rc::clone(runtime);
    app.on_save_mail_list_layout(move |layout| {
        let layout = if layout.as_str() == "table" {
            "table"
        } else {
            "cards"
        };
        let core = state_for_layout.borrow().core.clone();
        if let Some(core) = core {
            let _ = runtime_for_layout.block_on(
                core.update_settings(|settings| settings.mail_list_layout = layout.to_owned()),
            );
        }
    });

    let state_for_zoom = Rc::clone(state);
    let runtime_for_zoom = Rc::clone(runtime);
    app.on_save_calendar_zoom(move |zoom| {
        let core = state_for_zoom.borrow().core.clone();
        if let Some(core) = core {
            let _ = runtime_for_zoom.block_on(
                core.update_settings(|settings| settings.calendar_zoom = zoom.clamp(0.4, 4.0)),
            );
        }
    });

    let state_for_columns = Rc::clone(state);
    let runtime_for_columns = Rc::clone(runtime);
    app.on_save_mail_columns(move |sender, date| {
        let core = state_for_columns.borrow().core.clone();
        if let Some(core) = core {
            let _ = runtime_for_columns.block_on(core.update_settings(|settings| {
                settings.mail_table_sender_width = (sender.round() as i64).clamp(80, 2000);
                settings.mail_table_date_width = (date.round() as i64).clamp(60, 260);
            }));
        }
    });

    let app_weak = app.as_weak();
    let state_for_sort = Rc::clone(state);
    let runtime_for_sort = Rc::clone(runtime);
    app.on_sort_mail_list(move |key| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let current = crate::mail::mail_sort();
        let requested = crate::mail::parse_mail_sort(&format!("{key}-asc"));
        // A second click on the same column turns its order around; a new
        // column starts newest first for dates and A to Z for text.
        let sort = if requested.key == current.key {
            flectar_mail_core::models::MailSort {
                descending: !current.descending,
                ..current
            }
        } else {
            flectar_mail_core::models::MailSort {
                descending: requested.key == flectar_mail_core::models::MailSortKey::Date,
                ..requested
            }
        };
        let stored = crate::mail::mail_sort_key(sort);
        crate::mail::set_mail_sort(sort);
        app.set_mail_sort(stored.clone().into());
        let core = state_for_sort.borrow().core.clone();
        if let Some(core) = core {
            let _ = runtime_for_sort
                .block_on(core.update_settings(|settings| settings.mail_sort = stored));
        }
        {
            let mut state = state_for_sort.borrow_mut();
            state.page = 1;
            state.next_cursor = None;
            state.mail_groups.clear();
        }
        if let Err(error) =
            refresh_from_source(&app, &state_for_sort, &runtime_for_sort, false, &[])
        {
            app.set_render_status(UiMessage::detail("Mail refresh failed: {}", error));
        }
    });

    let app_weak = app.as_weak();
    let state_for_list_dates = Rc::clone(state);
    let runtime_for_list_dates = Rc::clone(runtime);
    app.on_save_list_date_format(move |format| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let format = crate::mail::normalized_list_date_format(format.as_str());
        app.set_list_date_format(format.into());
        date_format::reload(&app);
        let core = state_for_list_dates.borrow().core.clone();
        if let Some(core) = core {
            match runtime_for_list_dates.block_on(core.set_list_date_format(format)) {
                Ok(()) => app.set_sync_status(UiMessage::plain("Date display preference saved.")),
                Err(error) => app.set_sync_status(UiMessage::detail(
                    "Could not save date display preference: {}",
                    error,
                )),
            }
        }
    });

    let app_weak = app.as_weak();
    let state_for_plain_text = Rc::clone(state);
    let runtime_for_plain_text = Rc::clone(runtime);
    app.on_save_compose_plain_text(move |enabled| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        app.set_compose_plain_text_default(enabled);
        let core = state_for_plain_text.borrow().core.clone();
        if let Some(core) = core {
            match runtime_for_plain_text.block_on(core.set_compose_plain_text(enabled)) {
                Ok(()) => app.set_sync_status(UiMessage::plain("Message format preference saved.")),
                Err(error) => app.set_sync_status(UiMessage::detail(
                    "Could not save message format preference: {}",
                    error,
                )),
            }
        }
    });

    let state_for_sidebar = Rc::clone(state);
    let runtime_for_sidebar = Rc::clone(runtime);
    app.on_save_mail_sidebar_width(move |width| {
        let core = state_for_sidebar.borrow().core.clone();
        if let Some(core) = core {
            let _ = runtime_for_sidebar.block_on(core.set_mail_sidebar_width(width.round() as i64));
        }
    });

    let state_for_split = Rc::clone(state);
    let runtime_for_split = Rc::clone(runtime);
    app.on_save_reading_pane_bottom_fraction(move |fraction| {
        let percent = (fraction * 100.0).round() as i64;
        let core = state_for_split.borrow().core.clone();
        if let Some(core) = core {
            let _ = runtime_for_split.block_on(core.set_reading_pane_bottom_percent(percent));
        }
    });

    let app_weak = app.as_weak();
    let state_for_pane = Rc::clone(state);
    let runtime_for_pane = Rc::clone(runtime);
    app.on_save_reading_pane_position(move |position| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let core = state_for_pane.borrow().core.clone();
        if let Some(core) = core {
            match runtime_for_pane.block_on(core.set_reading_pane_position(position.as_str())) {
                Ok(()) => app.set_sync_status(UiMessage::plain("Reading pane position saved.")),
                Err(error) => app.set_sync_status(UiMessage::detail(
                    "Could not save reading pane position: {}",
                    error,
                )),
            }
        }
    });

    let app_weak = app.as_weak();
    let state_for_workspace_layout = Rc::clone(state);
    let runtime_for_workspace_layout = Rc::clone(runtime);
    app.on_save_workspace_layout(move |layout| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let Some(core) = state_for_workspace_layout.borrow().core.clone() else {
            app.set_sync_status(UiMessage::plain(
                "Workspace layout updated for this session.",
            ));
            return;
        };
        match runtime_for_workspace_layout.block_on(core.set_workspace_layout(layout.as_str())) {
            Ok(()) => app.set_sync_status(UiMessage::plain("Workspace layout preference saved.")),
            Err(error) => app.set_sync_status(UiMessage::detail(
                "Could not save workspace layout: {}",
                error,
            )),
        }
    });

    let app_weak = app.as_weak();
    let state_for_list_pane_width = Rc::clone(state);
    let runtime_for_list_pane_width = Rc::clone(runtime);
    app.on_save_workspace_list_pane_width(move |width| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let width =
            flectar_mail_core::models::normalized_workspace_list_pane_width(width.round() as i64);
        app.set_workspace_list_pane_width(width as f32);
        let Some(core) = state_for_list_pane_width.borrow().core.clone() else {
            return;
        };
        if let Err(error) =
            runtime_for_list_pane_width.block_on(core.set_workspace_list_pane_width(width))
        {
            app.set_sync_status(UiMessage::detail(
                "Could not save list pane width: {}",
                error,
            ));
        }
    });
}

/// Both entry points share one asynchronous persistence path and busy state.
pub(super) fn register_oauth_settings_callbacks(
    app: &AppWindow,
    state: &Rc<RefCell<InboxState>>,
    runtime: &Rc<tokio::runtime::Runtime>,
) {
    let weak = app.as_weak();
    let single_state = Rc::clone(state);
    let single_runtime = Rc::clone(runtime);
    app.on_save_oauth_app(move |provider, client_id, client_secret| {
        let Some(app) = weak.upgrade() else { return };
        let (google, microsoft) = match provider.as_str() {
            "gmail" => (
                Some((client_id.to_string(), client_secret.to_string())),
                None,
            ),
            "microsoft" => (None, Some(client_id.to_string())),
            _ => return,
        };
        save_oauth_settings(
            &app,
            &single_state,
            &single_runtime,
            google,
            microsoft,
            false,
        );
    });

    let weak = app.as_weak();
    let state = Rc::clone(state);
    let runtime = Rc::clone(runtime);
    app.on_save_oauth_apps(move |google_id, google_secret, microsoft_id| {
        let Some(app) = weak.upgrade() else { return };
        save_oauth_settings(
            &app,
            &state,
            &runtime,
            Some((google_id.to_string(), google_secret.to_string())),
            Some(microsoft_id.to_string()),
            true,
        );
    });
}

fn save_oauth_settings(
    app: &AppWindow,
    state: &Rc<RefCell<InboxState>>,
    runtime: &tokio::runtime::Runtime,
    google: Option<(String, String)>,
    microsoft: Option<String>,
    close_dialog: bool,
) {
    if app.get_oauth_settings_saving() || app.get_oauth_in_progress() {
        return;
    }
    let Some(core) = state.borrow().core.clone() else {
        let message = UiMessage::plain("Local mail data is unavailable. Retry startup.");
        app.set_sync_status(message.clone());
        app.set_oauth_settings_error(ui_message::translated(app, &message));
        return;
    };
    let update_google = google.is_some();
    let update_microsoft = microsoft.is_some();
    app.set_oauth_settings_error("".into());
    app.set_oauth_settings_saving(true);
    let weak = app.as_weak();
    runtime.spawn(async move {
        let result = core.set_oauth_apps(google, microsoft).await;
        let _ = weak.upgrade_in_event_loop(move |app| {
            app.set_oauth_settings_saving(false);
            match result {
                Ok(settings) => {
                    if update_google {
                        app.set_google_client_id(settings.google_client_id.clone().into());
                        app.set_google_client_secret(settings.google_client_secret.clone().into());
                    }
                    if update_microsoft {
                        app.set_ms_client_id(settings.ms_client_id.clone().into());
                    }
                    app.set_custom_oauth_configured(
                        !settings.google_client_id.is_empty() || !settings.ms_client_id.is_empty(),
                    );
                    startup::refresh_oauth_availability(&app);
                    app.set_sync_status(UiMessage::plain("OAuth app keys saved."));
                    if close_dialog {
                        app.set_oauth_setup_open(false);
                    }
                }
                Err(error) => {
                    let message = UiMessage::detail("OAuth settings failed: {}", error);
                    app.set_sync_status(message.clone());
                    app.set_oauth_settings_error(ui_message::translated(&app, &message));
                }
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hiding_settings_discards_an_unsaved_palette() {
        use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
        struct Headless(Rc<MinimalSoftwareWindow>);
        impl slint::platform::Platform for Headless {
            fn create_window_adapter(
                &self,
            ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
                Ok(self.0.clone())
            }
        }
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(Headless(window))).unwrap();
        let app = AppWindow::new().unwrap();
        let theme = app.global::<AppTheme>();
        let draft = app.global::<ThemeDraft>();
        let saved = theme.get_custom_light_primary();
        app.set_settings_open(true);
        slint::platform::update_timers_and_animations();

        // The editor previews a color before it is saved.
        draft.invoke_begin();
        theme.set_custom_light_primary(slint::Color::from_rgb_u8(0xff, 0, 0));
        app.set_settings_open(false);
        slint::platform::update_timers_and_animations();
        assert_eq!(theme.get_custom_light_primary(), saved);
        assert!(!draft.get_dirty());

        // A saved palette stays.
        app.set_settings_open(true);
        slint::platform::update_timers_and_animations();
        draft.invoke_begin();
        let chosen = slint::Color::from_rgb_u8(0, 0x80, 0);
        theme.set_custom_light_primary(chosen);
        draft.set_dirty(false);
        app.set_settings_open(false);
        slint::platform::update_timers_and_animations();
        assert_eq!(theme.get_custom_light_primary(), chosen);
    }
}
