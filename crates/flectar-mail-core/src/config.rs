use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Paths {
    /// Durable application state. Databases, raw mail, drafts, queues, and
    /// settings live here and must survive process death and cache eviction.
    pub data_dir: PathBuf,
    /// Reconstructable data such as downloaded attachment bodies and preview
    /// scratch files. Hosts may place this in an OS-managed cache directory.
    pub cache_dir: PathBuf,
}

/// The application's directory name under the platform's data and cache
/// directories.
#[cfg(desktop)]
const DIR_NAME: &str = "evanya";
/// The name before the app was renamed from Flectar Mail to Evanya.
#[cfg(desktop)]
const LEGACY_DIR_NAME: &str = "flectar-mail";

/// `root/name`, moving the directory of an earlier name there first. When the
/// move fails the earlier directory is used as it is, so existing mail,
/// accounts and settings are never left behind.
#[cfg(desktop)]
fn renamed_dir(root: &std::path::Path, legacy: &str, name: &str) -> PathBuf {
    let current = root.join(name);
    let earlier = root.join(legacy);
    if current.exists() || !earlier.is_dir() {
        return current;
    }
    match std::fs::rename(&earlier, &current) {
        Ok(()) => {
            tracing::info!(from = %earlier.display(), to = %current.display(), "moved the app directory to its new name");
            current
        }
        Err(error) => {
            tracing::warn!(%error, directory = %earlier.display(), "could not rename the app directory; using it as it is");
            earlier
        }
    }
}

impl Paths {
    /// Resolve desktop directories. Mobile hosts inject their sandbox paths
    /// through `Paths::new`; silently using the process working directory is
    /// never safe there.
    #[cfg(desktop)]
    pub fn default_dirs() -> crate::error::Result<Self> {
        let data_root = dirs::data_dir().ok_or_else(|| {
            crate::error::CoreError::Other(
                "the operating system has no application data directory".into(),
            )
        })?;
        let cache_root = dirs::cache_dir().ok_or_else(|| {
            crate::error::CoreError::Other(
                "the operating system has no application cache directory".into(),
            )
        })?;
        Ok(Self::new(
            renamed_dir(&data_root, LEGACY_DIR_NAME, DIR_NAME),
            renamed_dir(&cache_root, LEGACY_DIR_NAME, DIR_NAME),
        ))
    }

    pub fn new(data_dir: PathBuf, cache_dir: PathBuf) -> Self {
        Self {
            data_dir,
            cache_dir,
        }
    }

    pub fn for_tests(root: &std::path::Path) -> Self {
        Self::new(root.to_path_buf(), root.join("cache"))
    }

    pub fn db_file(&self) -> PathBuf {
        self.data_dir.join("flectar-mail.db")
    }

    /// Calendar data lives in its own SQLite database so large mailbox writes,
    /// FTS maintenance, and mail backfills never contend with calendar reads or
    /// sync bookkeeping.
    pub fn calendar_db_file(&self) -> PathBuf {
        self.data_dir.join("flectar-calendar.db")
    }

    pub fn files_db_file(&self) -> PathBuf {
        self.data_dir.join("flectar-files.db")
    }
    pub fn files_cache_dir(&self, account_id: i64) -> PathBuf {
        self.cache_dir.join("files").join(account_id.to_string())
    }
    pub fn files_staging_dir(&self, account_id: i64) -> PathBuf {
        self.data_dir
            .join("file_transfers")
            .join(account_id.to_string())
    }

    /// Small, reconstructable projection of the last visible mailbox page.
    /// The native shell may paint this before the databases finish opening;
    /// SQLite remains authoritative and replaces it during normal startup.
    pub fn warm_start_file(&self) -> PathBuf {
        self.cache_dir.join("warm-start-mailbox.json")
    }

