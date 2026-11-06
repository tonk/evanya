//! "Move to folder…": the folders a message can move to, for the picker.

use std::{cell::RefCell, rc::Rc};

use slint::{ComponentHandle, ModelRc, VecModel};

use crate::{AppWindow, InboxState, MoveTarget};

/// Every account and its folders, in sidebar order; with a filter, the
/// folders whose name or account holds it, under their account.
fn targets(state: &InboxState, filter: &str) -> Vec<MoveTarget> {
    let filter = filter.trim().to_lowercase();
    // A folder's full path, such as "Algemeen › Vakantie › Bilbao", from its
    // parents.
    let parents: std::collections::HashMap<(i64, i64), (&str, i64)> = state
        .mailboxes
        .iter()
        .filter(|mailbox| !mailbox.is_account && mailbox.folder_id >= 0)
        .map(|mailbox| {
            (
                (mailbox.account_id, mailbox.folder_id),
                (mailbox.label.as_str(), mailbox.parent_folder_id),
            )
        })
        .collect();
    let path = |account_id: i64, folder_id: i64| {
        let mut names = Vec::new();
        let mut next = folder_id;
        while let Some((label, parent)) = parents.get(&(account_id, next)) {
            names.push(*label);
            if names.len() > 32 {
                break;
            }
            next = *parent;
        }
        names.reverse();
        names.join(" › ")
    };
    let all: Vec<MoveTarget> = state
        .mailboxes
        .iter()
        .filter(|mailbox| mailbox.is_account || mailbox.folder_id >= 0)
        // As in the sidebar, mail is not moved into Sent or Drafts.
        .filter(|mailbox| {
            !(mailbox.is_standard && matches!(mailbox.label.as_str(), "Sent" | "Drafts"))
        })
        .filter_map(|mailbox| {
            let scope = mailbox
                .scope
                .map(|scope| scope.to_string())
                .unwrap_or_default();
            if !mailbox.is_account && scope.is_empty() {
                return None;
            }
            let account = crate::mail::account_name(mailbox.account_id)
                .unwrap_or_else(|| mailbox.context.clone());
            Some(MoveTarget {
                header: mailbox.is_account,
                label: if mailbox.is_account {
                    account.clone().into()
                } else {
                    mailbox.label.clone().into()
                },
                // Searching finds a folder by its path and its account; a
                // match then shows the whole path.
                search: format!(
                    "{} › {account}",
                    path(mailbox.account_id, mailbox.folder_id)
                )
                .to_lowercase()
                .into(),
                path: path(mailbox.account_id, mailbox.folder_id).into(),
                scope: scope.into(),
                account_id: i32::try_from(mailbox.account_id).unwrap_or(-1),
                folder_id: i32::try_from(mailbox.folder_id).unwrap_or(-1),
                depth: i32::try_from(mailbox.depth).unwrap_or(0),
            })
        })
        .collect();
    filtered(all, &filter)
}

/// The folders whose path or account holds `filter`, each under its account
/// and showing its whole path.
fn filtered(all: Vec<MoveTarget>, filter: &str) -> Vec<MoveTarget> {
    if filter.is_empty() {
        return all;
    }
    let mut shown = Vec::new();
    let mut heading: Option<MoveTarget> = None;
    for target in all {
        if target.header {
            heading = Some(target);
        } else if target.search.contains(filter) {
            if let Some(heading) = heading.take() {
                shown.push(heading);
            }
            shown.push(MoveTarget {
                depth: 0,
                label: target.path.clone(),
                ..target
            });
        }
    }
    shown
}

pub(crate) fn register(app: &AppWindow, state: &Rc<RefCell<InboxState>>) {
    let app_weak = app.as_weak();
    let state = Rc::clone(state);
    app.on_prepare_move_targets(move |filter| {
        if let Some(app) = app_weak.upgrade() {
            app.set_move_targets(ModelRc::new(VecModel::from(targets(
                &state.borrow(),
                &filter,
            ))));
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_filter_keeps_matching_folders_under_their_account() {
        use super::MoveTarget;
        let folder = |label: &str, path: &str| MoveTarget {
            label: label.into(),
            path: path.into(),
            search: format!("{path} › thuis").to_lowercase().into(),
            depth: 2,
            folder_id: 1,
            ..Default::default()
        };
        // The filtering step on its own, as targets() applies it.
        let all = vec![
            MoveTarget {
                header: true,
                label: "Thuis".into(),
                ..Default::default()
            },
            folder("Bilbao", "Algemeen › Vakantie › Bilbao"),
            folder("Tuin", "Algemeen › Tuin"),
        ];
        let shown = super::filtered(all, "vakantie");
        let labels: Vec<_> = shown
            .iter()
            .map(|target| target.label.to_string())
            .collect();
        assert_eq!(labels, ["Thuis", "Algemeen › Vakantie › Bilbao"]);
        assert_eq!(shown[1].depth, 0);
    }
}
