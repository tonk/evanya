//! The status line along the bottom of the window: per account, what mail
//! synchronization is doing right now.

use std::collections::HashMap;

use evanya_core::models::SyncStatus;
use slint::{ModelRc, SharedString, VecModel};

use crate::{AppWindow, InboxState, SyncStatusLine};

/// The latest sync snapshot and working folder of every account.
#[derive(Default)]
pub(crate) struct StatusLine {
    statuses: HashMap<i64, SyncStatus>,
    folders: HashMap<i64, String>,
}

impl StatusLine {
    /// Take newer snapshots and folders. Returns whether anything changed.
    pub(crate) fn merge(
        &mut self,
        statuses: HashMap<i64, SyncStatus>,
        folders: HashMap<i64, String>,
    ) -> bool {
        let changed = !statuses.is_empty() || !folders.is_empty();
        self.statuses.extend(statuses);
        self.folders.extend(folders);
        changed
    }

    /// Accounts with something to report, in the sidebar's account order.
    pub(crate) fn rows(&self, state: &InboxState) -> Vec<SyncStatusLine> {
        state
            .connected_accounts
            .iter()
            .filter_map(|account| {
                let status = self.statuses.get(&account.id)?;
                let fallback = account.display_name.as_deref().unwrap_or(&account.email);
                let busy = status.state == "syncing" || status.background.is_some();
                let trouble = !matches!(status.state.as_str(), "idle" | "syncing");
                if !busy && !trouble {
                    return None;
                }
                let background = status.background.as_ref();
                Some(SyncStatusLine {
                    account: crate::mail::account_name(account.id)
                        .unwrap_or_else(|| fallback.to_owned())
                        .into(),
                    state: status.state.as_str().into(),
                    phase: background
                        .map_or(status.foreground_phase.as_str(), |progress| {
                            progress.phase.as_str()
                        })
                        .into(),
                    folder: self
                        .folders
                        .get(&account.id)
                        .filter(|_| busy && !trouble)
                        .map_or_else(SharedString::default, |folder| folder.as_str().into()),
                    done: background.map_or(0, |progress| saturating_i32(progress.done)),
                    total: background.map_or(0, |progress| saturating_i32(progress.total)),
                    failed: background.map_or(0, |progress| saturating_i32(progress.failed)),
                    error: account.sync_error.clone().unwrap_or_default().into(),
                })
            })
            .collect()
    }

    pub(crate) fn apply(&self, app: &AppWindow, state: &InboxState) {
        app.set_sync_status_lines(ModelRc::new(VecModel::from(self.rows(state))));
    }
}

fn saturating_i32(value: u64) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}