    /// Superseded reconstructable caches that can be removed without reading.
    ///
    /// TODO(warm-start-v2-cleanup): After the first public release using the
    /// stable `warm-start-mailbox.json` filename has been superseded by a later
    /// release, remove this method, its startup cleanup loop, and the matching
    /// path test. Keeping it longer is harmless; removing it sooner only leaves
    /// the old, unread v2 cache orphaned on direct upgrades from older builds.
    pub fn obsolete_warm_start_files(&self) -> [PathBuf; 1] {
        [self.cache_dir.join("warm-start-mailbox-v2.json")]
    }

    /// Directory holding every account's [`Self::mail_dir`].
    pub fn mail_root(&self) -> PathBuf {
        self.data_dir.join("mail")
    }

    /// Directory for raw .eml files of one account.
    pub fn mail_dir(&self, account_id: i64) -> PathBuf {
        self.mail_root().join(account_id.to_string())
    }

    /// Directory for extracted attachment files.
    pub fn attachments_dir(&self, account_id: i64) -> PathBuf {
        self.cache_dir
            .join("attachments")
            .join(account_id.to_string())
    }

    /// Root for app-managed copies of outgoing (draft) attachments. Files
    /// picked in the composer are staged here so dispatch only ever reads
    /// files the app itself wrote, never an arbitrary caller-supplied path.
    pub fn draft_attachments_dir(&self) -> PathBuf {
        self.data_dir.join("draft_attachments")
    }

    /// Directory holding local embedding-model weights (`<models_dir>/<key>`).
    pub fn models_dir(&self) -> PathBuf {
        self.cache_dir.join("models")
    }

    pub fn temp_dir(&self) -> PathBuf {
        self.cache_dir.join("tmp")
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        for directory in [
            &self.data_dir,
            &self.cache_dir,
            &self.mail_root(),
            &self.draft_attachments_dir(),
            &self.temp_dir(),
        ] {
            std::fs::create_dir_all(directory)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.data_dir, std::fs::Permissions::from_mode(0o700))?;
            std::fs::set_permissions(&self.cache_dir, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_paths_separate_durable_and_reconstructable_state() {
        let root = std::path::Path::new("/private/app");
        let paths = Paths::new(root.join("data"), root.join("cache"));

        assert_eq!(paths.db_file(), root.join("data/flectar-mail.db"));
        assert_eq!(
            paths.calendar_db_file(),
            root.join("data/flectar-calendar.db")
        );
        assert_eq!(
            paths.warm_start_file(),
            root.join("cache/warm-start-mailbox.json")
        );
        assert_eq!(
            paths.obsolete_warm_start_files(),
            [root.join("cache/warm-start-mailbox-v2.json")]
        );
        assert_eq!(paths.mail_dir(7), root.join("data/mail/7"));
        assert_eq!(
            paths.draft_attachments_dir(),
            root.join("data/draft_attachments")
        );
        assert_eq!(paths.attachments_dir(7), root.join("cache/attachments/7"));
        assert_eq!(paths.temp_dir(), root.join("cache/tmp"));
    }
}

#[cfg(all(test, desktop))]
mod rename_tests {
    use super::renamed_dir;

    #[test]
    fn the_earlier_directory_moves_once_and_is_never_lost() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("old")).unwrap();
        std::fs::write(root.path().join("old/mail.db"), b"mail").unwrap();
        let moved = renamed_dir(root.path(), "old", "new");
        assert_eq!(moved, root.path().join("new"));
        assert_eq!(std::fs::read(moved.join("mail.db")).unwrap(), b"mail");
        assert!(!root.path().join("old").exists());
        // Later runs use the new directory, even when the old name returns.
        std::fs::create_dir(root.path().join("old")).unwrap();
        assert_eq!(
            renamed_dir(root.path(), "old", "new"),
            root.path().join("new")
        );
        // Without either, the new name is used.
        let fresh = tempfile::tempdir().unwrap();
        assert_eq!(
            renamed_dir(fresh.path(), "old", "new"),
            fresh.path().join("new")
        );
    }
}
