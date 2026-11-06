//! Settings → Accounts → Mail headers: header fields added to outgoing mail,
//! for all accounts or one. The text holds one `Name: value` per line.

use std::{cell::RefCell, rc::Rc};

use evanya_core::models::{CustomHeader, custom_header_problem};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::{
    AppWindow, InboxState,
    ui_message::{UiMessage, translated},
};

/// The headers as the text field shows them.
fn to_text(headers: &[CustomHeader]) -> String {
    headers
        .iter()
        .map(|header| format!("{}: {}", header.name, header.value))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Read the text field: each non-empty line is `Name: value`. On a line that
/// cannot be sent, returns its number and why.
fn parse(text: &str) -> Result<Vec<CustomHeader>, (usize, &'static str)> {
    let mut headers = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err((index + 1, "a header is written as Name: value"));
        };
        let (name, value) = (name.trim(), value.trim());
        if let Some(problem) = custom_header_problem(name, value) {
            return Err((index + 1, problem));
        }
        headers.push(CustomHeader {
            name: name.to_owned(),
            value: value.to_owned(),
        });
    }
    Ok(headers)
}

/// The translatable message for a reason a header cannot be sent.
fn problem_message(problem: &str) -> UiMessage {
    match problem {
        "a header needs a name" => UiMessage::plain("a header needs a name"),
        "a header name has only letters, digits and punctuation other than ':'" => {
            UiMessage::plain(
                "a header name has only letters, digits and punctuation other than ':'",
            )
        }
        "this header is written by Evanya itself" => {
            UiMessage::plain("this header is written by Evanya itself")
        }
        "a header value is one line" => UiMessage::plain("a header value is one line"),
        "a header is limited to 900 characters" => {
            UiMessage::plain("a header is limited to 900 characters")
        }
        _ => UiMessage::plain("a header is written as Name: value"),
    }
}

/// The account a choice in "Applies to" names: none for all accounts.
fn account_at(state: &InboxState, index: i32) -> Option<i64> {
    let index = usize::try_from(index).ok()?.checked_sub(1)?;
    state
        .connected_accounts
        .get(index)
        .map(|account| account.id)
}

/// Show the headers for the chosen scope.
fn show(app: &AppWindow, state: &Rc<RefCell<InboxState>>, runtime: &tokio::runtime::Runtime) {
    let index = app.get_custom_headers_scope_index();
    let state = state.borrow();
    let headers = match account_at(&state, index) {
        Some(account_id) => state
            .account_configs
            .iter()
            .find(|config| config.id == account_id)
            .map(|config| config.settings.custom_headers.clone())
            .unwrap_or_default(),
        None => state
            .core
            .as_ref()
            .and_then(|core| runtime.block_on(core.load_settings()).ok())
            .map(|settings| settings.custom_headers)
            .unwrap_or_default(),
    };
    app.set_custom_headers_text(to_text(&headers).into());
    app.set_custom_headers_error("".into());
}

/// Offer "All accounts" and every account under "Applies to".
pub(crate) fn refresh_scopes(app: &AppWindow, state: &InboxState) {
    let mut scopes: Vec<SharedString> = vec![translated(app, &UiMessage::plain("All accounts"))];
    scopes.extend(state.connected_accounts.iter().map(|account| {
        let name = crate::mail::account_name(account.id)
            .or_else(|| account.display_name.clone())
            .unwrap_or_else(|| account.email.clone());
        SharedString::from(format!("{name} ({})", account.email))
    }));
    if app.get_custom_headers_scope_index() as usize >= scopes.len() {
        app.set_custom_headers_scope_index(0);
    }
    app.set_custom_headers_scopes(ModelRc::new(VecModel::from(scopes)));
}

pub(crate) fn register(
    app: &AppWindow,
    state: &Rc<RefCell<InboxState>>,
    runtime: &Rc<tokio::runtime::Runtime>,
) {
    let app_weak = app.as_weak();
    let state_for_select = Rc::clone(state);
    let runtime_for_select = Rc::clone(runtime);
    app.on_select_custom_headers_scope(move |index| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        app.set_custom_headers_scope_index(index);
        show(&app, &state_for_select, &runtime_for_select);
    });

    let app_weak = app.as_weak();
    let state_for_save = Rc::clone(state);
    let runtime_for_save = Rc::clone(runtime);
    app.on_save_custom_headers(move |index, text| {
        let Some(app) = app_weak.upgrade() else {
            return;
        };
        let headers = match parse(&text) {
            Ok(headers) => headers,
            Err((line, problem)) => {
                let line = translated(&app, &UiMessage::detail("Line {}", line.to_string()));
                let problem = translated(&app, &problem_message(problem));
                app.set_custom_headers_error(format!("{line}: {problem}").into());
                return;
            }
        };
        let (core, account_id) = {
            let state = state_for_save.borrow();
            (state.core.clone(), account_at(&state, index))
        };
        let Some(core) = core else {
            return;
        };
        let saved = match account_id {
            Some(account_id) => {
                let result = runtime_for_save
                    .block_on(core.set_account_custom_headers(account_id, headers.clone()));
                if result.is_ok() {
                    // Keep the shown accounts in step without a reload.
                    let mut state = state_for_save.borrow_mut();
                    if let Some(config) = state
                        .account_configs
                        .iter_mut()
                        .find(|config| config.id == account_id)
                    {
                        config.settings.custom_headers = headers;
                    }
                }
                result
            }
            None => runtime_for_save
                .block_on(core.update_settings(move |settings| settings.custom_headers = headers))
                .map(|_| ()),
        };
        match saved {
            Ok(()) => {
                app.set_custom_headers_error("".into());
                show(&app, &state_for_save, &runtime_for_save);
                app.set_custom_headers_saved(true);
            }
            Err(error) => app.set_custom_headers_error(error.into()),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_become_headers_and_bad_lines_are_named() {
        let headers = parse("Organization: Kersten & Co\n\n  X-Mailer :Evanya  \n").unwrap();
        assert_eq!(
            to_text(&headers),
            "Organization: Kersten & Co\nX-Mailer: Evanya"
        );
        assert_eq!(parse("X-Ok: 1\nno colon").unwrap_err().0, 2);
        assert_eq!(parse("Subject: hi").unwrap_err().0, 1);
        assert!(parse("").unwrap().is_empty());
    }
}
